//! Microphone capture plus preparation for Whisper (16 kHz mono f32).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat, SizedSample};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

pub const RATE: u32 = 16_000;

pub struct Recorder {
    _stream: cpal::Stream,
    buffer: Arc<Mutex<Vec<f32>>>,
    /// Discontinuities reported by the driver; some devices flag nearly every packet.
    gaps: Arc<AtomicUsize>,
    rate: u32,
}

pub fn input_device(filter: &str) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if filter.trim().is_empty() {
        return host
            .default_input_device()
            .ok_or_else(|| "Kein Standardmikrofon gefunden".to_string());
    }
    let needle = filter.to_lowercase();
    host.input_devices()
        .map_err(|e| e.to_string())?
        .find(|d| name(d).to_lowercase().contains(&needle))
        .ok_or_else(|| format!("Kein Mikrofon mit \"{filter}\" im Namen gefunden"))
}

pub fn name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "unbekannt".into())
}

pub fn device_names() -> Vec<String> {
    cpal::default_host()
        .input_devices()
        .map(|devices| devices.map(|d| name(&d)).collect())
        .unwrap_or_default()
}

impl Recorder {
    /// Starts capturing immediately; samples are downmixed to mono at the device rate.
    pub fn start(device: &cpal::Device, max_seconds: u32) -> Result<Self, String> {
        let supported = device.default_input_config().map_err(|e| e.to_string())?;
        let config = supported.config();
        let rate = config.sample_rate;
        let channels = usize::from(config.channels).max(1);
        let limit = rate as usize * max_seconds as usize;
        let buffer = Arc::new(Mutex::new(Vec::with_capacity(rate as usize * 10)));
        let gaps = Arc::new(AtomicUsize::new(0));
        let shared = (&buffer, &gaps);
        let stream = match supported.sample_format() {
            SampleFormat::F32 => build::<f32>(device, &config, channels, limit, shared),
            SampleFormat::I16 => build::<i16>(device, &config, channels, limit, shared),
            SampleFormat::I32 => build::<i32>(device, &config, channels, limit, shared),
            SampleFormat::U16 => build::<u16>(device, &config, channels, limit, shared),
            other => return Err(format!("Nicht unterstütztes Audioformat {other:?}")),
        }?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            _stream: stream,
            buffer,
            gaps,
            rate,
        })
    }

    pub fn seconds(&self) -> f32 {
        self.buffer.lock().map(|b| b.len()).unwrap_or(0) as f32 / self.rate as f32
    }

    /// Copy of everything recorded so far as 16 kHz mono; capture continues.
    pub fn snapshot(&self) -> Vec<f32> {
        let samples = self.buffer.lock().map(|b| b.clone()).unwrap_or_default();
        resample(&samples, self.rate, RATE)
    }

    /// Stops capturing; returns 16 kHz mono audio and the number of reported gaps.
    pub fn finish(self) -> (Vec<f32>, usize) {
        let rate = self.rate;
        let buffer = Arc::clone(&self.buffer);
        let gaps = self.gaps.load(Ordering::Relaxed);
        drop(self); // stops the stream
        let samples = std::mem::take(&mut *buffer.lock().unwrap_or_else(|e| e.into_inner()));
        (resample(&samples, rate, RATE), gaps)
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    limit: usize,
    (buffer, gaps): (&Arc<Mutex<Vec<f32>>>, &Arc<AtomicUsize>),
) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let buffer = Arc::clone(buffer);
    let gaps = Arc::clone(gaps);
    device
        .build_input_stream::<T, _, _>(
            *config,
            move |data: &[T], _| {
                let Ok(mut buffer) = buffer.lock() else {
                    return;
                };
                for frame in data.chunks(channels) {
                    if buffer.len() >= limit {
                        return;
                    }
                    let sum: f32 = frame.iter().map(|s| f32::from_sample(*s)).sum();
                    buffer.push(sum / frame.len() as f32);
                }
            },
            move |err| {
                if err.kind() == cpal::ErrorKind::Xrun {
                    gaps.fetch_add(1, Ordering::Relaxed);
                } else {
                    crate::log::line(&format!("Audiofehler: {err}"));
                }
            },
            None,
        )
        .map_err(|e| format!("Mikrofon konnte nicht geöffnet werden: {e}"))
}

