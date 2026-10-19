//! Microphone capture for Utter.
//!
//! The microphone stays open while Utter runs and keeps its last [`HISTORY`] of audio, stamped
//! with when it arrived. A dictation starts from the moment the key went down, less
//! [`MicConfig::preroll_ms`], even when the pipeline gets to it late (busy finishing the last
//! one): the first word is never cut off (DESIGN.md section 4, step 1).
//!
//! Three parts, so nothing slow ever runs on the audio thread:
//! - the device callback (the OS's real-time audio thread) only downmixes to mono, measures
//!   the level and pushes samples into a lock-free ring; it never allocates or locks;
//! - a worker thread drains the ring, resamples to 16 kHz with a fixed-input FFT resampler, and
//!   feeds the history and, while recording, the utterance;
//! - the caller reads [`Microphone::take_new`] while recording (for segmenting at pauses) and
//!   [`Microphone::stop`] at the end, which has the worker resample what it still holds first.
//!
//! A microphone can die under Utter (unplugged, a Bluetooth headset asleep): [`Microphone::alive`]
//! says so, from the stream's errors and from audio that stops arriving, and the history is
//! cleared then, so no dictation hears stale audio. The pipeline reopens it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use audioadapter_buffers::direct::InterleavedSlice;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, Sample, SampleFormat};
use ringbuf::HeapRb;
use ringbuf::traits::{Consumer, Producer, Split};
use rubato::{Fft, FixedSync, Resampler};

/// The rate everything downstream works at.
pub const SAMPLE_RATE: u32 = 16_000;
/// How much audio the microphone keeps: more than the longest a key-down can wait for the
/// pipeline (the last dictation's cleanup timeout and paste).
pub const HISTORY: Duration = Duration::from_secs(10);
/// A device that has delivered nothing for this long has stopped (it delivers every few ms).
const SILENT_DEVICE: Duration = Duration::from_secs(1);
/// The longest `stop` waits for the worker to resample what it holds.
const FLUSH_WAIT: Duration = Duration::from_millis(100);
/// A stop or start this much later than the moment it is for trims or rewinds the audio to it;
/// closer than that, the device's own latency is the larger error.
const LATE: Duration = Duration::from_millis(150);
/// Audio kept past the key-up moment when trimming: the device delivers it a little later than
/// it was spoken.
const TAIL: Duration = Duration::from_millis(100);

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
    /// Audio kept from before the key went down, in milliseconds.
    pub preroll_ms: u32,
}

impl Default for MicConfig {
    fn default() -> Self {
        Self { device: None, preroll_ms: 300 }
    }
}

/// State the worker writes and the caller reads, behind one short-held lock.
struct Capture {
    /// The last [`HISTORY`] of audio, oldest first.
    history: VecDeque<f32>,
    /// When the newest sample of `history` arrived.
    newest_at: Option<Instant>,
    /// The utterance being recorded, from its start (with its pre-roll) on.
    recording: Option<Vec<f32>>,
    /// How much of `recording` `take_new` has handed out.
    taken: usize,
}

struct Shared {
    capture: Mutex<Capture>,
    /// RMS level of the latest device callback, as f32 bits.
    level: AtomicU32,
    stop: AtomicBool,
    /// The stream reported an error (the device is gone).
    failed: AtomicBool,
    /// When audio last arrived, in milliseconds since `opened`.
    last_audio_ms: AtomicU64,
    opened: Instant,
    /// Flush requests and the last one done, for `stop`.
    flush: Mutex<(u64, u64)>,
    flushed: Condvar,
}

impl Shared {
    fn new() -> Self {
        Self {
            capture: Mutex::new(Capture { history: VecDeque::new(), newest_at: None, recording: None, taken: 0 }),
            level: AtomicU32::new(0),
            stop: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            last_audio_ms: AtomicU64::new(0),
            opened: Instant::now(),
            flush: Mutex::new((0, 0)),
            flushed: Condvar::new(),
        }
    }

    fn since_opened_ms(&self) -> u64 {
        self.opened.elapsed().as_millis() as u64
    }

    fn start(&self, at: Instant, preroll: usize) {
        let mut c = self.capture.lock().expect("capture lock");
        let begin = c.history.len().saturating_sub(rewind(c.newest_at, at) + preroll);
        let recording: Vec<f32> = c.history.range(begin..).copied().collect();
        c.recording = Some(recording);
        c.taken = 0;
    }

