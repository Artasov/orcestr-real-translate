use std::f64::consts::PI;

use nnnoiseless::DenoiseState;

pub(crate) const REALTIME_SAMPLE_RATE: u32 = 24_000;
const DENOISE_SAMPLE_RATE: u32 = 48_000;
const LOW_PASS_TAPS: usize = 127;
const SPEECH_HIGH_PASS_HZ: f32 = 65.0;
const AGC_TARGET_RMS: f32 = 0.12;
const AGC_ACTIVE_FLOOR_RMS: f32 = 0.00035;
const AGC_MIN_GAIN: f32 = 0.35;
const AGC_MAX_GAIN: f32 = 24.0;
const INITIAL_NOISE_FLOOR_RMS: f32 = 0.001;
const MIN_NOISE_FLOOR_RMS: f32 = 0.000_01;
const MAX_NOISE_FLOOR_RMS: f32 = 0.04;
const VOICE_PROBABILITY_OPEN: f32 = 0.18;
const VOICE_PROBABILITY_POSSIBLE: f32 = 0.08;
const SPEECH_TO_NOISE_OPEN_RATIO: f32 = 1.65;
const SPEECH_HOLD_SECONDS: f32 = 0.22;
const COMPRESSOR_THRESHOLD_DB: f32 = -15.0;
const COMPRESSOR_RATIO: f32 = 3.5;
const LIMITER_START: f32 = 0.82;
const LIMITER_CEILING: f32 = 0.98;

/// Speech-oriented capture enhancement. Audio is first converted to the 48 kHz
/// frame format required by RNNoise. Its recurrent voice-aware suppressor
/// removes stationary noise before the adaptive gain stage, so the AGC lifts
/// distant speech instead of normalizing the room's noise floor.
pub(crate) struct SpeechDynamicsProcessor {
    source_resampler: StreamingMonoResampler,
    denoiser: Box<DenoiseState<'static>>,
    denoise_pending: Vec<f32>,
    sample_rate: f32,
    high_pass_coefficient: f32,
    previous_input: f32,
    previous_high_pass: f32,
    gain: f32,
    gate_gain: f32,
    noise_floor_rms: f32,
    speech_hold_samples: usize,
}

impl SpeechDynamicsProcessor {
    pub fn new(source_rate: u32) -> Self {
        let sample_rate = DENOISE_SAMPLE_RATE as f32;
        let high_pass_coefficient =
            (-2.0 * std::f32::consts::PI * SPEECH_HIGH_PASS_HZ / sample_rate).exp();
        Self {
            source_resampler: StreamingMonoResampler::new(source_rate, DENOISE_SAMPLE_RATE),
            denoiser: DenoiseState::new(),
            denoise_pending: Vec::with_capacity(DenoiseState::FRAME_SIZE * 2),
            sample_rate,
            high_pass_coefficient,
            previous_input: 0.0,
            previous_high_pass: 0.0,
            gain: 1.0,
            gate_gain: 0.0,
            noise_floor_rms: INITIAL_NOISE_FLOOR_RMS,
            speech_hold_samples: 0,
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }

        let resampled = self.source_resampler.process(input);
        self.process_48khz(&resampled)
    }

    pub fn finish(&mut self) -> Vec<f32> {
        let tail = self.source_resampler.finish();
        let mut output = self.process_48khz(&tail);
        let valid_samples = self.denoise_pending.len();
        if valid_samples > 0 {
            let mut frame = std::mem::take(&mut self.denoise_pending);
            frame.resize(DenoiseState::FRAME_SIZE, 0.0);
            let enhanced = self.denoise_frame(&frame);
            output.extend(enhanced.into_iter().take(valid_samples));
        }
        output
    }

    fn process_48khz(&mut self, input: &[f32]) -> Vec<f32> {
        let mut buffered = std::mem::take(&mut self.denoise_pending);
        buffered.extend(input.iter().copied().map(sanitize_sample));
        let complete_samples =
            (buffered.len() / DenoiseState::FRAME_SIZE) * DenoiseState::FRAME_SIZE;
        let mut output = Vec::with_capacity(complete_samples);
        for frame in buffered[..complete_samples].chunks_exact(DenoiseState::FRAME_SIZE) {
            output.extend(self.denoise_frame(frame));
        }
        self.denoise_pending
            .extend_from_slice(&buffered[complete_samples..]);
        output
    }

