use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Duration;

use tokio::sync::mpsc::{Sender, UnboundedSender};
use windows::core::{Interface, HRESULT};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

use super::audio_processing::{decode_packed_audio, PackedAudioFormat};
use super::{CapturePcmSink, WINDOWS_PROCESS_LOOPBACK_ID};

const WAVE_FORMAT_PCM: u16 = 0x0001;
const AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY_VALUE: u32 = 0x1;
const AUDCLNT_BUFFERFLAGS_SILENT_VALUE: u32 = 0x2;
const ACTIVATION_TIMEOUT: Duration = Duration::from_secs(5);
const PROCESS_LOOPBACK_SAMPLE_RATE: u32 = 48_000;
const PROCESS_LOOPBACK_CHANNELS: u16 = 2;
const PROCESS_LOOPBACK_BITS_PER_SAMPLE: u16 = 16;

unsafe extern "C" {
    fn ort_activate_process_loopback(
        excluded_process_id: u32,
        timeout_ms: u32,
        audio_client: *mut *mut std::ffi::c_void,
        activation_context: *mut *mut std::ffi::c_void,
    ) -> HRESULT;
    fn ort_release_process_loopback(activation_context: *mut std::ffi::c_void);
}

struct ProcessLoopbackClient {
    audio_client: IAudioClient,
    activation_context: *mut std::ffi::c_void,
}

impl Drop for ProcessLoopbackClient {
    fn drop(&mut self) {
        unsafe { ort_release_process_loopback(self.activation_context) };
    }
}

pub(super) fn run_wasapi_loopback(
    endpoint_id: Option<&str>,
    audio_tx: Sender<Vec<i16>>,
    level_tx: UnboundedSender<f32>,
    cancel: Arc<AtomicBool>,
    ready_tx: mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    // This worker owns COM for its complete lifetime. It is always spawned on
    // a dedicated native thread by audio::start_capture.
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|error| format!("Could not initialize COM for system audio: {error}"))?;
    }
    let result = unsafe {
        run_wasapi_loopback_initialized(endpoint_id, audio_tx, level_tx, &cancel, ready_tx)
    };
    unsafe { CoUninitialize() };
    result
}

unsafe fn run_wasapi_loopback_initialized(
    endpoint_id: Option<&str>,
    audio_tx: Sender<Vec<i16>>,
    level_tx: UnboundedSender<f32>,
    cancel: &Arc<AtomicBool>,
    ready_tx: mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    validate_process_loopback_id(endpoint_id)?;
    let process_loopback = unsafe { activate_process_loopback_client(std::process::id()) }
        .map_err(|error| format!("Could not activate Windows system-audio exclusion: {error}"))?;
    let audio_client = &process_loopback.audio_client;
    let block_align = PROCESS_LOOPBACK_CHANNELS * PROCESS_LOOPBACK_BITS_PER_SAMPLE / 8;
    let capture_wave_format = WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM,
        nChannels: PROCESS_LOOPBACK_CHANNELS,
        nSamplesPerSec: PROCESS_LOOPBACK_SAMPLE_RATE,
        nAvgBytesPerSec: PROCESS_LOOPBACK_SAMPLE_RATE * u32::from(block_align),
        nBlockAlign: block_align,
        wBitsPerSample: PROCESS_LOOPBACK_BITS_PER_SAMPLE,
        cbSize: 0,
    };
    let format = PackedAudioFormat {
        channels: PROCESS_LOOPBACK_CHANNELS,
        sample_rate: PROCESS_LOOPBACK_SAMPLE_RATE,
        block_align,
        container_bits: PROCESS_LOOPBACK_BITS_PER_SAMPLE,
        valid_bits: PROCESS_LOOPBACK_BITS_PER_SAMPLE,
    };
    unsafe {
        audio_client
            .Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                0,
                0,
                &capture_wave_format,
                None,
            )
            .map_err(|error| format!("Could not initialize WASAPI loopback: {error}"))?;
    }
    let capture_client: IAudioCaptureClient = unsafe { audio_client.GetService() }
        .map_err(|error| format!("Could not create the WASAPI capture client: {error}"))?;
    unsafe { audio_client.Start() }
        .map_err(|error| format!("Could not start WASAPI loopback: {error}"))?;

    let mut sink = CapturePcmSink::new(format.sample_rate, audio_tx, level_tx);
    let _ = ready_tx.send(Ok(()));
    let capture_result = unsafe { capture_packets(&capture_client, cancel, format, &mut sink) };
    let stop_result = unsafe { audio_client.Stop() }
        .map_err(|error| format!("Could not stop WASAPI loopback: {error}"));
    sink.finish();
    capture_result.and(stop_result)
}

