# Utter on Android

Goal: the desktop experience on a phone. Hold the bubble, speak, and clean text appears in the
focused app, with speech and cleanup running on the phone. The spec of what the person sees is
[ux/android.md](ux/android.md). This file holds the status, the architecture, the measurements
and the decisions behind it.

## Status (2026-10-20, Utter 0.1.11 on the desktop)

**The Android app is a work in progress. It runs on the emulator only. It has not been run on the
phone.** It is not released and has no installer.

| Part | State |
|---|---|
| Phone measurements: cleanup models, speech on CPU and GPU, LiteRT-LM on the TPU and GPU | Done (2026-10-13 to 10-15), through `utter-cli`, `llama-bench` and test apps on the test phone |
| Cleanup model for the phone (`utter-mini`, Gemma 3 1B) | Trained, judged, published; also shipped on the desktop as "Extra light" |
| Speech encoder on the phone's GPU | Converted and measured on the phone; runs from Rust (`utter-asr`'s `litert` module) |
| `crates/utter-mobile` (the dictation session behind JNI) | Built; tested on the emulator from recorded clips |
| `android/` (Kotlin app: accessibility service, bubble, pill, insertion, first run) | Built; dictates into other apps on the emulator |
| On the test phone: the app end to end | Not done |
| Model download in the app | Not built; development builds get models through `android/native/push-models.sh` |
| Models in `components.json` for Android | Not there: the cleanup model `utter-mini` is published, the `.tflite` speech encoder is not |
| History, bubble dragging and per-app position, release build and signing | Not built |

What exists, checked in the code:

- `android/`: its own Gradle build (`dev.utter.app`, minSdk 31, targetSdk 36, ABIs `arm64-v8a` and
  `x86_64`). Kotlin files: `UtterService` (the accessibility service), `Overlay` (bubble and pill as
  accessibility overlays), `Insert` (set-text, then paste, then clipboard), `Mic` (AudioRecord,
  16 kHz mono; a debug build reads `test-audio.wav` instead), `Core` and `Native` (JNI),
  `MainActivity` (first-run screens), `Models`.
- `android/native/build.sh` builds the Rust core (`libutter_mobile.so`) and llama-server
  (llama.cpp b11535 from source, checksummed, CPU backend) per ABI. onnxruntime 1.31.0 and LiteRT
  2.3.0 come from Maven.
- `crates/utter-mobile`: a worker thread owns the session; JNI calls are start, touch, push
  audio, cancel and a polled JSON event queue (`listening`, `words`, `finishing`, `final`,
  `cancelled`, `error`). `examples/replay.rs` drives it from clips without a phone.
- `crates/utter-pipeline`: the desktop's platform-free dictation loop is `Session` over a
  `Recognizer`; the desktop and `utter-mobile` both drive it.
- `utter-speech export-litert` (`tools/envs/litert`, run in WSL on Windows) writes the encoder
  as one `.tflite`.

## Architecture (decided 2026-10-16)

- **A Kotlin app around Utter's Rust core** (`crates/utter-mobile`, a cdylib behind JNI).
  Rust owns everything that decides: the dictation's state and timing, voice detection and the
  segmenter, speech, cleanup (prompt, gate, retry guard), and the models. Kotlin renders and
  reaches the system: the accessibility service, the bubble and pill, audio capture, insertion,
  the first-run screens. This is the rule of CLAUDE.md (the UI renders state and sends intent).
  Between them: a few JNI calls and one event queue Kotlin polls, so Rust is tested without a
  phone.
- **Insertion: an accessibility service with a floating bubble**, so the person keeps their own
  keyboard. The bubble is an accessibility overlay (no "draw over other apps" permission).
  A keyboard (InputMethodService) is the fallback if accessibility insertion proves unreliable on
  the phone; it is not built.
- **Speech: the encoder on the phone's GPU through LiteRT**, called from Rust over LiteRT's C
  API. One `.tflite` holds four fixed input lengths as signatures (500, 1,000, 2,000 and 3,000
  mel frames, 5 to 30 s) that share float16 weights (1.33 GB). The decoder and joint stay on
  onnxruntime's CPU build.
- **Cleanup: llama-server on the CPU**, the same GGUF path as the desktop, shipped as a native
  library of the app (Android runs executables only from the native library folder). 4 threads,
  flash attention off, n-gram drafting on. The model is `utter-mini`, Gemma 3 1B fully
  fine-tuned.
