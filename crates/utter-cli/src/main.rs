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
use utter_asr::boost::BoostConfig;
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
enum TierArg {
    Light,
    Quality,
}

impl From<TierArg> for utter_llm::cleanup::Tier {
    fn from(t: TierArg) -> Self {
        match t {
            TierArg::Light => Self::Light,
            TierArg::Quality => Self::Quality,
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
        /// Only these components (default: every component that is not optional; name an
        /// optional one, such as gemma4-26b-a4b-qat, to install it).
        ids: Vec<String>,
    },
    /// Show which runtime components and models are installed.
    Doctor,
    /// Dictate: hold Right Ctrl, speak, release; the text goes into the focused app. Double-tap
    /// Right Ctrl for hands-free, Esc to cancel, Ctrl+C here to quit.
    Dictate {
        /// Which cleanup model.
        #[arg(long, value_enum, default_value_t = TierArg::Light)]
        tier: TierArg,
        /// Dictionary words, comma-separated.
        #[arg(long, default_value = "")]
        dictionary: String,
        /// Insert raw transcripts, no cleanup model.
        #[arg(long)]
        no_cleanup: bool,
        /// An input device whose name contains this text (default: the system default).
        #[arg(long)]
        device: Option<String>,
    },
    /// Replay eval clips through the whole dictation pipeline in real time (segments while
    /// "speaking", gate, cleanup; no insertion) and measure key up -> text ready.
    E2eBench {
        /// Comma-separated set names.
        #[arg(long, default_value = "en-ami,en-voxpopuli")]
        sets: String,
        /// Clips per set.
        #[arg(long, default_value_t = 40)]
        limit: usize,
        /// Results name (results/e2e/<name>/ and results/asr/e2e-<name>/).
        #[arg(long, default_value = "light")]
        name: String,
        #[arg(long, value_enum, default_value_t = TierArg::Light)]
        tier: TierArg,
        /// Seconds of silence between clips, as between dictations.
        #[arg(long, default_value_t = 1.0)]
        gap: f64,
        /// A cleanup system prompt file instead of the product's.
        #[arg(long)]
        prompt: Option<PathBuf>,
        /// Clean only the dictations the gate flags (the old behavior), not every one.
        #[arg(long)]
        gated: bool,
    },
    /// Show where voice activity detection cuts a 16 kHz mono WAV file into segments.
    Vad { file: PathBuf },
    /// Record from the microphone (16 kHz mono, pre-roll included) to a WAV file.
    Record {
        /// Output WAV file.
        out: PathBuf,
        /// How long to record.
        #[arg(long, default_value_t = 3.0)]
        seconds: f64,
        /// An input device whose name contains this text (default: the system default).
        #[arg(long)]
        device: Option<String>,
    },
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
        /// Before each case, prefill the prompt cache with this fraction of its words, as the
        /// pipeline does while the user speaks (0: off). The timed request is the one after.
        #[arg(long, default_value_t = 0.0)]
        prefill_fraction: f64,
        /// Wait this long before each case, as a dictation does while the user speaks, to
        /// time a request that arrives after the server has sat idle (0: back to back).
        #[arg(long, default_value_t = 0)]
        gap_ms: u64,
        /// During the gap, send a one-token request this often to keep the server warm, as
        /// the pipeline does while the user speaks (0: none).
        #[arg(long, default_value_t = 0)]
        keepalive_ms: u64,
        /// Hold the GPU's clocks up through each gap and stop at its end, as the pipeline
        /// does from key down to key up.
        #[arg(long)]
        warm_gpu: bool,
        /// A system prompt file to use instead of the product's (prompts/cleanup.md).
        #[arg(long)]
        prompt: Option<PathBuf>,
    },
    /// Clean up dictated text with the installed cleanup model and its product settings.
    Clean {
        /// The text, as the speech model would write it.
        text: Vec<String>,
        /// Dictionary words, comma-separated.
        #[arg(long, default_value = "")]
        dictionary: String,
        /// Which cleanup model.
        #[arg(long, value_enum, default_value_t = TierArg::Light)]
        tier: TierArg,
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
        /// Boost these phrases while decoding, one per line (the user's dictionary).
        #[arg(long)]
        phrases: Option<PathBuf>,
        /// Log-probability bonus for a token that starts a phrase.
        #[arg(long, default_value_t = BoostConfig::default().start_score)]
        boost_start: f32,
        /// Log-probability bonus for a token that continues a phrase.
        #[arg(long, default_value_t = BoostConfig::default().score)]
        boost_score: f32,
        /// Added per token of depth into a phrase.
        #[arg(long, default_value_t = BoostConfig::default().depth_bonus)]
        boost_depth: f32,
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
        Command::Record { out, seconds, device } => return record(out, *seconds, device.clone()),
        Command::Vad { file } => return vad(file),
        Command::E2eBench { sets, limit, name, tier, gap, prompt, gated } => {
            let prompt = prompt.as_ref().map(std::fs::read_to_string).transpose().context("reading --prompt")?;
            let cfg = utter_pipeline::PipelineConfig {
                tier: (*tier).into(),
                prompt,
                always_clean: !*gated,
                ..Default::default()
            };
            return e2e_bench(sets, *limit, name, cfg, *gap);
        }
        Command::Dictate { tier, dictionary, no_cleanup, device } => {
            return dictate((*tier).into(), dictionary, *no_cleanup, device.clone());
        }
        Command::Chat { gguf, system, message, server_args } => return chat(gguf, system, message, server_args),
        Command::Clean { text, dictionary, tier } => return clean(&text.join(" "), dictionary, (*tier).into()),
        Command::LlmBench {
            gguf,
            cases,
            name,
            server_args,
            limit,
            prefill_fraction,
            gap_ms,
            keepalive_ms,
            warm_gpu,
            prompt,
        } => {
            let pacing = Pacing { gap_ms: *gap_ms, keepalive_ms: *keepalive_ms };
            let opts = LlmBenchOptions {
                prefill_fraction: *prefill_fraction,
                pacing,
                warm_gpu: *warm_gpu,
                prompt: prompt.clone(),
            };
            return llm_bench(gguf, cases, name, server_args, *limit, &opts);
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
        Command::Bench { sets, limit, name, gap_ms, keepalive_ms, phrases, boost_start, boost_score, boost_depth } => {
            if let Some(path) = phrases {
                let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
                let list: Vec<String> =
                    text.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_owned).collect();
                let cfg = BoostConfig {
                    start_score: boost_start,
                    score: boost_score,
                    depth_bonus: boost_depth,
                    ..BoostConfig::default()
                };
                let usable = asr.set_phrases(&list, cfg);
                println!("boosting {usable} of {} phrases ({cfg:?})", list.len());
            }
            bench(&mut asr, &sets, limit, &name, Pacing { gap_ms, keepalive_ms })
        }
        Command::Setup { .. }
        | Command::Doctor
        | Command::Chat { .. }
        | Command::LlmBench { .. }
        | Command::Clean { .. }
        | Command::Record { .. }
        | Command::Vad { .. }
        | Command::Dictate { .. }
        | Command::E2eBench { .. } => {
            unreachable!("handled before the speech model loads")
        }
    }
}