    fn take_new(&self) -> Vec<f32> {
        let mut c = self.capture.lock().expect("capture lock");
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

    fn stop(&self, until: Instant) -> Vec<f32> {
        self.flush();
        let mut c = self.capture.lock().expect("capture lock");
        c.taken = 0;
        let mut recording = c.recording.take().unwrap_or_default();
        let cut = trim(c.newest_at, until);
        recording.truncate(recording.len().saturating_sub(cut));
        recording
    }

    /// Has the worker resample everything it holds now, and waits for it (briefly).
    fn flush(&self) {
        let mut flush = self.flush.lock().expect("flush lock");
        flush.0 += 1;
        let want = flush.0;
        let (flush, timeout) = self.flushed.wait_timeout_while(flush, FLUSH_WAIT, |f| f.1 < want).expect("flush lock");
        if timeout.timed_out() {
            tracing::debug!(done = flush.1, want, "the microphone's worker did not flush in time");
        }
    }
}

/// How far before the newest audio a recording for a key-down at `at` starts: none when the
/// pipeline got to it in time.
fn rewind(newest: Option<Instant>, at: Instant) -> usize {
    let late = newest.map_or(Duration::ZERO, |n| n.saturating_duration_since(at));
    if late >= LATE { samples(late) } else { 0 }
}

/// How much of a recording's end arrived after the key-up at `until`, to leave out: none when
/// the pipeline got to it in time, else all but [`TAIL`].
fn trim(newest: Option<Instant>, until: Instant) -> usize {
    let after = newest.map_or(Duration::ZERO, |n| n.saturating_duration_since(until));
    if after >= LATE { samples(after - TAIL) } else { 0 }
}

pub struct Microphone {
    /// None for [`Microphone::none`].
    _stream: Option<cpal::Stream>,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    device_name: String,
    native_rate: u32,
    preroll: usize,
}

/// The names of the input devices there are now, for a picker; `MicConfig::device` matches
/// part of one.
pub fn input_devices() -> Vec<String> {
    let Ok(devices) = cpal::default_host().input_devices() else { return Vec::new() };
    let mut names: Vec<String> =
        devices.filter_map(|d| d.description().ok().map(|desc| desc.name().to_owned())).collect();
    names.dedup();
    names
}

/// The system's default input device's name now, if there is one.
pub fn default_input_device() -> Option<String> {
    cpal::default_host().default_input_device().and_then(|d| d.description().ok()).map(|d| d.name().to_owned())
}

/// Samples in `d` at [`SAMPLE_RATE`].
fn samples(d: Duration) -> usize {
    (d.as_secs_f64() * SAMPLE_RATE as f64) as usize
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
        let shared = Arc::new(Shared::new());

        let stream_config: cpal::StreamConfig = supported.into();
        let failed = shared.clone();
        // Only an error that means the device or the stream is gone fails the microphone; a
        // glitch (an overrun, a busy moment) is noted and audio goes on.
        let err_fn = move |e: cpal::Error| {
            use cpal::ErrorKind::{DeviceNotAvailable, HostUnavailable, PermissionDenied, StreamInvalidated};
            if matches!(e.kind(), DeviceNotAvailable | HostUnavailable | PermissionDenied | StreamInvalidated) {
                tracing::warn!("audio stream lost: {e}");
                failed.failed.store(true, Ordering::Relaxed);
            } else {
                tracing::debug!("audio stream: {e}");
            }
        };
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &stream_config, channels, producer, &shared, err_fn),
            SampleFormat::I16 => build::<i16>(&device, &stream_config, channels, producer, &shared, err_fn),
            SampleFormat::I32 => build::<i32>(&device, &stream_config, channels, producer, &shared, err_fn),
            SampleFormat::U16 => build::<u16>(&device, &stream_config, channels, producer, &shared, err_fn),
            other => return Err(AudioError::Device(format!("sample format {other} not supported"))),
        }?;
        stream.play().map_err(|e| AudioError::Device(e.to_string()))?;

        // About 10 ms a chunk at any rate (the resampler rounds it to what its ratio needs), so
        // what the worker holds at key up is small; `stop` resamples it anyway.
        let chunk = (native_rate as usize / 100).max(1);
        let resampler = Fft::<f32>::new(native_rate as usize, SAMPLE_RATE as usize, chunk, 1, FixedSync::Input)
            .map_err(|e| AudioError::Resampler(e.to_string()))?;
        let worker_shared = shared.clone();
        let worker = std::thread::Builder::new()
            .name("utter-audio".into())
            .spawn(move || worker_loop(consumer, resampler, worker_shared))
            .map_err(|e| AudioError::Device(e.to_string()))?;
        tracing::info!(device = %device_name, native_rate, channels, ?format, "microphone open");
        let preroll = (SAMPLE_RATE * config.preroll_ms / 1000) as usize;
        Ok(Self { _stream: Some(stream), shared, worker: Some(worker), device_name, native_rate, preroll })
    }

    /// No microphone: a stand-in that is never alive, for starting without one (none
    /// connected); the pipeline opens a real one when it appears.
    pub fn none() -> Self {
        let shared = Arc::new(Shared::new());
        shared.failed.store(true, Ordering::Relaxed);
        Self { _stream: None, shared, worker: None, device_name: String::new(), native_rate: 0, preroll: 0 }
    }

    pub fn device_name(&self) -> &str {
        &self.device_name
    }

    pub fn native_rate(&self) -> u32 {
        self.native_rate
    }

    /// Whether the device still delivers audio: no stream error, and audio within the last
    /// second (a device that vanished without an error goes quiet instead).
    pub fn alive(&self) -> bool {
        let quiet_ms = self.shared.since_opened_ms().saturating_sub(self.shared.last_audio_ms.load(Ordering::Relaxed));
        // A device just opened has a moment to deliver its first audio.
        let started = self.shared.opened.elapsed() > SILENT_DEVICE;
        !self.shared.failed.load(Ordering::Relaxed) && (!started || quiet_ms < SILENT_DEVICE.as_millis() as u64)
    }

    /// Whether any audio has arrived since the device was opened.
    pub fn delivered(&self) -> bool {
        self.shared.last_audio_ms.load(Ordering::Relaxed) > 0
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

    /// Starts an utterance at `at` (when the key went down) with the pre-roll before it: from
    /// the history when the pipeline got to it late.
    pub fn start(&self, at: Instant) {
        self.shared.start(at, self.preroll);
    }

    /// The utterance's samples since the last call (empty when not recording).
    pub fn take_new(&self) -> Vec<f32> {
        self.shared.take_new()
    }

    /// Ends the utterance at `until` (when the key went up) and returns all of it, pre-roll
    /// included. The worker first resamples what it still holds, so the end of the last word
    /// is in; audio that arrived well after `until` (the pipeline got to the key up late) is
    /// left out.
    pub fn stop(&self, until: Instant) -> Vec<f32> {
        self.shared.stop(until)
    }
}

