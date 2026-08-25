use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::header::{HeaderValue, AUTHORIZATION};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Error as WebSocketError, Message};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::audio::{
    start_capture, start_playback, system_capture_output_match, AudioSource, CaptureHandle,
    PlaybackHandle,
};
use crate::secret_store::SecretStore;

const TRANSCRIPTION_ENDPOINT: &str = "wss://api.openai.com/v1/realtime";
const TRANSLATION_ENDPOINT: &str =
    "wss://api.openai.com/v1/realtime/translations?model=gpt-realtime-translate";
const TRANSCRIPTION_MODEL: &str = "gpt-live-transcribe";
const REALTIME_EVENT: &str = "realtime:event";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const SETUP_EVENT_TIMEOUT: Duration = Duration::from_secs(10);
const GRACEFUL_CLOSE_TIMEOUT: Duration = Duration::from_secs(3);
const WORKER_STOP_TIMEOUT: Duration = Duration::from_secs(5);
const PLAYBACK_CHANGE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_WEBSOCKET_MESSAGE_BYTES: usize = 4 * 1024 * 1024;
const MAX_AUDIO_SAMPLES_PER_CHUNK: usize = 48_000;
const MAX_OUTPUT_AUDIO_BYTES: usize = 2 * 1024 * 1024;
const MAX_TRANSCRIPT_DELTA_BYTES: usize = 64 * 1024;
const MAX_TRANSCRIPT_BYTES: usize = 256 * 1024;
const MAX_DEVICE_ID_BYTES: usize = 2 * 1024;
const MAX_PROVIDER_MESSAGE_BYTES: usize = 320;
const LEVEL_EVENT_INTERVAL: Duration = Duration::from_millis(50);