fn clean(text: &str, dictionary: &str, tier: utter_llm::cleanup::Tier) -> Result<()> {
    use utter_llm::{ChatParams, LlamaServer, cleanup};

    let dictionary: Vec<String> =
        dictionary.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
    let start = std::time::Instant::now();
    let server = LlamaServer::start(&cleanup::server_options(tier)?).context("starting the cleanup model")?;
    println!("cleanup model ({tier:?}) ready in {:.1} s", start.elapsed().as_secs_f64());
    let params = ChatParams { max_tokens: (text.len() as u32 / 2).max(64), ..ChatParams::default() };
    for round in ["cold", "warm"] {
        let out = server.chat(&cleanup::messages(tier, text, &dictionary), &params)?;
        println!("{round}: {:.0} ms", out.wall.as_secs_f64() * 1e3);
        if round == "warm" {
            println!("{}", cleanup::tidy_output(&out.text));
        }
    }
    Ok(())
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

/// How llm-bench runs each case (its command-line options).
struct LlmBenchOptions {
    prefill_fraction: f64,
    pacing: Pacing,
    warm_gpu: bool,
    prompt: Option<PathBuf>,
}

fn llm_bench(
    gguf: &Path,
    case_files: &[PathBuf],
    name: &str,
    server_args: &[String],
    limit: usize,
    bench: &LlmBenchOptions,
) -> Result<()> {
    use utter_llm::{ChatParams, LlamaServer, ServerOptions, cleanup};

    let LlmBenchOptions { prefill_fraction, pacing, warm_gpu, .. } = *bench;
    let system = match &bench.prompt {
        Some(path) => std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
        None => cleanup::SYSTEM_PROMPT.to_owned(),
    };
    let messages = |text: &str, dictionary: &[String]| {
        cleanup::messages_with(&system, text, dictionary, &cleanup::Context::default())
    };
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
            "prefill_fraction": prefill_fraction,
            "gap_ms": pacing.gap_ms,
            "keepalive_ms": pacing.keepalive_ms,
            "warm_gpu": warm_gpu,
            "llama_cpp": utter_runtime::component("llama-cpp")?.version,
            "prompt": bench.prompt.as_ref().map_or("crates/utter-llm/prompts/cleanup.md".to_owned(), |p| p.display().to_string()),
        }))?,
    )?;
    // Cleanup output is about as long as its input; a cap stops a model that starts
    // answering instead of cleaning from running on.
    let warmer = warm_gpu.then(utter_runtime::gpu::GpuWarmer::new);
    let params = |input: &str| ChatParams { max_tokens: (input.len() as u32 / 2).max(64), ..ChatParams::default() };
    for warm in ["Um, this is a warm-up.", "The the second warm-up, no, third."] {
        server.chat(&messages(warm, &[]), &params(warm))?;
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
            let gap_end = std::time::Instant::now() + std::time::Duration::from_millis(pacing.gap_ms);
            if let Some(w) = &warmer {
                w.start();
            }
            loop {
                let left = gap_end.saturating_duration_since(std::time::Instant::now());
                if pacing.keepalive_ms == 0 || left.is_zero() {
                    std::thread::sleep(left);
                    break;
                }
                std::thread::sleep(left.min(std::time::Duration::from_millis(pacing.keepalive_ms)));
                if std::time::Instant::now() < gap_end {
                    server.keep_warm(&messages("", &dictionary))?;
                }
            }
            if let Some(w) = &warmer {
                w.stop();
            }
            if prefill_fraction > 0.0 {
                let words: Vec<&str> = input.split_whitespace().collect();
                let keep = ((words.len() as f64) * prefill_fraction).floor() as usize;
                if keep > 0 {
                    let partial = words[..keep].join(" ");
                    let pre = server.prefill(&messages(&partial, &dictionary))?;
                    row.insert("prefill_ms".into(), (pre.wall.as_secs_f64() * 1e3).into());
                    row.insert("prefill_predicted_n".into(), pre.server.predicted_n.into());
                }
            }
            let out = server.chat(&messages(&input, &dictionary), &params(&input))?;
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
        utter_runtime::components().iter().filter(|c| !c.optional).collect()
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

fn dictate(tier: utter_llm::cleanup::Tier, dictionary: &str, no_cleanup: bool, device: Option<String>) -> Result<()> {
    use utter_pipeline::{Pipeline, PipelineConfig, Settings};

    // The app's settings, with --dictionary in place of its dictionary when given.
    let settings = Settings::load_or_create(&Settings::path()).map_err(|e| anyhow::anyhow!(e))?;
    let mut cfg = PipelineConfig {
        tier,
        no_cleanup,
        mic: utter_audio::MicConfig { device, ..Default::default() },
        ..Default::default()
    };
    settings.apply(&mut cfg);
    let dictionary: Vec<String> =
        dictionary.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
    if !dictionary.is_empty() {
        cfg.dictionary = dictionary;
    }
    let start = std::time::Instant::now();
    println!("Loading (speech model, cleanup model {tier:?}, microphone)...");
    let mut pipeline = Pipeline::start(cfg)?;
    println!(
        "Ready in {:.1} s. Microphone: {}. Hold Right Ctrl and speak; release to insert. Double-tap for hands-free, Esc cancels, Ctrl+C here quits.",
        start.elapsed().as_secs_f64(),
        pipeline.microphone().device_name()
    );
    pipeline.run(|event| {
        use utter_pipeline::Event;

        let d = match event {
            Event::Listening { hands_free, .. } => {
                print!("\r\x1b[2K{}", if hands_free { "listening (hands-free; tap Right Ctrl to stop)..." } else { "listening..." });
                let _ = std::io::stdout().flush();
                return;
            }
            Event::Words { committed, preview } => {
                // One line, rewritten as words arrive: the newest 110 characters.
                let line = format!("{committed} {preview}");
                let line = line.trim();
                let tail: String = line.chars().rev().take(110).collect::<Vec<_>>().into_iter().rev().collect();
                print!("\r\x1b[2K> {tail}");
                let _ = std::io::stdout().flush();
                return;
            }
            Event::Finishing | Event::Final { .. } => return,
            Event::Cancelled => {
                println!("\r\x1b[2Kcancelled");
                return;
            }
            Event::Done(d) => d,
        };
        let target = d.target.as_ref().map(|w| w.title.as_str()).unwrap_or("?");
        print!("\r\x1b[2K");
        println!("  heard:    {}", d.raw);
        if d.text != d.raw {
            println!("  cleaned:  {}", d.text);
        }
        match &d.inserted {
            Ok(()) => println!("  inserted into \"{target}\""),
            Err(e) => println!("  NOT inserted: {e}"),
        }
        println!(
            "  {:.1} s of audio in {} segment(s) | key up -> transcribed {:.0} ms | cleanup {} | key up -> pasted {:.0} ms",
            d.audio_s,
            d.segments,
            d.asr_ms,
            match d.gate {
                Some(why) => format!("{:.0} ms ({why})", d.cleanup_ms),
                None => "skipped".to_owned(),
            },
            d.total_ms
        );
    })?;
    Ok(())
}

fn e2e_bench(sets: &str, limit: usize, name: &str, cfg: utter_pipeline::PipelineConfig, gap: f64) -> Result<()> {
    use utter_pipeline::Pipeline;

    let mut pipeline = Pipeline::start(cfg)?;
    let eval = utter_core::data_dir().join("eval");
    let e2e_dir = eval.join("results").join("e2e").join(name);
    let asr_dir = eval.join("results").join("asr").join(format!("e2e-{name}"));
    std::fs::create_dir_all(&e2e_dir)?;
    std::fs::create_dir_all(&asr_dir)?;
    let mut all_total = Vec::new();
    for set in sets.split(',') {
        let rows = read_manifest(&eval.join(set).join("manifest.jsonl"), limit)?;
        let mut e2e = BufWriter::new(File::create(e2e_dir.join(format!("{set}.jsonl")))?);
        let mut asr = BufWriter::new(File::create(asr_dir.join(format!("{set}.jsonl")))?);
        let (mut total, mut asr_t, mut cleaned) = (Vec::new(), Vec::new(), 0usize);
        for row in &rows {
            let audio = read_wav(&eval.join(set).join(&row.audio))?;
            // Idle between dictations, as in real use (the GPU's clocks drop meanwhile).
            std::thread::sleep(std::time::Duration::from_secs_f64(gap));
            let d = pipeline.replay(&audio)?;
            serde_json::to_writer(
                &mut e2e,
                &serde_json::json!({
                    "id": row.id, "ref": row.text, "raw": d.raw, "text": d.text, "gate": d.gate,
                    "segments": d.segments, "audio_s": d.audio_s, "asr_ms": d.asr_ms,
                    "cleanup_ms": d.cleanup_ms, "total_ms": d.total_ms,
                }),
            )?;
            e2e.write_all(b"\n")?;
            serde_json::to_writer(
                &mut asr,
                &serde_json::json!({
                    "id": row.id, "lang": row.lang, "ref": row.text, "hyp": d.raw, "detected": null,
                    "latency_s": d.asr_ms / 1e3, "duration_s": row.duration_s,
                }),
            )?;
            asr.write_all(b"\n")?;
            total.push(d.total_ms);
            asr_t.push(d.asr_ms);
            cleaned += d.gate.is_some() as usize;
        }
        e2e.flush()?;
        asr.flush()?;
        let (t50, t95) = pct(&mut total);
        let (a50, a95) = pct(&mut asr_t);
        println!(
            "{set:>14}: {} clips | key up -> text ready p50 {t50:.0} ms p95 {t95:.0} | transcribed p50 {a50:.0} p95 {a95:.0} | cleanup ran on {cleaned}",
            rows.len()
        );
        all_total.extend(total);
    }
    let (t50, t95) = pct(&mut all_total);
    println!("{:>14}: key up -> text ready p50 {t50:.0} ms p95 {t95:.0} ms", "all");
    Ok(())
}

fn pct(v: &mut [f64]) -> (f64, f64) {
    v.sort_by(f64::total_cmp);
    (percentile(v, 0.5), percentile(v, 0.95))
}

fn vad(file: &Path) -> Result<()> {
    use utter_asr::vad::{FRAME, Segmenter, SegmenterConfig, SileroVad};

    let audio = read_wav(file)?;
    let mut vad = SileroVad::load_installed()?;
    let start = std::time::Instant::now();
    // Fed in 100 ms pieces, as the pipeline does while recording.
    let mut probs = Vec::new();
    for chunk in audio.chunks(1600) {
        probs.extend(vad.push(chunk)?);
    }
    let elapsed = start.elapsed();
    let mut seg = Segmenter::new(SegmenterConfig::default());
    let cuts = seg.push(&probs);
    let secs = |frame: usize| frame as f64 * FRAME as f64 / 16_000.0;
    let speech = probs.iter().filter(|&&p| p >= 0.5).count();
    println!(
        "{}: {:.2} s, {} frames ({} speech), VAD {:.1} ms in all ({:.3} ms per frame)",
        file.display(),
        audio.len() as f64 / 16_000.0,
        probs.len(),
        speech,
        elapsed.as_secs_f64() * 1e3,
        elapsed.as_secs_f64() * 1e3 / probs.len().max(1) as f64
    );
    let line: String = probs
        .iter()
        .map(|&p| {
            if p >= 0.5 {
                '#'
            } else if p >= 0.35 {
                '+'
            } else {
                '.'
            }
        })
        .collect();
    for (i, row) in line.as_bytes().chunks(100).enumerate() {
        println!("  {:>6.2}s {}", secs(i * 100), std::str::from_utf8(row).unwrap_or(""));
    }
    println!("cuts at: {:?}", cuts.iter().map(|&c| format!("{:.2}s", secs(c))).collect::<Vec<_>>());
    Ok(())
}

fn record(out: &Path, seconds: f64, device: Option<String>) -> Result<()> {
    use std::time::{Duration, Instant};

    let mic = utter_audio::Microphone::open(&utter_audio::MicConfig { device, ..Default::default() })?;
    println!("Device: {} ({} Hz native)", mic.device_name(), mic.native_rate());
    // Let the pre-roll fill, as it would have long before any key press.
    std::thread::sleep(Duration::from_millis(400));
    mic.start();
    let start = Instant::now();
    let mut peak = 0.0f32;
    while start.elapsed().as_secs_f64() < seconds {
        peak = peak.max(mic.level());
        std::thread::sleep(Duration::from_millis(20));
    }
    let audio = mic.stop(Duration::from_millis(80));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: utter_audio::SAMPLE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(out, spec)?;
    for s in &audio {
        w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16)?;
    }
    w.finalize()?;
    let rms = (audio.iter().map(|s| s * s).sum::<f32>() / audio.len().max(1) as f32).sqrt();
    println!(
        "{}: {:.2} s ({} samples, pre-roll included), RMS {:.4}, peak callback RMS {:.4}",
        out.display(),
        audio.len() as f64 / utter_audio::SAMPLE_RATE as f64,
        audio.len(),
        rms,
        peak
    );
    Ok(())
}