    fn denoise_frame(&mut self, frame: &[f32]) -> Vec<f32> {
        let input = frame
            .iter()
            .map(|sample| sanitize_sample(*sample).clamp(-1.0, 1.0) * i16::MAX as f32)
            .collect::<Vec<_>>();
        let mut denoised = vec![0.0; DenoiseState::FRAME_SIZE];
        let voice_probability = self.denoiser.process_frame(&mut denoised, &input);
        for sample in &mut denoised {
            *sample = sanitize_sample(*sample / i16::MAX as f32).clamp(-1.0, 1.0);
        }
        self.process_dynamics_frame(&denoised, voice_probability)
    }

    fn process_dynamics_frame(&mut self, input: &[f32], voice_probability: f32) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }

        let mut filtered = Vec::with_capacity(input.len());
        for sample in input {
            let sample = sanitize_sample(*sample).clamp(-1.0, 1.0);
            let high_pass = sanitize_sample(
                sample - self.previous_input + self.high_pass_coefficient * self.previous_high_pass,
            );
            self.previous_input = sample;
            self.previous_high_pass = high_pass;
            filtered.push(high_pass.clamp(-1.0, 1.0));
        }

        let level = rms(&filtered);
        let duration_seconds = filtered.len() as f32 / self.sample_rate;
        let voice_probability = sanitize_sample(voice_probability).clamp(0.0, 1.0);

        // RNNoise's VAD is the strongest signal. During non-voice frames the
        // residual RMS continuously calibrates the local noise floor. A low
        // VAD probability plus a clear SNR rise still opens for distant speech.
        if voice_probability < VOICE_PROBABILITY_POSSIBLE {
            let noise_time_seconds = if level < self.noise_floor_rms {
                0.08
            } else {
                0.45
            };
            let noise_blend = 1.0 - (-duration_seconds / noise_time_seconds).exp();
            self.noise_floor_rms += (level - self.noise_floor_rms) * noise_blend;
            self.noise_floor_rms = self
                .noise_floor_rms
                .clamp(MIN_NOISE_FLOOR_RMS, MAX_NOISE_FLOOR_RMS);
        }
        let above_noise = level >= AGC_ACTIVE_FLOOR_RMS
            && level >= self.noise_floor_rms * SPEECH_TO_NOISE_OPEN_RATIO;
        let detected_voice = voice_probability >= VOICE_PROBABILITY_OPEN
            || (voice_probability >= VOICE_PROBABILITY_POSSIBLE && above_noise);
        if detected_voice {
            self.speech_hold_samples = (self.sample_rate * SPEECH_HOLD_SECONDS) as usize;
        } else {
            self.speech_hold_samples = self.speech_hold_samples.saturating_sub(filtered.len());
        }
        let active = detected_voice || self.speech_hold_samples > 0;
        let desired_gain = if active {
            (AGC_TARGET_RMS / level.max(AGC_ACTIVE_FLOOR_RMS)).clamp(AGC_MIN_GAIN, AGC_MAX_GAIN)
        } else {
            1.0
        };
        let gain_time_seconds = if desired_gain < self.gain {
            0.025
        } else if active {
            0.090
        } else {
            0.250
        };
        let gain_blend = 1.0 - (-duration_seconds / gain_time_seconds).exp();
        let start_gain = self.gain;
        self.gain += (desired_gain - self.gain) * gain_blend;

        let gate_target = if active { 1.0 } else { 0.0 };
        let gate_time_seconds = if gate_target > self.gate_gain {
            0.006
        } else {
            0.080
        };
        let gate_step = 1.0 - (-1.0 / (self.sample_rate * gate_time_seconds)).exp();

        let mut output = Vec::with_capacity(filtered.len());
        let sample_count = filtered.len() as f32;
        for (index, sample) in filtered.into_iter().enumerate() {
            let progress = (index + 1) as f32 / sample_count;
            let gain = start_gain + (self.gain - start_gain) * progress;
            self.gate_gain += (gate_target - self.gate_gain) * gate_step;
            let normalized = sample * gain * self.gate_gain;
            output.push(soft_limit(compress_sample(normalized)));
        }
        output
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PackedAudioFormat {
    pub channels: u16,
    pub sample_rate: u32,
    pub block_align: u16,
    pub container_bits: u16,
    pub valid_bits: u16,
}

