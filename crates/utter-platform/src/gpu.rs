//! The graphics card as the OS sees it, whatever its maker (docs/plan-vulkan.md): what first run
//! checks, what picks the Vulkan device for llama-server and the WebGPU device for the speech
//! model, so all three are the same card. NVIDIA's own driver answers more (CUDA's version,
//! utter_runtime::gpu); this is for every card.

/// A graphics card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Adapter {
    pub name: String,
    /// The PCI vendor (0x10DE NVIDIA, 0x1002 AMD, 0x8086 Intel) and device ids.
    pub vendor_id: u32,
    pub device_id: u32,
    /// The OS's id for this adapter until reboot (low and high parts), which names it exactly
    /// where two cards are the same model.
    pub luid: (u32, i32),
    /// Its own memory (an integrated GPU's is small: it shares the system's).
    pub memory_bytes: u64,
    /// What the OS budgets this process now, less what it uses: the room left.
    pub free_bytes: Option<u64>,
}

/// The card to run on: the first in the OS's high-performance order (a discrete card before an
/// integrated one), software renderers skipped. `UTTER_GPU_INDEX` picks another place in that
/// order, to test on a second card (an integrated GPU beside the discrete one): llama-server
/// follows it, the speech model's WebGPU provider does not (utter_asr::parakeet).
pub fn high_performance_adapter() -> Option<Adapter> {
    let index = std::env::var("UTTER_GPU_INDEX").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    imp::adapters().into_iter().nth(index)
}

/// Whether the GPU driver installed Vulkan (its loader, vulkan-1.dll): llama.cpp's Vulkan build
/// needs nothing else.
pub fn vulkan_available() -> bool {
    imp::vulkan_available()
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE,
        DXGI_MEMORY_SEGMENT_GROUP_LOCAL, DXGI_QUERY_VIDEO_MEMORY_INFO, IDXGIAdapter1, IDXGIAdapter3, IDXGIFactory6,
    };
    use windows::core::Interface;

    use super::Adapter;

    /// Hardware adapters in high-performance order.
    pub fn adapters() -> Vec<Adapter> {
        // SAFETY: DXGI's documented calls; the interfaces are released when dropped.
        unsafe {
            let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory6>() else { return Vec::new() };
            let mut out = Vec::new();
            for i in 0.. {
                let Ok(adapter) =
                    factory.EnumAdapterByGpuPreference::<IDXGIAdapter1>(i, DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE)
                else {
                    break;
                };
                let Ok(desc) = adapter.GetDesc1() else { continue };
                if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                    continue;
                }
                let len = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
                let free_bytes = adapter.cast::<IDXGIAdapter3>().ok().and_then(|a| {
                    let mut info = DXGI_QUERY_VIDEO_MEMORY_INFO::default();
                    a.QueryVideoMemoryInfo(0, DXGI_MEMORY_SEGMENT_GROUP_LOCAL, &mut info).ok()?;
                    Some(info.Budget.saturating_sub(info.CurrentUsage))
                });
                out.push(Adapter {
                    name: String::from_utf16_lossy(&desc.Description[..len]),
                    vendor_id: desc.VendorId,
                    device_id: desc.DeviceId,
                    luid: (desc.AdapterLuid.LowPart, desc.AdapterLuid.HighPart),
                    memory_bytes: desc.DedicatedVideoMemory as u64,
                    free_bytes,
                });
            }
            out
        }
    }

    pub fn vulkan_available() -> bool {
        use windows_sys::Win32::Foundation::FreeLibrary;
        use windows_sys::Win32::System::LibraryLoader::LoadLibraryW;
        let name: Vec<u16> = "vulkan-1.dll\0".encode_utf16().collect();
        // SAFETY: loads and at once unloads a system library by name; nothing of it is called.
        unsafe {
            let module = LoadLibraryW(name.as_ptr());
            if module.is_null() {
                return false;
            }
            FreeLibrary(module);
            true
        }
    }
}

/// Linux: Vulkan's physical devices, through the driver's loader (libvulkan.so.1): discrete
/// cards first, then integrated ones; CPU implementations (lavapipe, llvmpipe) skipped, as DXGI's
/// software adapters are on Windows. The free memory is not asked (it takes an extension), so it
/// is unknown.
#[cfg(target_os = "linux")]
mod imp {
    use ash::vk;

    use super::Adapter;

    pub fn adapters() -> Vec<Adapter> {
        // SAFETY: loading the system's Vulkan loader; Vulkan calls with valid create infos, the
        // instance destroyed before returning.
        unsafe {
            let Ok(entry) = ash::Entry::load() else { return Vec::new() };
            let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_1);
            let info = vk::InstanceCreateInfo::default().application_info(&app);
            let Ok(instance) = entry.create_instance(&info, None) else { return Vec::new() };
            let mut found: Vec<(u8, Adapter)> = instance
                .enumerate_physical_devices()
                .unwrap_or_default()
                .into_iter()
                .filter_map(|device| {
                    let p = instance.get_physical_device_properties(device);
                    let rank = match p.device_type {
                        vk::PhysicalDeviceType::DISCRETE_GPU => 0,
                        vk::PhysicalDeviceType::INTEGRATED_GPU => 1,
                        vk::PhysicalDeviceType::VIRTUAL_GPU => 2,
                        _ => return None,
                    };
                    let memory = instance.get_physical_device_memory_properties(device);
                    let local = memory.memory_heaps[..memory.memory_heap_count as usize]
                        .iter()
                        .filter(|h| h.flags.contains(vk::MemoryHeapFlags::DEVICE_LOCAL))
                        .map(|h| h.size)
                        .max()
                        .unwrap_or(0);
                    let name = p.device_name_as_c_str().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    Some((
                        rank,
                        Adapter {
                            name,
                            vendor_id: p.vendor_id,
                            device_id: p.device_id,
                            luid: (0, 0),
                            memory_bytes: local,
                            free_bytes: None,
                        },
                    ))
                })
                .collect();
            instance.destroy_instance(None);
            found.sort_by_key(|(rank, a)| (*rank, std::cmp::Reverse(a.memory_bytes)));
            found.into_iter().map(|(_, a)| a).collect()
        }
    }

    /// The loader is there and offers a real GPU.
    pub fn vulkan_available() -> bool {
        !adapters().is_empty()
    }
}

/// macOS: Apple Silicon's GPU, through Metal (macos.rs).
#[cfg(target_os = "macos")]
mod imp {
    use super::Adapter;

    pub fn adapters() -> Vec<Adapter> {
        crate::macos::gpu_adapters()
    }

    pub fn vulkan_available() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "reads this machine's graphics cards"]
    fn the_high_performance_card_is_found() {
        let a = high_performance_adapter().expect("a graphics card");
        println!("{a:?}, Vulkan: {}", vulkan_available());
        assert!(!a.name.is_empty() && a.memory_bytes > 0);
    }
}