fn doctor() -> Result<()> {
    println!("Data folder: {}", utter_core::data_dir().display());
    println!("Runtime components:");
    for c in utter_runtime::components() {
        let state = match (c.is_installed(), c.optional) {
            (true, _) => "installed".to_owned(),
            (false, true) => format!("optional (utter-cli setup {})", c.id),
            (false, false) => "MISSING (utter-cli setup)".to_owned(),
        };
        println!(
            "  {:<26} {:<10} {:>7.0} MB  {state}  -- {}",
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
            let (text, t) = transcribe_any_length(asr, audio)?;
            serde_json::to_writer(
                &mut w,
                &ResultRow {
                    id: &row.id,
                    lang: &row.lang,
                    reference: &row.text,
                    hyp: &text,
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

/// Transcribes a clip of any length: one past the encoder's limit (about 41 s) is split at its
/// quietest 100 ms in the middle fifth, as the live segmenter would cut it at a pause, and the
/// halves are transcribed in turn. Timings add up.
fn transcribe_any_length(asr: &mut ParakeetTdt, audio: &[f32]) -> Result<(String, Timings)> {
    match asr.transcribe(audio) {
        Ok(out) => Ok((out.text, out.timings)),
        Err(utter_asr::AsrError::TooLong { .. }) => {
            let at = quietest_split(audio);
            let (a, ta) = transcribe_any_length(asr, &audio[..at])?;
            let (b, tb) = transcribe_any_length(asr, &audio[at..])?;
            let text = [a.trim(), b.trim()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" ");
            let t = Timings {
                frontend_s: ta.frontend_s + tb.frontend_s,
                encoder_s: ta.encoder_s + tb.encoder_s,
                decode_s: ta.decode_s + tb.decode_s,
                encoder_frames: ta.encoder_frames + tb.encoder_frames,
                joint_calls: ta.joint_calls + tb.joint_calls,
                decoder_calls: ta.decoder_calls + tb.decoder_calls,
            };
            Ok((text, t))
        }
        Err(e) => Err(e.into()),
    }
}

/// The start of the quietest 100 ms window between 40% and 60% of the clip.
fn quietest_split(audio: &[f32]) -> usize {
    const WINDOW: usize = 1_600;
    let (lo, hi) = (audio.len() * 2 / 5, audio.len() * 3 / 5);
    (lo..hi.saturating_sub(WINDOW).max(lo + 1))
        .step_by(WINDOW / 4)
        .min_by(|&a, &b| {
            let energy = |s: usize| audio[s..(s + WINDOW).min(audio.len())].iter().map(|x| x * x).sum::<f32>();
            energy(a).total_cmp(&energy(b))
        })
        .unwrap_or(audio.len() / 2)
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
