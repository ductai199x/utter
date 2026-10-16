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
- **Speech, every export on the CPU** (encoder p50 / p95 over 60 clips, WER against the
  desktop's 6.82%): float32 1.6 / 3.9 s (6.82%); int8 weights and activations (dynamic
  quantization) 1.3 / 6.1 s (6.76%); the desktop's 8-bit 1.4 / 4.0 s (6.82%); 8-bit weights with
  float32 activations 5.2 / 6.1 s. Accuracy holds at every precision, but the 0.6B encoder is
  compute-bound on the CPU: speech needs the TPU or the GPU too.
- **LiteRT-LM 0.18 on the TPU and GPU** (Google's Gemma 4 E2B builds for Tensor G5 and for GPU,
  through a test app; stock Gemma ignores the short cleanup prompt and writes long replies, so
  only speeds count here):

  | Gemma 4 E2B | Prompt tokens/s | First token | Reply tokens/s | Engine load |
  |---|---|---|---|---|
  | TPU (Tensor G5 build) | 1,102 | 0.15 s | 19.9 | 1.2 s |
  | TPU, speculative decoding on | ~1,000 | 0.14 s | ~36 | 2.2 s |
  | GPU (OpenCL build) | 286 | 0.48 s | 15.5 | 5.7 s |

  The TPU solves the prompt (ten times the CPU), and with speculative decoding replies run
  about twice the CPU's speed: a 49-token cleanup in about 1.4 s against 2.6 s. The GPU build
  also did not stop its replies (about 4,000 tokens). The TPU runtime is a plain Maven library
  (`com.google.ai.edge.litert:litert-npu-runtime-google-tensor`), no Play delivery needed to test.
  Running our fine-tuned model there needs it converted and compiled for Tensor G5.
- **Utter's Rust CLI builds for aarch64-linux-android unchanged** and runs the speech bench on
  the phone, which supports sharing the Rust core.
- **Memory:** 16 GB, but with Tai's apps open about 6 GB is free; Light's 3.1 GB model loaded in
  51 s while memory was short (swap full).

## A cleanup model for the phone (2026-10-14)

