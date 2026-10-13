//! Which GPU backend Utter runs on (docs/plan-vulkan.md): CUDA on an NVIDIA card, where it is
//! fastest (the Vulkan build processes prompts twice as slowly there, 2026-10-13); Vulkan
//! everywhere else (AMD, Intel), with the speech model on onnxruntime's WebGPU provider. Each
//! backend has its own runtime components (utter_pipeline::components_for).

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Cuda,
    Vulkan,
}

impl Backend {
    fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "cuda" => Some(Backend::Cuda),
            "vulkan" => Some(Backend::Vulkan),
            _ => None,
        }
    }
}

/// The backend for this process, decided once: CUDA when the NVIDIA driver answers
/// (gpu::probe), Vulkan otherwise. `UTTER_GPU_BACKEND` (cuda or vulkan) overrides it, to run the
/// Vulkan path on an NVIDIA card for testing.
pub fn backend() -> Backend {
    static BACKEND: OnceLock<Backend> = OnceLock::new();
    *BACKEND.get_or_init(|| {
        let chosen = std::env::var("UTTER_GPU_BACKEND").ok().and_then(|v| Backend::from_name(&v));
        let backend = chosen.unwrap_or(if crate::gpu::probe().is_ok() { Backend::Cuda } else { Backend::Vulkan });
        tracing::info!(?backend, overridden = chosen.is_some(), "GPU backend");
        backend
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backends_are_named_as_the_override_spells_them() {
        assert_eq!(Backend::from_name("Vulkan"), Some(Backend::Vulkan));
        assert_eq!(Backend::from_name("cuda"), Some(Backend::Cuda));
        assert_eq!(Backend::from_name("metal"), None);
    }
}
