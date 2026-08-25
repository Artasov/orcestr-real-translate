use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex};

use libpulse_binding::sample::{Format, Spec};
use libpulse_binding::stream::Direction;
use libpulse_simple_binding::Simple;
use tokio::sync::mpsc::{Sender, UnboundedSender};

use super::{normalize_i16, CapturePcmSink};

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u8 = 2;
const READ_FRAMES: usize = 960;

pub(super) fn run_default_monitor(
    endpoint_id: Option<&str>,
    audio_tx: Sender<Vec<i16>>,
    level_tx: UnboundedSender<f32>,
    cancel: Arc<AtomicBool>,
    ready_tx: std_mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    super::validate_linux_monitor_id(endpoint_id)?;

    let spec = Spec {
        format: Format::S16le,
        channels: CHANNELS,
        rate: SAMPLE_RATE,
    };
    if !spec.is_valid() {
        return Err("PulseAudio rejected the system-audio sample format".to_string());
    }

    let monitor = Simple::new(
        None,
        "Orcestr Real Translate",
        Direction::Record,
        Some("@DEFAULT_MONITOR@"),
        "System audio translation",
        &spec,
        None,
        None,
    )
    .map_err(|error| {
        format!(
            "Could not open the default PipeWire/PulseAudio monitor: {error}. Ensure PipeWire Pulse or PulseAudio is running."
        )
    })?;

    let sink = Arc::new(Mutex::new(CapturePcmSink::new(
        SAMPLE_RATE,
        audio_tx,
        level_tx,
    )));
    let _ = ready_tx.send(Ok(()));
    let mut bytes = vec![0_u8; READ_FRAMES * CHANNELS as usize * size_of::<i16>()];

    while !cancel.load(Ordering::Acquire) {
        monitor
            .read(&mut bytes)
            .map_err(|error| format!("PipeWire/PulseAudio system capture failed: {error}"))?;
        let samples = bytes
            .chunks_exact(2)
            .map(|pair| normalize_i16(i16::from_le_bytes([pair[0], pair[1]])))
            .collect::<Vec<_>>();
        if let Ok(mut sink) = sink.lock() {
            sink.push_interleaved(&samples, CHANNELS.into());
        }
    }

    if let Ok(mut sink) = sink.lock() {
        sink.finish();
    }
    Ok(())
}
