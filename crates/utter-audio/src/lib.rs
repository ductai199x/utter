//! Microphone capture for Utter.
//!
//! The microphone stays open while Utter runs, so every dictation starts with the last
//! [`MicConfig::preroll_ms`] of audio before the key went down: the first word is never cut
//! off (DESIGN.md section 4, step 1).
//!
//! Three parts, so nothing slow ever runs on the audio thread:
//! - the device callback (the OS's real-time audio thread) only downmixes to mono, measures
//!   the level and pushes samples into a lock-free ring; it never allocates or locks;
//! - a worker thread drains the ring, resamples to 16 kHz with a fixed-input FFT resampler, and
//!   feeds the pre-roll and, while recording, the utterance;
//! - the caller reads [`Microphone::take_new`] while recording (for segmenting at pauses) and
//!   [`Microphone::stop`] at the end.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use audioadapter_buffers::direct::InterleavedSlice;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Producer, Split};
use rubato::{Fft, FixedSync, Resampler};

/// The rate everything downstream works at.
pub const SAMPLE_RATE: u32 = 16_000;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("no input device{0}")]
    NoDevice(String),
    #[error("audio device: {0}")]
    Device(String),
    #[error("resampler: {0}")]
    Resampler(String),
}

#[derive(Debug, Clone)]
pub struct MicConfig {
    /// A device whose name contains this text (default: the system's default input).
    pub device: Option<String>,
    /// Audio kept from before `start`, in milliseconds.
    pub preroll_ms: u32,
}

impl Default for MicConfig {
    fn default() -> Self {
        Self { device: None, preroll_ms: 300 }
    }
}

/// State the worker writes and the caller reads, behind one short-held lock.
struct Capture {
    preroll: VecDeque<f32>,
    preroll_len: usize,
    /// The utterance being recorded, from `start` (with its pre-roll) on.
    recording: Option<Vec<f32>>,
    /// How much of `recording` `take_new` has handed out.
    taken: usize,
}

struct Shared {
    capture: Mutex<Capture>,
    /// RMS level of the latest device callback, as f32 bits.
    level: AtomicU32,
    stop: AtomicBool,
}

pub struct Microphone {
    _stream: cpal::Stream,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    device_name: String,
    native_rate: u32,
}

impl Microphone {
    pub fn open(config: &MicConfig) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = match &config.device {
            None => host.default_input_device().ok_or_else(|| AudioError::NoDevice(String::new()))?,
            Some(want) => host
                .input_devices()
                .map_err(|e| AudioError::Device(e.to_string()))?
                .find(|d| d.description().map(|desc| desc.name().contains(want.as_str())).unwrap_or(false))
                .ok_or_else(|| AudioError::NoDevice(format!(" matching {want:?}")))?,
        };
        let device_name = device.description().map(|d| d.name().to_owned()).unwrap_or_else(|_| "unknown".into());
        let supported = device.default_input_config().map_err(|e| AudioError::Device(e.to_string()))?;
        let native_rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let format = supported.sample_format();

        // One second of native-rate mono between the callback and the worker.
        let (producer, consumer) = HeapRb::<f32>::new(native_rate as usize).split();
        let shared = Arc::new(Shared {
            capture: Mutex::new(Capture {
                preroll: VecDeque::new(),
                preroll_len: (SAMPLE_RATE * config.preroll_ms / 1000) as usize,
                recording: None,
                taken: 0,
            }),
            level: AtomicU32::new(0),
            stop: AtomicBool::new(false),
        });