type RealtimeSocket = WebSocketStream<MaybeTlsStream<TcpStream>>;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RealtimeChannel {
    Microphone,
    System,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RealtimeMode {
    Transcribe,
    Translate,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RealtimeStartRequest {
    pub channel: RealtimeChannel,
    pub mode: RealtimeMode,
    pub playback_enabled: bool,
    pub input_device_id: Option<String>,
    pub output_device_id: Option<String>,
    pub target_language: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum RealtimeEventKind {
    Status,
    InputTranscriptDelta,
    InputTranscriptCompleted,
    OutputTranscriptDelta,
    OutputTranscriptCompleted,
    Level,
    Error,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RealtimeEventPayload {
    channel: RealtimeChannel,
    kind: RealtimeEventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    segment_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delta: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    status: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    level: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

#[derive(Clone, Debug)]
struct NormalizedRequest {
    channel: RealtimeChannel,
    mode: RealtimeMode,
    playback_enabled: bool,
    input_device_id: Option<String>,
    output_device_id: Option<String>,
    target_language: Option<String>,
}

#[derive(Clone, Debug)]
struct ReservedRoute {
    generation: u64,
    mode: RealtimeMode,
    playback_enabled: bool,
    input_device_id: Option<String>,
    output_device_id: Option<String>,
}

struct ActiveSession {
    generation: u64,
    commands: mpsc::Sender<SessionCommand>,
}

enum SessionCommand {
    Stop(oneshot::Sender<()>),
    SetPlaybackEnabled {
        enabled: bool,
        result: oneshot::Sender<Result<(), String>>,
    },
}

#[derive(Default)]
struct ChannelSlot {
    generation: AtomicU64,
    active: Mutex<Option<ActiveSession>>,
}

#[derive(Default)]
pub struct RealtimeManager {
    microphone: ChannelSlot,
    system: ChannelSlot,
    routes: Mutex<HashMap<RealtimeChannel, ReservedRoute>>,
}

impl RealtimeManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn start(
        self: &Arc<Self>,
        app: AppHandle,
        secrets: Arc<SecretStore>,
        request: RealtimeStartRequest,
    ) -> Result<(), String> {
        let request = normalize_request(request)?;
        let slot = self.slot(request.channel);
        let generation = slot
            .generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
            .max(1);
        emit_status(&app, request.channel, "starting");

        if let Err(error) = self.reserve_route(generation, &request).await {
            // Generation was advanced before the async route check so Stop can
            // always supersede setup. A rejected replacement must therefore
            // also stop the now-stale live session instead of leaving capture
            // running behind an error state.
            self.routes.lock().await.remove(&request.channel);
            self.stop_active(request.channel).await;
            emit_error(&app, request.channel, &error);
            return Err(error);
        }

        let result = self
            .start_generation(app.clone(), secrets, request.clone(), generation)
            .await;
        if let Err(error) = result {
            self.clear_route_if_current(request.channel, generation)
                .await;
            if slot.generation.load(Ordering::Acquire) != generation {
                // Stop or a newer Start intentionally superseded this setup.
                return Ok(());
            }
            emit_error(&app, request.channel, &error);
            return Err(error);
        }
        Ok(())
    }

    async fn start_generation(
        self: &Arc<Self>,
        app: AppHandle,
        secrets: Arc<SecretStore>,
        request: NormalizedRequest,
        generation: u64,
    ) -> Result<(), String> {
        self.stop_active_older_than(request.channel, generation)
            .await;
        if !self.is_current(request.channel, generation) {
            return Err("Realtime setup was superseded".to_string());
        }

        let api_key = secrets
            .get_openai_api_key()
            .await?
            .ok_or_else(|| "OpenAI API key is not configured".to_string())?;
        if !self.is_current(request.channel, generation) {
            return Err("Realtime setup was superseded".to_string());
        }

        let mut socket = connect_and_configure(
            self.slot(request.channel),
            generation,
            request.channel,
            request.mode,
            request.target_language.as_deref(),
            &api_key,
        )
        .await?;
        drop(api_key);

        let mut playback = if request.playback_enabled {
            Some(start_playback(request.output_device_id.as_deref())?)
        } else {
            None
        };
        if !self.is_current(request.channel, generation) {
            stop_playback(playback.take()).await;
            let _ = socket.close(None).await;
            return Err("Realtime setup was superseded".to_string());
        }

        let (audio_tx, audio_rx) = mpsc::channel(64);
        let (level_tx, level_rx) = mpsc::unbounded_channel();
        let source = match request.channel {
            RealtimeChannel::Microphone => AudioSource::Microphone,
            RealtimeChannel::System => AudioSource::System,
        };
        let mut capture = match start_capture(
            source,
            request.input_device_id.as_deref(),
            audio_tx,
            level_tx,
        ) {
            Ok(capture) => Some(capture),
            Err(error) => {
                stop_playback(playback.take()).await;
                let _ = socket.close(None).await;
                return Err(error);
            }
        };
        if !self.is_current(request.channel, generation) {
            stop_capture_handle(capture.take()).await;
            stop_playback(playback.take()).await;
            let _ = socket.close(None).await;
            return Err("Realtime setup was superseded".to_string());
        }

        let (commands, command_rx) = mpsc::channel(4);
        let (begin_tx, begin_rx) = oneshot::channel();
        let manager = self.clone();
        let worker_app = app.clone();
        let channel = request.channel;
        let mode = request.mode;
        let output_device_id = request.output_device_id.clone();
        tauri::async_runtime::spawn(async move {
            if begin_rx.await.is_err() {
                stop_capture_handle(capture.take()).await;
                stop_playback(playback.take()).await;
                let _ = socket.close(None).await;
                return;
            }
            run_session(
                worker_app,
                manager,
                channel,
                mode,
                generation,
                socket,
                capture,
                playback,
                output_device_id,
                audio_rx,
                level_rx,
                command_rx,
            )
            .await;
        });

        {
            let slot = self.slot(channel);
            let mut active = slot.active.lock().await;
            if slot.generation.load(Ordering::Acquire) != generation {
                drop(active);
                drop(begin_tx);
                return Err("Realtime setup was superseded".to_string());
            }
            *active = Some(ActiveSession {
                generation,
                commands,
            });
        }
        let _ = begin_tx.send(());
        emit_status_for_generation(self, &app, channel, generation, "listening");
        Ok(())
    }

    pub async fn stop(&self, app: &AppHandle, channel: RealtimeChannel) {
        let slot = self.slot(channel);
        slot.generation.fetch_add(1, Ordering::AcqRel);
        self.routes.lock().await.remove(&channel);
        emit_status(app, channel, "stopping");
        self.stop_active(channel).await;
        emit_status(app, channel, "idle");
    }

    pub async fn stop_all(&self, app: &AppHandle) {
        let microphone = self.stop(app, RealtimeChannel::Microphone);
        let system = self.stop(app, RealtimeChannel::System);
        tokio::join!(microphone, system);
    }

    pub async fn set_playback_enabled(
        &self,
        channel: RealtimeChannel,
        enabled: bool,
    ) -> Result<(), String> {
        let (generation, commands) = {
            let active = self.slot(channel).active.lock().await;
            let active = active
                .as_ref()
                .ok_or_else(|| "Realtime channel is not active".to_string())?;
            (active.generation, active.commands.clone())
        };

        let previous_enabled = {
            let mut routes = self.routes.lock().await;
            let current = routes
                .get(&channel)
                .filter(|route| route.generation == generation)
                .cloned()
                .ok_or_else(|| "Realtime channel route is not active".to_string())?;
            if current.mode != RealtimeMode::Translate {
                return Err("Speech playback is only available for translation".to_string());
            }
            if current.playback_enabled == enabled {
                return Ok(());
            }
            let mut candidate = current.clone();
            candidate.playback_enabled = enabled;
            validate_feedback_route(channel, &candidate, &routes)?;
            routes.insert(channel, candidate);
            current.playback_enabled
        };

        let (result_tx, result_rx) = oneshot::channel();
        if commands
            .send(SessionCommand::SetPlaybackEnabled {
                enabled,
                result: result_tx,
            })
            .await
            .is_err()
        {
            self.restore_playback_route(channel, generation, previous_enabled)
                .await;
            return Err("Realtime channel stopped before playback changed".to_string());
        }
        let result = match tokio::time::timeout(PLAYBACK_CHANGE_TIMEOUT, result_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Realtime channel stopped before playback changed".to_string()),
            Err(_) => Err("Timed out while changing speech playback".to_string()),
        };
        if result.is_err() {
            self.restore_playback_route(channel, generation, previous_enabled)
                .await;
        }
        result
    }

    async fn restore_playback_route(
        &self,
        channel: RealtimeChannel,
        generation: u64,
        enabled: bool,
    ) {
        let mut routes = self.routes.lock().await;
        if let Some(route) = routes
            .get_mut(&channel)
            .filter(|route| route.generation == generation)
        {
            route.playback_enabled = enabled;
        }
    }

    fn slot(&self, channel: RealtimeChannel) -> &ChannelSlot {
        match channel {
            RealtimeChannel::Microphone => &self.microphone,
            RealtimeChannel::System => &self.system,
        }
    }

    fn is_current(&self, channel: RealtimeChannel, generation: u64) -> bool {
        self.slot(channel).generation.load(Ordering::Acquire) == generation
    }

    async fn stop_active_older_than(&self, channel: RealtimeChannel, generation: u64) {
        let active = {
            let mut active = self.slot(channel).active.lock().await;
            if active
                .as_ref()
                .is_some_and(|active| active.generation < generation)
            {
                active.take()
            } else {
                None
            }
        };
        stop_active_session(active).await;
    }

    async fn stop_active(&self, channel: RealtimeChannel) {
        let active = self.slot(channel).active.lock().await.take();
        stop_active_session(active).await;
    }

    async fn reserve_route(
        &self,
        generation: u64,
        request: &NormalizedRequest,
    ) -> Result<(), String> {
        let candidate = ReservedRoute {
            generation,
            mode: request.mode,
            playback_enabled: request.playback_enabled,
            input_device_id: request.input_device_id.clone(),
            output_device_id: request.output_device_id.clone(),
        };
        let mut routes = self.routes.lock().await;
        validate_feedback_route(request.channel, &candidate, &routes)?;
        routes.insert(request.channel, candidate);
        Ok(())
    }

    async fn clear_route_if_current(&self, channel: RealtimeChannel, generation: u64) {
        let mut routes = self.routes.lock().await;
        if routes
            .get(&channel)
            .is_some_and(|route| route.generation == generation)
        {
            routes.remove(&channel);
        }
    }

    async fn finish_session(&self, channel: RealtimeChannel, generation: u64) {
        let mut active = self.slot(channel).active.lock().await;
        if active
            .as_ref()
            .is_some_and(|active| active.generation == generation)
        {
            active.take();
        }
        drop(active);
        self.clear_route_if_current(channel, generation).await;
    }
}

async fn stop_active_session(active: Option<ActiveSession>) {
    let Some(active) = active else { return };
    let (finished_tx, finished_rx) = oneshot::channel();
    if active
        .commands
        .send(SessionCommand::Stop(finished_tx))
        .await
        .is_ok()
    {
        let _ = tokio::time::timeout(WORKER_STOP_TIMEOUT, finished_rx).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_session(
    app: AppHandle,
    manager: Arc<RealtimeManager>,
    channel: RealtimeChannel,
    mode: RealtimeMode,
    generation: u64,
    mut socket: RealtimeSocket,
    mut capture: Option<CaptureHandle>,
    mut playback: Option<PlaybackHandle>,
    output_device_id: Option<String>,
    mut audio_rx: mpsc::Receiver<Vec<i16>>,
    mut level_rx: mpsc::UnboundedReceiver<f32>,
    mut commands: mpsc::Receiver<SessionCommand>,
) {
    let mut segments = SegmentTracker::new(generation, mode == RealtimeMode::Translate);
    let mut last_level_event = Instant::now() - LEVEL_EVENT_INTERVAL;
    let mut failed = None;
    let mut stopped_by_command = false;

    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(SessionCommand::Stop(waiter)) => {
                        stopped_by_command = true;
                        emit_status(&app, channel, "stopping");
                        stop_capture_handle(capture.take()).await;
                        graceful_close(
                            &app,
                            channel,
                            mode,
                            &mut socket,
                            playback.as_ref(),
                            &mut segments,
                        ).await;
                        stop_playback(playback.take()).await;
                        let _ = waiter.send(());
                        break;
                    }
                    Some(SessionCommand::SetPlaybackEnabled { enabled, result }) => {
                        let change = if mode != RealtimeMode::Translate {
                            Err("Speech playback is only available for translation".to_string())
                        } else if enabled && playback.is_none() {
                            match start_playback_async(output_device_id.clone()).await {
                                Ok(handle) => {
                                    playback = Some(handle);
                                    Ok(())
                                }
                                Err(error) => Err(error),
                            }
                        } else if !enabled && playback.is_some() {
                            stop_playback(playback.take()).await;
                            Ok(())
                        } else {
                            Ok(())
                        };
                        let _ = result.send(change);
                    }
                    None => {
                        failed = Some("Realtime controller stopped unexpectedly".to_string());
                        break;
                    }
                }
            }
            audio = audio_rx.recv() => {
                let Some(samples) = audio else {
                    failed = Some("Audio capture stopped unexpectedly".to_string());
                    break;
                };
                if let Err(error) = send_audio(&mut socket, mode, &samples).await {
                    failed = Some(error);
                    break;
                }
            }
            level = level_rx.recv() => {
                if let Some(level) = level {
                    if last_level_event.elapsed() >= LEVEL_EVENT_INTERVAL {
                        emit_level(&app, channel, level);
                        last_level_event = Instant::now();
                    }
                }
            }
            frame = socket.next() => {
                let frame = match frame {
                    Some(Ok(frame)) => frame,
                    Some(Err(error)) => {
                        failed = Some(format!("OpenAI Realtime connection failed: {}", sanitize_message(&error.to_string())));
                        break;
                    }
                    None => {
                        failed = Some("OpenAI Realtime connection closed unexpectedly".to_string());
                        break;
                    }
                };
                if let Message::Ping(data) = frame {
                    if socket.send(Message::Pong(data)).await.is_err() {
                        failed = Some("OpenAI Realtime connection stopped responding".to_string());
                        break;
                    }
                    continue;
                }
                match decode_json_frame(frame) {
                    Ok(Some(event)) => {
                        match process_provider_event(
                            &app,
                            channel,
                            &event,
                            playback.as_ref(),
                            &mut segments,
                        ) {
                            Ok(ProviderEventResult::Continue) => {}
                            Ok(ProviderEventResult::Closed) => {
                                failed = Some("OpenAI Realtime session closed unexpectedly".to_string());
                                break;
                            }
                            Err(error) => {
                                failed = Some(error);
                                break;
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        failed = Some(error);
                        break;
                    }
                }
            }
        }
    }

    stop_capture_handle(capture.take()).await;
    stop_playback(playback.take()).await;
    let _ = socket.close(None).await;
    manager.finish_session(channel, generation).await;

    if let Some(error) = failed {
        if manager.is_current(channel, generation) {
            emit_error(&app, channel, &error);
        }
    } else if stopped_by_command && manager.is_current(channel, generation) {
        emit_status(&app, channel, "idle");
    }
}

async fn connect_and_configure(
    slot: &ChannelSlot,
    generation: u64,
    channel: RealtimeChannel,
    mode: RealtimeMode,
    target_language: Option<&str>,
    api_key: &str,
) -> Result<RealtimeSocket, String> {
    let endpoint = match mode {
        RealtimeMode::Transcribe => TRANSCRIPTION_ENDPOINT,
        RealtimeMode::Translate => TRANSLATION_ENDPOINT,
    };
    let mut request = endpoint
        .into_client_request()
        .map_err(|_| "Could not initialize OpenAI Realtime".to_string())?;
    request.headers_mut().insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|_| "OpenAI API key has an invalid format".to_string())?,
    );

    let mut websocket_config = WebSocketConfig::default();
    websocket_config.max_message_size = Some(MAX_WEBSOCKET_MESSAGE_BYTES);
    websocket_config.max_frame_size = Some(MAX_WEBSOCKET_MESSAGE_BYTES);
    let connection = tokio::time::timeout(
        CONNECT_TIMEOUT,
        tokio_tungstenite::connect_async_with_config(request, Some(websocket_config), false),
    );
    tokio::pin!(connection);
    let connected = tokio::select! {
        result = &mut connection => Some(result),
        _ = wait_for_superseded(slot, generation) => None,
    };
    let Some(connected) = connected else {
        return Err("Realtime setup was superseded".to_string());
    };
    let (mut socket, _) = connected
        .map_err(|_| "OpenAI Realtime connection timed out".to_string())?
        .map_err(openai_connect_error)?;

    wait_for_setup_event(slot, generation, &mut socket, "session.created").await?;
    let update = session_update(channel, mode, target_language)?;
    socket
        .send(Message::Text(update.to_string().into()))
        .await
        .map_err(|_| "Could not configure OpenAI Realtime".to_string())?;
    wait_for_setup_event(slot, generation, &mut socket, "session.updated").await?;
    Ok(socket)
}

async fn wait_for_superseded(slot: &ChannelSlot, generation: u64) {
    loop {
        if slot.generation.load(Ordering::Acquire) != generation {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

async fn wait_for_setup_event(
    slot: &ChannelSlot,
    generation: u64,
    socket: &mut RealtimeSocket,
    expected: &str,
) -> Result<(), String> {
    let event = tokio::time::timeout(SETUP_EVENT_TIMEOUT, async {
        loop {
            let frame = socket
                .next()
                .await
                .ok_or_else(|| "OpenAI closed Realtime during setup".to_string())?
                .map_err(|error| {
                    format!(
                        "OpenAI Realtime setup failed: {}",
                        sanitize_message(&error.to_string())
                    )
                })?;
            if let Message::Ping(data) = frame {
                socket
                    .send(Message::Pong(data))
                    .await
                    .map_err(|_| "OpenAI Realtime setup stopped responding".to_string())?;
                continue;
            }
            let Some(event) = decode_json_frame(frame)? else {
                continue;
            };
            if let Some(error) = provider_error(&event) {
                return Err(error);
            }
            if event.get("type").and_then(Value::as_str) == Some(expected) {
                return Ok(());
            }
        }
    });
    tokio::pin!(event);
    tokio::select! {
        result = &mut event => result
            .map_err(|_| format!("OpenAI did not confirm {expected}"))?,
        _ = wait_for_superseded(slot, generation) => {
            Err("Realtime setup was superseded".to_string())
        }
    }
}

fn session_update(
    _channel: RealtimeChannel,
    mode: RealtimeMode,
    target_language: Option<&str>,
) -> Result<Value, String> {
    match mode {
        RealtimeMode::Transcribe => Ok(json!({
            "type": "session.update",
            "session": {
                "type": "transcription",
                "audio": {
                    "input": {
                        "format": {"type": "audio/pcm", "rate": 24_000},
                        "noise_reduction": {
                            // Both desktop sources can contain speech captured
                            // at room distance. This matches the far-field
                            // profile used by the existing xexamai client.
                            "type": "far_field"
                        },
                        "transcription": {
                            "model": TRANSCRIPTION_MODEL,
                            "delay": "low"
                        },
                        "turn_detection": {
                            "type": "server_vad",
                            "threshold": 0.35,
                            "prefix_padding_ms": 300,
                            "silence_duration_ms": 500
                        }
                    }
                }
            }
        })),
        RealtimeMode::Translate => {
            let target_language = target_language
                .ok_or_else(|| "Translation target language is required".to_string())?;
            Ok(json!({
                "type": "session.update",
                "session": {
                    "audio": {
                        "output": {"language": target_language}
                    }
                }
            }))
        }
    }
}

async fn send_audio(
    socket: &mut RealtimeSocket,
    mode: RealtimeMode,
    samples: &[i16],
) -> Result<(), String> {
    if samples.is_empty() {
        return Ok(());
    }
    if samples.len() > MAX_AUDIO_SAMPLES_PER_CHUNK {
        return Err("Audio capture returned an oversized chunk".to_string());
    }
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    let event_type = match mode {
        RealtimeMode::Transcribe => "input_audio_buffer.append",
        RealtimeMode::Translate => "session.input_audio_buffer.append",
    };
    let payload = json!({
        "type": event_type,
        "audio": BASE64_STANDARD.encode(bytes),
    });
    socket
        .send(Message::Text(payload.to_string().into()))
        .await
        .map_err(|error| {
            format!(
                "Could not send audio to OpenAI Realtime: {}",
                sanitize_message(&error.to_string())
            )
        })
}

async fn graceful_close(
    app: &AppHandle,
    channel: RealtimeChannel,
    mode: RealtimeMode,
    socket: &mut RealtimeSocket,
    playback: Option<&PlaybackHandle>,
    segments: &mut SegmentTracker,
) {
    let close_type = match mode {
        RealtimeMode::Transcribe => "input_audio_buffer.commit",
        RealtimeMode::Translate => "session.close",
    };
    if socket
        .send(Message::Text(
            json!({"type": close_type}).to_string().into(),
        ))
        .await
        .is_err()
    {
        return;
    }

    let _ = tokio::time::timeout(GRACEFUL_CLOSE_TIMEOUT, async {
        loop {
            let frame = match socket.next().await {
                Some(Ok(frame)) => frame,
                _ => break,
            };
            if let Message::Ping(data) = frame {
                let _ = socket.send(Message::Pong(data)).await;
                continue;
            }
            let event = match decode_json_frame(frame) {
                Ok(Some(event)) => event,
                _ => continue,
            };
            match process_provider_event(app, channel, &event, playback, segments) {
                Ok(ProviderEventResult::Closed) => break,
                Ok(ProviderEventResult::Continue) => {
                    if mode == RealtimeMode::Transcribe
                        && event.get("type").and_then(Value::as_str)
                            == Some("conversation.item.input_audio_transcription.completed")
                    {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
    .await;
}

enum ProviderEventResult {
    Continue,
    Closed,
}

fn process_provider_event(
    app: &AppHandle,
    channel: RealtimeChannel,
    event: &Value,
    playback: Option<&PlaybackHandle>,
    segments: &mut SegmentTracker,
) -> Result<ProviderEventResult, String> {
    if let Some(error) = provider_error(event) {
        return Err(error);
    }
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let descriptor = match event_type {
        "conversation.item.input_audio_transcription.delta" => Some((
            RealtimeEventKind::InputTranscriptDelta,
            false,
            false,
            "delta",
        )),
        "conversation.item.input_audio_transcription.completed" => Some((
            RealtimeEventKind::InputTranscriptCompleted,
            false,
            true,
            "transcript",
        )),
        "session.input_transcript.delta" => Some((
            RealtimeEventKind::InputTranscriptDelta,
            false,
            false,
            "delta",
        )),
        "session.input_transcript.completed" => Some((
            RealtimeEventKind::InputTranscriptCompleted,
            false,
            true,
            "transcript",
        )),
        "session.output_transcript.delta" => Some((
            RealtimeEventKind::OutputTranscriptDelta,
            true,
            false,
            "delta",
        )),
        "session.output_transcript.completed" => Some((
            RealtimeEventKind::OutputTranscriptCompleted,
            true,
            true,
            "transcript",
        )),
        _ => None,
    };
    if let Some((kind, output, completed, field)) = descriptor {
        let text = event.get(field).and_then(Value::as_str).unwrap_or_default();
        let maximum = if completed {
            MAX_TRANSCRIPT_BYTES
        } else {
            MAX_TRANSCRIPT_DELTA_BYTES
        };
        let text = bounded_text(text, maximum, "OpenAI returned an oversized transcript")?;
        let segment_id = segments.segment_id(event, output, completed);
        emit_transcript(app, channel, kind, segment_id, text, completed);
    }

    if event_type == "session.output_audio.delta" {
        let encoded = event
            .get("delta")
            .and_then(Value::as_str)
            .ok_or_else(|| "OpenAI returned invalid translated audio".to_string())?;
        if encoded.len() > MAX_OUTPUT_AUDIO_BYTES.saturating_mul(2) {
            return Err("OpenAI returned oversized translated audio".to_string());
        }
        if let Some(playback) = playback {
            let audio = BASE64_STANDARD
                .decode(encoded)
                .map_err(|_| "OpenAI returned invalid translated audio".to_string())?;
            if audio.len() > MAX_OUTPUT_AUDIO_BYTES || audio.len() % 2 != 0 {
                return Err("OpenAI returned invalid translated audio".to_string());
            }
            playback.push_pcm16(&audio)?;
        }
    }

    Ok(if event_type == "session.closed" {
        ProviderEventResult::Closed
    } else {
        ProviderEventResult::Continue
    })
}

struct SegmentTracker {
    generation: u64,
    next: u64,
    shared_translation_turns: bool,
    shared: Option<String>,
    input: Option<String>,
    output: Option<String>,
}

impl SegmentTracker {
    fn new(generation: u64, shared_translation_turns: bool) -> Self {
        Self {
            generation,
            next: 0,
            shared_translation_turns,
            shared: None,
            input: None,
            output: None,
        }
    }

    fn segment_id(&mut self, event: &Value, output: bool, completed: bool) -> String {
        let provider_id = event
            .get("item_id")
            .or_else(|| event.get("response_id"))
            .and_then(Value::as_str)
            .filter(|value| valid_provider_id(value))
            .map(str::to_string);
        if self.shared_translation_turns {
            let id = self
                .shared
                .clone()
                .or(provider_id)
                .unwrap_or_else(|| self.next_fallback());
            // Dedicated translation does not guarantee a shared item_id for
            // the source and translated streams. Keep both sides on the same
            // renderer row until the translated output completes the turn.
            if output && completed {
                self.shared = None;
            } else {
                self.shared = Some(id.clone());
            }
            return id;
        }
        let current_value = if output {
            self.output.clone()
        } else {
            self.input.clone()
        };
        let id = provider_id
            .or(current_value)
            .unwrap_or_else(|| self.next_fallback());
        let current = if output {
            &mut self.output
        } else {
            &mut self.input
        };
        if !completed {
            *current = Some(id.clone());
        } else {
            *current = None;
        }
        id
    }

    fn next_fallback(&mut self) -> String {
        self.next = self.next.wrapping_add(1);
        format!("{}-{}", self.generation, self.next)
    }
}

fn decode_json_frame(frame: Message) -> Result<Option<Value>, String> {
    let bytes = match frame {
        Message::Text(text) => {
            if text.len() > MAX_WEBSOCKET_MESSAGE_BYTES {
                return Err("OpenAI returned an oversized Realtime event".to_string());
            }
            return serde_json::from_str(text.as_ref())
                .map(Some)
                .map_err(|_| "OpenAI returned an unreadable Realtime event".to_string());
        }
        Message::Binary(bytes) => bytes,
        Message::Close(_) => return Err("OpenAI closed the Realtime connection".to_string()),
        Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => return Ok(None),
    };
    if bytes.len() > MAX_WEBSOCKET_MESSAGE_BYTES {
        return Err("OpenAI returned an oversized Realtime event".to_string());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "OpenAI returned an unreadable Realtime event".to_string())
}

fn provider_error(event: &Value) -> Option<String> {
    (event.get("type").and_then(Value::as_str) == Some("error")).then(|| {
        let message = event
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("OpenAI rejected the Realtime session");
        format!("OpenAI Realtime error: {}", sanitize_message(message))
    })
}

fn openai_connect_error(error: WebSocketError) -> String {
    match error {
        WebSocketError::Http(response) if matches!(response.status().as_u16(), 401 | 403) => {
            "OpenAI API key is invalid or cannot access Realtime".to_string()
        }
        WebSocketError::Http(response) if response.status().as_u16() == 429 => {
            "OpenAI Realtime rate limit or quota was reached".to_string()
        }
        WebSocketError::Http(response) => format!(
            "OpenAI Realtime connection failed (HTTP {})",
            response.status().as_u16()
        ),
        other => format!(
            "Could not reach OpenAI Realtime: {}",
            sanitize_message(&other.to_string())
        ),
    }
}

fn normalize_request(request: RealtimeStartRequest) -> Result<NormalizedRequest, String> {
    let input_device_id = normalize_device_id(request.input_device_id)?;
    let output_device_id = normalize_device_id(request.output_device_id)?;
    let playback_enabled = request.mode == RealtimeMode::Translate && request.playback_enabled;
    let target_language = match request.mode {
        RealtimeMode::Transcribe => None,
        RealtimeMode::Translate => Some(normalize_target_language(
            request
                .target_language
                .as_deref()
                .ok_or_else(|| "Translation target language is required".to_string())?,
        )?),
    };
    Ok(NormalizedRequest {
        channel: request.channel,
        mode: request.mode,
        playback_enabled,
        input_device_id,
        output_device_id,
        target_language,
    })
}

fn normalize_device_id(value: Option<String>) -> Result<Option<String>, String> {
    let Some(value) = value else { return Ok(None) };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > MAX_DEVICE_ID_BYTES || value.chars().any(char::is_control) {
        return Err("Audio device identifier is invalid".to_string());
    }
    Ok(Some(value.to_string()))
}

fn normalize_target_language(value: &str) -> Result<String, String> {
    let value = value.trim().to_ascii_lowercase();
    let mut parts = value.split('-');
    let primary = parts.next().unwrap_or_default();
    let primary_valid = matches!(primary.len(), 2 | 3)
        && primary
            .bytes()
            .all(|character| character.is_ascii_lowercase());
    let extensions = parts.collect::<Vec<_>>();
    let extensions_valid = extensions.len() <= 3
        && extensions.iter().all(|part| {
            (2..=8).contains(&part.len())
                && part
                    .bytes()
                    .all(|character| character.is_ascii_lowercase() || character.is_ascii_digit())
        });
    if value.len() <= 35 && primary_valid && extensions_valid {
        Ok(value)
    } else {
        Err("Unsupported realtime translation language".to_string())
    }
}

fn validate_feedback_route(
    channel: RealtimeChannel,
    candidate: &ReservedRoute,
    routes: &HashMap<RealtimeChannel, ReservedRoute>,
) -> Result<(), String> {
    if channel == RealtimeChannel::System
        && candidate.mode == RealtimeMode::Translate
        && candidate.playback_enabled
        && endpoints_may_match(
            candidate.input_device_id.as_deref(),
            candidate.output_device_id.as_deref(),
        )
    {
        return Err(
            "System audio input and translated output must use different devices to prevent an audio loop"
                .to_string(),
        );
    }

    for (other_channel, other) in routes {
        if *other_channel == channel {
            continue;
        }
        if channel == RealtimeChannel::System
            && other.mode == RealtimeMode::Translate
            && other.playback_enabled
            && endpoints_may_match(
                candidate.input_device_id.as_deref(),
                other.output_device_id.as_deref(),
            )
        {
            return Err(
                "The microphone translation output is routed into active system-audio capture"
                    .to_string(),
            );
        }
        if *other_channel == RealtimeChannel::System
            && candidate.mode == RealtimeMode::Translate
            && candidate.playback_enabled
            && endpoints_may_match(
                other.input_device_id.as_deref(),
                candidate.output_device_id.as_deref(),
            )
        {
            return Err(
                "The selected translation output is routed into active system-audio capture"
                    .to_string(),
            );
        }
    }
    Ok(())
}

fn endpoints_may_match(left: Option<&str>, right: Option<&str>) -> bool {
    if let Some(matches) = system_capture_output_match(left, right) {
        return matches;
    }
    if let Some(matches) = system_capture_output_match(right, left) {
        return matches;
    }
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}

fn bounded_text(value: &str, maximum: usize, message: &str) -> Result<String, String> {
    if value.len() > maximum || value.chars().any(|character| character == '\0') {
        return Err(message.to_string());
    }
    Ok(value.to_string())
}

fn valid_provider_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && !value.chars().any(|character| character.is_control())
}

fn sanitize_message(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_PROVIDER_MESSAGE_BYTES)
        .collect()
}

fn emit_status(app: &AppHandle, channel: RealtimeChannel, status: &'static str) {
    let _ = app.emit_to(
        "main",
        REALTIME_EVENT,
        RealtimeEventPayload {
            channel,
            kind: RealtimeEventKind::Status,
            segment_id: None,
            delta: None,
            text: None,
            status: Some(status),
            level: None,
            message: None,
        },
    );
}

fn emit_status_for_generation(
    manager: &RealtimeManager,
    app: &AppHandle,
    channel: RealtimeChannel,
    generation: u64,
    status: &'static str,
) {
    if manager.is_current(channel, generation) {
        emit_status(app, channel, status);
    }
}

fn emit_error(app: &AppHandle, channel: RealtimeChannel, message: &str) {
    let _ = app.emit_to(
        "main",
        REALTIME_EVENT,
        RealtimeEventPayload {
            channel,
            kind: RealtimeEventKind::Error,
            segment_id: None,
            delta: None,
            text: None,
            status: Some("error"),
            level: None,
            message: Some(sanitize_message(message)),
        },
    );
}

fn emit_level(app: &AppHandle, channel: RealtimeChannel, level: f32) {
    let _ = app.emit_to(
        "main",
        REALTIME_EVENT,
        RealtimeEventPayload {
            channel,
            kind: RealtimeEventKind::Level,
            segment_id: None,
            delta: None,
            text: None,
            status: None,
            level: Some(if level.is_finite() {
                level.clamp(0.0, 1.0)
            } else {
                0.0
            }),
            message: None,
        },
    );
}

fn emit_transcript(
    app: &AppHandle,
    channel: RealtimeChannel,
    kind: RealtimeEventKind,
    segment_id: String,
    value: String,
    completed: bool,
) {
    let _ = app.emit_to(
        "main",
        REALTIME_EVENT,
        RealtimeEventPayload {
            channel,
            kind,
            segment_id: Some(segment_id),
            delta: (!completed).then_some(value.clone()),
            text: completed.then_some(value),
            status: None,
            level: None,
            message: None,
        },
    );
}

async fn stop_capture_handle(capture: Option<CaptureHandle>) {
    if let Some(capture) = capture {
        let _ = tauri::async_runtime::spawn_blocking(move || capture.stop()).await;
    }
}

async fn start_playback_async(output_device_id: Option<String>) -> Result<PlaybackHandle, String> {
    tauri::async_runtime::spawn_blocking(move || start_playback(output_device_id.as_deref()))
        .await
        .map_err(|_| "Audio playback worker terminated unexpectedly".to_string())?
}

async fn stop_playback(playback: Option<PlaybackHandle>) {
    if let Some(playback) = playback {
        let _ = tauri::async_runtime::spawn_blocking(move || playback.stop()).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(channel: RealtimeChannel, mode: RealtimeMode) -> RealtimeStartRequest {
        RealtimeStartRequest {
            channel,
            mode,
            playback_enabled: mode == RealtimeMode::Translate,
            input_device_id: None,
            output_device_id: None,
            target_language: (mode == RealtimeMode::Translate).then(|| "ru".to_string()),
        }
    }

    #[test]
    fn ipc_event_uses_the_exact_camel_case_contract() {
        let event = RealtimeEventPayload {
            channel: RealtimeChannel::Microphone,
            kind: RealtimeEventKind::InputTranscriptDelta,
            segment_id: Some("segment-1".to_string()),
            delta: Some("hello".to_string()),
            text: None,
            status: None,
            level: None,
            message: None,
        };
        assert_eq!(
            serde_json::to_value(event).unwrap(),
            json!({
                "channel": "microphone",
                "kind": "input_transcript_delta",
                "segmentId": "segment-1",
                "delta": "hello"
            })
        );
    }

    #[test]
    fn transcription_and_translation_sessions_are_model_scoped() {
        let transcription =
            session_update(RealtimeChannel::Microphone, RealtimeMode::Transcribe, None).unwrap();
        assert_eq!(transcription["session"]["type"], "transcription");
        assert_eq!(
            transcription["session"]["audio"]["input"]["transcription"]["model"],
            TRANSCRIPTION_MODEL
        );
        assert_eq!(
            transcription["session"]["audio"]["input"]["format"]["rate"],
            24_000
        );
        assert_eq!(
            transcription["session"]["audio"]["input"]["turn_detection"]["type"],
            "server_vad"
        );
        assert_eq!(
            transcription["session"]["audio"]["input"]["noise_reduction"]["type"],
            "far_field"
        );
        assert_eq!(
            transcription["session"]["audio"]["input"]["turn_detection"]["threshold"],
            0.35
        );

        let translation =
            session_update(RealtimeChannel::System, RealtimeMode::Translate, Some("en")).unwrap();
        assert_eq!(translation["session"]["audio"]["output"]["language"], "en");
        assert!(translation["session"]["audio"].get("input").is_none());
    }

    #[test]
    fn request_validation_requires_a_supported_translation_language() {
        let mut valid = request(RealtimeChannel::Microphone, RealtimeMode::Translate);
        valid.target_language = Some(" RU ".to_string());
        assert_eq!(
            normalize_request(valid).unwrap().target_language.as_deref(),
            Some("ru")
        );

        let mut invalid = request(RealtimeChannel::Microphone, RealtimeMode::Translate);
        invalid.target_language = Some("klingon".to_string());
        assert!(normalize_request(invalid).is_err());
        for language in ["nl", "sv", "cs", "el", "he", "pt-br"] {
            assert_eq!(
                normalize_target_language(language).unwrap(),
                language.to_string()
            );
        }
        assert!(normalize_target_language("x").is_err());
        assert!(normalize_target_language("en--us").is_err());
        assert!(
            normalize_request(request(RealtimeChannel::System, RealtimeMode::Transcribe))
                .unwrap()
                .target_language
                .is_none()
        );
    }

    #[test]
    fn translation_source_and_output_share_one_fallback_segment() {
        let mut tracker = SegmentTracker::new(7, true);
        let source_delta = tracker.segment_id(&json!({}), false, false);
        let source_completed = tracker.segment_id(&json!({}), false, true);
        let output_delta = tracker.segment_id(&json!({}), true, false);
        let output_completed = tracker.segment_id(&json!({}), true, true);
        assert_eq!(source_delta, source_completed);
        assert_eq!(source_delta, output_delta);
        assert_eq!(source_delta, output_completed);
        assert_ne!(source_delta, tracker.segment_id(&json!({}), false, false));
    }

    #[test]
    fn feedback_validation_rejects_same_and_cross_channel_routes() {
        let same = ReservedRoute {
            generation: 1,
            mode: RealtimeMode::Translate,
            playback_enabled: true,
            input_device_id: Some("speakers".to_string()),
            output_device_id: Some("speakers".to_string()),
        };
        assert!(validate_feedback_route(RealtimeChannel::System, &same, &HashMap::new()).is_err());

        let muted = ReservedRoute {
            playback_enabled: false,
            ..same.clone()
        };
        assert!(validate_feedback_route(RealtimeChannel::System, &muted, &HashMap::new()).is_ok());

        let mut active = HashMap::new();
        active.insert(
            RealtimeChannel::System,
            ReservedRoute {
                generation: 2,
                mode: RealtimeMode::Transcribe,
                playback_enabled: false,
                input_device_id: Some("speakers".to_string()),
                output_device_id: None,
            },
        );
        let microphone = ReservedRoute {
            generation: 3,
            mode: RealtimeMode::Translate,
            playback_enabled: true,
            input_device_id: Some("microphone".to_string()),
            output_device_id: Some("speakers".to_string()),
        };
        assert!(
            validate_feedback_route(RealtimeChannel::Microphone, &microphone, &active).is_err()
        );

        #[cfg(windows)]
        {
            use crate::audio::WINDOWS_PROCESS_LOOPBACK_ID;
            let excluded_system = ReservedRoute {
                input_device_id: Some(WINDOWS_PROCESS_LOOPBACK_ID.to_string()),
                output_device_id: Some("speakers".to_string()),
                ..same
            };
            assert!(validate_feedback_route(
                RealtimeChannel::System,
                &excluded_system,
                &HashMap::new(),
            )
            .is_ok());

            active
                .get_mut(&RealtimeChannel::System)
                .unwrap()
                .input_device_id = Some(WINDOWS_PROCESS_LOOPBACK_ID.to_string());
            assert!(
                validate_feedback_route(RealtimeChannel::Microphone, &microphone, &active,).is_ok()
            );
        }
    }

    #[test]
    fn transcription_never_reserves_or_starts_speech_playback() {
        let mut transcribe = request(RealtimeChannel::Microphone, RealtimeMode::Transcribe);
        transcribe.playback_enabled = true;
        transcribe.output_device_id = Some("speakers".to_string());
        let normalized = normalize_request(transcribe).unwrap();
        assert!(!normalized.playback_enabled);
        assert!(normalized.target_language.is_none());
    }

    #[test]
    fn provider_text_and_identifiers_are_bounded() {
        assert!(valid_provider_id("item-1"));
        assert!(!valid_provider_id("item\n1"));
        assert!(bounded_text("hello", 5, "too large").is_ok());
        assert!(bounded_text("hello!", 5, "too large").is_err());
        assert_eq!(
            sanitize_message("  provider   failed \n now "),
            "provider failed now"
        );
    }
}