/// Stateful mono resampler. Downsampling uses a Blackman-windowed low-pass
/// filter so content above the destination Nyquist frequency is not folded
/// back into speech. The interpolation phase is preserved between packets.
#[derive(Debug)]
pub(crate) struct StreamingMonoResampler {
    source_rate: u32,
    target_rate: u32,
    previous_sample: Option<f32>,
    next_source_position: f64,
    low_pass: Option<Vec<f64>>,
    low_pass_history: Vec<f32>,
}

impl StreamingMonoResampler {
    pub fn new(source_rate: u32, target_rate: u32) -> Self {
        let source_rate = source_rate.max(1);
        let target_rate = target_rate.max(1);
        let low_pass = create_low_pass(source_rate, target_rate);
        let low_pass_history = low_pass
            .as_ref()
            .map(|filter| vec![0.0; filter.len().saturating_sub(1)])
            .unwrap_or_default();
        Self {
            source_rate,
            target_rate,
            previous_sample: None,
            next_source_position: 0.0,
            low_pass,
            low_pass_history,
        }
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if input.is_empty() {
            return Vec::new();
        }
        let filtered = self.apply_low_pass(input);
        let has_previous = self.previous_sample.is_some();
        let data_len = filtered.len() + usize::from(has_previous);
        let previous = self.previous_sample.unwrap_or_default();
        let sample_at = |index: usize| -> f32 {
            if has_previous {
                if index == 0 {
                    previous
                } else {
                    filtered[index - 1]
                }
            } else {
                filtered[index]
            }
        };
        let step = self.source_rate as f64 / self.target_rate as f64;
        let mut output = Vec::with_capacity(
            ((input.len() as u64 * self.target_rate as u64) / self.source_rate as u64 + 2) as usize,
        );

        while self.next_source_position < data_len.saturating_sub(1) as f64 {
            let first = self.next_source_position.floor() as usize;
            let second = (first + 1).min(data_len - 1);
            let fraction = (self.next_source_position - first as f64) as f32;
            let a = sample_at(first);
            let b = sample_at(second);
            output.push(sanitize_sample(a + (b - a) * fraction));
            self.next_source_position += step;
        }

        self.next_source_position -= data_len.saturating_sub(1) as f64;
        self.previous_sample = filtered.last().copied();
        output
    }

    /// Emits the sub-sample tail necessary to preserve captured duration.
    pub fn finish(&mut self) -> Vec<f32> {
        let Some(previous) = self.previous_sample else {
            return Vec::new();
        };
        let step = self.source_rate as f64 / self.target_rate as f64;
        let mut output = Vec::new();
        while self.next_source_position < 1.0 - 1e-9 {
            output.push(previous);
            self.next_source_position += step;
        }
        self.previous_sample = None;
        self.next_source_position = 0.0;
        if !self.low_pass_history.is_empty() {
            self.low_pass_history.fill(0.0);
        }
        output
    }

    fn apply_low_pass(&mut self, input: &[f32]) -> Vec<f32> {
        let Some(filter) = self.low_pass.as_ref() else {
            return input.iter().copied().map(sanitize_sample).collect();
        };
        let history_len = filter.len() - 1;
        let mut samples = Vec::with_capacity(history_len + input.len());
        samples.extend_from_slice(&self.low_pass_history);
        samples.extend(input.iter().copied().map(sanitize_sample));

        let mut output = Vec::with_capacity(input.len());
        for input_index in 0..input.len() {
            let newest = history_len + input_index;
            let mut filtered = 0.0_f64;
            for (tap, coefficient) in filter.iter().enumerate() {
                filtered += coefficient * samples[newest - tap] as f64;
            }
            output.push(sanitize_sample(filtered as f32));
        }
        self.low_pass_history
            .copy_from_slice(&samples[samples.len() - history_len..]);
        output
    }
}