- **Not used:** Gemini Nano (public APIs run only Google's own adapters, so it cannot load our
  model); the phone's TPU (see Decisions); llama.cpp's Vulkan backend (slower than the CPU).
- **Why a bubble and not a keyboard (2026-10-13).** A keyboard (InputMethodService) has the
  official text API (`commitText`) and may use the microphone while it shows, but it replaces
  Gboard while in use. An accessibility service with a floating bubble keeps Gboard, but inserts
  through accessibility actions, needs a Play Store policy declaration, and some OEMs drop the
  connection ([Wispr Flow's Android notes][wispr], who chose the bubble).
- **Other ways to run the models, not taken.** Parakeet runs on Android in sherpa-onnx's int8
  builds ([sherpa-onnx][sherpa]; APKs published, no latency figures) and onnxruntime's Android
  build runs our own export on the CPU (measured below). llama.cpp runs GGUF as-is on the CPU
  (its Q4_0 repack uses Arm's int8 matrix instructions). LiteRT-LM runs Gemma 4 E2B officially:
  52 tokens/s decode and 3,808 tokens/s prefill on a Galaxy S26 Ultra GPU, no figure published
  for the test phone ([model card][e2b]); the phone's GPU compute is about half the previous
  generation's, with Vulkan drivers still maturing.

## Measurements

All on the test phone (16 GB) unless noted, as its owner uses it: warm, other apps open.

### Cleanup on the CPU (2026-10-13)

llama.cpp b11535 built for arm64 (Armv9.2 with int8 matrix instructions), 4 threads; 30 cases of
the kokoro-real eval through `utter-cli llm-bench` with the app's prompt and server settings,
cleanups paced 10 s apart.

- Light v2 (Gemma 4 E2B): p50 2.6 s, p95 3.7 s, against 0.28 s on a desktop RTX 3090 Ti; 29 of
  30 outputs identical. Prompt 10.5 ms a token, reply 37 ms a token with n-gram drafting.
- Flash attention must be off on the CPU: on, prompt processing at a warm cache (768 tokens)
  falls from 90 to 8.6 tokens/s.
- llama-bench at the real request shape (68-token prompt on a 768-token cache, 49-token reply,
  no drafting):

  | Model | Prompt tokens/s | Reply tokens/s |
  |---|---|---|
  | Gemma 4 E2B Q4_0 (Light v2) | 102 | 17 |
  | Gemma 3 1B Q4_0 | 286 | 36 |
  | Gemma 3 270M Q8_0 | 898 | 90 |

- llama.cpp's Vulkan backend on the phone's GPU (PowerVR DXT-48-1536: float16, no int8 dot
  product, no matrix cores) is slower than the CPU. Gemma 3 1B ran at 26 prompt tokens/s at
  Q4_0, and at 33 prompt and 3.8 reply tokens/s at float16, against the CPU's 286 and 36. First
  use spent 10 minutes compiling shaders.
- Memory: with the owner's apps open about 6 GB was free; Light's 3.1 GB model loaded in 51 s
  while memory was short (swap full).
- Utter's Rust CLI builds for aarch64-linux-android unchanged and ran the speech bench there.

### The phone cleanup model (2026-10-14, 2026-10-15)

Gemma 3 270M and 1B (Google's QAT checkpoints) trained on v2's data, judged against Light v2 on
all 25,469 held-out dictations (method: [evals.md](evals.md), model work: [cleanup.md](cleanup.md)).

| Model | Ready to send, Kokoro real | Unseen voices | Text test | Replies cut off |
|---|---|---|---|---|
| Light v2 (Gemma 4 E2B, LoRA) | 85.7 | 74.6 | 64.3 | 3 |
| Gemma 3 1B, full fine-tune, 1 epoch | 78.0 | 67.8 | 52.8 | 39 |
| Gemma 3 1B, LoRA | 75.0 | 63.6 | 50.7 | 806 |
| Gemma 3 270M, full fine-tune | 60.0 | 49.7 | 34.7 | 564 |
| Gemma 3 270M, LoRA | 46.5 | 36.3 | 24.1 | 3,294 |

A cut-off reply never stops (the app would type the raw text). Small models start a bullet list on
plain prose and loop. A rank-16 LoRA leaves a small model its pretrained habits; training every
weight cuts the 1B's cut-offs twenty-fold. The dataset is not the cause: its list targets all come
from dictations that enumerate items, and the failures spread evenly over every domain and
dictation style. The 270M is 25 points behind Light even fully trained, and its replies on hard
dictations are often wrong, not only long. The 1B's 39 cut-offs are 0.15% of the 25,469.

With the retry guard (`utter_llm::cleanup::chat` retries once with a grammar forbidding a leading
list marker and a mild repetition penalty; shipped on the desktop in 0.1.7), retried outputs
included:

| Model | Kokoro real | Unseen voices | Text test | Runaways (retried) | Cut off after retry |
|---|---|---|---|---|---|
| Light v2 | 85.7 | 74.6 | 64.3 | 3 | 0 |
| 1B full, 2 epochs, lr 2e-5 | 79.7 | 67.0 | 51.9 | 18 | 1 |
| 1B full, 1 epoch, lr 2e-5 | 78.0 | 67.8 | 52.8 | 39 | 3 |
| 1B full, 1 epoch, lr 5e-5 | 77.7 | 66.4 | 49.5 | 35 | 10 |
| 1B full, 2 epochs, lr 5e-5 | 76.9 | 65.2 | 48.2 | 26 | 7 |

The guard brings Light to no cut-off, and its scores do not move. The one cut-off left on the 1B
(2 epochs) is a restart over a passage whose numbers the recognizer had lost. The 1B stays 6
points behind Light on real speech: that is what a model fast enough on the phone's CPU costs so
far.

On the phone (2026-10-15, the same 30 real cases, app settings, drafting, paced 10 s apart): the
1B (2 epochs) cleans up in 1.49 s p50 / 1.98 s p95; Light in 2.69 / 3.81 s. Prompt 5.1 ms a token,
reply 18.5 ms a token (Light: 10.5 and 37.4).

### Speech on the CPU (2026-10-13)

onnxruntime 1.31 for Android, 4 threads, 60 clips, WER against the desktop's 6.82%. Encoder
p50 / p95:

| Export | p50 / p95 | WER |
|---|---|---|
| float32 | 1.6 / 3.9 s | 6.82% |
| int8 weights and activations (dynamic) | 1.3 / 6.1 s | 6.76% |
| the desktop's 8-bit | 1.4 / 4.0 s | 6.82% |
| 8-bit weights, float32 activations | 5.2 / 6.1 s | not stated |

(An earlier run of the desktop's 8-bit export measured p50 2.4 s, AMI 1.0 s and VoxPopuli 3.8 s
a clip, and float16 3.1 s. Both kept float16 activations, which suit a GPU.) Accuracy holds at
every precision, but the 0.6B encoder is compute-bound on the CPU, so speech needs the GPU. The
process aborts at exit in onnxruntime's teardown ("pthread_mutex_lock called on a destroyed
mutex"); not yet fixed.

### LiteRT-LM on the TPU and GPU (2026-10-13, superseded as a path)

Google's Gemma 4 E2B builds through a test app, LiteRT-LM 0.18. Stock Gemma ignores the short
cleanup prompt, so only speeds count:

| Gemma 4 E2B | Prompt tokens/s | First token | Reply tokens/s | Engine load |
|---|---|---|---|---|
| TPU | 1,102 | 0.15 s | 19.9 | 1.2 s |
| TPU, speculative decoding on | ~1,000 | 0.14 s | ~36 | 2.2 s |
| GPU (OpenCL build) | 286 | 0.48 s | 15.5 | 5.7 s |

The TPU would clean a 49-token reply in about 1.4 s against Light's 2.6 s on the CPU. The GPU
build also did not stop its replies (about 4,000 tokens). The TPU runtime is a plain Maven library
(`com.google.ai.edge.litert:litert-npu-runtime-google-tensor`), so testing needed no Play
delivery; running our fine-tuned model there would need it converted and compiled for the phone's
chip. It was not pursued: see Decisions.

### Speech encoder on the phone's GPU (2026-10-15)

`litert-torch` 0.9, Transformers' Parakeet encoder and joint projection, one fixed input length
per model (500 mel frames = 5 s here).

- The stock graph has boolean and int64 mask tensors, length arithmetic and 48 masked fills:
  LiteRT's GPU took 48 of 3,499 ops and failed. Rewritten float-only (float mask inputs, one per
  subsampling stage; attention masking an added bias; the convolution module's a multiply): every
  op on the GPU, 1.4e-4 from Transformers on the valid frames.
- Float16 needed one more exact rewrite. On real speech the encoder's LayerNorm inputs reach
  |x| 7,320 and a mean square of 355,518, past float16's 65,504. LayerNorm(x) = LayerNorm(x / c)
  with eps / c^2; with c = 64 the mean square stays under 100.
- 15 real AMI clips under 5 s, transcripts against Transformers:

  | Encoder | Transcripts identical | Per 5 s clip |
  |---|---|---|
  | GPU float16, LayerNorm rewritten | 15 of 15 | 325 ms |
  | GPU float32 | 15 of 15 | 573 ms |
  | GPU float16, stock | 2 of 15 | 328 ms |
  | CPU (LiteRT, XNNPACK, float32) | not compared | 1,030 ms |

- Quality, 100 real clips (50 AMI, 50 VoxPopuli, up to 10 s, a 1,000-frame model, float16
  weights): 97 of 100 transcripts identical to Transformers, the other three differing in a comma
  or a filler; WER against what was said 9.26% on the phone and 9.26% in Transformers. 373 ms per
  10 s clip.
- Run as the bubble will (a foreground service, another app in front, screen on, 1-2 s between
  runs): float16 steady at 310-373 ms per 5 s clip, float32 495-569 ms. Slower runs (433 ms to
  1.4 s) came only from the test activity in its own window, restarted repeatedly; not reproduced
  as a service.
- Load time: weights stored as float16 at export (2.35 GB to 1.18 GB) plus LiteRT's on-phone cache
  of compiled programs and GPU-ready weights: first load 52-107 s before, 15.8 s with the cache
  built, 8.1-9.1 s after. The app therefore loads the encoder once when its service starts and
  keeps it resident.
- 8-bit weights (595 MB) crash LiteRT 2.3.0's GPU accelerator (a null read in
  `libLiteRtClGlAccelerator.so`, with or without `allowSrcQuantizedFcConvOps`) and run only on the
  CPU, at 960 ms.

### From Rust, on the emulator (2026-10-16)

- The four-signature `.tflite` (500, 1,000, 2,000, 3,000 frames), each signature within 7e-5 of
  Transformers; the export is reproducible byte for byte.
- x86_64 emulator, LiteRT's CPU, 12 eval clips of 3.8-23.5 s using every length: 12 of 12
  transcripts identical to the ONNX encoder's. Each signature first unpacked its own float32
  weights, and four took the 8 GB emulator down; XNNPACK's weight cache file (mapped, shared)
  brought loading from 17.3 s to 3.9 s and the peak to 2.4-4.0 GB. On the GPU the signatures
  share constants (`enable_constant_tensors_sharing`). The phone's load time and memory with four
  signatures are not measured.
- `utter-mobile` replay (clips pressed, fed in 30 ms chunks at speaking pace, released): live
  words, a pause cleaned while speaking, the final text; release to text 1.6-4.2 s on the
  emulator's CPU for 4.7-23 s clips.
- The app on the emulator: the core loads in the service, the bubble appears beside a focused
  field with the keyboard up, holding shows live words in the pill, release inserts through
  set-text 1.7-2.8 s later (emulator CPU). A second dictation goes in after the first with a
  space; a stray tap and a drag off cancel; a double tap listens hands-free until a tap.
- Emulator timings say nothing about the phone. The phone's budget is unmeasured in the app.
- Bug found and fixed: dropping the core only disconnected the cleanup worker, so a process that
  exited right after left llama-server orphaned (five on the emulator, 630 MB each). `Mobile`'s
  `Drop` now waits for the worker, the models and the server to end.
- Bug found and fixed: keep-alive passes queued behind a preview replaced it, so where a pass
  takes ~300 ms (the emulator's CPU, the phone's GPU) live words stalled for up to 6 s. The
  desktop's 20 ms passes hid it.
- The emulator's microphone and speaker stay off: tests feed recorded clips (`examples/replay.rs`,
  and `test-audio.wav` in debug builds).
- Desktop regression from the `Session` split: 20 clips with Light, 20 of 20 transcripts and
  final texts identical to before.

## Decisions

| Date | Decision | Why |
|---|---|---|
| 2026-10-13 | Insertion through an accessibility bubble; a keyboard only as fallback | Keeps the person's keyboard (Tai) |
| 2026-10-13 | The Rust core is shared through JNI; Kotlin owns UI, audio, insertion | One decision path on every platform (Tai) |
| 2026-10-13 | A smaller cleanup model for the phone | Light takes 2.6 s p50 on the CPU (Tai) |
| 2026-10-13 | llama-server on the CPU, 4 threads, flash attention off | Vulkan is slower than the CPU; flash attention costs 10x on the CPU |
| 2026-10-14 | The phone model is Gemma 3 1B, full fine-tune; 270M dropped; LoRA dropped for small models | 1B is 6 points behind Light on real speech and cleans in 1.49 s p50; the 270M is 25 points behind; LoRA leaves hundreds of runaways |
| 2026-10-14 | Two epochs at lr 2e-5 | Halves the 1B's runaways at the same quality; higher lr worse on both |
| 2026-10-15 | Speech on the GPU through LiteRT, not the TPU | The Tensor ML SDK's terms (4.16-4.18) forbid distributing what it builds without Google's written permission and publishing Tensor performance data, which an open, published Utter cannot accept (Tai agreed to try the GPU first). LiteRT's GPU backend is Apache-licensed |
| 2026-10-15 | Encoder weights stored in float16 with LayerNorm rescaled | Identical transcripts at 325 ms per 5 s clip; stock float16 broke 13 of 15 |
| 2026-10-15 | The encoder loads once at service start and stays resident | First load 15.8 s, later 8-9 s |
| 2026-10-16 | One `.tflite` with several fixed-length signatures sharing weights | Pads a clip into the shortest signature that holds it; 1.33 GB total |
| 2026-10-16 | A keep-alive pass never replaces a preview (`session::newest_job`) | Found on the emulator: passes of ~300 ms stalled live words for up to 6 s |
| 2026-10-18 | The phone model also ships on the desktop as "Extra light" (`utter-mini`) | Fastest on a computer without a graphics card |

## Steps

Done:

1. Every runtime measured on the phone.
2. Speech in Rust: `utter-speech export-litert`, `utter-asr`'s `litert` module (`Device::LiteRt`).
3. The shared dictation session (`utter_pipeline::Session`) and `utter-mobile` with JNI.
4. The app's first version on the emulator.

Remaining:

- **Run the app on the test phone** and measure the checks in [ux/android.md](ux/android.md) and
  DESIGN.md section 12. Unknowns to settle there: encoder load time and memory with four
  signatures on the GPU; whether the keep-alive passes are needed on a phone (they hold desktop
  GPU clocks up; on a phone they cost battery; `session.rs` enables them when the backend is not
  the CPU, and what that returns on Android is unchecked); whether the microphone needs a
  foreground service when the app is in the background; whether the clipboard can be restored after
  a paste (Android limits a background service's clipboard reads).
- Fix the onnxruntime teardown abort at exit.
- Model download in the app (models are not in `components.json` for Android; the `.tflite`
  encoder is not published) and a release build.
- Bubble dragging and per-app position; keeping the bubble off the field's text (it can cover the
  end of a long field); History on the phone.
- Play Store policy for an accessibility service, if the app is ever published there (Wispr Flow's
  docs say it needs a policy declaration). Not looked into.

## Known blockers and upstream bugs

- **LiteRT 2.3.0 GPU accelerator crashes on 8-bit weights**: a null read in
  `libLiteRtClGlAccelerator.so`, reported with a one-layer reproducer as
  google-ai-edge/LiteRT#10934. Workaround: float16 weights. The 8-bit encoder (595 MB) would
  cut the load; it runs only on the CPU until this is fixed.
- **onnxruntime Android aborts in teardown at exit** (mutex destroyed); no upstream report is recorded.
- **Gemma 3 Terms of Use** apply to `utter-mini` (see its `components.json` entry).

[e2b]: https://huggingface.co/litert-community/gemma-4-E2B-it-litert-lm
[sherpa]: https://k2-fsa.github.io/sherpa/onnx/nemo/index.html
[wispr]: https://docs.wisprflow.ai/articles/7669452251-accessibility-permission-on-android
