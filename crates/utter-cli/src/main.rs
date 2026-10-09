//! `utter-cli`: the developer command line of Utter's core.
//!
//! - `setup`: download, verify and install the runtime components.
//! - `doctor`: show what is installed.
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
    /// A local export folder to use instead of the installed speech model component
    /// (e.g. a fresh tools/export output under <data_dir>/models/).
    #[arg(long, global = true)]
    model_dir: Option<PathBuf>,
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
    /// Download, verify and install Utter's runtime components (crates/utter-runtime/components.json).
    Setup {
        /// Only these components (default: all).
        ids: Vec<String>,
    },
    /// Show which runtime components and models are installed.
    Doctor,
    /// Run the cleanup cases through Utter's llama-server on a GGUF model and write the outputs
    /// for tools/eval/cleanup_score.py.
    LlmBench {
        /// The GGUF model file.
        #[arg(long)]
        gguf: PathBuf,
        /// Case files (JSONL with id, input, optional dictionary).
        #[arg(long, required = true)]
        cases: Vec<PathBuf>,
        /// Name the results are written under (results/llm/<name>/).
        #[arg(long)]
        name: String,
        /// Extra arguments for llama-server, e.g. --server-arg=--spec-type --server-arg=ngram-mod.
        #[arg(long = "server-arg", allow_hyphen_values = true)]
        server_args: Vec<String>,
        /// Cases per file (0: all).
        #[arg(long, default_value_t = 0)]
        limit: usize,
    },
    /// Start Utter's llama-server on a GGUF model and send one chat message.
    Chat {
        /// The GGUF model file.
        #[arg(long)]
        gguf: PathBuf,
        /// System prompt.
        #[arg(long, default_value = "You are a helpful assistant.")]
        system: String,
        /// The user message.
        message: String,
        /// Extra arguments for llama-server, e.g. --server-arg=--spec-type --server-arg=ngram-mod.
        #[arg(long = "server-arg", allow_hyphen_values = true)]
        server_args: Vec<String>,
    },
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
    match &cli.command {
        Command::Setup { ids } => return setup(ids),
        Command::Doctor => return doctor(),
        Command::Chat { gguf, system, message, server_args } => return chat(gguf, system, message, server_args),
        Command::LlmBench { gguf, cases, name, server_args, limit } => {
            return llm_bench(gguf, cases, name, server_args, *limit);
        }
        _ => {}
    }
    let opts = ParakeetOptions {
        decoder: cli.decoder_device.into(),
        tf32: !cli.no_tf32,
        conv_search: cli.conv_search.into(),
        memory_pattern: !cli.no_memory_pattern,
        cpu_threads: cli.cpu_threads,
        cpu_spinning: !cli.no_cpu_spinning,
        ..Default::default()
    };
    let dir = match &cli.model_dir {
        Some(dir) => dir.clone(),
        None => utter_asr::installed_model_dir()?,
    };
    let mut asr = ParakeetTdt::load(&dir, opts).with_context(|| format!("loading {}", dir.display()))?;
    match cli.command {
        Command::Transcribe { files } => transcribe(&mut asr, &files),
        Command::Bench { sets, limit, name, gap_ms, keepalive_ms } => {
            bench(&mut asr, &sets, limit, &name, Pacing { gap_ms, keepalive_ms })
        }
        Command::Setup { .. } | Command::Doctor | Command::Chat { .. } | Command::LlmBench { .. } => {
            unreachable!("handled before the speech model loads")
        }
    }
}

fn chat(gguf: &Path, system: &str, message: &str, server_args: &[String]) -> Result<()> {
    use utter_llm::{ChatParams, LlamaServer, Message, ServerOptions};

    let mut opts = ServerOptions::new(gguf);
    opts.extra_args = server_args.to_vec();
    let start = std::time::Instant::now();
    let server = LlamaServer::start(&opts).with_context(|| format!("starting llama-server on {}", gguf.display()))?;
    println!(
        "llama-server ready in {:.1} s at {} (log: {})",
        start.elapsed().as_secs_f64(),
        server.base_url(),
        server.log_path().display()
    );
    let messages = [Message::system(system), Message::user(message)];
    for round in 1..=2 {
        let out = server.chat(&messages, &ChatParams::default())?;
        let s = out.server;
        println!(
            "round {round}: {:.1} ms wall | prompt {} tokens ({} cached) in {:.1} ms | {} tokens in {:.1} ms ({:.0} tok/s)",
            out.wall.as_secs_f64() * 1e3,
            s.prompt_n,
            s.cache_n,
            s.prompt_ms,
            s.predicted_n,
            s.predicted_ms,
            s.predicted_n as f64 / (s.predicted_ms / 1e3).max(1e-9)
        );
        if round == 1 {
            println!("{}", out.text);
        }
    }
    Ok(())
}