pub(crate) fn downmix_interleaved_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    let channels = channels.max(1) as usize;
    let mut mono = Vec::with_capacity(samples.len() / channels);
    for frame in samples.chunks_exact(channels) {
        let sample = match channels {
            1 => frame[0],
            2 => (frame[0] + frame[1]) * 0.5,
            _ => {
                // Keep dialogue-heavy L/R/C channels prominent while folding
                // remaining surround channels in at a bounded weight.
                let mut sum = frame[0] * 0.3 + frame[1] * 0.3 + frame[2] * 0.4;
                let mut weight = 1.0_f32;
                for value in frame.iter().skip(3) {
                    sum += *value * 0.15;
                    weight += 0.15;
                }
                sum / weight
            }
        };
        mono.push(sanitize_sample(sample).clamp(-1.0, 1.0));
    }
    mono
}

pub(crate) fn quantize_i16(samples: &[f32]) -> Vec<i16> {
    samples
        .iter()
        .map(|sample| {
            let value = sanitize_sample(*sample).clamp(-1.0, 1.0);
            if value <= -1.0 {
                i16::MIN
            } else {
                (value * i16::MAX as f32).round() as i16
            }
        })
        .collect()
}

pub(crate) fn normalize_i16(sample: i16) -> f32 {
    if sample < 0 {
        sample as f32 / 32_768.0
    } else {
        sample as f32 / 32_767.0
    }
}

pub(crate) fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum = samples
        .iter()
        .map(|sample| {
            let value = sanitize_sample(*sample) as f64;
            value * value
        })
        .sum::<f64>();
    (sum / samples.len() as f64).sqrt() as f32
}