/// Reads the microphone's level; cheap to clone, safe to share.
#[derive(Clone)]
pub struct LevelMeter(Arc<Shared>);

impl std::fmt::Debug for LevelMeter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("LevelMeter").field(&self.level()).finish()
    }
}

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
    let history_len = samples(HISTORY);
    let mut cleared = false;
    while !shared.stop.load(Ordering::Relaxed) {
        // A dead device's history is stale: no dictation may hear it.
        if shared.failed.load(Ordering::Relaxed) {
            if !cleared {
                let mut c = shared.capture.lock().expect("capture lock");
                c.history.clear();
                c.newest_at = None;
                shared.level.store(0f32.to_bits(), Ordering::Relaxed);
                cleared = true;
            }
            // Nothing more will come: a stop waiting for the rest has it.
            let requested = shared.flush.lock().expect("flush lock").0;
            finish_flush(&shared, requested);
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let got = consumer.pop_slice(&mut input[filled..]);
        filled += got;
        if got > 0 {
            shared.last_audio_ms.store(shared.since_opened_ms(), Ordering::Relaxed);
        }
        let (requested, done) = *shared.flush.lock().expect("flush lock");
        let flushing = requested > done;
        if filled < chunk_in && !(flushing && filled > 0) {
            if flushing {
                finish_flush(&shared, requested);
            } else {
                std::thread::sleep(Duration::from_millis(4));
            }
            continue;
        }
        // A flush resamples a partial chunk too, padded with silence: the end of the last word
        // comes out now instead of waiting for a chunk to fill.
        input[filled..].fill(0.0);
        let real = filled;
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
        {
            let mut c = shared.capture.lock().expect("capture lock");
            c.history.extend(samples.iter().copied());
            let excess = c.history.len().saturating_sub(history_len);
            c.history.drain(..excess);
            c.newest_at = Some(Instant::now());
            if let Some(rec) = c.recording.as_mut() {
                rec.extend_from_slice(samples);
            }
        }
        if flushing && real < chunk_in {
            finish_flush(&shared, requested);
        }
    }
}

