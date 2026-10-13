# Plan: Utter on Android (Pixel 10 Pro XL), 2026-10-13

Goal: the desktop experience on Tai's Pixel 10 Pro XL (Tensor G5, 16 GB): hold, speak, and clean
text appears in the focused app, fully on the phone, with the same models (Parakeet, Utter's
cleanup v2). Nothing is built until step 1 has measured each stage on the phone: no public
number exists for these models on Tensor G5, and the desktop budget (key up to text 231-260 ms)
cannot be assumed.

## What is known (2026-10-13)

- **Tensor G5's GPU is weak for compute** (Imagination PowerVR, replacing Mali): Geekbench GPU
  compute about half the G4's, Vulkan drivers still maturing ([Android Authority][aa],
  [Notebookcheck][nbc]). The GPU is the least likely home for either model.
- **Tensor's TPU is open to apps** through the Tensor ML SDK, in beta with LiteRT, on the Pixel
  10 series ([Google][tensor-sdk]). LiteRT-LM runs Gemma 4 E2B officially (52 tokens/s decode,
  3,808 tokens/s prefill on an S26 Ultra GPU; no Pixel number published) ([model card][e2b]).
  Our cleanup models are Gemma 4 E2B and E4B fine-tunes, so converting them is plausible.
- **llama.cpp on the phone's CPU** works for GGUF as-is (the Q4_0 repack uses Arm's int8
  matrix instructions); no Tensor G5 figure exists.
- **Parakeet on Android** runs in sherpa-onnx's int8 builds (APKs published, no latency
  figures) ([sherpa-onnx][sherpa]); onnxruntime's Android build runs our own export on the CPU.
- **Gemini Nano** (ML Kit Prompt API, best on Pixel 10) cannot load our cleanup model: public
  APIs run Google's own adapters only ([Android Developers][mlkit]). Not a path.
- **Insertion into other apps.** Android has two ways:
  - **A keyboard** (InputMethodService): the official text API (`commitText`), and the
    microphone is allowed while the keyboard shows. But it replaces Gboard while in use.
  - **An accessibility service with a floating bubble**, which is Wispr Flow's choice. It
    keeps Gboard, but inserts through accessibility actions, needs a Play Store policy
    declaration, and OEMs sometimes drop the connection ([Wispr Flow docs][wispr]).

## Measured on the Pixel 10 Pro XL (2026-10-13)

Tai's phone as he uses it: warm, other apps open, plugged in with the screen on. Cleanups are
paced 10 s apart as real dictations are; llama.cpp b11535 built for arm64 (Armv9.2 with int8
matrix instructions), 4 threads; 30 cases of the kokoro-real eval through `utter-cli llm-bench`
with the app's prompt and server settings.

- **Cleanup, Light v2 (Gemma 4 E2B) on the CPU:** p50 2.6 s, p95 3.7 s, against 0.28 s on the
  3090 Ti; 29 of 30 outputs identical. Prompt 10.5 ms a token, reply 37 ms a token with n-gram
  drafting (49-token replies at p50).
- **Flash attention must be off on the CPU:** on, prompt processing at a warm cache (768 tokens)
  falls from 90 to 8.6 tokens/s.
- **Smaller models, llama-bench at the real request shape** (68-token prompt on a 768-token cache,
  49-token reply, no drafting):

  | Model | Prompt tokens/s | Reply tokens/s |
  |---|---|---|
  | Gemma 4 E2B Q4_0 (Light v2) | 102 | 17 |
  | Gemma 3 1B Q4_0 | 286 | 36 |
  | Gemma 3 270M Q8_0 | 898 | 90 |

- **The GPU through llama.cpp's Vulkan backend is slower than the CPU.** The GPU is a PowerVR
  DXT-48-1536, which has float16 but no int8 dot product and no matrix cores. Gemma 3 1B ran at
  26 prompt tokens/s at Q4_0, and at 33 prompt and 3.8 reply tokens/s at float16, against the
  CPU's 286 and 36. The first use also spent 10 minutes compiling shaders. ggml's Vulkan kernels
  are not tuned for this GPU; LiteRT's GPU backend (OpenCL) is the GPU path to measure next.