/// Maps the processed RMS level onto a conventional -60 dBFS..0 dBFS meter.
/// A logarithmic meter remains readable for quiet speech without changing the
/// actual samples sent to the provider.
pub(crate) fn meter_level(samples: &[f32]) -> f32 {
    let level = rms(samples);
    if level <= 0.000_001 {
        return 0.0;
    }
    ((20.0 * level.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

fn compress_sample(sample: f32) -> f32 {
    let magnitude = sanitize_sample(sample).abs();
    if magnitude <= f32::EPSILON {
        return 0.0;
    }
    let input_db = 20.0 * magnitude.log10();
    if input_db <= COMPRESSOR_THRESHOLD_DB {
        return sample;
    }
    let output_db =
        COMPRESSOR_THRESHOLD_DB + (input_db - COMPRESSOR_THRESHOLD_DB) / COMPRESSOR_RATIO;
    sample.signum() * 10.0_f32.powf(output_db / 20.0)
}

fn soft_limit(sample: f32) -> f32 {
    let sample = sanitize_sample(sample);
    let magnitude = sample.abs();
    if magnitude <= LIMITER_START {
        return sample;
    }
    let limited = LIMITER_START
        + (1.0 - (-(magnitude - LIMITER_START) * 5.0).exp()) * (LIMITER_CEILING - LIMITER_START);
    sample.signum() * limited.min(LIMITER_CEILING)
}

pub(crate) fn decode_packed_audio(
    data: &[u8],
    frames: usize,
    format: PackedAudioFormat,
    silent: bool,
) -> Result<Vec<f32>, String> {
    let channels = format.channels.max(1) as usize;
    let block_align = format.block_align as usize;
    let bytes_per_sample = format.container_bits.div_ceil(8) as usize;
    if bytes_per_sample == 0 || block_align < channels.saturating_mul(bytes_per_sample) {
        return Err("Invalid audio block alignment".to_string());
    }
    let required = frames
        .checked_mul(block_align)
        .ok_or_else(|| "Audio packet size overflow".to_string())?;
    if !silent && data.len() < required {
        return Err("Audio packet is shorter than its declared frame count".to_string());
    }
    if silent {
        return Ok(vec![0.0; frames.saturating_mul(channels)]);
    }

    let mut samples = Vec::with_capacity(frames.saturating_mul(channels));
    for frame in 0..frames {
        let frame_offset = frame * block_align;
        for channel in 0..channels {
            let offset = frame_offset + channel * bytes_per_sample;
            let bytes = &data[offset..offset + bytes_per_sample];
            let sample = decode_pcm_integer(bytes, format.container_bits, format.valid_bits)?;
            samples.push(sanitize_sample(sample).clamp(-1.0, 1.0));
        }
    }
    Ok(samples)
}

fn create_low_pass(source_rate: u32, target_rate: u32) -> Option<Vec<f64>> {
    if source_rate <= target_rate {
        return None;
    }
    let cutoff = (target_rate as f64 / source_rate as f64) * 0.45;
    let midpoint = (LOW_PASS_TAPS - 1) as f64 / 2.0;
    let mut coefficients = Vec::with_capacity(LOW_PASS_TAPS);
    let mut sum = 0.0;
    for index in 0..LOW_PASS_TAPS {
        let distance = index as f64 - midpoint;
        let ideal = if distance == 0.0 {
            2.0 * cutoff
        } else {
            (2.0 * PI * cutoff * distance).sin() / (PI * distance)
        };
        let window = 0.42 - 0.5 * (2.0 * PI * index as f64 / (LOW_PASS_TAPS - 1) as f64).cos()
            + 0.08 * (4.0 * PI * index as f64 / (LOW_PASS_TAPS - 1) as f64).cos();
        let coefficient = ideal * window;
        coefficients.push(coefficient);
        sum += coefficient;
    }
    for coefficient in &mut coefficients {
        *coefficient /= sum;
    }
    Some(coefficients)
}

fn decode_pcm_integer(bytes: &[u8], container_bits: u16, valid_bits: u16) -> Result<f32, String> {
    if container_bits == 8 {
        return Ok((bytes[0] as f32 - 128.0) / 128.0);
    }
    if !matches!(container_bits, 16 | 24 | 32) {
        return Err(format!("Unsupported PCM container: {container_bits} bits"));
    }
    let mut raw = match container_bits {
        16 => i16::from_le_bytes(bytes.try_into().expect("validated i16 width")) as i32,
        24 => {
            let value = bytes[0] as i32 | ((bytes[1] as i32) << 8) | ((bytes[2] as i32) << 16);
            if value & 0x0080_0000 != 0 {
                value | !0x00ff_ffff
            } else {
                value
            }
        }
        32 => i32::from_le_bytes(bytes.try_into().expect("validated i32 width")),
        _ => unreachable!(),
    };
    let valid_bits = valid_bits.clamp(1, container_bits);
    let padding_bits = container_bits - valid_bits;
    if padding_bits > 0 {
        // WAVEFORMATEXTENSIBLE stores valid PCM bits left-aligned.
        raw >>= padding_bits;
    }
    let denominator = (1_u64 << (valid_bits - 1)) as f64;
    Ok((raw as f64 / denominator) as f32)
}

fn sanitize_sample(sample: f32) -> f32 {
    if sample.is_finite() {
        sample
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(resampler: &mut StreamingMonoResampler, packets: &[&[f32]]) -> Vec<f32> {
        let mut output = Vec::new();
        for packet in packets {
            output.extend(resampler.process(packet));
        }
        output.extend(resampler.finish());
        output
    }

    #[test]
    fn packeted_resampling_preserves_exact_duration() {
        let input = (0..4_800)
            .map(|index| (index as f32 * 0.037).sin())
            .collect::<Vec<_>>();
        let mut one_shot = StreamingMonoResampler::new(48_000, 24_000);
        let expected = collect(&mut one_shot, &[&input]);
        let mut packeted = StreamingMonoResampler::new(48_000, 24_000);
        let actual = collect(
            &mut packeted,
            &[&input[..317], &input[317..1_641], &input[1_641..]],
        );
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 2_400);
    }

    #[test]
    fn upsampling_preserves_packet_boundaries() {
        let input = [0.0, 0.25, 0.5, 0.75];
        let mut one_shot = StreamingMonoResampler::new(24_000, 48_000);
        let expected = collect(&mut one_shot, &[&input]);
        let mut packeted = StreamingMonoResampler::new(24_000, 48_000);
        let actual = collect(&mut packeted, &[&input[..2], &input[2..]]);
        assert_eq!(actual, expected);
        assert_eq!(actual.len(), 8);
    }

    #[test]
    fn downmix_handles_mono_stereo_and_dialogue_centre() {
        assert_eq!(
            downmix_interleaved_to_mono(&[0.5, -0.5], 1),
            vec![0.5, -0.5]
        );
        assert_eq!(downmix_interleaved_to_mono(&[1.0, -1.0], 2), vec![0.0]);
        let centre = downmix_interleaved_to_mono(&[0.0, 0.0, 1.0, 0.0, 0.0, 0.0], 6);
        assert!(centre[0] > 0.25);
    }

    #[test]
    fn packed_decoder_handles_pcm32() {
        let pcm = PackedAudioFormat {
            channels: 1,
            sample_rate: 48_000,
            block_align: 4,
            container_bits: 32,
            valid_bits: 32,
        };
        assert!(decode_packed_audio(&i32::MAX.to_le_bytes(), 1, pcm, false).unwrap()[0] > 0.999);
    }

    #[test]
    fn quiet_speech_is_lifted_without_clipping() {
        let input = (0..48_000)
            .map(|index| {
                (2.0 * std::f32::consts::PI * 220.0 * index as f32 / 48_000.0).sin() * 0.004
            })
            .collect::<Vec<_>>();
        let mut processor = SpeechDynamicsProcessor::new(48_000);
        let mut output = Vec::with_capacity(input.len());
        for packet in input.chunks(480) {
            output.extend(processor.process(packet));
        }

        let input_level = rms(&input[24_000..]);
        let output_level = rms(&output[24_000..]);
        assert!(output_level > input_level * 10.0);
        assert!(output_level > 0.04);
        assert!(output.iter().all(|sample| sample.abs() <= LIMITER_CEILING));
    }

    #[test]
    fn loud_speech_is_compressed_and_limited() {
        let input = (0..48_000)
            .map(|index| {
                (2.0 * std::f32::consts::PI * 220.0 * index as f32 / 48_000.0).sin() * 0.95
            })
            .collect::<Vec<_>>();
        let mut processor = SpeechDynamicsProcessor::new(48_000);
        let mut output = Vec::with_capacity(input.len());
        for packet in input.chunks(480) {
            output.extend(processor.process(packet));
        }

        assert!(rms(&output[24_000..]) < rms(&input[24_000..]) * 0.55);
        assert!(output.iter().all(|sample| sample.abs() <= LIMITER_CEILING));
    }

    #[test]
    fn digital_silence_stays_silent() {
        let mut processor = SpeechDynamicsProcessor::new(48_000);
        let output = processor.process(&vec![0.0; 48_000]);
        assert!(output.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn stationary_background_noise_does_not_open_the_agc_or_meter() {
        let mut processor = SpeechDynamicsProcessor::new(48_000);
        let mut seed = 0x1234_5678_u32;
        let mut output = Vec::new();
        for _ in 0..100 {
            let noise = (0..DenoiseState::FRAME_SIZE)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    ((seed >> 8) as f32 / 0x00ff_ffff as f32 * 2.0 - 1.0) * 0.012
                })
                .collect::<Vec<_>>();
            output.extend(processor.process_dynamics_frame(&noise, 0.0));
        }

        assert_eq!(meter_level(&output[output.len() / 2..]), 0.0);
        assert!(output.iter().all(|sample| *sample == 0.0));
    }

    #[test]
    fn enhancement_preserves_exact_duration_at_non_native_sample_rates() {
        let input = vec![0.0; 44_100];
        let mut processor = SpeechDynamicsProcessor::new(44_100);
        let mut output = Vec::new();
        for packet in input.chunks(317) {
            output.extend(processor.process(packet));
        }
        output.extend(processor.finish());

        assert_eq!(output.len(), 48_000);
    }

    #[test]
    fn meter_uses_a_readable_decibel_scale() {
        let quiet = meter_level(&[0.01; 32]);
        let speech = meter_level(&[0.1; 32]);
        assert!((quiet - 1.0 / 3.0).abs() < 0.01);
        assert!((speech - 2.0 / 3.0).abs() < 0.01);
        assert_eq!(meter_level(&[0.0; 32]), 0.0);
    }
}