/// Marks flush `requested` done and wakes `stop`.
fn finish_flush(shared: &Shared, requested: u64) {
    let mut flush = shared.flush.lock().expect("flush lock");
    flush.1 = flush.1.max(requested);
    shared.flushed.notify_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_start_in_time_takes_only_the_preroll() {
        let now = Instant::now();
        assert_eq!(rewind(Some(now), now), 0);
        assert_eq!(rewind(Some(now + Duration::from_millis(100)), now), 0, "within LATE: the device's own latency");
        assert_eq!(rewind(None, now), 0);
    }

    #[test]
    fn a_late_start_rewinds_to_the_key_down() {
        let key_down = Instant::now();
        assert_eq!(rewind(Some(key_down + Duration::from_secs(2)), key_down), 32_000);
    }

    #[test]
    fn a_late_stop_leaves_out_what_came_after_the_key_up() {
        let key_up = Instant::now();
        assert_eq!(trim(Some(key_up + Duration::from_millis(30)), key_up), 0);
        // 1.1 s late: all but the 100 ms tail goes.
        assert_eq!(trim(Some(key_up + Duration::from_millis(1100)), key_up), 16_000);
    }

    /// A worker on a synthetic ring at `rate`, as `Microphone::open` runs it.
    fn worker(rate: usize) -> (Arc<Shared>, impl Producer<Item = f32>, JoinHandle<()>) {
        let (producer, consumer) = HeapRb::<f32>::new(rate).split();
        let resampler = Fft::<f32>::new(rate, SAMPLE_RATE as usize, (rate / 100).max(1), 1, FixedSync::Input).unwrap();
        let shared = Arc::new(Shared::new());
        let s = shared.clone();
        let handle = std::thread::spawn(move || worker_loop(consumer, resampler, s));
        (shared, producer, handle)
    }

    fn end(shared: &Shared, handle: JoinHandle<()>) {
        shared.stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }

    #[test]
    fn stop_resamples_the_partial_chunk_at_any_rate() {
        for rate in [48_000, 16_000] {
            let (shared, mut producer, handle) = worker(rate);
            shared.start(Instant::now(), 0);
            // Less than a chunk (10 ms): before, it waited in the worker until more audio came.
            let tail = rate / 200;
            for _ in 0..tail {
                producer.try_push(0.5).unwrap();
            }
            let started = Instant::now();
            let audio = shared.stop(Instant::now());
            assert!(started.elapsed() < FLUSH_WAIT, "{rate} Hz: the flush did not finish in time");
            let expected = tail * SAMPLE_RATE as usize / rate;
            assert!(audio.len() >= expected, "{rate} Hz: {} samples back, at least {expected} expected", audio.len());
            end(&shared, handle);
        }
    }

    #[test]
    fn a_late_start_still_hears_what_was_said_after_the_key_down() {
        let (shared, mut producer, handle) = worker(16_000);
        let key_down = Instant::now();
        // Half a second of speech arrives, as a device delivers it, while the pipeline is busy
        // elsewhere.
        for _ in 0..25 {
            for _ in 0..320 {
                producer.try_push(0.25).unwrap();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        shared.flush();
        shared.start(key_down, 0);
        let audio = shared.stop(Instant::now());
        assert!(audio.len() >= 6_000, "only {} samples of the speech since the key down", audio.len());
        end(&shared, handle);
    }

    #[test]
    fn a_failed_device_keeps_no_stale_audio() {
        let (shared, mut producer, handle) = worker(16_000);
        for _ in 0..4_000 {
            producer.try_push(0.25).unwrap();
        }
        shared.flush();
        assert!(!shared.capture.lock().unwrap().history.is_empty());
        shared.failed.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(60));
        shared.start(Instant::now(), 4_800);
        assert!(shared.stop(Instant::now()).is_empty(), "a dead device's history was replayed");
        end(&shared, handle);
    }
}