- **Speech, the desktop's encoder exports on the CPU** (onnxruntime 1.31 for Android, 4 threads,
  60 clips): 8-bit encoder p50 2.4 s (AMI 1.0 s, VoxPopuli 3.8 s a clip), float16 3.1 s. Both
  keep float16 activations, which suit a GPU; a CPU export (int8 weights, float32 activations)
  is next. The process aborts at exit in onnxruntime's teardown ("pthread_mutex_lock called on
  a destroyed mutex"), to be fixed before the app.
- **Utter's Rust CLI builds for aarch64-linux-android unchanged** and runs the speech bench on
  the phone, which supports sharing the Rust core.
- **Memory:** 16 GB, but with Tai's apps open about 6 GB is free; Light's 3.1 GB model loaded in
  51 s while memory was short (swap full).

## Architecture (proposed)

- **A Kotlin app around Utter's Rust core.** The core is shared through JNI: the segmenter,
  the cleanup gate and prompt, the speech decoding loop (utter-asr already runs on `ort`,
  which supports Android) and History. Kotlin owns the UI, the keyboard or bubble, audio
  capture (AudioRecord, 16 kHz) and insertion. This keeps one implementation of the logic,
  as DESIGN.md asks, and amends "Android is separate Kotlin" to "separate Kotlin UI".
- **Speech:** onnxruntime Android on the CPU with the 8-bit encoder, unless step 1 shows
  LiteRT on the TPU is much faster.
- **Cleanup:** the faster of llama.cpp on the CPU (the GGUF as shipped), and LiteRT-LM on the
  TPU or GPU (a converted .litertlm). Light (E2B) first; Quality only if it meets the budget.
- **Models** download on first run like the desktop's: pinned and checksummed, with Android
  entries in components.json.

## Steps

1. **Measure on the phone**, no app yet, over adb with binaries pushed to the device:
   - the speech encoder (8-bit and float16) on the CPU through onnxruntime, then LiteRT if a
     conversion works: p50/p95 per clip on the bench sets, transcripts against the desktop;
   - cleanup Light v2: llama.cpp `llama-bench` and real cleanup cases on the CPU (and Vulkan,
     for completeness); then LiteRT-LM on GPU and TPU, stock Gemma 4 E2B first, then our model
     converted: prompt and per-token times, outputs against the desktop;
   - memory, sustained speed over 50 dictations (thermal throttling), and battery per dictation.
2. **Choose the runtime per stage** from those numbers, and set the Android latency budget
   (DESIGN.md section 5) from what the phone can do.
3. **Write docs/ux/android.md** for the bubble: hold-to-talk on
   the screen, the live text, errors, first run.
4. **Build:** the Rust core for aarch64-linux-android behind JNI, the Kotlin app, first-run
   download, dictation, insertion, History; then the latency, eval and UX checks of DESIGN.md
   section 12, measured on the phone.

## Decided (Tai, 2026-10-13)

1. **Insertion: the bubble**, with a keyboard as the fallback if accessibility insertion proves
   unreliable on the Pixel.
2. **The Rust core is shared through JNI**; Kotlin owns the UI, audio capture and insertion.
3. **A smaller cleanup model for the phone** is on the table (Tai): the measurements above size it.

[aa]: https://www.androidauthority.com/pixel-10-game-emulation-test-3605149/
[nbc]: https://www.notebookcheck.net/Tensor-G5-Disappointing-CPU-performance-and-atrocious-GPU-numbers-in-Google-Pixel-10-Pro-XL-hands-on-benchmarks.1097374.0.html
[tensor-sdk]: https://developers.googleblog.com/en/google-tensor-sdk-beta-with-litert/
[e2b]: https://huggingface.co/litert-community/gemma-4-E2B-it-litert-lm
[sherpa]: https://k2-fsa.github.io/sherpa/onnx/nemo/index.html
[mlkit]: https://developer.android.com/blog/posts/ml-kit-s-prompt-api-unlock-custom-on-device-gemini-nano-experiences
[wispr]: https://docs.wisprflow.ai/articles/7669452251-accessibility-permission-on-android