fn llm_bench(gguf: &Path, case_files: &[PathBuf], name: &str, server_args: &[String], limit: usize) -> Result<()> {
    use utter_llm::{ChatParams, LlamaServer, ServerOptions, cleanup};

    let mut opts = ServerOptions::new(gguf);
    opts.extra_args = server_args.to_vec();
    let server = LlamaServer::start(&opts).with_context(|| format!("starting llama-server on {}", gguf.display()))?;
    let out_dir = utter_core::data_dir().join("eval").join("results").join("llm").join(name);
    std::fs::create_dir_all(&out_dir)?;
    std::fs::write(
        out_dir.join("notes.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "gguf": gguf.display().to_string(),
            "server_args": server_args,
            "llama_cpp": utter_runtime::component("llama-cpp")?.version,
            "prompt": "crates/utter-llm/prompts/cleanup.md",
        }))?,
    )?;
    // Cleanup output is about as long as its input; a cap stops a model that starts
    // answering instead of cleaning from running on.
    let params = |input: &str| ChatParams { max_tokens: (input.len() as u32 / 2).max(64), ..ChatParams::default() };
    for warm in ["Um, this is a warm-up.", "The the second warm-up, no, third."] {
        server.chat(&cleanup::messages(warm, &[]), &params(warm))?;
    }
    for file in case_files {
        let mut rows: Vec<serde_json::Map<String, serde_json::Value>> = Vec::new();
        for line in BufReader::new(File::open(file).with_context(|| format!("opening {}", file.display()))?).lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            rows.push(serde_json::from_str(&line)?);
            if limit > 0 && rows.len() >= limit {
                break;
            }
        }
        let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("cases");
        let part = out_dir.join(format!("{stem}.jsonl.part"));
        let mut w = BufWriter::new(File::create(&part)?);
        let mut walls = Vec::with_capacity(rows.len());
        for mut row in rows {
            let input = row.get("input").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
            let dictionary: Vec<String> = row
                .get("dictionary")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            let out = server.chat(&cleanup::messages(&input, &dictionary), &params(&input))?;
            let s = out.server;
            row.insert("output".into(), cleanup::tidy_output(&out.text).into());
            row.insert("raw_output".into(), out.text.clone().into());
            row.insert("wall_ms".into(), (out.wall.as_secs_f64() * 1e3).into());
            row.insert("prompt_n".into(), s.prompt_n.into());
            row.insert("cache_n".into(), s.cache_n.into());
            row.insert("prompt_ms".into(), s.prompt_ms.into());
            row.insert("predicted_n".into(), s.predicted_n.into());
            row.insert("predicted_ms".into(), s.predicted_ms.into());
            serde_json::to_writer(&mut w, &row)?;
            w.write_all(b"\n")?;
            walls.push(out.wall.as_secs_f64());
        }
        w.flush()?;
        drop(w);
        std::fs::rename(&part, out_dir.join(format!("{stem}.jsonl")))?;
        walls.sort_by(f64::total_cmp);
        println!(
            "{stem:>20}: {} cases | wall p50 {:.0} ms p95 {:.0} ms",
            walls.len(),
            percentile(&walls, 0.5) * 1e3,
            percentile(&walls, 0.95) * 1e3
        );
    }
    Ok(())
}

fn setup(ids: &[String]) -> Result<()> {
    let chosen: Vec<&utter_runtime::Component> = if ids.is_empty() {
        utter_runtime::components().iter().collect()
    } else {
        ids.iter().map(|id| utter_runtime::component(id)).collect::<Result<_, _>>()?
    };
    let pending: u64 = chosen.iter().filter(|c| !c.is_installed()).map(|c| c.download_size()).sum();
    println!("Runtime folder: {}", utter_runtime::runtime_dir().display());
    println!("To download: {:.2} GB", pending as f64 / 1e9);
    for c in chosen {
        if c.is_installed() {
            println!("  {} {}: installed", c.id, c.version);
            continue;
        }
        let mut last_pct = u64::MAX;
        utter_runtime::install(c, |p| match p {
            utter_runtime::Progress::Downloading { done, total } => {
                let pct = done * 100 / total.max(1);
                if pct / 10 != last_pct / 10 {
                    println!("  {} {}: downloading {pct}%", c.id, c.version);
                    last_pct = pct;
                }
            }
            utter_runtime::Progress::Verifying => println!("  {} {}: verifying SHA-256", c.id, c.version),
            utter_runtime::Progress::Extracting => println!("  {} {}: extracting", c.id, c.version),
        })
        .with_context(|| format!("installing {}", c.id))?;
        println!("  {} {}: installed", c.id, c.version);
    }
    Ok(())
}

fn doctor() -> Result<()> {
    println!("Data folder: {}", utter_core::data_dir().display());
    println!("Runtime components:");
    for c in utter_runtime::components() {
        let state = if c.is_installed() { "installed" } else { "MISSING (utter-cli setup)" };
        println!(
            "  {:<20} {:<12} {:>8.0} MB  {state}  -- {}",
            c.id,
            c.version,
            c.download_size() as f64 / 1e6,
            c.purpose
        );
    }
    Ok(())
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
