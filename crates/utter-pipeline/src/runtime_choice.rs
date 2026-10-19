//! What runs the models on this computer: the backends it can run, Utter's pick among them, and
//! the person's choice in Settings (2026-10-18: CUDA, Vulkan or the processor, offering only
//! what the computer has; with no usable GPU, the processor with Light or Extra light, Quality
//! unavailable).

use std::sync::OnceLock;

use utter_runtime::Backend;

use crate::settings::{Model, Runtime};

/// The backends this computer can run, best first: CUDA where NVIDIA's driver is (fastest on
/// its cards), Vulkan where a graphics card offers it (any maker's, NVIDIA's too), then the
/// processor; on a Mac, Metal then the processor. Asked once per process.
pub fn available() -> &'static [Backend] {
    static AVAILABLE: OnceLock<Vec<Backend>> = OnceLock::new();
    AVAILABLE.get_or_init(|| {
        let mut found = Vec::new();
        if cfg!(target_os = "macos") {
            // Every Apple Silicon Mac has Metal.
            found.push(Backend::Metal);
        } else {
            if utter_runtime::gpu::probe().is_ok() {
                found.push(Backend::Cuda);
            }
            if utter_platform::gpu::high_performance_adapter().is_some() && utter_platform::gpu::vulkan_available() {
                found.push(Backend::Vulkan);
            }
        }
        found.push(Backend::Cpu);
        tracing::info!(?found, "backends this computer runs");
        found
    })
}

/// The backend `choice` names, if this computer runs it; `Auto`: the best it runs.
pub fn resolve(choice: Runtime) -> Backend {
    let wanted = match choice {
        Runtime::Auto => None,
        Runtime::Cuda => Some(Backend::Cuda),
        Runtime::Vulkan => Some(Backend::Vulkan),
        Runtime::Metal => Some(Backend::Metal),
        Runtime::Cpu => Some(Backend::Cpu),
    };
    let available = available();
    match wanted {
        Some(b) if available.contains(&b) => b,
        Some(b) => {
            tracing::warn!(?b, "the chosen backend is not available here; using the best there is");
            available[0]
        }
        None => available[0],
    }
}

/// Makes the backend `choice` resolves to this process's backend (utter_runtime::backend).
/// Call it at start, before anything asks for the backend.
pub fn select(choice: Runtime) {
    utter_runtime::select_backend(resolve(choice));
}

/// The setting for `backend`, as Settings names it.
pub fn setting_for(backend: Backend) -> Runtime {
    match backend {
        Backend::Cuda => Runtime::Cuda,
        Backend::Vulkan => Runtime::Vulkan,
        Backend::Metal => Runtime::Metal,
        Backend::Cpu => Runtime::Cpu,
    }
}

/// Whether `model` runs on `backend`. Quality needs a graphics card: on the processor its
/// cleanup takes several seconds. Extra light is for the processor; with a graphics card Light
/// is as fast.
pub fn runs_on(model: Model, backend: Backend) -> bool {
    match model {
        Model::Quality => backend != Backend::Cpu,
        Model::Light => true,
        Model::Mini => backend == Backend::Cpu,
    }
}

/// The models offered on `backend`, best first.
pub fn models_on(backend: Backend) -> Vec<Model> {
    [Model::Quality, Model::Light, Model::Mini].into_iter().filter(|&m| runs_on(m, backend)).collect()
}

/// The model to run: `model`, or where it does not run on this process's backend, the one
/// there instead (Quality on the processor: Extra light, the fast one; Extra light with a
/// graphics card: Light).
pub fn effective_model(model: Model) -> Model {
    let backend = utter_runtime::backend();
    if runs_on(model, backend) {
        model
    } else if backend == Backend::Cpu {
        Model::Mini
    } else {
        Model::Light
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_does_not_run_on_the_processor() {
        assert!(!runs_on(Model::Quality, Backend::Cpu));
        assert!(runs_on(Model::Light, Backend::Cpu));
        assert!(runs_on(Model::Quality, Backend::Vulkan));
        assert!(runs_on(Model::Mini, Backend::Cpu));
        assert!(!runs_on(Model::Mini, Backend::Cuda));
    }

    #[test]
    fn the_processor_is_always_there() {
        assert_eq!(available().last(), Some(&Backend::Cpu));
        assert_eq!(resolve(Runtime::Cpu), Backend::Cpu);
    }
}
