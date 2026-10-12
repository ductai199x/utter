//! What went wrong, in words a person can act on (docs/ux/first-run.md, "Problem mode" and
//! "Download errors"). Errors are recognized by their kind first and by what they say second
//! (onnxruntime and llama-server report GPU memory and driver failures only as text).

use serde::Serialize;
use utter_llm::cleanup::Tier;
use utter_pipeline::PipelineError;
use utter_runtime::RuntimeError;

/// A failure to start dictation, as the problem screen shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// "gpu-memory", "no-microphone", "damaged", "driver" or "other".
    pub kind: &'static str,
    pub title: String,
    pub message: String,
    /// The error itself, for "Details".
    pub detail: String,
    /// The actions offered, in order: "retry", "repair", "logs", "quit" or "close".
    pub actions: Vec<&'static str>,
}


const OUT_OF_MEMORY: &[&str] =
    &["out of memory", "outofmemory", "cudaerrormemoryallocation", "failed to allocate", "cuda_error_out_of_memory"];
const DRIVER: &[&str] = &[
    "insufficient driver",
    "driver version is insufficient",
    "cudaerrorinsufficientdriver",
    "no cuda-capable device",
    "cuda_error_no_device",
    "cudaerrornodevice",
    "nvcuda",
];
const DAMAGED: &[&str] = &[
    "protobuf",
    "invalid model",
    "failed to load model",
    "not installed",
    "no such file",
    "cannot find the file",
    "gguf",
];

fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

/// The last 8 KB of llama-server's log, where it says why it stopped.
fn server_log_tail(e: &PipelineError) -> String {
    let path = match e {
        PipelineError::Llm(utter_llm::LlmError::Exited(_, p) | utter_llm::LlmError::StartTimeout(_, p)) => p,
        _ => return String::new(),
    };
    let Ok(bytes) = std::fs::read(path) else { return String::new() };
    String::from_utf8_lossy(&bytes[bytes.len().saturating_sub(8192)..]).into_owned()
}

pub fn diagnose(e: &PipelineError, tier: Tier) -> Problem {
    let detail = e.to_string();
    let text = format!("{detail}\n{}", server_log_tail(e)).to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| text.contains(w));
    let problem = |kind, title: &str, message: String, actions| Problem {
        kind,
        title: title.to_owned(),
        message,
        detail: detail.clone(),
        actions,
    };
    let free = utter_runtime::gpu::probe().ok().and_then(|g| g.free_bytes);
    let need = utter_pipeline::gpu_need(tier);

    if matches!(e, PipelineError::Audio(utter_audio::AudioError::NoDevice(_))) {
        return problem(
            "no-microphone",
            "No microphone found",
            "Plug in a microphone or headset, then retry.".into(),
            vec!["retry", "quit"],
        );
    }
    if has(OUT_OF_MEMORY) || free.is_some_and(|f| f < need) {
        let now = free.map(|f| format!("; {} {} free now", gb(f), if f < 2_000_000_000 { "is" } else { "are" }));
        return problem(
            "gpu-memory",
            "Not enough free GPU memory",
            format!(
                "Utter needs about {} free on the graphics card{}. Close games or other apps using the GPU, then retry.{}",
                gb(need),
                now.unwrap_or_default(),
                if tier == Tier::Quality && Tier::Light.offered() {
                    format!(" Or choose the Light model in Settings: it needs about {}.", gb(utter_pipeline::gpu_need(Tier::Light)))
                } else {
                    String::new()
                }
            ),
            vec!["retry", "quit"],
        );
    }
    if has(DRIVER) {
        return problem(
            "driver",
            "The graphics driver could not run Utter",
            "Update the NVIDIA driver, restart the computer, then retry.".into(),
            vec!["retry", "logs", "quit"],
        );
    }
    let damaged = matches!(
        e,
        PipelineError::Asr(utter_asr::AsrError::Io { .. } | utter_asr::AsrError::Model(_))
            | PipelineError::Llm(utter_llm::LlmError::Runtime(_))
    ) || has(DAMAGED);
    if damaged {
        return problem(
            "damaged",
            "Some of Utter's files are damaged",
            "Repair checks every file and downloads again the ones that are damaged.".into(),
            vec!["repair", "quit"],
        );
    }
    problem("other", "Utter could not start", first_line(&detail), vec!["retry", "logs", "quit"])
}

/// A model switch that failed after its download (the pipeline reports it as text).
pub fn model_switch(error: &str) -> Problem {
    let lower = error.to_lowercase();
    let (kind, title, message) = if OUT_OF_MEMORY.iter().any(|w| lower.contains(w)) {
        (
            "gpu-memory",
            "Not enough free GPU memory for this model",
            "Close games or other apps using the GPU, then retry, or choose a lighter model in Settings.",
        )
    } else {
        ("other", "The new model could not start", "The previous model is still in use. Retry, or open the logs.")
    };
    Problem {
        kind,
        title: title.into(),
        message: message.into(),
        detail: error.to_owned(),
        // Dictation keeps the previous model: closing is enough.
        actions: vec!["retry", "logs", "close"],
    }
}

fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or(s);
    if line.chars().count() > 160 { format!("{}...", line.chars().take(157).collect::<String>()) } else { line.into() }
}

/// What to tell the person when a download fails, and why.
pub fn download_message(e: &RuntimeError) -> String {
    match e {
        RuntimeError::Io { path, source }
            if source.kind() == std::io::ErrorKind::StorageFull || source.raw_os_error() == Some(112) =>
        {
            let drive =
                path.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned()).unwrap_or_default();
            let free = path.parent().and_then(utter_platform::disk::free_bytes);
            match free {
                Some(f) => format!("The disk is full ({} free on {drive}). Free up some space, then retry.", gb(f)),
                None => "The disk is full. Free up some space, then retry.".into(),
            }
        }
        RuntimeError::Download { message, .. } => {
            let m = message.to_lowercase();
            match m.find("http status") {
                Some(_) => {
                    let status: String = m.chars().filter(|c| c.is_ascii_digit()).take(3).collect();
                    format!("The download server answered {status}. Try again in a few minutes.")
                }
                None => "Can't reach the download server. Check your internet connection, then retry.".into(),
            }
        }
        RuntimeError::Checksum { .. } | RuntimeError::Size { .. } => {
            "A file arrived damaged. Retry downloads it again.".into()
        }
        other => format!("Download interrupted ({}). Retry picks up where it stopped.", first_line(&other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_microphone_says_so() {
        let p = diagnose(&PipelineError::Audio(utter_audio::AudioError::NoDevice(String::new())), Tier::Quality);
        assert_eq!(p.kind, "no-microphone");
        assert_eq!(p.actions, ["retry", "quit"]);
    }

    #[test]
    fn gpu_out_of_memory_is_recognized_from_onnxruntime_text() {
        let e = PipelineError::Asr(utter_asr::AsrError::Runtime(
            "Failed to allocate memory for requested buffer of size 52428800".into(),
        ));
        assert_eq!(diagnose(&e, Tier::Quality).kind, "gpu-memory");
    }

    #[test]
    fn download_failures_name_their_cause() {
        let net = RuntimeError::Download { url: "u".into(), message: "io: Connection refused".into() };
        assert!(download_message(&net).starts_with("Can't reach"));
        let http = RuntimeError::Download { url: "u".into(), message: "http status: 503".into() };
        assert_eq!(download_message(&http), "The download server answered 503. Try again in a few minutes.");
        let bad = RuntimeError::Checksum { id: "x".into(), url: "u".into(), expected: "a".into(), actual: "b".into() };
        assert!(download_message(&bad).starts_with("A file arrived damaged"));
    }
}
