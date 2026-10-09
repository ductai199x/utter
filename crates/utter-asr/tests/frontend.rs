//! The Rust front end against Transformers' ParakeetFeatureExtractor.
//!
//! Fixtures (`tests/fixtures/`, written by `tools/export/check_parakeet.py --fixture`): a
//! synthetic 1.37 s clip (chirp, tone, quiet noise; an odd length), the features Transformers
//! computed for it, and the mel filterbank the export wrote.

use std::path::PathBuf;

use utter_asr::ModelConfig;
use utter_asr::frontend::MelFrontEnd;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join(name)
}

fn read_f32(name: &str) -> Vec<f32> {
    std::fs::read(fixture(name)).unwrap().as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect()
}

/// The front-end half of parakeet-ultra's config.json (the decoder half is unused here).
fn config() -> ModelConfig {
    serde_json::from_str(
        r#"{"weights": "", "vocab_size": 8193, "blank_id": 8192, "durations": [0,1,2,3,4],
            "max_symbols_per_step": 10, "decoder_layers": 2, "decoder_hidden": 640,
            "sample_rate": 16000, "n_fft": 512, "win_length": 400, "hop_length": 160,
            "n_mels": 128, "preemphasis": 0.97, "log_guard": 5.960464477539063e-08,
            "norm_epsilon": 1e-05}"#,
    )
    .unwrap()
}

fn frontend() -> MelFrontEnd {
    MelFrontEnd::new(&config(), read_f32("mel_filters.f32"))
}

#[test]
fn features_match_transformers() {
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(fixture("frontend.json")).unwrap()).unwrap();
    let audio = read_f32("frontend_audio.f32");
    let expected = read_f32("frontend_features.f32");
    assert_eq!(audio.len(), meta["samples"].as_u64().unwrap() as usize);

    let got = frontend().compute(&audio).unwrap();
    assert_eq!(got.frames, meta["frames"].as_u64().unwrap() as usize);
    assert_eq!(got.mels, 128);
    assert_eq!(got.data.len(), expected.len());

    // Normalized features are O(1); both sides compute the spectrum in f32, so they differ
    // by float rounding only.
    let worst = got.data.iter().zip(&expected).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
    assert!(worst < 2e-3, "largest difference from Transformers: {worst}");
}

#[test]
fn valid_frames_are_samples_over_hop() {
    let fe = frontend();
    assert_eq!(fe.frames_for(16_000), 100);
    assert_eq!(fe.frames_for(16_159), 100);
    assert_eq!(fe.frames_for(16_160), 101);
}

#[test]
fn a_clip_under_two_frames_is_refused() {
    let err = frontend().compute(&[0.1; 319]).unwrap_err();
    assert!(matches!(err, utter_asr::AsrError::TooShort { samples: 319 }));
}

#[test]
fn every_frame_is_normalized() {
    let audio = read_f32("frontend_audio.f32");
    let f = frontend().compute(&audio).unwrap();
    for m in 0..f.mels {
        let col: Vec<f64> = (0..f.frames).map(|t| f.data[t * f.mels + m] as f64).collect();
        let mean = col.iter().sum::<f64>() / col.len() as f64;
        assert!(mean.abs() < 1e-4, "mel {m}: mean {mean}");
    }
}