        let stream_config: cpal::StreamConfig = supported.into();
        let err_fn = |e: cpal::Error| tracing::warn!("audio stream: {e}");
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &stream_config, channels, producer, &shared, err_fn),
            SampleFormat::I16 => build::<i16>(&device, &stream_config, channels, producer, &shared, err_fn),
            SampleFormat::I32 => build::<i32>(&device, &stream_config, channels, producer, &shared, err_fn),
            SampleFormat::U16 => build::<u16>(&device, &stream_config, channels, producer, &shared, err_fn),
            other => return Err(AudioError::Device(format!("sample format {other} not supported"))),
        }?;
        stream.play().map_err(|e| AudioError::Device(e.to_string()))?;

        let resampler = Fft::<f32>::new(native_rate as usize, SAMPLE_RATE as usize, 1024, 1, FixedSync::Input)
            .map_err(|e| AudioError::Resampler(e.to_string()))?;
        let worker_shared = shared.clone();
        let worker = std::thread::Builder::new()
            .name("utter-audio".into())
            .spawn(move || worker_loop(consumer, resampler, worker_shared))
            .map_err(|e| AudioError::Device(e.to_string()))?;
        tracing::info!(device = %device_name, native_rate, channels, ?format, "microphone open");
        Ok(Self { _stream: stream, shared, worker: Some(worker), device_name, native_rate })
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn native_rate(&self) -> u32 {
        self.native_rate
    }

    /// RMS level of the latest audio, 0 to 1 (for the waveform).
    pub fn level(&self) -> f32 {
        f32::from_bits(self.shared.level.load(Ordering::Relaxed))
    }

    /// A handle that reads the level from any thread (the UI's), independent of who holds the
    /// microphone.
    pub fn level_meter(&self) -> LevelMeter {
        LevelMeter(self.shared.clone())
    }

    /// Starts an utterance with the pre-roll already in it.
    pub fn start(&self) {
        let mut c = self.shared.capture.lock().expect("capture lock");
        let preroll: Vec<f32> = c.preroll.iter().copied().collect();
        c.recording = Some(preroll);
        c.taken = 0;
    }

    /// The utterance's samples since the last call (empty when not recording).
    pub fn take_new(&self) -> Vec<f32> {
        let mut c = self.shared.capture.lock().expect("capture lock");
        let Capture { recording, taken, .. } = &mut *c;
        match recording {
            Some(rec) => {
                let new = rec[*taken..].to_vec();
                *taken = rec.len();
                new
            }
            None => Vec::new(),
        }
    }

    /// Ends the utterance and returns all of it, pre-roll included. The worker may still hold
    /// a fraction of a resampler chunk (under 64 ms at 16 kHz); `settle` waits that long
    /// first, so the end of the last word is in.
    pub fn stop(&self, settle: Duration) -> Vec<f32> {
        if !settle.is_zero() {
            std::thread::sleep(settle);
        }
        let mut c = self.shared.capture.lock().expect("capture lock");
        c.taken = 0;
        c.recording.take().unwrap_or_default()
    }
}

/// Reads the microphone's level; cheap to clone, safe to share.
#[derive(Clone)]
pub struct LevelMeter(Arc<Shared>);

impl LevelMeter {
    /// RMS level of the latest audio, 0 to 1.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.0.level.load(Ordering::Relaxed))
    }
}

impl Drop for Microphone {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(w) = self.worker.take() {
            let _ = w.join();
        }
    }
}

fn build<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    mut producer: impl Producer<Item = f32> + Send + 'static,
    shared: &Arc<Shared>,
    err_fn: impl FnMut(cpal::Error) + Send + 'static,
) -> Result<cpal::Stream, AudioError>
where
    T: Sample + cpal::SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let shared = shared.clone();
    device
        .build_input_stream(
            *config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                // Real-time thread: no allocation, no locks.
                let mut sum_sq = 0.0f32;
                let mut frames = 0usize;
                for frame in data.chunks_exact(channels) {
                    let mono = frame.iter().map(|&s| f32::from_sample(s)).sum::<f32>() / channels as f32;
                    sum_sq += mono * mono;
                    frames += 1;
                    // A full ring means the worker stalled for a second; dropping beats blocking.
                    let _ = producer.try_push(mono);
                }
                if frames > 0 {
                    shared.level.store((sum_sq / frames as f32).sqrt().to_bits(), Ordering::Relaxed);
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| AudioError::Device(e.to_string()))
}

fn worker_loop(mut consumer: impl Consumer<Item = f32>, mut resampler: Fft<f32>, shared: Arc<Shared>) {
    let chunk_in = resampler.input_frames_next();
    let mut input = vec![0.0f32; chunk_in];
    let mut output = vec![0.0f32; resampler.output_frames_max()];
    let mut filled = 0usize;
    while !shared.stop.load(Ordering::Relaxed) {
        let got = consumer.pop_slice(&mut input[filled..]);
        filled += got;
        if filled < chunk_in {
            std::thread::sleep(Duration::from_millis(4));
            continue;
        }
        filled = 0;
        let produced = {
            let Ok(inp) = InterleavedSlice::new(&input[..], 1, chunk_in) else { continue };
            let out_len = output.len();
            let Ok(mut out) = InterleavedSlice::new_mut(&mut output[..], 1, out_len) else { continue };
            match resampler.process_into_buffer(&inp, &mut out, None) {
                Ok((_, produced)) => produced,
                Err(e) => {
                    tracing::warn!("resampler: {e}");
                    continue;
                }
            }
        };
        let samples = &output[..produced];
        let mut c = shared.capture.lock().expect("capture lock");
        c.preroll.extend(samples.iter().copied());
        let excess = c.preroll.len().saturating_sub(c.preroll_len);
        c.preroll.drain(..excess);
        if let Some(rec) = c.recording.as_mut() {
            rec.extend_from_slice(samples);
        }
    }
}