/// Windowed-sinc resampler; low-passes below the target Nyquist to avoid aliasing.
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || input.is_empty() {
        return input.to_vec();
    }
    const HALF_TAPS: i64 = 16;
    let ratio = f64::from(from) / f64::from(to);
    let cutoff = (1.0 / ratio).min(1.0) * 0.95;
    let len = (input.len() as f64 / ratio).floor() as usize;
    (0..len)
        .map(|i| {
            let center = i as f64 * ratio;
            let base = center.floor() as i64;
            let mut acc = 0.0;
            let mut norm = 0.0;
            for k in
                (base - HALF_TAPS * ratio.ceil() as i64)..=(base + HALF_TAPS * ratio.ceil() as i64)
            {
                if k < 0 || k as usize >= input.len() {
                    continue;
                }
                let x = (k as f64 - center) * cutoff;
                let window_pos = (k as f64 - center) / (HALF_TAPS as f64 * ratio.max(1.0));
                if window_pos.abs() >= 1.0 {
                    continue;
                }
                let sinc = if x.abs() < 1e-9 {
                    1.0
                } else {
                    (std::f64::consts::PI * x).sin() / (std::f64::consts::PI * x)
                };
                let window = 0.5 + 0.5 * (std::f64::consts::PI * window_pos).cos();
                let w = sinc * window;
                acc += f64::from(input[k as usize]) * w;
                norm += w;
            }
            if norm.abs() > 1e-9 {
                (acc / norm) as f32
            } else {
                0.0
            }
        })
        .collect()
}

/// Finds the span containing speech by frame energy; `None` if there is too little.
/// Silence makes Whisper hallucinate ("Untertitel im Auftrag des ZDF"), so it is never transcribed.
pub fn speech_span(samples: &[f32]) -> Option<(usize, usize)> {
    const FRAME: usize = RATE as usize / 50; // 20 ms
    let rms: Vec<f32> = samples
        .chunks(FRAME)
        .map(|f| (f.iter().map(|s| s * s).sum::<f32>() / f.len() as f32).sqrt())
        .collect();
    if rms.is_empty() {
        return None;
    }
    let mut sorted = rms.clone();
    sorted.sort_by(f32::total_cmp);
    let floor = sorted[sorted.len() / 10];
    let threshold = (floor * 3.0).max(0.006);
    let loud: Vec<usize> = (0..rms.len()).filter(|&i| rms[i] > threshold).collect();
    // At least 0.2 s of voiced frames.
    if loud.len() < 10 {
        return None;
    }
    let pad = 15; // 300 ms
    let first = loud[0].saturating_sub(pad) * FRAME;
    let last = ((loud[loud.len() - 1] + 1 + pad) * FRAME).min(samples.len());
    Some((first, last))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, rate: u32, seconds: f32, amp: f32) -> Vec<f32> {
        (0..(rate as f32 * seconds) as usize)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn resample_keeps_speech_band_and_removes_aliases() {
        let pass = resample(&tone(1000.0, 48_000, 1.0, 0.5), 48_000, RATE);
        assert_eq!(pass.len(), 16_000);
        assert!((rms(&pass[1000..15000]) - 0.354).abs() < 0.02);
        // 12 kHz would alias to 4 kHz without low-pass filtering.
        let stop = resample(&tone(12_000.0, 48_000, 1.0, 0.5), 48_000, RATE);
        assert!(rms(&stop[1000..15000]) < 0.02);
        assert_eq!(resample(&[0.1, 0.2], RATE, RATE), vec![0.1, 0.2]);
    }

    #[test]
    fn speech_span_rejects_silence_and_trims_padding() {
        let mut quiet = vec![0.0005; RATE as usize * 2];
        assert_eq!(speech_span(&quiet), None);
        let speech = tone(300.0, RATE, 1.0, 0.2);
        quiet[RATE as usize / 2..RATE as usize / 2 + speech.len()].copy_from_slice(&speech);
        let (start, end) = speech_span(&quiet).unwrap();
        assert!(start > 0 && start <= RATE as usize / 2);
        assert!(end >= RATE as usize * 3 / 2 && end < quiet.len());
    }
}