unsafe fn capture_packets(
    capture_client: &IAudioCaptureClient,
    cancel: &Arc<AtomicBool>,
    format: PackedAudioFormat,
    sink: &mut CapturePcmSink,
) -> Result<(), String> {
    while !cancel.load(Ordering::Acquire) {
        let mut packet_frames = unsafe { capture_client.GetNextPacketSize() }
            .map_err(|error| format!("WASAPI could not query its next packet: {error}"))?;
        if packet_frames == 0 {
            thread::sleep(Duration::from_millis(5));
            continue;
        }

        while packet_frames > 0 && !cancel.load(Ordering::Acquire) {
            let mut data_ptr = std::ptr::null_mut();
            let mut available_frames = 0_u32;
            let mut flags = 0_u32;
            let mut device_position = 0_u64;
            let mut qpc_position = 0_u64;
            unsafe {
                capture_client.GetBuffer(
                    &mut data_ptr,
                    &mut available_frames,
                    &mut flags,
                    Some(&mut device_position),
                    Some(&mut qpc_position),
                )
            }
            .map_err(|error| format!("WASAPI could not acquire a capture packet: {error}"))?;

            // ReleaseBuffer is paired with every successful GetBuffer, even
            // when the packet is silent or decoding fails.
            let silent = flags & AUDCLNT_BUFFERFLAGS_SILENT_VALUE != 0;
            let decoded = if available_frames == 0 {
                Ok(Vec::new())
            } else if silent {
                decode_packed_audio(&[], available_frames as usize, format, true)
            } else if data_ptr.is_null() {
                Err("WASAPI returned a null non-silent packet".to_string())
            } else {
                match (available_frames as usize).checked_mul(format.block_align as usize) {
                    Some(byte_len) => {
                        let bytes = unsafe { std::slice::from_raw_parts(data_ptr, byte_len) };
                        decode_packed_audio(bytes, available_frames as usize, format, false)
                    }
                    None => Err("WASAPI packet size overflow".to_string()),
                }
            };
            let release_result = unsafe { capture_client.ReleaseBuffer(available_frames) }
                .map_err(|error| format!("WASAPI could not release a capture packet: {error}"));
            let samples = decoded?;
            release_result?;

            if flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY_VALUE != 0 {
                // A discontinuity is recoverable. The streaming resampler keeps
                // bounded state and resumes at the next packet.
            }
            if !samples.is_empty() {
                sink.push_interleaved(&samples, format.channels);
            }
            packet_frames = unsafe { capture_client.GetNextPacketSize() }
                .map_err(|error| format!("WASAPI could not drain capture packets: {error}"))?;
        }
    }
    Ok(())
}

fn validate_process_loopback_id(stable_id: Option<&str>) -> Result<(), String> {
    if stable_id.is_none_or(|value| value == WINDOWS_PROCESS_LOOPBACK_ID) {
        Ok(())
    } else {
        Err("Selected system-audio source is no longer available".to_string())
    }
}

unsafe fn activate_process_loopback_client(
    process_id: u32,
) -> windows::core::Result<ProcessLoopbackClient> {
    let mut audio_client = std::ptr::null_mut();
    let mut activation_context = std::ptr::null_mut();
    let result = unsafe {
        ort_activate_process_loopback(
            process_id,
            ACTIVATION_TIMEOUT.as_millis() as u32,
            &mut audio_client,
            &mut activation_context,
        )
    };
    result.ok()?;
    if audio_client.is_null() || activation_context.is_null() {
        if !activation_context.is_null() {
            unsafe { ort_release_process_loopback(activation_context) };
        }
        return Err(windows::core::Error::new(
            AUDCLNT_E_DEVICE_INVALIDATED,
            "Windows did not return a process-loopback audio client",
        ));
    }
    Ok(ProcessLoopbackClient {
        audio_client: unsafe { IAudioClient::from_raw(audio_client) },
        activation_context,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_loopback_only_accepts_the_virtual_system_source() {
        assert!(validate_process_loopback_id(None).is_ok());
        assert!(validate_process_loopback_id(Some(WINDOWS_PROCESS_LOOPBACK_ID)).is_ok());
        assert!(validate_process_loopback_id(Some("wasapi:speakers")).is_err());
    }
}
