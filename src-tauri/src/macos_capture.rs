use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc as std_mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use screencapturekit::prelude::*;
use tokio::sync::mpsc::{Sender, UnboundedSender};

use super::CapturePcmSink;

const SAMPLE_RATE: u32 = 48_000;
const CHANNELS: u16 = 2;

pub(super) fn run_system_audio(
    endpoint_id: Option<&str>,
    audio_tx: Sender<Vec<i16>>,
    level_tx: UnboundedSender<f32>,
    cancel: Arc<AtomicBool>,
    ready_tx: std_mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    super::validate_macos_system_id(endpoint_id)?;

    let content = SCShareableContent::get().map_err(|error| {
        format!(
            "Could not access macOS system audio: {error}. Allow Screen & System Audio Recording in Privacy & Security, then restart the app."
        )
    })?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .ok_or_else(|| "macOS did not expose a display for system-audio capture".to_string())?;
    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&[])
        .build();
    let configuration = SCStreamConfiguration::new()
        .with_width(2)
        .with_height(2)
        .with_captures_audio(true)
        .with_excludes_current_process_audio(true)
        .with_sample_rate(SAMPLE_RATE as i32)
        .with_channel_count(CHANNELS as i32);

    let sink = Arc::new(Mutex::new(CapturePcmSink::new(
        SAMPLE_RATE,
        audio_tx,
        level_tx,
    )));
    let callback_sink = sink.clone();
    let callback_cancel = cancel.clone();
    let mut stream = SCStream::new(&filter, &configuration);
    let handler_id = stream.add_output_handler(
        move |sample: CMSampleBuffer, output_type: SCStreamOutputType| {
            if output_type != SCStreamOutputType::Audio
                || callback_cancel.load(Ordering::Acquire)
                || !sample.is_valid()
            {
                return;
            }
            let Some(samples) = sample_buffer_samples(&sample) else {
                return;
            };
            if let Ok(mut sink) = callback_sink.try_lock() {
                sink.push_interleaved(&samples, CHANNELS);
            }
        },
        SCStreamOutputType::Audio,
    );
    if handler_id.is_none() {
        return Err("macOS rejected the system-audio stream output".to_string());
    }
    stream.start_capture().map_err(|error| {
        format!(
            "Could not start macOS system audio: {error}. Allow Screen & System Audio Recording in Privacy & Security, then restart the app."
        )
    })?;
    let _ = ready_tx.send(Ok(()));

    while !cancel.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(20));
    }
    stream
        .stop_capture()
        .map_err(|error| format!("Could not stop macOS system audio: {error}"))?;
    if let Ok(mut sink) = sink.lock() {
        sink.finish();
    }
    Ok(())
}

fn sample_buffer_samples(sample: &CMSampleBuffer) -> Option<Vec<f32>> {
    let description = sample.format_description()?;
    let channel_count = description.audio_channel_count()? as usize;
    if channel_count != CHANNELS as usize || description.audio_sample_rate()? as u32 != SAMPLE_RATE
    {
        return None;
    }
    let bits = description.audio_bits_per_channel()?;
    let is_float = description.audio_is_float();
    let is_big_endian = description.audio_is_big_endian();
    let buffers = sample.audio_buffer_list()?;

    if buffers.num_buffers() == 1 {
        let buffer = buffers.buffer(0)?;
        return decode_pcm(buffer.data(), bits, is_float, is_big_endian);
    }

    let channels = (0..channel_count)
        .map(|index| {
            let buffer = buffers.buffer(index)?;
            decode_pcm(buffer.data(), bits, is_float, is_big_endian)
        })
        .collect::<Option<Vec<_>>>()?;
    let frames = channels.iter().map(Vec::len).min()?;
    let mut interleaved = Vec::with_capacity(frames * channel_count);
    for frame in 0..frames {
        for channel in &channels {
            interleaved.push(channel[frame]);
        }
    }
    Some(interleaved)
}

fn decode_pcm(bytes: &[u8], bits: u32, is_float: bool, is_big_endian: bool) -> Option<Vec<f32>> {
    match (is_float, bits) {
        (true, 32) => Some(
            bytes
                .chunks_exact(4)
                .map(|value| {
                    let raw = [value[0], value[1], value[2], value[3]];
                    if is_big_endian {
                        f32::from_be_bytes(raw)
                    } else {
                        f32::from_le_bytes(raw)
                    }
                })
                .collect(),
        ),
        (false, 16) => Some(
            bytes
                .chunks_exact(2)
                .map(|value| {
                    let raw = [value[0], value[1]];
                    let sample = if is_big_endian {
                        i16::from_be_bytes(raw)
                    } else {
                        i16::from_le_bytes(raw)
                    };
                    f32::from(sample) / f32::from(i16::MAX)
                })
                .collect(),
        ),
        _ => None,
    }
}
