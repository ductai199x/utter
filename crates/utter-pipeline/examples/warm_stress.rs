//! Speech passes of growing length while the GPU warmer spins, as during a long dictation;
//! reports any pass that stalls. `cargo run --release -p utter-pipeline --example warm_stress`

use std::time::{Duration, Instant};

use utter_asr::{ParakeetOptions, ParakeetTdt};
use utter_runtime::gpu::GpuWarmer;

fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let warm = std::env::args().nth(1).as_deref() != Some("--no-warm");
    let dir = utter_asr::installed_model_dir().expect("model installed");
    let mut asr = ParakeetTdt::load(&dir, ParakeetOptions::default()).expect("load");
    let warmer = GpuWarmer::new();
    std::thread::sleep(Duration::from_millis(500));
    // Low noise, so the decoder has a little to do.
    let audio: Vec<f32> = (0..16_000 * 25).map(|i| (i as f32 * 0.37).sin() * 0.01).collect();
    let mut worst = Duration::ZERO;
    for round in 0..20 {
        if warm {
            warmer.start();
        }
        for len_s in (1..=22).step_by(1) {
            let t = Instant::now();
            asr.transcribe(&audio[..16_000 * len_s]).expect("transcribe");
            let took = t.elapsed();
            worst = worst.max(took);
            if took > Duration::from_millis(500) {
                println!("round {round} len {len_s}s: STALL {took:?}");
            }
            // Keep-alive sized pass, as the pipeline interleaves.
            asr.transcribe(&audio[..8_000]).expect("transcribe");
        }
        warmer.stop();
        println!("round {round} done, worst so far {worst:?}");
        std::thread::sleep(Duration::from_millis(300));
    }
}
