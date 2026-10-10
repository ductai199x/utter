# Eval results

Every number here was measured on the development machine (RTX 3090 Ti, Windows 11) by
`tools/eval`. Raw outputs are under `C:\Users\Tai\.cache\utter\eval\results\` (see
`docs/local-setup.md`). Newest first.

---

## 2026-10-10: final-text baselines (eval v1)

The eval docs/plan-final-text.md describes:
- **Gold:** 1000 real written passages, 200 each of technical, everyday, science, chat and
  prose.
- **Spoken forms:** 5 kinds, rendered by Gemma 4 26B with the gold known by construction (4756
  renditions after checks).
- **Audio:** Kokoro (12 voices).
- **Recognizer:** Parakeet, through Utter's runtime.
- **Scored set:** passages the gold screen did not flag for typos, capitalization, shorthand or
  non-prose: 3683 dictations.

The cleanup prompt v4 (tools/eval/finaltext/prompts/cleanup-v4.md) adds punctuation inferred
from meaning, spoken symbols, acronyms and correction scope to the product's v3.

| | Parakeet alone | E4B v3 (product) | E4B v4 | 26B v3 | 26B v4 |
|---|---|---|---|---|---|
| no edit needed | 5.0% | 7.9% | 8.4% | 10.4% | **11.8%** |
| word error | 20.6% | 15.3% | 11.1% | 11.4% | **7.4%** |
| retraction commands: no edit / word error | 0 / 59% | 1.8 / 50% | 5.4 / 28% | 5.4 / 34% | 11.9 / 15% |
| corrections: no edit | 0% | 3.6% | 3.9% | 9.2% | 9.4% |
| disfluent: no edit | 0% | 2.8% | 2.8% | 2.7% | 3.3% |
| comma F1 | 0.58 | 0.67 | 0.67 | 0.70 | 0.70 |
| colon / semicolon F1 | 0.16 / 0 | 0.43 / 0.25 | 0.42 / 0.29 | 0.42 / 0.24 | 0.43 / 0.33 |
| parentheses / quotes F1 | 0 / 0 | 0.22 / 0.17 | 0.38 / 0.30 | 0.26 / 0.30 | 0.41 / 0.45 |
| exact: numbers / symbols / terms | 65 / 14 / 51% | 73 / 27 / 56% | 70 / 33 / 63% | 72 / 33 / 58% | 73 / 45 / 69% |
| cleanup time p50 / p95 | | 196 / 444 ms | 209 / 488 ms | 212 / 457 ms | 238 / 561 ms |

What this says:
- **The product today gets about 1 dictation in 13 exactly right.** Prompting helps the 26B
  more than the E4B. The best prompted model is still near 1 in 8.
- **Retraction commands and disfluencies are where it fails most.** "Never mind, let's start
  over" drops one sentence, not everything before it. Injected false starts and fillers survive.
- **Most misses are 2-3 small differences**: punctuation conventions (semicolon or period,
  quote style), recognition errors on names (no vocabulary yet), and quirks of the gold that
  the screen misses.
- Fine-tuning (next) is the lever both the research and these numbers point to.

Known gaps in eval v1:
- No sentence-scope corrections: the LLM never chose that scope; a supplement set fills it.
- The gold screen is noisy: gold fixes replace it.
- Kokoro is one TTS engine; a Chatterbox set with cloned donated voices follows.

---

## 2026-10-10: GPU clocks while the user speaks

The pipeline's first end-to-end bench (`utter-cli e2e-bench`, clips replayed in real time
with 1 s gaps) had cleanup at 140-240 ms on short texts, against 95-124 ms for the same
requests sent back to back. Measured with `llm-bench --gap-ms` (Gemma 4 E4B, product
settings, 30 curated cases):

| Idle before the request | p50 | prompt (19 tokens) | per generated token |
|---|---|---|---|
| none | 95 ms | 19 ms | 7.3 ms |
| 0.5 s | 101 ms | 18 ms | 7.6 ms |
| 1 s | 110 ms | 20 ms | 7.8 ms |
| 3 s | 229 ms | 107 ms | 12.3 ms |

`nvidia-smi` sampled every 100 ms shows why: under light load the GPU sits in **P3, memory
clock 5,001 MHz, half of P2's 10,251**. Decoding is bound by memory bandwidth. The speech
keep-alive passes keep the GPU from P5 but not out of P3.

| Keeping it warm through a 3 s gap | p50 | GPU state, mean power |
|---|---|---|
| nothing | 229 ms | P3/P5 |
| one-token llama-server request every 250 / 500 / 1000 ms | 187 / 171 / 205 ms | mostly P3 |
| ... every 50 / 100 ms | 142 / 166 ms | P3 two thirds of the time, 114 / 101 W |
| ... back to back | 89 ms | P2, 262 W |
| a spin kernel in another process, full / half duty | 249 / 121 ms | P2, 157 W: time-slicing between processes slows the request |
| **one GPU thread in a `nanosleep` kernel, in Utter's process, stopped at key up** | **86 ms** (p95 143) | **P2, 157 W while the key is down** |

The last is `utter_runtime::gpu::GpuWarmer`: the clocks stay up for about a second after it
stops, so cleanup runs at full speed with nothing competing. It uses the CUDA driver API
from the GPU driver (no component). A first version ended each kernel on a `%globaltimer`
deadline and hung the pipeline twice in the first long clip after a run of short ones; each
kernel now ends after a fixed count of naps (at most 4 ms). `UTTER_NO_GPU_WARM=1` turns it
off.

End to end (40 clips per set, 1 s gaps, light tier), key up to text ready:

| | before | with the warmer |
|---|---|---|
| AMI p50 / p95 | 55 / 253 ms | **31 / 163 ms** |
| VoxPopuli p50 / p95 | 74 / 496 ms | **50 / 368 ms** |
| transcribed (AMI) p50 / p95 | 36 / 99 ms | **26 / 49 ms** |
| cleanup, the 10 AMI clips that needed it | 141-237 ms | **89-167 ms** |

No hang in the 80 clips. What is left of the tail: two VoxPopuli clips spoken as one 20 s
segment took 205 and 244 ms to transcribe after key up. The final pass waits behind a
preview pass of nearly the same 20 s that is already running.

So a segment over 8 s is now cut at a 128 ms pause instead of 384 ms (VoxPopuli, 40 clips):

| | 384 ms pauses only | soft cut after 8 s |
|---|---|---|
| key up to text p50 / p95 | 50 / 368 ms | **28 / 315 ms** |
| transcribed p50 / p95 | 47 / 108 ms | **27 / 73 ms** |
| WER | | +0.10 points, 95% CI -0.21 to +0.43 (noise) |

Two clips still arrive as one segment (no pause of 128 ms in 18 s) and take 114 and 191 ms.

---

## 2026-10-09: a 5 GB footprint (tiers)

Tai: the quality setup (about 21 GB of GPU memory, 18 GB to download) is far too heavy; aim for
about 5 GB, with the heavy model as an option. Measured with `nvidia-smi`, above the idle
desktop (2.8 GB), each model under load:

| Piece | GPU memory | Notes |
|---|---|---|
| Speech model, encoder float32 | 2.3 GB | 2.4 GB weights file |
| **Speech model, encoder float16** | **2.3 GB** | 1.2 GB file; WER the same as the reference (-0.02 points, within noise); encoder 18-21 ms p50, a little faster |
| Speech model, encoder weights 8-bit (MatMulNBits) | 1.9 GB | WER the same; encoder about 26 ms, 6-7 ms slower (weights unpacked every pass) |
| Speech model, encoder weights 4-bit | 1.6 GB | +0.08 WER points (not significant, worse in 90% of resamples) |
| **Gemma 4 E4B (llama-server, 4096 context)** | **3.3 GB** | its 2.7 GB of per-layer embeddings stay in system RAM (llama.cpp's default) |
| Gemma 4 E4B, 2048 context | 3.2 GB | no latency change; Tai kept 4096 |
| Gemma 4 26B-A4B | about 15 GB | |

The speech model's footprint does not depend on clip length (2.3 GB for 3 s and 25 s clips):
about 1.1 GB above its weights is fixed per-process cost (CUDA context, cuBLAS, cuDNN).
Tried and dropped: onnxruntime's memory pool growing by exact requests (3.1 GB: blocks of
every new length fragment) and weights outside the pool (2.4 GB).

Tai chose the float16 encoder over 8-bit weights: 0.4 GB more for about 7 ms less.

### Prompt v3

Two examples added (grammar and slang kept; a correction with "I mean"), new sentences, not
copies of test cases. Gemma 4 E4B: checks 96% -> **97%** (fillers and clean text 100%), real-speech
added words 20.0% -> 15.5%, 124 ms p50. Gemma 4 26B-A4B: 99% unchanged, added words 9.1% -> 8.1%.

### Decision

- **Light tier, the default: speech encoder float16 + Gemma 4 E4B. About 5.6 GB of GPU memory,
  97% of the cleanup checks, 124 ms p50 cleanup on real speech; loads in 2.5 s.**
- Quality tier, optional: Gemma 4 26B-A4B (about 15 GB more, 99%, 137 ms).

Components: `parakeet-ultra-onnx-fp16` (Hugging Face `ductai199x/utter-parakeet-ultra-onnx`,
`fp16/`), `gemma4-e4b-qat` (Google's repo), and `gemma4-26b-a4b-qat` marked optional.

---

## 2026-10-09: cleanup model

### Method

- **Rules:** `docs/cleanup-spec.md`. **Prompt:** `crates/utter-llm/prompts/cleanup.md` (the
  product's own, so the bench tests what ships).
- **Curated cases** (`tools/eval/cleanup_cases.jsonl`, 112): fillers, corrections ("2, no, 3"),
  repeats, lists, spoken formatting, numbers and addresses, clean text that must not change,
  dictated questions and instructions that must not be answered or obeyed, dictionary words,
  code, long dictation. Scored by code (`tools/eval/cleanup_score.py`): each case's checks,
  exact match, WER against the expected text, words added.
- **Real speech** (516 utterances): the speech model's own transcripts of AMI meetings and
  Monsoon conversations, with no expected text. Scored by rules: fillers left, words added
  that were never said, share of the speaker's words kept, outputs that look like answers.
- **Runs:** every model through Utter's own llama-server (`utter-cli llm-bench`), one request
  at a time, thinking off, temperature 0, the system prompt in the prompt cache. Latency is the
  wall time of the HTTP request on the RTX 3090 Ti (shared with desktop apps: 30% GPU load).

### Candidates (all 4-bit; first pass, prompt v1)

| Model | Checks | Corrections | Real: added | p50 curated / real |
|---|---|---|---|---|
| Gemma 4 12B (QAT) | 100% | 100% | 15.7% | 319 / 446 ms |
| Qwen3.8-27B (Q4_K_M) | 99% | 93% | 13.4% | 383 / 616 ms |
| Gemma 4 26B-A4B (QAT, MoE) | 98% | 93% | 16.9% | 215 / 284 ms |
| Qwen3.6-35B-A3B (Q3_K_XL, MoE) | 97% | 93% | 14.9% | 153 / 223 ms |
| Gemma 4 E4B (QAT) | 96% | 87% | 25.4% | 119 / 187 ms |
| Qwen3.5-4B / 9B (Q4_K_M) | 94% / 94% | 67% / 73% | 13.2% / 7.9% | 116 / 177, 150 / 253 ms |
| Gemma 4 E2B (QAT) | 86% | 67% | 12.8% | 81 / 123 ms |
| Qwen3.5-2B, MiniCPM5-2B (Q4_K_M) | 76%, 74% | 27%, 40% | 7.6%, 3.3% | 70 / 106, 57 / 98 ms |

"Added" on real speech is mostly grammar repair ("a", "makes", "fewer", "He and I") and, for
Gemma E4B, style changes ("gonna" -> "going to"): the spec forbids both. Higher-precision
quantizations were not pursued: they are slower, and latency comes first (Tai, 2026-10-09).

### Making it fast (each change measured)

| Change | Gemma 4 26B-A4B, p50 curated / real |
|---|---|
| Baseline | 215 / 284 ms |
| `--swa-full`: a full-size cache for the sliding-window layers. Without it, every request rebuilds that cache from a checkpoint; prompt processing drops from about 95 to 25 ms. | 107 / 182 ms |
| + n-gram drafting from the prompt (3-token match, 6-token draft; acceptance about 60%) | 98 / 136 ms |

Not used:
- **N-gram drafting on the Qwen models:** no gain even at 62% acceptance. They are hybrid
  models whose recurrent layers verify drafted tokens one by one.
- **llama-server's default n-gram sizes** (12-token match, 48-token draft, made for code
  edits): 5% acceptance, slower. Cleanup diverges from its input every few words.
- **Prefilling the prompt while the user speaks:** a chat-format prefill is not reusable (the
  template's end tokens follow the partial transcript); a raw-prefix prefill is, but with
  `--swa-full` the prompt is already cheap, and prefilling saved 2-3 ms. Not worth the
  complexity.

Drafting is effectively lossless: all curated outputs identical, 97% of real ones (the rest
differ by float rounding of batched verification).

### Prompt v2

Added: keep the speaker's grammar, slang, contractions and spelling variants. Gemma 4
26B-A4B: checks 98% -> **99%** (clean text 100%), real-speech added words 16.3% -> **9.1%**,
latency unchanged. The one failure left: "6 chairs" for "six chairs".

### Decision

**Gemma 4 26B-A4B, Google's QAT 4-bit build (Apache-2.0), with `--swa-full` and n-gram
drafting, prompt v2: 99% of checks, 101 ms p50 on curated cases, 136 ms on real speech,
p95 about 255 ms.** 14.4 GB of weights; fits beside the speech model in 24 GB. Pinned as the
`gemma4-26b-a4b-qat` component.

Gemma 4 E4B is the fallback for smaller GPUs (96%, 126 ms with the same settings, 4.9 GB),
pending a prompt check of its style changes.

---

## 2026-10-09: parakeet-ultra in Utter's Rust runtime

### What was built

- `tools/export/export_parakeet.py` exports the weights from Transformers into ONNX graphs:
  encoder (projection folded in), decoder step, joint, and a fused decoder-step-plus-joint.
  `tools/export/check_parakeet.py` checks them: 70 of 70 clips give the same tokens and text
  as Transformers.
- `crates/utter-asr`: the audio front end in Rust (matches Transformers' features; test
  `tests/frontend.rs`), and greedy TDT decoding on onnxruntime (CUDA for the encoder).
- `utter-cli bench` writes results that `tools/eval/asr_bench.py` scores, so the Rust
  runtime is measured by the same yardstick as the Python candidates.

### Accuracy

**The same as Transformers:** pooled English WER difference +0.00 points (95% interval +0.00
to +0.01) over 2,095 clips; 7.3% mean English WER.

### Latency (RTX 3090 Ti, per clip, batch 1, clips back to back)

| | median of set p50 | median of set p95 | all clips p50 / p95 |
|---|---|---|---|
| Photon (Moondream's engine, Python bench) | 31 ms | 40 ms | — |
| **Utter, Rust** | **33 ms** | **46 ms** | 34.3 / 59.4 ms |

Per stage, all clips, p50: front end 1.0 ms, encoder 20.5 ms, decode 12.5 ms. Short clips
(AMI, 1–5 s) take 27.5 ms in total. The target was Photon's 31 ms; Utter is about 2 ms behind
on medians. The two runs are not under identical load: the GPU showed 31–35% utilization
from other applications (Moonlight, Teams, browsers, LM Studio) during the Rust runs.

### What moved the numbers (each measured on the eval clips)

| Change | Effect |
|---|---|
| First working runtime | 75.8 ms p50 |
| Encoder convolutions rewritten (`tools/export/conv_free.py`) | onnxruntime's cuDNN Conv redoes its setup on every new input length: 2.3 ms on a repeated length, about 42 ms on a new one, and every utterance is a new length. Pointwise convs became MatMuls; each depthwise conv runs on an input padded to a fixed 512 frames (41 s) and is sliced back, which is exact and keeps cuDNN's setup to once. Writing the depthwise conv as 9 shifted multiply-adds instead removed the setup but launched about 650 more small kernels: slower. |
| Sparse mel filterbank | Front end 7–19 ms -> 0.3–1.9 ms. |
| Decoder and joint on the CPU | 12.6 ms decode p50, against 20.1 ms for the fused GPU step (one call per emitted token, with IoBinding). Per-call overhead on the GPU outweighs the small graphs' arithmetic. The GPU path stays as an option. |
| Not used | cuDNN's default convolution algorithm (750 ms per clip); memory-pattern planning off (no effect); one CPU thread (decode 20 ms); no spinning (encoder faster, decode slower). |

### Idle GPU: the finding that matters for the product

Clips spaced like real utterances are much slower than clips back to back. After 1.5 s idle
the GPU drops to P5 (525 MHz of 2,115) and the next encoder pass takes about 108 ms instead
of 20 (AMI). Keep-alive passes every 100 ms during the gap bring short clips back to warm
speed (28.7 ms) but long ones only part of the way (62 ms against 32).

So the pipeline must keep the GPU busy from the key press on: a warm-up pass on key down,
then the live preview's passes while the user speaks. The final pass, after key up, then runs
warm. Measuring this end to end belongs to the pipeline's bench (phase 1, next).

### Left on the table (in order of expected gain)

1. CUDA graphs per length bucket for the encoder (launch overhead is now the encoder's cost).
2. FP16 encoder weights.
3. The subsampling convolutions still use the 9-tap rewrite (about 135 small launches); the
   fixed-length padding used for the depthwise convs applies to them too.

---

## 2026-10-09: speech recognition, first bench

### Method

- **Data:** 11 public test sets, a seeded random sample of 300 clips each, 1–30 s per clip
  (`eval.fetch_sets`). English: AMI (meetings), Earnings-22, GigaSpeech, LibriSpeech
  test-other, Voice Arena Monsoon (Indian English conversations), URGENT24 (noise and
  reverberation), VoxPopuli. Vietnamese: Bud500, FLEURS, GigaSpeech2, ViMD (63 dialects).
- **Runs:** each model in its official runtime, one clip at a time (batch 1, as in
  dictation), after 3 warm-up clips. The language is left to the model wherever it can detect
  it. Cohere cannot, so it is told the language, which favours it.
- **Scoring:** WER after normalization. English uses Whisper's English normalizer (the Open
  ASR Leaderboard's). Vietnamese is lower-cased with punctuation removed and diacritics kept.
  Known cost: numbers written as digits in one text and as words in the other count as errors
  in Vietnamese, for every model alike.
- **Significance:** `asr_bench compare` runs a paired bootstrap (10,000 resamples) of pooled
  WER over the clips both models transcribed.
- **Latency** is the Python runtime's, not Utter's. It ranks models; Utter's own runtime is
  measured separately.

### Results: WER %, lower is better

| Model | ami | earn22 | giga | libri-o | monsoon | urgent | voxpop | **en mean** | bud500 | fleurs | giga2 | vimd | **vi mean** | lang-ID miss | p50 / p95 ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **parakeet-ultra-0.6b** (Photon) | 9.9 | 10.0 | 10.6 | 3.3 | 4.8 | 9.3 | 3.0 | **7.3** | — | — | — | — | — | — | **31 / 40** |
| parakeet-ultra-0.6b (Transformers) | 9.8 | 10.0 | 10.7 | 3.3 | 4.8 | 9.3 | 3.0 | **7.3** | — | — | — | — | — | — | 74 / 111 |
| qwen3-asr-1.7b | 10.8 | 9.4 | 9.9 | 3.4 | 5.0 | 9.2 | 3.1 | **7.3** | 4.7 | 6.6 | 8.2 | 9.4 | **7.2** | 0.8% | 671 / 1090 |
| cohere-transcribe (told the language) | 7.9 | 10.5 | 11.1 | 2.4 | 5.5 | 9.8 | 5.6 | 7.5 | 3.8 | 7.0 | 9.7 | 9.2 | 7.4 | — | 162 / 230 |
| parakeet-tdt-0.6b-v3 | 10.4 | 10.8 | 11.1 | 3.3 | 5.5 | 10.4 | 3.0 | 7.8 | — | — | — | — | — | — | 72 / 92 |
| qwen3-asr-0.6b | 11.4 | 10.8 | 10.0 | 4.3 | 5.5 | 10.4 | 2.9 | 7.9 | 7.5 | 9.1 | 11.5 | 10.4 | 9.6 | 0.9% | 709 / 1109 |
| whisper-large-v3-turbo | 15.5 | 11.3 | 11.2 | 3.7 | 6.0 | 13.7 | 10.3 | 10.2 | 64.9 | 9.3 | 18.0 | 14.6 | 26.7 | 1.0% | 189 / 233 |
| nemotron-3.5-streaming-0.6b | 18.0 | 19.3 | 13.6 | 6.4 | 55.9 | 20.7 | 4.3 | 19.7 | 10.5 | 15.7 | 12.9 | 16.6 | 13.9 | 17.4% | 184 / 267 |
| parakeet-ctc-0.6b-vi | ≈100 on every English set | | | | | | | 101.9 | 7.5 | 13.3 | 12.2 | 10.2 | 10.8 | — | 55 / 65 |

Paired bootstrap, pooled WER over the same clips (English: 2,095 clips; Vietnamese: 1,124):

| Comparison | Difference (A − B) | 95% interval | Verdict |
|---|---|---|---|
| ultra vs qwen3-asr-1.7b, en | +0.06 | −0.21 to +0.32 | tie |
| ultra vs parakeet v3, en | −0.51 | −0.77 to −0.28 | ultra better |
| ultra vs cohere, en | −0.61 | −0.90 to −0.32 | ultra better |
| ultra in Transformers vs in Photon, en | +0.00 | −0.04 to +0.04 | the same model |
| qwen3-asr-1.7b vs cohere, vi | −0.29 | −0.96 to +0.34 | tie |
| parakeet-ctc-vi vs qwen3-asr-1.7b, vi | +2.72 | +2.22 to +3.25 | qwen better |

### Findings

1. **English: parakeet-ultra.** It ties the most accurate model (Qwen3-ASR-1.7B) and beats
   Parakeet v3 and Cohere, at a fraction of Qwen's latency. Same architecture and tokenizer as
   Parakeet TDT 0.6B v3, CC-BY-4.0.
2. **Its weights are portable.** In plain Transformers (`ParakeetForTDT`, with v3's processor
   and generation config) it is exactly as accurate as in Moondream's Photon engine. So Utter
   does not need Photon, which matters: **Photon sends usage telemetry to api.moondream.ai
   every minute** (hostname, GPU, request counts; no audio or text found) with no setting to
   turn it off. The bench ran it with the network blocked.
3. **The runtime matters as much as the model.** The same Ultra weights take 31 ms in Photon
   and 74 ms in Transformers; Parakeet v3 takes 72 ms in onnx-asr. Utter's runtime target is
   Photon's speed or better.
4. **Vietnamese: Qwen3-ASR-1.7B or Cohere** (a tie). Qwen detects the language itself; Cohere
   needs to be told. Both are about 0.7 s and 0.16 s per clip in Python. Deferred: English
   first (Tai, 2026-10-09).
5. **Nemotron 3.5 streaming loses** despite native streaming. With language auto-detection it
   heard Indian-accented English as Hindi on Monsoon (55.9% WER) and missed the language on
   17% of clips overall.
6. **The Vietnamese Parakeet** turns any English into Vietnamese words. Its "code-switching"
   means English words inside Vietnamese speech.
7. **Not investigated:** Whisper's 64.9% on Bud500 (probably hallucinations on short clips).
   It does not change any decision.

### Decisions

- English speech recognition: **parakeet-ultra weights in Utter's own runtime**, via an ONNX
  export of the encoder and the decoder-joint (the same layout as the Parakeet v3 ONNX export),
  run with onnxruntime from Rust. Done when its transcripts match the Transformers reference
  and its p50 is at or under Photon's 31 ms on this machine.
- Vietnamese: revisit after English ships. Candidates: Qwen3-ASR-1.7B and Cohere Transcribe.
