//! `utter-cli`: the developer command line of Utter's core.
//!
//! - `transcribe <wav>...`: print each file's transcript and per-stage timings.
//! - `bench`: run the core's recognizer over the eval sets that `tools/eval/fetch_sets.py`
//!   fetched, and write results in the format `tools/eval/asr_bench.py report|compare`
//!   scores, so the Rust runtime is measured by the same yardstick as the Python candidates.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};
use utter_asr::{ConvSearch, Device, ParakeetOptions, ParakeetTdt, Timings};

const WARMUP_CLIPS: usize = 3;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// Exported model folder name under <data_dir>/models.
    #[arg(long, global = true, default_value = "parakeet-ultra")]
    model: String,
    /// Where the decoder and joint graphs run.
    #[arg(long, global = true, value_enum, default_value_t = DeviceArg::Cpu)]
    decoder_device: DeviceArg,
    /// Disable TensorFloat-32 on the GPU.
    #[arg(long, global = true)]
    no_tf32: bool,
    /// cuDNN convolution algorithm search.
    #[arg(long, global = true, value_enum, default_value_t = ConvSearchArg::Heuristic)]
    conv_search: ConvSearchArg,
    /// Disable onnxruntime's memory-pattern planning.
    #[arg(long, global = true)]
    no_memory_pattern: bool,
    /// Threads for graphs on the CPU (0: one per core).
    #[arg(long, global = true, default_value_t = 0)]
    cpu_threads: usize,
    /// Stop idle CPU worker threads from spin-waiting.
    #[arg(long, global = true)]
    no_cpu_spinning: bool,
}

#[derive(Clone, Copy, ValueEnum)]
enum ConvSearchArg {
    Exhaustive,
    Heuristic,
    Default,
}

impl From<ConvSearchArg> for ConvSearch {
    fn from(c: ConvSearchArg) -> Self {
        match c {
            ConvSearchArg::Exhaustive => ConvSearch::Exhaustive,
            ConvSearchArg::Heuristic => ConvSearch::Heuristic,
            ConvSearchArg::Default => ConvSearch::Default,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum DeviceArg {
    Cuda,
    Cpu,
}

impl From<DeviceArg> for Device {
    fn from(d: DeviceArg) -> Self {
        match d {
            DeviceArg::Cuda => Device::Cuda,
            DeviceArg::Cpu => Device::Cpu,
        }
    }
}

#[derive(Subcommand)]
enum Command {
    /// Transcribe 16 kHz mono WAV files.
    Transcribe { files: Vec<PathBuf> },
    /// Transcribe eval sets and write results for tools/eval/asr_bench.py.
    Bench {
        /// Comma-separated set names (default: every English set).
        #[arg(long, default_value = "")]
        sets: String,
        /// Clips per set (0: all).
        #[arg(long, default_value_t = 0)]
        limit: usize,
        /// Name the results are written under (results/asr/<name>/).
        #[arg(long, default_value = "parakeet-ultra-rust")]
        name: String,
        /// Idle time between clips, as between utterances in real use (0: back to back,
        /// which lets one clip's leftover work slow the next).
        #[arg(long, default_value_t = 0)]
        gap_ms: u64,
        /// During the gap, transcribe a short clip of quiet noise this often, as the live
        /// preview does while the user speaks (0: stay idle). An idle GPU lowers its clocks,
        /// and the next clip pays for it.
        #[arg(long, default_value_t = 0)]
        keepalive_ms: u64,
    },
}

#[derive(Deserialize)]
struct ManifestRow {
    id: String,
    audio: String,
    text: String,
    lang: String,
    duration_s: f64,
}

#[derive(Serialize)]
struct ResultRow<'a> {
    id: &'a str,
    lang: &'a str,
    #[serde(rename = "ref")]
    reference: &'a str,
    hyp: &'a str,
    detected: Option<&'a str>,
    latency_s: f64,
    duration_s: f64,
    frontend_s: f64,
    encoder_s: f64,
    decode_s: f64,
}

fn main() -> Result<()> {
    utter_core::init_tracing();
    let cli = Cli::parse();
    let opts = ParakeetOptions {
        decoder: cli.decoder_device.into(),
        tf32: !cli.no_tf32,
        conv_search: cli.conv_search.into(),
        memory_pattern: !cli.no_memory_pattern,
        cpu_threads: cli.cpu_threads,
        cpu_spinning: !cli.no_cpu_spinning,
        ..Default::default()
    };
    let dir = utter_core::model_dir(&cli.model);
    let mut asr = ParakeetTdt::load(&dir, opts).with_context(|| format!("loading {}", dir.display()))?;
    match cli.command {
        Command::Transcribe { files } => transcribe(&mut asr, &files),
        Command::Bench { sets, limit, name, gap_ms, keepalive_ms } => {
            bench(&mut asr, &sets, limit, &name, Pacing { gap_ms, keepalive_ms })
        }
    }
}

fn transcribe(asr: &mut ParakeetTdt, files: &[PathBuf]) -> Result<()> {
    for path in files {
        let audio = read_wav(path)?;
        let out = asr.transcribe(&audio)?;
        let t = out.timings;
        println!("{}: {}", path.display(), out.text);
        println!(
            "  {:.1} ms = front end {:.1} + encoder {:.1} + decode {:.1} ({} frames, {} joint, {} decoder calls)",
            t.total_s() * 1e3,
            t.frontend_s * 1e3,
            t.encoder_s * 1e3,
            t.decode_s * 1e3,
            t.encoder_frames,
            t.joint_calls,
            t.decoder_calls
        );
    }
    Ok(())
}

/// How clips are spaced in a bench run.
#[derive(Clone, Copy)]
struct Pacing {
    gap_ms: u64,
    keepalive_ms: u64,
}

impl Pacing {
    /// Waits out the gap before a clip, running keep-alive passes through it if asked.
    fn wait(&self, asr: &mut ParakeetTdt, keepalive_clip: &[f32]) -> Result<()> {
        use std::time::{Duration, Instant};

        if self.gap_ms == 0 {
            return Ok(());
        }
        let end = Instant::now() + Duration::from_millis(self.gap_ms);
        if self.keepalive_ms == 0 {
            std::thread::sleep(end - Instant::now());
            return Ok(());
        }
        loop {
            let next = Instant::now() + Duration::from_millis(self.keepalive_ms);
            if next >= end {
                std::thread::sleep(end.saturating_duration_since(Instant::now()));
                return Ok(());
            }
            asr.transcribe(keepalive_clip)?;
            std::thread::sleep(next.saturating_duration_since(Instant::now()));
        }
    }
}

/// Half a second of quiet, deterministic noise: what a keep-alive pass transcribes.
fn keepalive_clip() -> Vec<f32> {
    let mut state = 0x2545_f491_u32;
    (0..8_000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32 / u32::MAX as f32 - 0.5) * 1e-3
        })
        .collect()
}

