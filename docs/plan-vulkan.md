# Plan: every GPU (Vulkan), 2026-10-13

Today Utter needs an NVIDIA card: llama.cpp's CUDA build, onnxruntime's CUDA provider with the
CUDA runtime and cuDNN, a GPU probe through the CUDA driver, and the GPU clock hold (a napping
CUDA kernel, DESIGN.md "Idle GPU"). This plan makes AMD and Intel graphics cards run Utter, with
nothing installed but the driver, without costing NVIDIA anything.

## Measured so far (RTX 3090 Ti, Ryzen 9 9900X, 2026-10-13)

Quality v2 (Gemma 4 E4B Q4_0) through `utter-cli llm-bench`, the app's prompt and server
settings, on 100 cases of tonight's kokoro-real eval; llama.cpp b11535, the official Windows
Vulkan zip (`llama-b11535-bin-win-vulkan-x64.zip`, 33,459,069 bytes, sha256 30f30bb9...) against
the CUDA build Utter ships:

| | CUDA, 3090 Ti | Vulkan, 3090 Ti | Vulkan, Radeon iGPU (2 CUs) |
|---|---|---|---|
| Cleanup wall p50 / p95 | **250 / 387 ms** | 274 / 417 ms | 6,464 / 9,481 ms (10 cases) |
| Prompt processing p50 | 21.3 ms | 45.3 ms | 1,619 ms |
| Per generated token p50 | 4.20 ms | 4.21 ms | 107.5 ms |
| Outputs identical to CUDA | - | 99 of 100 | 9 of 10 |

- Vulkan generates as fast as CUDA on NVIDIA; prompt processing is twice as slow, 24 ms of
  key-up-to-text. **NVIDIA keeps CUDA.**
- The one differing output differs in case only ("the bistro" / "The Bistro"): float
  differences, not a backend bug.
- The Vulkan build sees the iGPU too (`Vulkan1: AMD Radeon(TM) Graphics`) and by default splits
  the model across every GPU it finds: **Utter must always pass `--device`.**
- The 2-CU iGPU runs it correctly but far too slowly: integrated graphics this small are not a
  target. A real AMD or Intel card is needed to measure the target.

Speech: Utter's Parakeet encoder exports on onnxruntime 1.31 with the WebGPU plugin provider
(`onnxruntime-ep-webgpu` 0.4.0, Dawn over D3D12; PyPI wheel 13,149,847 bytes, sha256 checked),
against the CUDA provider, 60 clips of the AMI and VoxPopuli bench sets:

| Encoder | CUDA p50 / p95 | WebGPU p50 / p95 | Transcripts identical to CUDA |
|---|---|---|---|
| fp16 | 20.0 / 24.5 ms | 47.8 / 76.3 ms | 59 of 60 |
| 8-bit (MatMulNBits) | 24.8 / 28.3 ms | 81.3 / 106.8 ms | 59 of 60 |

- The one differing clip is the same in both and a borderline word ("L label" / "Label").
- WebGPU is about 2.5 times slower than CUDA on this NVIDIA card at steady state (33.9 against
  13.5 ms for 500 frames), and a new input length costs nothing extra (pipelines are reused across
  lengths). Validation mode and the buffer cache mode change nothing.
- The 8-bit encoder is slower on WebGPU than fp16: **WebGPU runs the fp16 encoder.**
- About 900 nodes stay on the CPU, as with CUDA: shape arithmetic and the padding amounts of
  each layer's convolution, all small integer tensors; no audio data crosses.
- The plugin lists every adapter with its LUID and DXGI index (the 3090 Ti and the Radeon iGPU),
  which ties it to the probe and to llama.cpp's Vulkan device.

## Architecture

- **Backends.** `cuda` (NVIDIA, as today) and `vulkan` (AMD, Intel; NVIDIA only if a
  measurement ever shows parity). components.json gains a `backend` per component
  (`cuda`, `vulkan`, `any`); `components_for(tier, backend)` and `gpu_need(tier, backend)` replace
  today's single lists. New components: `llama-cpp-vulkan` (b11535, the official zip: llama-server,
  ggml-vulkan.dll and the CPU backends; needs only the driver's vulkan-1.dll), `onnxruntime`
  (the CPU build of 1.31) and `onnxruntime-ep-webgpu` (the WebGPU plugin provider, pinned from its
  PyPI wheel like cuDNN is today).
- **The GPU probe** moves from the CUDA driver to DXGI: adapters by high-performance preference,
  software adapters skipped, vendor (0x10DE NVIDIA, 0x1002 AMD, 0x8086 Intel), dedicated memory,
  and the OS budget (IDXGIAdapter3::QueryVideoMemoryInfo). The adapter's LUID ties it to the
  matching Vulkan device (`--device VulkanN`) and the WebGPU adapter, whose orders differ.
  `card_need` is redefined around the budget, which is per process, not global free memory.
- **Cleanup on Vulkan.** llama-server from the Vulkan component, always with `--device`,
  `--flash-attn auto` (flash attention hurts prompt processing on some Intel cards).
- **Speech on WebGPU.** onnxruntime's WebGPU plugin provider (Dawn over D3D12 or Vulkan) runs the
  fp16 and the 8-bit encoder (its MatMulNBits supports 8 bits); the ort crate already registers
  plugin libraries (`Environment::register_ep_library`). Like CUDA today, it fails loudly rather
  than falling back to the CPU. DirectML (in sustained engineering, packages stopped at 1.24) is a
  fallback only if WebGPU fails on a card.
- **The clock hold** on Vulkan: a napping compute dispatch, as the CUDA kernel does, so a pass
  after a pause is not five times slower.
- **First run** recommends by backend and memory; the model cards show each tier's need on that
  backend.

## Steps

1. Speech on WebGPU, measured on the 3090 Ti: the encoder through the plugin provider (D3D12,
   then Vulkan under Dawn), fp16 and 8-bit, against CUDA. Transcripts must equal CUDA's on the
   bench sets; latency per clip p50/p95.
2. The DXGI probe and the LUID match, with the backend chosen from the vendor.
3. Backends in components.json and the runtime; `components_for`/`gpu_need` per backend; the
   Vulkan llama-server with `--device`.
4. The clock hold on Vulkan, measured cold against warm.
5. First run, Settings and the problem screens for AMD and Intel (driver too old for Vulkan 1.2,
   not enough memory, the integrated GPU only).
6. Measure on real AMD and Intel cards (RDNA 3 and Arc B580 or similar): latency per stage, peak
   memory per tier (to set `gpu_need`), cold start with an empty shader cache (against the 5 s
   bar), outputs and transcripts against CUDA. Ship when they meet the budgets, not before.

## Risks

- Intel driver bugs at small batch sizes (llama.cpp #28648, Arc 140V iGPU, -ub 128), and flash
  attention or cooperative-matrix problems per GPU; escape hatches are the GGML_VK_DISABLE_* variables.
- Cold pipeline compilation on a first run (no pipeline cache of ggml-vulkan's own).
- Two GPU APIs on one card (Vulkan for llama.cpp, D3D12 under Dawn); the plugin is young (five
  releases in five months) and must match onnxruntime's version.
- No global free-memory number: the OS budget is per process.
