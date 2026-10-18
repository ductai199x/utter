//! Which GPU backend Utter runs on (docs/plan-vulkan.md, docs/plan-cross-platform.md): CUDA on
//! an NVIDIA card, where it is fastest (the Vulkan build processes prompts twice as slowly there,
//! 2026-10-13); Vulkan on other cards (AMD, Intel) on Windows and Linux; Metal on Apple Silicon.
//! On Vulkan and Metal the speech model runs on onnxruntime's WebGPU provider (Dawn over D3D12,
//! Vulkan or Metal). Each backend has its own runtime components (utter_pipeline::components_for).
//!
//! `Cpu` runs everything on the processor: a computer without a usable GPU, or the person's choice
//! in Settings (2026-10-18; the Light model only). The app picks the backend
//! (utter_pipeline::runtime_choice) and [`select`]s it before anything asks.

use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Cuda,
    Vulkan,
    Metal,
    Cpu,
}

impl Backend {
    fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "cuda" => Some(Backend::Cuda),
            "vulkan" => Some(Backend::Vulkan),
            "metal" => Some(Backend::Metal),
            "cpu" => Some(Backend::Cpu),
            _ => None,
        }
    }

    /// Runs the speech model on onnxruntime's WebGPU provider (Vulkan, Metal).
    pub fn uses_webgpu(self) -> bool {
        matches!(self, Backend::Vulkan | Backend::Metal)
    }
}

/// The backend for this process, decided once: Metal on macOS; elsewhere CUDA when the NVIDIA
/// driver answers (gpu::probe), Vulkan otherwise. `UTTER_GPU_BACKEND` (cuda, vulkan, metal, cpu)
/// overrides it, to run another path for testing.
/// The backend the app chose (utter_pipeline::runtime_choice), before anything asked for one.
static SELECTED: OnceLock<Backend> = OnceLock::new();

/// Makes `backend` this process's backend. Call it before anything calls [`backend`]; later
/// calls do nothing. `UTTER_GPU_BACKEND` still wins (a developer's override).
pub fn select(backend: Backend) {
    let _ = SELECTED.set(backend);
}

pub fn backend() -> Backend {
    static BACKEND: OnceLock<Backend> = OnceLock::new();
    *BACKEND.get_or_init(|| {
        let chosen = std::env::var("UTTER_GPU_BACKEND").ok().and_then(|v| Backend::from_name(&v));
        let chosen = chosen.or_else(|| SELECTED.get().copied());
        let detected = if cfg!(target_os = "macos") {
            Backend::Metal
        } else if crate::gpu::probe().is_ok() {
            Backend::Cuda
        } else {
            Backend::Vulkan
        };
        let backend = chosen.unwrap_or(detected);
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
        assert_eq!(Backend::from_name("metal"), Some(Backend::Metal));
        assert_eq!(Backend::from_name("CPU"), Some(Backend::Cpu));
        assert_eq!(Backend::from_name("directml"), None);
    }
}
