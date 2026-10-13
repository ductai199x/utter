# Plan: Utter on Android (the test phone), 2026-10-13

Goal: the desktop experience on Tai's the test phone (the phone's chip, 16 GB): hold, speak, and clean
text appears in the focused app, fully on the phone, with the same models (Parakeet, Utter's
cleanup v2). Nothing is built until step 1 has measured each stage on the phone: no public
number exists for these models on the phone's chip, and the desktop budget (key up to text 231-260 ms)
cannot be assumed.

## What is known (2026-10-13)

- **the phone's chip's GPU is weak for compute** (Imagination PowerVR, replacing Mali): Geekbench GPU
  compute about half the G4's, Vulkan drivers still maturing ([Android Authority][aa],
  [Notebookcheck][nbc]). The GPU is the least likely home for either model.
- **Tensor's TPU is open to apps** through the Tensor ML SDK, in beta with LiteRT, on the Pixel
  10 series ([Google][tensor-sdk]). LiteRT-LM runs Gemma 4 E2B officially (52 tokens/s decode,
  3,808 tokens/s prefill on an S26 Ultra GPU; no Pixel number published) ([model card][e2b]).
  Our cleanup models are Gemma 4 E2B and E4B fine-tunes, so converting them is plausible.
- **llama.cpp on the phone's CPU** works for GGUF as-is (the Q4_0 repack uses Arm's int8
  matrix instructions); no the phone's chip figure exists.
- **Parakeet on Android** runs in sherpa-onnx's int8 builds (APKs published, no latency
  figures) ([sherpa-onnx][sherpa]); onnxruntime's Android build runs our own export on the CPU.
- **Gemini Nano** (ML Kit Prompt API, best on Google's own phones) cannot load our cleanup model: public
  APIs run Google's own adapters only ([Android Developers][mlkit]). Not a path.
- **Insertion into other apps.** Android has two ways:
  - **A keyboard** (InputMethodService): the official text API (`commitText`), and the
    microphone is allowed while the keyboard shows. But it replaces Gboard while in use.
  - **An accessibility service with a floating bubble**, which is Wispr Flow's choice. It
    keeps Gboard, but inserts through accessibility actions, needs a Play Store policy
    declaration, and OEMs sometimes drop the connection ([Wispr Flow docs][wispr]).

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
3. **Decide insertion** (keyboard or bubble), then write docs/ux/android.md: hold-to-talk on
   the screen, the live text, errors, first run.
4. **Build:** the Rust core for aarch64-linux-android behind JNI, the Kotlin app, first-run
   download, dictation, insertion, History; then the latency, eval and UX checks of DESIGN.md
   section 12, measured on the phone.

## Decisions needed from Tai

1. **Insertion: keyboard or bubble.** The bubble keeps Gboard and is what Wispr ships; the
   keyboard is the sturdier API. My recommendation: the bubble, since keeping your keyboard is
   the point of a dictation app, with the keyboard as the fallback if accessibility insertion
   proves unreliable on the Pixel.
2. **Share the Rust core through JNI** (recommended) rather than reimplementing it in Kotlin.
3. **Set up the measurement:**
   - turn on developer options and USB debugging on the phone, and connect it to this PC;
   - let me install the Android command-line tools and NDK (development only; nothing ships
     to the desktop app).

[tensor-sdk]: https://developers.googleblog.com/en/google-tensor-sdk-beta-with-litert/
[e2b]: https://huggingface.co/litert-community/gemma-4-E2B-it-litert-lm
[sherpa]: https://k2-fsa.github.io/sherpa/onnx/nemo/index.html
[mlkit]: https://developer.android.com/blog/posts/ml-kit-s-prompt-api-unlock-custom-on-device-gemini-nano-experiences
[wispr]: https://docs.wisprflow.ai/articles/7669452251-accessibility-permission-on-android