Gemma 3 270M and 1B (Google's QAT checkpoints) trained on v2's data (`utter-train`, one A6000
each), judged with Light v2 by `utter-eval` (26B judge) on all 25,469 held-out dictations:

| Model | Ready to send, Kokoro real | Unseen voices (Chatterbox real) | Text test | Replies cut off |
|---|---|---|---|---|
| Light v2 (Gemma 4 E2B, LoRA) | **85.7** | **74.6** | **64.3** | 3 |
| Gemma 3 1B, full fine-tune | 78.0 | 67.8 | 52.8 | 39 |
| Gemma 3 1B, LoRA (Light's recipe) | 75.0 | 63.6 | 50.7 | 806 |
| Gemma 3 270M, full fine-tune | 60.0 | 49.7 | 34.7 | 564 |
| Gemma 3 270M, LoRA | 46.5 | 36.3 | 24.1 | 3,294 |

- **A cut-off reply** (one that never stops: the app would type the raw text) is how a small model
  fails. It starts a bullet list on plain prose ("- name: Ubuntu / - version: 18 / ...") and
  loops. The dataset is not the cause: its list targets all come from dictations that enumerate
  items, and the failures spread evenly over every domain and dictation style. A rank-16 LoRA
  leaves a small model its pretrained habits; training every weight (`--full`) cuts the 1B's
  cut-offs twenty-fold.
- **The 270M is out**: 25 points behind Light even fully trained, and its replies on hard
  dictations are often wrong, not only long.
- **The 1B is the candidate**: about 1 s a cleanup on the phone's CPU (estimated from llama-bench;
  Light takes 2.6 s), 7 points behind Light on real speech, and 39 cut-offs (0.15%), where the
  bar is none.

**The retry guard and more training (2026-10-14).** The app now retries a reply that runs away
(`utter_llm::cleanup::chat`): once more, with a grammar that forbids a leading list marker and a
mild repetition penalty. Judged on the same 25,469 dictations, the retried outputs included:

| Model | Kokoro real | Unseen voices | Text test | Runaways (retried) | Cut off after the retry |
|---|---|---|---|---|---|
| Light v2 | **85.7** | **74.6** | **64.3** | 3 | **0** |
| 1B full, 2 epochs, lr 2e-5 | 79.7 | 67.0 | 51.9 | 18 | 1 |
| 1B full, 1 epoch, lr 2e-5 | 78.0 | 67.8 | 52.8 | 39 | 3 |
| 1B full, 1 epoch, lr 5e-5 | 77.7 | 66.4 | 49.5 | 35 | 10 |
| 1B full, 2 epochs, lr 5e-5 | 76.9 | 65.2 | 48.2 | 26 | 7 |

- The guard brings Light to no cut-off at all, and its scores do not move.
- Two epochs at lr 2e-5 halve the 1B's runaways at the same quality; a higher learning rate is
  worse on both. The one cut-off left is a restart over a passage whose numbers the recognizer
  lost ("Drogden Sill depth of sets a limit...").
- The 1B stays 6 points behind Light on real speech: the size of model the phone can run fast
  on its CPU costs that much, so far.
- **Measured on the phone** (2026-10-15, the same 30 real cases, app settings, drafting, paced
  10 s apart, the phone as Tai uses it): the 1B (2 epochs) cleans up in 1.49 s p50 / 1.98 s p95,
  Light in 2.69 / 3.81 s. Prompt 5.1 ms a token, reply 18.5 ms a token (Light: 10.5 and 37.4).
- The retry guard shipped to the desktop in 0.1.7.

## Speech on the phone's GPU (2026-10-15)

Not the TPU: compiling for it takes Google's Tensor ML SDK, whose terms forbid distributing what
it builds without Google's written permission and publishing Tensor performance data (4.16-4.18),
which an open, published Utter cannot accept (Tai agreed to try the GPU first). LiteRT's GPU
backend is Apache-licensed.

- **Conversion** (`litert-torch` 0.9, Transformers' Parakeet encoder and joint projection, one
  fixed input length per model, 500 mel frames = 5 s here): the stock graph has boolean and int64
  mask tensors, length arithmetic and 48 masked fills, so LiteRT's GPU took 48 of 3,499 ops and
  failed. Rewritten float-only (the masks are float inputs, one per subsampling stage; attention
  masking an added bias; the convolution module's a multiply): every op on the GPU, 1.4e-4 from
  Transformers on the valid frames.
- **Float16 needed one more exact rewrite.** On real speech the encoder's LayerNorm inputs reach
  |x| 7,320 and a mean square of 355,518, past float16's 65,504, so float16 broke the output
  (2 of 15 transcripts). LayerNorm(x) = LayerNorm(x / c) with eps / c^2; with c = 64 the mean
  square stays under 100.
- **Measured** (15 real AMI clips under 5 s, transcripts against Transformers):

  | Encoder | Transcripts identical | Per 5 s clip |
  |---|---|---|
  | GPU float16, LayerNorm rewritten | 15 of 15 | 325 ms |
  | GPU float32 | 15 of 15 | 573 ms |
  | GPU float16, stock | 2 of 15 | 328 ms |
  | CPU (LiteRT, XNNPACK, float32) | - | 1,030 ms |

- **Quality, 100 real clips** (50 AMI, 50 VoxPopuli, up to 10 s, a 1,000-frame model with
  float16 weights): 97 of 100 transcripts identical to Transformers, the other three differing in
  a comma or a filler; WER against what was said 9.26% on the phone and 9.26% in Transformers.
  373 ms per 10 s clip.
- **Run as the bubble will** (a foreground service, another app in front, the screen on, 1-2 s
  between runs): float16 steady at 310-373 ms per 5 s clip, float32 495-569 ms. The slower runs
  above (433 ms to 1.4 s) came only from the test activity in its own window, restarted
  repeatedly; not reproduced as a service.
- **Load time.** Weights stored as float16 at export (ai-edge-quantizer's float casting; 2.35 GB
  -> 1.18 GB), and LiteRT's on-phone cache of compiled programs and GPU-ready weights: 52-107 s
  -> 15.8 s for the first load, 8.1-9.1 s after. The rest is reading the 1.18 GB cache back to
  the GPU. 8-bit weights (595 MB) crash LiteRT 2.3.0's GPU accelerator (a null read in
  libLiteRtClGlAccelerator.so, with or without allowSrcQuantizedFcConvOps) and run only on the
  CPU, 960 ms (reported upstream with a one-layer reproducer: google-ai-edge/LiteRT#10934). So
  the app loads the encoder once when its service starts and keeps it resident.

## Architecture (decided by the measurements, 2026-10-16)

- **A Kotlin app (`android/`, its own Gradle build) around Utter's Rust core
  (`crates/utter-mobile`, a cdylib behind JNI).**
  - Rust owns everything that decides: the dictation's state and timing, voice detection and
    the segmenter (`utter-asr::vad`), speech (`utter-asr`), cleanup (`utter-llm`: the prompt,
    the gate, the retry guard, tidy and strayed), History, and the models (`utter-runtime`,
    with Android entries in components.json).
  - Kotlin renders and reaches the system: the accessibility service, the bubble and the pill
    (docs/ux/android.md), audio capture (AudioRecord, 16 kHz mono, frames handed to Rust),
    insertion, the first-run screens.
  - Between them: a few JNI calls (start, push audio, stop, cancel) and one event queue Kotlin
    polls (state, live words, final text, error), so Rust can be tested without a phone.
- **Speech: the encoder on the GPU through LiteRT**, called from Rust over LiteRT's C API, as
  one `.tflite` holding several fixed input lengths as signatures that share their float16
  weights; the decoder and joint stay on onnxruntime's CPU build, as now. The export is a
  tool (`utter-speech`), with the float-only rewrite and the LayerNorm scaling above.
- **Cleanup: llama-server on the CPU**, the GGUF as on the desktop, shipped as a native library
  of the app (Android runs executables only from the app's native library folder), 4 threads,
  flash attention off, drafting on; the 1B model (Gemma 3, full fine-tune, two epochs).

## Steps

1. Done: measured every runtime on the phone (above).
2. **Speech in Rust**: an encoder backend in `utter-asr` (onnxruntime on the desktop, LiteRT on
   Android) and the LiteRT export as a tool; checked against the desktop's transcripts.
3. **`utter-mobile`**: the dictation session over `utter-asr` and `utter-llm`, fed audio frames,
   producing events; tested on the emulator from recorded clips (the emulator's microphone and
   speaker stay off).
4. **The app**: the Gradle project, the JNI bridge, the accessibility service, the bubble and
   pill, insertion, first run.
5. **On the phone**: the checks of docs/ux/android.md and DESIGN.md section 12, measured.

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
