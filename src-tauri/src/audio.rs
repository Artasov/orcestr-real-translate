use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    Device, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig,
    SupportedStreamConfig, I24, U24,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{self, error::TrySendError, Sender, UnboundedSender};

#[path = "audio_processing.rs"]
mod audio_processing;
#[cfg(target_os = "macos")]
#[path = "macos_capture.rs"]
mod macos_capture;
#[cfg(target_os = "linux")]
#[path = "pulse_capture.rs"]
mod pulse_capture;
#[cfg(windows)]
#[path = "wasapi_capture.rs"]
mod wasapi_capture;

use audio_processing::{
    downmix_interleaved_to_mono, meter_level, normalize_i16, quantize_i16, SpeechDynamicsProcessor,
    StreamingMonoResampler, REALTIME_SAMPLE_RATE,
};

const START_TIMEOUT: Duration = Duration::from_secs(5);
const CAPTURE_CHUNK_SAMPLES: usize = (REALTIME_SAMPLE_RATE as usize * 40) / 1_000;
const LEVEL_INTERVAL: Duration = Duration::from_millis(50);
const PLAYBACK_QUEUE_CHUNKS: usize = 24;
const MAX_PLAYBACK_INPUT_BYTES: usize = REALTIME_SAMPLE_RATE as usize * 2 * 4;
const MAX_PLAYBACK_BUFFER_SECONDS: usize = 5;
#[cfg(windows)]
pub(crate) const WINDOWS_PROCESS_LOOPBACK_ID: &str = "windows-process-loopback";
#[cfg(target_os = "macos")]
pub(crate) const MACOS_SYSTEM_AUDIO_ID: &str = "macos-screencapturekit-system-audio";
#[cfg(target_os = "linux")]
pub(crate) const LINUX_DEFAULT_MONITOR_PREFIX: &str = "linux-default-monitor:";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDeviceInventory {
    pub inputs: Vec<AudioEndpoint>,
    pub outputs: Vec<AudioEndpoint>,
    pub system_sources: Vec<AudioEndpoint>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AudioEndpoint {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub channels: u16,
    pub sample_rate: u32,
    pub monitored_output_id: Option<String>,
    pub excludes_current_process_audio: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AudioSource {
    Microphone,
    System,
}

#[must_use = "dropping CaptureHandle does not synchronously join its native worker; call stop"]
pub struct CaptureHandle {
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl CaptureHandle {
    pub fn stop(mut self) -> Result<(), String> {
        self.cancel.store(true, Ordering::Release);
        let panicked = self
            .worker
            .take()
            .is_some_and(|worker| worker.join().is_err());
        if panicked {
            return Err("The audio capture worker terminated unexpectedly".to_string());
        }
        take_error(&self.error).map_or(Ok(()), Err)
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

#[must_use = "dropping PlaybackHandle does not synchronously join its native worker; call stop"]
pub struct PlaybackHandle {
    audio_tx: Sender<Vec<i16>>,
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl PlaybackHandle {
    /// Queues little-endian mono PCM16 at 24 kHz. The method never waits for
    /// the hardware callback; a full queue is reported to the caller.
    pub fn push_pcm16(&self, bytes: &[u8]) -> Result<(), String> {
        if bytes.is_empty() || bytes.len() % 2 != 0 || bytes.len() > MAX_PLAYBACK_INPUT_BYTES {
            return Err("Invalid playback PCM16 chunk".to_string());
        }
        if let Some(error) = peek_error(&self.error) {
            return Err(error);
        }
        let samples = bytes
            .chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
            .collect::<Vec<_>>();
        self.audio_tx
            .try_send(samples)
            .map_err(|error| match error {
                TrySendError::Full(_) => "Audio playback queue is full".to_string(),
                TrySendError::Closed(_) => peek_error(&self.error)
                    .unwrap_or_else(|| "Audio playback has stopped".to_string()),
            })
    }

    pub fn stop(mut self) -> Result<(), String> {
        self.cancel.store(true, Ordering::Release);
        let panicked = self
            .worker
            .take()
            .is_some_and(|worker| worker.join().is_err());
        if panicked {
            return Err("The audio playback worker terminated unexpectedly".to_string());
        }
        take_error(&self.error).map_or(Ok(()), Err)
    }
}

impl Drop for PlaybackHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

pub fn list_audio_devices() -> Result<AudioDeviceInventory, String> {
    let host = cpal::default_host();
    let default_input_id = host
        .default_input_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());
    let default_output_id = host
        .default_output_device()
        .and_then(|device| device.id().ok())
        .map(|id| id.to_string());

    let mut inputs = Vec::new();
    for device in host
        .input_devices()
        .map_err(|error| format!("Could not enumerate audio inputs: {error}"))?
    {
        if let Ok(endpoint) = endpoint_from_input(&device, default_input_id.as_deref()) {
            push_unique_endpoint(&mut inputs, endpoint);
        }
    }

    let mut outputs = Vec::new();
    for device in host
        .output_devices()
        .map_err(|error| format!("Could not enumerate audio outputs: {error}"))?
    {
        if let Ok(endpoint) = endpoint_from_output(&device, default_output_id.as_deref()) {
            push_unique_endpoint(&mut outputs, endpoint);
        }
    }
    sort_endpoints(&mut inputs);
    sort_endpoints(&mut outputs);

    #[cfg(windows)]
    let system_sources = vec![AudioEndpoint {
        id: WINDOWS_PROCESS_LOOPBACK_ID.to_string(),
        name: "All system audio".to_string(),
        is_default: true,
        channels: 2,
        sample_rate: 48_000,
        monitored_output_id: None,
        excludes_current_process_audio: true,
    }];
    #[cfg(target_os = "macos")]
    let system_sources = vec![AudioEndpoint {
        id: MACOS_SYSTEM_AUDIO_ID.to_string(),
        name: "All system audio".to_string(),
        is_default: true,
        channels: 2,
        sample_rate: 48_000,
        monitored_output_id: None,
        excludes_current_process_audio: true,
    }];
    #[cfg(target_os = "linux")]
    let system_sources = vec![AudioEndpoint {
        id: format!(
            "{LINUX_DEFAULT_MONITOR_PREFIX}{}",
            default_output_id.as_deref().unwrap_or_default()
        ),
        name: "Default system audio".to_string(),
        is_default: true,
        channels: 2,
        sample_rate: 48_000,
        monitored_output_id: default_output_id.clone(),
        excludes_current_process_audio: false,
    }];
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    let system_sources = inputs
        .iter()
        .filter(|endpoint| is_monitor_name(&endpoint.name))
        .cloned()
        .collect();

    Ok(AudioDeviceInventory {
        inputs,
        outputs,
        system_sources,
    })
}

pub fn start_capture(
    source: AudioSource,
    device_id: Option<&str>,
    audio_tx: Sender<Vec<i16>>,
    level_tx: UnboundedSender<f32>,
) -> Result<CaptureHandle, String> {
    let requested_id = normalize_requested_id(device_id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let error = Arc::new(Mutex::new(None));
    let (ready_tx, ready_rx) = std_mpsc::sync_channel(1);
    let worker_cancel = cancel.clone();
    let worker_error = error.clone();
    let worker = thread::Builder::new()
        .name(
            match source {
                AudioSource::Microphone => "orcestr-microphone-capture",
                AudioSource::System => "orcestr-system-capture",
            }
            .to_string(),
        )
        .spawn(move || match source {
            AudioSource::Microphone => run_cpal_capture(
                requested_id.as_deref(),
                false,
                audio_tx,
                level_tx,
                worker_cancel,
                worker_error,
                ready_tx,
            ),
            AudioSource::System => run_system_capture(
                requested_id.as_deref(),
                audio_tx,
                level_tx,
                worker_cancel,
                worker_error,
                ready_tx,
            ),
        })
        .map_err(|error| format!("Could not start the audio capture worker: {error}"))?;

    match ready_rx.recv_timeout(START_TIMEOUT) {
        Ok(Ok(())) => Ok(CaptureHandle {
            cancel,
            error,
            worker: Some(worker),
        }),
        Ok(Err(message)) => {
            cancel.store(true, Ordering::Release);
            let _ = worker.join();
            Err(message)
        }
        Err(std_mpsc::RecvTimeoutError::Timeout) => {
            cancel.store(true, Ordering::Release);
            let _ = worker.join();
            Err("Timed out while starting audio capture".to_string())
        }
        Err(std_mpsc::RecvTimeoutError::Disconnected) => {
            cancel.store(true, Ordering::Release);
            let _ = worker.join();
            Err(take_error(&error)
                .unwrap_or_else(|| "Audio capture stopped before becoming ready".to_string()))
        }
    }
}

pub fn start_playback(output_id: Option<&str>) -> Result<PlaybackHandle, String> {
    let requested_id = normalize_requested_id(output_id)?;
    let cancel = Arc::new(AtomicBool::new(false));
    let error = Arc::new(Mutex::new(None));
    let (audio_tx, audio_rx) = mpsc::channel(PLAYBACK_QUEUE_CHUNKS);
    let (ready_tx, ready_rx) = std_mpsc::sync_channel(1);
    let worker_cancel = cancel.clone();
    let worker_error = error.clone();
    let worker = thread::Builder::new()
        .name("orcestr-audio-playback".to_string())
        .spawn(move || {
            run_playback_worker(
                requested_id.as_deref(),
                audio_rx,
                worker_cancel,
                worker_error,
                ready_tx,
            )
        })
        .map_err(|error| format!("Could not start the audio playback worker: {error}"))?;

    match ready_rx.recv_timeout(START_TIMEOUT) {
        Ok(Ok(())) => Ok(PlaybackHandle {
            audio_tx,
            cancel,
            error,
            worker: Some(worker),
        }),
        Ok(Err(message)) => {
            cancel.store(true, Ordering::Release);
            let _ = worker.join();
            Err(message)
        }
        Err(std_mpsc::RecvTimeoutError::Timeout) => {
            cancel.store(true, Ordering::Release);
            let _ = worker.join();
            Err("Timed out while starting audio playback".to_string())
        }
        Err(std_mpsc::RecvTimeoutError::Disconnected) => {
            cancel.store(true, Ordering::Release);
            let _ = worker.join();
            Err(take_error(&error)
                .unwrap_or_else(|| "Audio playback stopped before becoming ready".to_string()))
        }
    }
}

pub(super) struct CapturePcmSink {
    dynamics: SpeechDynamicsProcessor,
    resampler: StreamingMonoResampler,
    pending: VecDeque<i16>,
    audio_tx: Sender<Vec<i16>>,
    level_tx: UnboundedSender<f32>,
    last_level_at: Instant,
}

impl CapturePcmSink {
    pub(super) fn new(
        source_rate: u32,
        audio_tx: Sender<Vec<i16>>,
        level_tx: UnboundedSender<f32>,
    ) -> Self {
        Self {
            dynamics: SpeechDynamicsProcessor::new(source_rate),
            // Speech enhancement emits RNNoise's native 48 kHz stream.
            resampler: StreamingMonoResampler::new(48_000, REALTIME_SAMPLE_RATE),
            pending: VecDeque::with_capacity(CAPTURE_CHUNK_SAMPLES * 2),
            audio_tx,
            level_tx,
            last_level_at: Instant::now() - LEVEL_INTERVAL,
        }
    }

    pub(super) fn push_interleaved(&mut self, samples: &[f32], channels: u16) {
        let mono = downmix_interleaved_to_mono(samples, channels);
        let processed = self.dynamics.process(&mono);
        if self.last_level_at.elapsed() >= LEVEL_INTERVAL {
            let _ = self.level_tx.send(meter_level(&processed));
            self.last_level_at = Instant::now();
        }
        let resampled = self.resampler.process(&processed);
        self.pending.extend(quantize_i16(&resampled));
        self.send_complete_chunks();
    }

    fn finish(&mut self) {
        let enhanced_tail = self.dynamics.finish();
        let resampled_tail = self.resampler.process(&enhanced_tail);
        self.pending.extend(quantize_i16(&resampled_tail));
        let tail = self.resampler.finish();
        self.pending.extend(quantize_i16(&tail));
        self.send_complete_chunks();
        if !self.pending.is_empty() {
            let chunk = self.pending.drain(..).collect::<Vec<_>>();
            let _ = self.audio_tx.try_send(chunk);
        }
    }

    fn send_complete_chunks(&mut self) {
        while self.pending.len() >= CAPTURE_CHUNK_SAMPLES {
            let chunk = self
                .pending
                .drain(..CAPTURE_CHUNK_SAMPLES)
                .collect::<Vec<_>>();
            // Capture must never block the real-time callback. If the consumer
            // is behind, discard this oldest completed packet and keep latency
            // bounded rather than accumulating unbounded audio.
            let _ = self.audio_tx.try_send(chunk);
        }
    }
}

fn run_cpal_capture(
    device_id: Option<&str>,
    require_monitor: bool,
    audio_tx: Sender<Vec<i16>>,
    level_tx: UnboundedSender<f32>,
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    ready_tx: std_mpsc::SyncSender<Result<(), String>>,
) {
    let result = (|| -> Result<(), String> {
        let host = cpal::default_host();
        let device = if require_monitor {
            select_monitor_input(&host, device_id)?
        } else {
            select_input_device(&host, device_id)?
        };
        let (supported, sample_format) = choose_input_config(&device)?;
        let config: StreamConfig = supported.into();
        let sink = Arc::new(Mutex::new(CapturePcmSink::new(
            config.sample_rate,
            audio_tx,
            level_tx,
        )));
        let stream = build_input_stream(
            &device,
            &config,
            sample_format,
            sink.clone(),
            cancel.clone(),
            error.clone(),
        )?;
        stream
            .play()
            .map_err(|error| format!("Could not start audio input: {error}"))?;
        let _ = ready_tx.send(Ok(()));

        while !cancel.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(20));
        }
        drop(stream);
        if let Ok(mut sink) = sink.lock() {
            sink.finish();
        }
        Ok(())
    })();
    if let Err(message) = result {
        set_first_error(&error, message.clone());
        cancel.store(true, Ordering::Release);
        let _ = ready_tx.try_send(Err(message));
    }
}

#[cfg(windows)]
fn run_system_capture(
    device_id: Option<&str>,
    audio_tx: Sender<Vec<i16>>,
    level_tx: UnboundedSender<f32>,
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    ready_tx: std_mpsc::SyncSender<Result<(), String>>,
) {
    let result = wasapi_capture::run_wasapi_loopback(
        device_id,
        audio_tx,
        level_tx,
        cancel.clone(),
        ready_tx.clone(),
    );
    if let Err(message) = result {
        set_first_error(&error, message.clone());
        cancel.store(true, Ordering::Release);
        let _ = ready_tx.try_send(Err(message));
    }
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn run_system_capture(
    device_id: Option<&str>,
    audio_tx: Sender<Vec<i16>>,
    level_tx: UnboundedSender<f32>,
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    ready_tx: std_mpsc::SyncSender<Result<(), String>>,
) {
    run_cpal_capture(device_id, true, audio_tx, level_tx, cancel, error, ready_tx);
}

#[cfg(target_os = "macos")]
fn run_system_capture(
    device_id: Option<&str>,
    audio_tx: Sender<Vec<i16>>,
    level_tx: UnboundedSender<f32>,
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    ready_tx: std_mpsc::SyncSender<Result<(), String>>,
) {
    let result = macos_capture::run_system_audio(
        device_id,
        audio_tx,
        level_tx,
        cancel.clone(),
        ready_tx.clone(),
    );
    if let Err(message) = result {
        set_first_error(&error, message.clone());
        cancel.store(true, Ordering::Release);
        let _ = ready_tx.try_send(Err(message));
    }
}

#[cfg(target_os = "linux")]
fn run_system_capture(
    device_id: Option<&str>,
    audio_tx: Sender<Vec<i16>>,
    level_tx: UnboundedSender<f32>,
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    ready_tx: std_mpsc::SyncSender<Result<(), String>>,
) {
    let result = pulse_capture::run_default_monitor(
        device_id,
        audio_tx,
        level_tx,
        cancel.clone(),
        ready_tx.clone(),
    );
    if let Err(message) = result {
        set_first_error(&error, message.clone());
        cancel.store(true, Ordering::Release);
        let _ = ready_tx.try_send(Err(message));
    }
}

fn build_input_stream(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    sink: Arc<Mutex<CapturePcmSink>>,
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<Stream, String> {
    macro_rules! build {
        ($sample:ty) => {
            build_input_stream_typed::<$sample>(device, config, sink, cancel, error)
        };
    }
    match sample_format {
        SampleFormat::I8 => build!(i8),
        SampleFormat::I16 => build!(i16),
        SampleFormat::I24 => build!(I24),
        SampleFormat::I32 => build!(i32),
        SampleFormat::I64 => build!(i64),
        SampleFormat::U8 => build!(u8),
        SampleFormat::U16 => build!(u16),
        SampleFormat::U24 => build!(U24),
        SampleFormat::U32 => build!(u32),
        SampleFormat::U64 => build!(u64),
        SampleFormat::F32 => build!(f32),
        SampleFormat::F64 => build!(f64),
        other => Err(format!("Unsupported input sample format: {other}")),
    }
}

fn build_input_stream_typed<T>(
    device: &Device,
    config: &StreamConfig,
    sink: Arc<Mutex<CapturePcmSink>>,
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<Stream, String>
where
    T: SizedSample + Copy + Send + 'static,
    f32: FromSample<T>,
{
    let channels = config.channels;
    let callback_cancel = cancel.clone();
    let error_cancel = cancel;
    let stream_error = error;
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                if callback_cancel.load(Ordering::Acquire) {
                    return;
                }
                let samples = data
                    .iter()
                    .map(|sample| f32::from_sample(*sample))
                    .collect::<Vec<_>>();
                if let Ok(mut sink) = sink.try_lock() {
                    sink.push_interleaved(&samples, channels);
                }
            },
            move |stream_error_value| {
                set_first_error(
                    &stream_error,
                    format!("Audio input failed: {stream_error_value}"),
                );
                error_cancel.store(true, Ordering::Release);
            },
            None,
        )
        .map_err(|error| format!("Could not open audio input: {error}"))
}

fn run_playback_worker(
    output_id: Option<&str>,
    mut audio_rx: mpsc::Receiver<Vec<i16>>,
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    ready_tx: std_mpsc::SyncSender<Result<(), String>>,
) {
    let result = (|| -> Result<(), String> {
        let host = cpal::default_host();
        let device = select_output_device(&host, output_id)?;
        let (supported, sample_format) = choose_output_config(&device)?;
        let config: StreamConfig = supported.into();
        let processor = PlaybackProcessor::new(config.sample_rate);
        let stream = build_output_stream(
            &device,
            &config,
            sample_format,
            processor,
            &mut audio_rx,
            cancel.clone(),
            error.clone(),
        )?;
        stream
            .play()
            .map_err(|error| format!("Could not start audio output: {error}"))?;
        let _ = ready_tx.send(Ok(()));
        while !cancel.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(20));
        }
        drop(stream);
        Ok(())
    })();
    if let Err(message) = result {
        set_first_error(&error, message.clone());
        cancel.store(true, Ordering::Release);
        let _ = ready_tx.try_send(Err(message));
    }
}

struct PlaybackProcessor {
    resampler: StreamingMonoResampler,
    pending: VecDeque<f32>,
    max_pending: usize,
}

impl PlaybackProcessor {
    fn new(output_rate: u32) -> Self {
        Self {
            resampler: StreamingMonoResampler::new(REALTIME_SAMPLE_RATE, output_rate),
            pending: VecDeque::new(),
            max_pending: output_rate as usize * MAX_PLAYBACK_BUFFER_SECONDS,
        }
    }

    fn ingest_available(&mut self, audio_rx: &mut mpsc::Receiver<Vec<i16>>) {
        while let Ok(chunk) = audio_rx.try_recv() {
            let normalized = chunk.into_iter().map(normalize_i16).collect::<Vec<_>>();
            self.pending.extend(self.resampler.process(&normalized));
            while self.pending.len() > self.max_pending {
                self.pending.pop_front();
            }
        }
    }

    fn next_sample(&mut self) -> f32 {
        self.pending.pop_front().unwrap_or(0.0)
    }
}

fn build_output_stream(
    device: &Device,
    config: &StreamConfig,
    sample_format: SampleFormat,
    processor: PlaybackProcessor,
    audio_rx: &mut mpsc::Receiver<Vec<i16>>,
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<Stream, String> {
    // Receiver has no async runtime dependency in try_recv and is owned by the
    // hardware callback after the stream is constructed.
    let receiver = std::mem::replace(audio_rx, mpsc::channel(1).1);
    macro_rules! build {
        ($sample:ty) => {
            build_output_stream_typed::<$sample>(device, config, processor, receiver, cancel, error)
        };
    }
    match sample_format {
        SampleFormat::I8 => build!(i8),
        SampleFormat::I16 => build!(i16),
        SampleFormat::I24 => build!(I24),
        SampleFormat::I32 => build!(i32),
        SampleFormat::I64 => build!(i64),
        SampleFormat::U8 => build!(u8),
        SampleFormat::U16 => build!(u16),
        SampleFormat::U24 => build!(U24),
        SampleFormat::U32 => build!(u32),
        SampleFormat::U64 => build!(u64),
        SampleFormat::F32 => build!(f32),
        SampleFormat::F64 => build!(f64),
        other => Err(format!("Unsupported output sample format: {other}")),
    }
}

fn build_output_stream_typed<T>(
    device: &Device,
    config: &StreamConfig,
    mut processor: PlaybackProcessor,
    mut audio_rx: mpsc::Receiver<Vec<i16>>,
    cancel: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let channels = config.channels.max(1) as usize;
    let callback_cancel = cancel.clone();
    let error_cancel = cancel;
    device
        .build_output_stream(
            config,
            move |output: &mut [T], _| {
                if callback_cancel.load(Ordering::Acquire) {
                    for sample in output {
                        *sample = T::from_sample(0.0);
                    }
                    return;
                }
                processor.ingest_available(&mut audio_rx);
                for frame in output.chunks_mut(channels) {
                    let value = processor.next_sample();
                    for sample in frame {
                        *sample = T::from_sample(value);
                    }
                }
            },
            move |stream_error| {
                set_first_error(&error, format!("Audio output failed: {stream_error}"));
                error_cancel.store(true, Ordering::Release);
            },
            None,
        )
        .map_err(|error| format!("Could not open audio output: {error}"))
}

fn select_input_device(host: &cpal::Host, requested_id: Option<&str>) -> Result<Device, String> {
    if let Some(requested_id) = requested_id {
        let device = find_device_by_stable_id(host, requested_id, "input")?;
        choose_input_config(&device)?;
        return Ok(device);
    }
    host.default_input_device()
        .ok_or_else(|| "No default microphone is available".to_string())
}

fn select_output_device(host: &cpal::Host, requested_id: Option<&str>) -> Result<Device, String> {
    if let Some(requested_id) = requested_id {
        let device = find_device_by_stable_id(host, requested_id, "output")?;
        choose_output_config(&device)?;
        return Ok(device);
    }
    host.default_output_device()
        .ok_or_else(|| "No default audio output is available".to_string())
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn select_monitor_input(host: &cpal::Host, requested_id: Option<&str>) -> Result<Device, String> {
    if let Some(requested_id) = requested_id {
        let device = find_device_by_stable_id(host, requested_id, "system-audio")?;
        if !is_monitor_name(&device_name(&device)) {
            return Err("Selected endpoint is not a system-audio monitor".to_string());
        }
        choose_input_config(&device)?;
        return Ok(device);
    }
    let mut monitors = host
        .input_devices()
        .map_err(|error| format!("Could not enumerate system-audio monitors: {error}"))?
        .filter(|device| is_monitor_name(&device_name(device)))
        .collect::<Vec<_>>();
    monitors.sort_by_key(|device| device_name(device).to_lowercase());
    monitors.into_iter().next().ok_or_else(|| {
        "No system-audio monitor is available. Enable a PipeWire/PulseAudio monitor on Linux or install a virtual loopback device on macOS."
            .to_string()
    })
}

#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
fn select_monitor_input(_host: &cpal::Host, _requested_id: Option<&str>) -> Result<Device, String> {
    Err("Native system audio never uses the CPAL monitor selector".to_string())
}

fn find_device_by_stable_id(
    host: &cpal::Host,
    requested_id: &str,
    direction: &str,
) -> Result<Device, String> {
    let parsed = requested_id
        .parse::<cpal::DeviceId>()
        .map_err(|_| format!("Invalid {direction} device ID"))?;
    host.device_by_id(&parsed).ok_or_else(|| {
        format!("Selected {direction} device is no longer available: {requested_id}")
    })
}

fn choose_input_config(device: &Device) -> Result<(SupportedStreamConfig, SampleFormat), String> {
    let supported = match device.default_input_config() {
        Ok(config) => config,
        Err(_) => device
            .supported_input_configs()
            .map_err(|error| format!("Could not query audio input formats: {error}"))?
            .next()
            .map(|config| config.with_max_sample_rate())
            .ok_or_else(|| "Audio input has no supported configuration".to_string())?,
    };
    let sample_format = supported.sample_format();
    Ok((supported, sample_format))
}

fn choose_output_config(device: &Device) -> Result<(SupportedStreamConfig, SampleFormat), String> {
    let supported = match device.default_output_config() {
        Ok(config) => config,
        Err(_) => device
            .supported_output_configs()
            .map_err(|error| format!("Could not query audio output formats: {error}"))?
            .next()
            .map(|config| config.with_max_sample_rate())
            .ok_or_else(|| "Audio output has no supported configuration".to_string())?,
    };
    let sample_format = supported.sample_format();
    Ok((supported, sample_format))
}

fn endpoint_from_input(device: &Device, default_id: Option<&str>) -> Result<AudioEndpoint, String> {
    let (config, _) = choose_input_config(device)?;
    endpoint_from_config(device, &config, default_id)
}

fn endpoint_from_output(
    device: &Device,
    default_id: Option<&str>,
) -> Result<AudioEndpoint, String> {
    let (config, _) = choose_output_config(device)?;
    endpoint_from_config(device, &config, default_id)
}

fn endpoint_from_config(
    device: &Device,
    config: &SupportedStreamConfig,
    default_id: Option<&str>,
) -> Result<AudioEndpoint, String> {
    let id = device
        .id()
        .map_err(|error| format!("Could not read audio endpoint ID: {error}"))?
        .to_string();
    Ok(AudioEndpoint {
        is_default: default_id == Some(id.as_str()),
        id,
        name: device_name(device),
        channels: config.channels(),
        sample_rate: config.sample_rate(),
        monitored_output_id: None,
        excludes_current_process_audio: false,
    })
}

fn device_name(device: &Device) -> String {
    device
        .description()
        .map(|description| description.name().to_string())
        .unwrap_or_else(|_| "Unknown audio endpoint".to_string())
}

fn normalize_requested_id(value: Option<&str>) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_control) {
        return Err("Invalid audio endpoint ID".to_string());
    }
    if is_native_system_endpoint_id(value) {
        return Ok(Some(value.to_string()));
    }
    // Parsing here rejects display names and prevents an explicit selection
    // from silently falling back to a default endpoint.
    value
        .parse::<cpal::DeviceId>()
        .map_err(|_| "Invalid audio endpoint ID".to_string())?;
    Ok(Some(value.to_string()))
}

fn is_native_system_endpoint_id(value: &str) -> bool {
    #[cfg(windows)]
    if value == WINDOWS_PROCESS_LOOPBACK_ID {
        return true;
    }
    #[cfg(target_os = "macos")]
    if value == MACOS_SYSTEM_AUDIO_ID {
        return true;
    }
    #[cfg(target_os = "linux")]
    if value.starts_with(LINUX_DEFAULT_MONITOR_PREFIX) {
        return true;
    }
    false
}

#[cfg(target_os = "macos")]
pub(super) fn validate_macos_system_id(value: Option<&str>) -> Result<(), String> {
    match value {
        None | Some(MACOS_SYSTEM_AUDIO_ID) => Ok(()),
        Some(_) => Err("Selected macOS system-audio source is invalid".to_string()),
    }
}

#[cfg(target_os = "linux")]
pub(super) fn validate_linux_monitor_id(value: Option<&str>) -> Result<(), String> {
    match value {
        None => Ok(()),
        Some(value) if value.starts_with(LINUX_DEFAULT_MONITOR_PREFIX) => Ok(()),
        Some(_) => Err("Selected Linux system-audio source is invalid".to_string()),
    }
}

pub(crate) fn system_capture_output_match(
    system_input: Option<&str>,
    output: Option<&str>,
) -> Option<bool> {
    let _ = output;
    let input = system_input?;
    #[cfg(windows)]
    if input == WINDOWS_PROCESS_LOOPBACK_ID {
        return Some(false);
    }
    #[cfg(target_os = "macos")]
    if input == MACOS_SYSTEM_AUDIO_ID {
        return Some(false);
    }
    #[cfg(target_os = "linux")]
    if let Some(monitored_output) = input.strip_prefix(LINUX_DEFAULT_MONITOR_PREFIX) {
        return Some(match output {
            None => true,
            Some(_) if monitored_output.is_empty() => true,
            Some(output) => output == monitored_output,
        });
    }
    None
}

fn push_unique_endpoint(endpoints: &mut Vec<AudioEndpoint>, endpoint: AudioEndpoint) {
    if !endpoints.iter().any(|existing| existing.id == endpoint.id) {
        endpoints.push(endpoint);
    }
}

fn sort_endpoints(endpoints: &mut [AudioEndpoint]) {
    endpoints.sort_by(|left, right| {
        right
            .is_default
            .cmp(&left.is_default)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.id.cmp(&right.id))
    });
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn is_monitor_name(name: &str) -> bool {
    let name = name.to_lowercase();
    [
        "monitor",
        "loopback",
        "blackhole",
        "soundflower",
        "vb-audio",
        "pipewire",
        "pulse",
    ]
    .iter()
    .any(|marker| name.contains(marker))
}

fn set_first_error(slot: &Arc<Mutex<Option<String>>>, message: String) {
    if let Ok(mut slot) = slot.lock() {
        if slot.is_none() {
            *slot = Some(message);
        }
    }
}

fn peek_error(slot: &Arc<Mutex<Option<String>>>) -> Option<String> {
    slot.lock().ok().and_then(|slot| slot.clone())
}

fn take_error(slot: &Arc<Mutex<Option<String>>>) -> Option<String> {
    slot.lock().ok().and_then(|mut slot| slot.take())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_ids_must_be_native_cpal_ids() {
        assert!(normalize_requested_id(None).unwrap().is_none());
        assert!(normalize_requested_id(Some("Microphone name")).is_err());
        let id = format!("{}:native-id", cpal::default_host().id());
        assert!(normalize_requested_id(Some(&id)).unwrap().is_some());
        let whitespace = format!(" {id}");
        assert!(normalize_requested_id(Some(&whitespace)).is_err());
    }

    #[test]
    fn endpoint_sorting_keeps_default_first_and_is_deterministic() {
        let mut endpoints = vec![
            AudioEndpoint {
                id: "host:b".to_string(),
                name: "Zulu".to_string(),
                is_default: false,
                channels: 2,
                sample_rate: 48_000,
                monitored_output_id: None,
                excludes_current_process_audio: false,
            },
            AudioEndpoint {
                id: "host:a".to_string(),
                name: "Alpha".to_string(),
                is_default: true,
                channels: 1,
                sample_rate: 48_000,
                monitored_output_id: None,
                excludes_current_process_audio: false,
            },
        ];
        sort_endpoints(&mut endpoints);
        assert!(endpoints[0].is_default);
        assert_eq!(endpoints[1].name, "Zulu");
    }
}
