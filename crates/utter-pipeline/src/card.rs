//! The graphics card Utter runs on, as first run, Settings and the problem screens describe it,
//! whatever the backend (utter_runtime::backend): NVIDIA's driver answers on CUDA, DXGI on
//! Vulkan (docs/plan-vulkan.md).

use utter_runtime::Backend;

#[derive(Debug, Clone)]
pub struct Card {
    pub name: String,
    pub memory_bytes: u64,
    /// The room left on it now, when known: free memory on CUDA, this process's budget left on
    /// Vulkan.
    pub free_bytes: Option<u64>,
    pub driver: Driver,
}

/// What the driver offers the backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Driver {
    /// The newest CUDA version it supports (1000 * major + 10 * minor).
    Cuda(i32),
    /// Whether it installed Vulkan.
    Vulkan(bool),
}

impl Driver {
    /// New enough for Utter's runtime components.
    pub fn ok(self) -> bool {
        match self {
            Driver::Cuda(version) => version >= utter_runtime::gpu::MIN_DRIVER_CUDA,
            Driver::Vulkan(present) => present,
        }
    }
}

/// The card, or why there is none to run on.
pub fn graphics_card() -> Result<Card, String> {
    match utter_runtime::backend() {
        Backend::Cuda => utter_runtime::gpu::probe().map(|g| Card {
            name: g.name,
            memory_bytes: g.memory_bytes,
            free_bytes: g.free_bytes,
            driver: Driver::Cuda(g.driver_cuda),
        }),
        Backend::Vulkan => {
            let a = utter_platform::gpu::high_performance_adapter().ok_or("no graphics card found")?;
            Ok(Card {
                name: a.name,
                memory_bytes: a.memory_bytes,
                free_bytes: a.free_bytes,
                driver: Driver::Vulkan(utter_platform::gpu::vulkan_available()),
            })
        }
    }
}