fn bench(asr: &mut ParakeetTdt, sets: &str, limit: usize, name: &str, pacing: Pacing) -> Result<()> {
    let keepalive = keepalive_clip();
    let eval = utter_core::data_dir().join("eval");
    let set_names: Vec<String> = if sets.is_empty() {
        let mut names: Vec<String> = std::fs::read_dir(&eval)
            .with_context(|| format!("reading {}", eval.display()))?
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| n.starts_with("en-") && eval.join(n).join("manifest.jsonl").is_file())
            .collect();
        names.sort();
        names
    } else {
        sets.split(',').map(str::to_owned).collect()
    };
    if set_names.is_empty() {
        bail!("no eval sets under {}: run tools/eval/fetch_sets.py", eval.display());
    }
    let out_dir = eval.join("results").join("asr").join(name);
    std::fs::create_dir_all(&out_dir)?;
    let mut all: Vec<Timings> = Vec::new();
    for set in &set_names {
        let rows = read_manifest(&eval.join(set).join("manifest.jsonl"), limit)?;
        let clips: Vec<Vec<f32>> =
            rows.iter().map(|r| read_wav(&eval.join(set).join(&r.audio))).collect::<Result<_>>()?;
        for audio in clips.iter().take(WARMUP_CLIPS) {
            asr.transcribe(audio)?;
        }
        let part = out_dir.join(format!("{set}.jsonl.part"));
        let mut w = BufWriter::new(File::create(&part)?);
        let mut timings = Vec::with_capacity(rows.len());
        for (row, audio) in rows.iter().zip(&clips) {
            pacing.wait(asr, &keepalive)?;
            let out = asr.transcribe(audio)?;
            let t = out.timings;
            serde_json::to_writer(
                &mut w,
                &ResultRow {
                    id: &row.id,
                    lang: &row.lang,
                    reference: &row.text,
                    hyp: &out.text,
                    detected: None,
                    latency_s: t.total_s(),
                    duration_s: row.duration_s,
                    frontend_s: t.frontend_s,
                    encoder_s: t.encoder_s,
                    decode_s: t.decode_s,
                },
            )?;
            w.write_all(b"\n")?;
            timings.push(t);
        }
        w.flush()?;
        drop(w);
        std::fs::rename(&part, out_dir.join(format!("{set}.jsonl")))?;
        print_stats(set, &timings);
        all.extend(timings);
    }
    print_stats("all", &all);
    Ok(())
}

fn print_stats(label: &str, timings: &[Timings]) {
    let pick = |f: fn(&Timings) -> f64| {
        let mut v: Vec<f64> = timings.iter().map(f).collect();
        v.sort_by(f64::total_cmp);
        (percentile(&v, 0.5) * 1e3, percentile(&v, 0.95) * 1e3)
    };
    let (t50, t95) = pick(Timings::total_s);
    let (f50, _) = pick(|t| t.frontend_s);
    let (e50, e95) = pick(|t| t.encoder_s);
    let (d50, d95) = pick(|t| t.decode_s);
    println!(
        "{label:>16}: {} clips | total p50 {t50:.1} ms p95 {t95:.1} | front end p50 {f50:.2} | encoder p50 {e50:.1} p95 {e95:.1} | decode p50 {d50:.1} p95 {d95:.1}",
        timings.len()
    );
}

fn percentile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

fn read_manifest(path: &Path, limit: usize) -> Result<Vec<ManifestRow>> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut rows = Vec::new();
    for line in BufReader::new(file).lines() {
        rows.push(serde_json::from_str(&line?)?);
        if limit > 0 && rows.len() >= limit {
            break;
        }
    }
    Ok(rows)
}

/// Reads a 16 kHz mono WAV as f32 in [-1, 1] (PCM16 divided by 32768, as soundfile does).
fn read_wav(path: &Path) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path).with_context(|| format!("opening {}", path.display()))?;
    let spec = reader.spec();
    if spec.sample_rate != 16_000 || spec.channels != 1 {
        bail!("{}: {} Hz, {} channels; expected 16 kHz mono", path.display(), spec.sample_rate, spec.channels);
    }
    match spec.sample_format {
        hound::SampleFormat::Int if spec.bits_per_sample == 16 => {
            reader.samples::<i16>().map(|s| Ok(s? as f32 / 32768.0)).collect()
        }
        hound::SampleFormat::Float => reader.samples::<f32>().map(|s| Ok(s?)).collect(),
        _ => bail!("{}: {}-bit integer WAV; expected 16-bit or float", path.display(), spec.bits_per_sample),
    }
}
