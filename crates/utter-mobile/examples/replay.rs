//! Dictates recorded clips through the core as the app does: the bubble pressed, the audio
//! handed over in 30 ms chunks at speaking pace, the bubble released after the clip; prints
//! every event with its time since the press. For the emulator, whose microphone stays off.
//!
//!     replay <config.json> <clip.wav>...

use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use utter_mobile::{Config, Event, Mobile};

const CHUNK: usize = 480;

fn main() -> Result<()> {
    let result = run();
    // As in utter-cli: on Android, onnxruntime aborts in the exit hooks; leave without them.
    #[cfg(target_os = "android")]
    {
        use std::io::Write;
        if let Err(e) = &result {
            eprintln!("Error: {e:?}");
        }
        let _ = std::io::stdout().flush();
        // SAFETY: _exit ends the process at once; nothing runs after it.
        unsafe { libc::_exit(i32::from(result.is_err())) }
    }
    #[cfg(not(target_os = "android"))]
    result
}

fn run() -> Result<()> {
    utter_core::init_tracing();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((config, clips)) = args.split_first() else { bail!("usage: replay <config.json> <clip.wav>...") };
    let cfg: Config = serde_json::from_str(&std::fs::read_to_string(config).context("reading the config")?)?;
    let loaded = Instant::now();
    let mobile = Mobile::load(&cfg)?;
    println!("loaded in {:.1} s", loaded.elapsed().as_secs_f64());
    let clock = Instant::now();
    let uptime = || clock.elapsed().as_millis() as u32;
    for clip in clips {
        let audio = read_wav(clip)?;
        println!("\n{clip} ({:.2} s)", audio.len() as f64 / 16_000.0);
        let pressed = Instant::now();
        mobile.touch(true, uptime());
        for chunk in audio.chunks(CHUNK) {
            mobile.push_audio(chunk.to_vec());
            std::thread::sleep(Duration::from_millis(30));
            print_events(&mobile, pressed, Duration::ZERO);
        }
        mobile.touch(false, uptime());
        loop {
            let Some(e) = mobile.next_event(Duration::from_secs(30)) else { bail!("no final text within 30 s") };
            let last = matches!(e, Event::Final { .. } | Event::Error { .. } | Event::Cancelled);
            print_event(&e, pressed);
            if last {
                break;
            }
        }
        // As between dictations.
        std::thread::sleep(Duration::from_secs(1));
    }
    Ok(())
}

fn print_events(mobile: &Mobile, pressed: Instant, wait: Duration) {
    while let Some(e) = mobile.next_event(wait) {
        print_event(&e, pressed);
    }
}

fn print_event(e: &Event, pressed: Instant) {
    println!("{:>7.0} ms  {}", pressed.elapsed().as_secs_f64() * 1e3, serde_json::to_string(e).unwrap_or_default());
}

fn read_wav(path: &str) -> Result<Vec<f32>> {
    let mut reader = hound::WavReader::open(path).with_context(|| format!("opening {path}"))?;
    let spec = reader.spec();
    if spec.sample_rate != 16_000 || spec.channels != 1 {
        bail!("{path}: {} Hz, {} channels; 16 kHz mono expected", spec.sample_rate, spec.channels);
    }
    Ok(match spec.sample_format {
        hound::SampleFormat::Int => {
            reader.samples::<i16>().map(|s| s.map(|v| f32::from(v) / 32_768.0)).collect::<Result<_, _>>()?
        }
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
    })
}
