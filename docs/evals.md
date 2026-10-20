# Evals

How Utter is measured, the current numbers, and a dated log of every run. True for Utter 0.1.10
(2026-10-20). Every figure carries its date and setup; a figure is only as current as its date.

Unless an entry says otherwise, figures come from the development machine (RTX 3090 Ti, Ryzen 9
9900X, Windows 11), shared with ordinary desktop apps. Cleanup training and the large judge runs
ran on a GPU cluster (RTX A6000s). Raw outputs are under `%LOCALAPPDATA%\Utter_dev\eval\results\`
(`~/.local/share/utter_dev/eval/results/` on Linux; `UTTER_DATA_DIR` moves the root). How the
cleanup model is trained and what it must do: [cleanup.md](cleanup.md). Backends in depth:
[platforms.md](platforms.md). Setting up the tools: [development.md](development.md).

---

## 1. What we measure and how

### Metrics

| Metric | What it is | Measured by |
|---|---|---|
| **Ready to send** (the cleanup headline) | Share of dictations a careful writer would send exactly as typed: the output equals the intended text (spacing aside), or the judge finds only equally valid choices differ | `utter-eval` |
| Dictionary terms right | Share of dictionary terms in the intended text that come out spelled exactly | No tool today (it was computed by the removed `tools/eval` scripts; last figure 2026-10-11) |
| Replies cut off | Cleanup replies that never stop (the app would retry, then type the raw text) | `utter-eval`, last row of its table |
| WER | Word error rate after normalization: English by Whisper's English normalizer (the Open ASR Leaderboard's); Vietnamese lower-cased, punctuation removed, diacritics kept | `utter-bench report` (also scores `utter-cli bench` and `e2e-bench` transcripts) |
| Speech, cleanup latency | Per clip: front end, encoder, decode, total; per cleanup: wall time of the request to Utter's llama-server; p50 / p95 | `utter-cli bench`, `transcribe`; `utter-cli llm-bench` |
| Key up to text ready | Clips replayed in real time through the pipeline (segmenting while "speaking", recognition, cleanup; no insertion) | `utter-cli e2e-bench` |
| Key up to text in the app | The same plus the paste, in the running app: per dictation in History (speech, cleanup, insert, total) and the dictation log | The app, with `UTTER_TEST_AUDIO` |
| Launch to ready | Process start to the end of the splash's wait | The app's log, `<data dir>\logs\utter.log`: "ready at X s after launch" |
| GPU memory | Peak memory of Utter and its llama-server together, under load | `nvidia-smi` by hand; no repo tool. Each tier's need is `utter_pipeline::gpu_need` |

Older log entries also use metrics of the removed `tools/eval` scripts, which no current tool
computes: **no edit needed** (exact match), **characters to fix** (character error rate against
the intended text), **word error** of the cleaned text, punctuation F1, and **exact spans**.

### The eval sets

**Cleanup: the datasets' test splits.** `utter-eval` builds its sets from
[dictation-to-text](https://huggingface.co/datasets/ductai199x/dictation-to-text) and
[dictation-speech](https://huggingface.co/datasets/ductai199x/dictation-speech) at a pinned
revision (default: main), and recognizes the speech with Utter's own Parakeet (`utter-cli
bench`). A passage's rows are always in one split, so no test passage is trained on.

| Set | n (2026-10-14) | What it holds |
|---|---|---|
| `text-test` | 9,995 | The text dataset's test rows: the recorded transcript, every kind |
| `kokoro` | 4,642 | The main set: real written passages (technical, everyday, science, chat, prose) in five spoken kinds, Kokoro voices |
| `chatterbox` | 4,229 | The same, spoken by Chatterbox cloning donated voices no training clip uses |
| `kokoro-real` | 937 | Realistic dictation: everyday messages, emails, notes, AI prompts and updates, plus short real forum and chat posts, each spoken one way (plain, fillers, self-correction, command, later edit, spoken punctuation, broken grammar) |
| `chatterbox-real` | 937 | `kokoro-real` in unseen voices |
| `kokoro-directed` | 292 | Text that must be typed, never answered: questions, requests to AI assistants, instructions to coding agents, text telling the app to ignore what came before (85 "injection" passages), plain or with fillers |
| `kokoro-restate` | 563 | A whole sentence said wrong, then said again in other words partway through |
| `kokoro-grammar` | 676 | Grammar slips of real speakers, to be corrected |
| `kokoro-lists` | 803 | Lists spoken item by item |
| `kokoro-amend` | 1,166 | Far-back edits (change, remove, insert, position) |
| `kokoro-real`, `kokoro-directed` + dictionary | 937, 292 | Again with a 300- and a 231-term dictionary (`tools/utter_tools/dictionaries/`), boosted in recognition and sent to cleanup |

Sizes move with dataset revisions: on 2026-10-12 `kokoro-real` held 950 dictations. The sets are
grown only through `utter-data` (passages, proofreading and grammar fixes, LLM-rendered spoken
versions with the intended text known by construction, Kokoro or Chatterbox speech, publishing
after `utter-data check`); see [cleanup.md](cleanup.md).

**Speech: public test sets.** `utter-bench fetch` takes a seeded random sample of 300 clips of
1-30 s from each of 11 sets, resampled to 16 kHz. English: AMI (meetings), Earnings-22,
GigaSpeech, LibriSpeech test-other, VoiceArena Monsoon (Indian English), URGENT24 (noise and
reverberation), VoxPopuli. Vietnamese: Bud500, FLEURS, GigaSpeech2, ViMD (63 dialects).

**Curated cleanup cases** (`crates/utter-pipeline/tests/fixtures/cleanup_cases.jsonl`, 112):
fillers, corrections, lists, spoken formatting, numbers, clean text, dictated questions, code.
They were the first cleanup bench (2026-10-09); today only a unit test reads them (`cargo test
-p utter-pipeline`: the gate never skips a case whose cleanup changes words). `tools/tests`
(pytest) tests the dataset builders, not the models.

### The graders

- **Ready to send.** Exact match first; every other output goes to Gemma 4 26B-A4B with
  [`tools/utter_tools/prompts/judge.md`](../tools/utter_tools/prompts/judge.md), given what was
  heard, meant and typed. It lists the differences and returns `acceptable` or `needs_edit` with a
  cause (style, recognition, punctuation, capitalization, grammar, correction, list,
  numbers-symbols, words, other). Rubric calibrated on Tai's review of 20 verdicts (2026-10-10):
  punctuation is the writer's choice unless it garbles or changes the meaning. Verdicts are cached
  with the output, intended text and rubric hash; a change to any re-judges.
- **Which 26B.** vLLM on a GPU cluster (`--judge-url`, `vllm-26b`) or Utter's llama-server locally
  (`local-26b`). They grade a little differently (2026-10-11): compare rows graded the same way.
- **WER** significance: paired bootstrap (10,000 resamples) over clips both systems transcribed.
  The tool (`asr_bench compare`) was removed; its results stay in the log.

### Commands

Python tools run from `tools/` with uv; `utter-cli` from the repository root (run `setup` once).

```bash
# Cleanup, ready to send (resumes; also --limit N, --text-ref/--speech-ref REV, --judge NAME)
uv run utter-eval --model quality-v2=PATH.gguf --model light-v2=PATH.gguf [--judge-url URL --workers 64]
# Speech accuracy: candidates in their official runtimes, the Rust runtime, one WER table
uv run utter-bench fetch [SET ...] && uv run utter-bench run [--model qwen3-asr-1.7b] [--limit N]
cargo run --release -p utter-cli -- bench [--sets en-ami,en-voxpopuli] [--name parakeet-ultra-rust]
uv run utter-bench report
# Speech latency and settings: idle gaps, keep-alive, phrase boosting, session flags (global flags
# go before the subcommand: --encoder-device, --decoder-device, --model-dir, --memory-pattern,
# --weights-outside-pool, --arena-exact, --cpu-threads N, --cpu-spinning, --no-conv-max-workspace)
cargo run --release -p utter-cli -- bench --gap-ms 1500 --keepalive-ms 100 [--phrases terms.txt]
cargo run --release -p utter-cli -- transcribe some.wav
# Cleanup latency (also --prompt FILE, --server-arg=ARG, --prefill-fraction F, --limit N)
cargo run --release -p utter-cli -- llm-bench --gguf F.gguf --cases cases.jsonl --name N [--gap-ms 3000 --keepalive-ms 100 --warm-gpu]
cargo run --release -p utter-cli -- clean --tier light "um so the text"
# End to end, key up to text ready (also --gap SECONDS, --name N, --gated, --threads high)
cargo run --release -p utter-cli -- e2e-bench --tier quality|light|mini [--sets en-ami,en-voxpopuli --limit 40]
# The speech export against Transformers, token for token
uv run utter-speech check
```

Environment: `UTTER_TEST_AUDIO=<16 kHz mono WAV>` feeds every dictation of a development build
from that file, in real time, with nothing played (use a separate `UTTER_DATA_DIR`);
`UTTER_NO_GPU_WARM=1` turns off the clock hold; `UTTER_GPU_BACKEND=cuda|vulkan|metal|cpu` forces
a backend; `UTTER_CLI` and `UTTER_LLAMA_SERVER` point the Python tools at builds elsewhere (a Linux
machine). Benches are sensitive to anything else on the GPU: note what else is running.

---

## 2. Current numbers

Each table takes the newest run for its figures. Model names: **Quality** = Gemma 4 E4B
fine-tuned (`utter-quality-v2-q4_0.gguf`), **Light** = Gemma 4 E2B fine-tuned
(`utter-light-v2-q4_0.gguf`), **Extra light** = Gemma 3 1B fully fine-tuned
(`utter-mini-v1-q4_0.gguf`), speech = parakeet-ultra 0.6B in Utter's ONNX runtime (8-bit
encoder weights on CUDA, float16 on WebGPU, int8 on the processor).

### Latency, key up to text

| Figure | Value | Date, setup |
|---|---|---|
| Key up to text **in the app** (with the paste) | **231-260 ms** | 2026-10-12, Quality, 3 dictations of a 5.7 s recording via `UTTER_TEST_AUDIO`, into Notepad |
| of which speech after key up / cleanup / the paste | 48-71 / 131-158 / 47-61 ms | same; the paste's details in 3.3 |
| `e2e-bench`, Quality, Windows, CUDA | 199 / 345 ms p50 / p95 | 2026-10-18, 20 clips each of en-ami and en-voxpopuli ([platforms.md](platforms.md)) |
| `e2e-bench`, Quality, Linux (WSL 2), CUDA | 197 / 393 ms | 2026-10-18, same clips; speech text identical, 40 of 40 |
| `e2e-bench`, Quality, CUDA, 80 dictations | 180 / 361 ms | 2026-10-13, Utter 0.1.5 running on the same card |
| `e2e-bench`, Light, CUDA | 167 / 323 ms | 2026-10-11, 120 clips (AMI, VoxPopuli, Earnings-22) |
| Speech per clip (Rust, warm, back to back) | 34.3 / 59.4 ms; front end 1.0, encoder 20.5, decode 12.5 ms p50 | 2026-10-09, float32 encoder, all eval clips; the 8-bit encoder now shipped is about 5 ms slower (2026-10-11) |
| Cleanup per request, Quality v2 | 250 / 387 ms | 2026-10-13, `llm-bench`, 100 kokoro-real cases, back to back |

### Launch to ready

| | Value | Date, setup |
|---|---|---|
| Warm (files in the cache) | **3.54-3.57 s** | 2026-10-12, Quality, 8-thread read-ahead; 3.55-3.75 s over 5 runs before it |
| Cold (after a reboot) | **5.59-6.01 s** | 2026-10-12, Quality, about 7.2 GB read from disk |
| Processor only, Ubuntu VM, Extra light | 2.77 s relaunch | 2026-10-18 ([platforms.md](platforms.md)) |
| Processor only, Fedora 44, 6 cores, no graphics card | 4.17 s (4.13 s with the page cache dropped) | 2026-10-18 |

### Cleanup quality: ready to send %

Newest full comparison of the shipped desktop models, 2026-10-12 (vLLM 26B judge; held-out
speech, neither model version trained on these passages):

| Set | n | Quality v2 | Light v2 |
|---|---|---|---|
| Realistic, Kokoro voices | 950 | **88.1** | 85.4 |
| Realistic, unseen Chatterbox voices | 950 | 80.1 | 75.5 |
| Directed | 292 | 87.3 | 82.5 |
| Realistic, 300-term dictionary | 950 | 87.8 | 85.7 |
| Directed, dictionary | 292 | 87.7 | 83.6 |
| Text test split | 9,995 | 70.3 | 64.5 |

Every set, 2026-10-14, the dataset at a later revision (vLLM 26B judge, outputs after the app's
retry of runaway replies). Light v2 against the 1B run measured on the test phone (full
fine-tune, 2 epochs, lr 2e-5); the repo does not record which 1B run was published as
`utter-mini-v1`:

| Set | n | Light v2 | Gemma 3 1B |
|---|---|---|---|
| text-test | 9,995 | 64.3 | 51.9 |
| kokoro | 4,642 | 59.3 | 43.1 |
| chatterbox | 4,229 | 52.5 | 37.1 |
| kokoro-real | 937 | **85.7** | 79.7 |
| chatterbox-real | 937 | 74.6 | 67.0 |
| kokoro-real + dictionary | 937 | 86.7 | 80.0 |
| kokoro-directed | 292 | 83.2 | 77.4 |
| kokoro-directed + dictionary | 292 | 84.6 | 76.0 |
| kokoro-restate | 563 | 62.0 | 45.1 |
| kokoro-grammar | 676 | 53.7 | 37.0 |
| kokoro-lists | 803 | 79.7 | 73.3 |
| kokoro-amend | 1,166 | 72.3 | 62.1 |
| Replies cut off (of 25,469) | | 0 | 1 |

Quality v2 has not been run on every set at full size (a 100-row smoke run on 2026-10-12 only).
**Dictionary terms right: 97.8%**, Quality v1 (ft6), 2026-10-11, local 26B grader; not measured
for v2. Goal for ready to send on the realistic set: 90% (Tai, 2026-10-11).

### Speech recognition: WER %

| Model, runtime | WER | Date, setup |
|---|---|---|
| parakeet-ultra, Photon / Transformers | 7.3 (English mean of 7 sets) | 2026-10-09, 2,095 clips |
| parakeet-ultra, Utter's Rust runtime | same as Transformers: +0.00 points (95% CI +0.00 to +0.01) | 2026-10-09 |
| float16 encoder | -0.02 points against the reference (noise) | 2026-10-09 |
| 8-bit encoder (shipped on CUDA) | the same; 240 of 240 transcripts identical across memory settings | 2026-10-09, 2026-10-11 |
| CUDA vs WebGPU (Vulkan backend), 120 clips | 6.76 vs 6.79; 118 of 120 transcripts identical | 2026-10-13 |
| 8-bit (W8), float32, int8 on the processor, 500 clips of five sets | 6.09, 6.11, 6.29 (listed in that order at the source) | 2026-10-18 ([platforms.md](platforms.md)) |
| Phrase boosting, default (+3, depth +1) | term recall 50.1% -> 56.2%, WER 6.86 -> 6.87 | 2026-10-10, 4,756 dictations |

Candidates and Vietnamese: the 2026-10-09 bench below.

### GPU memory (peak, Utter plus llama-server)

| | Quality | Light | Date, setup |
|---|---|---|---|
| CUDA, 3090 Ti | **4,572 MiB (4.8 GB)** | **3,085 MiB (3.2 GB)** | 2026-10-11, v1 models, 120 clips end to end |
| CUDA, Quality v2 | 4,550 MiB | | 2026-10-13 |
| Vulkan on the 3090 Ti | 5,690 MiB | 4,143 MiB | 2026-10-13 |
| Metal, processor | not measured (system memory) | | |
| `gpu_need` (what first run checks; the card it is offered on adds 1 GB) | CUDA 4.8 GB, Vulkan 6.2 GB | CUDA 3.25 GB, Vulkan 4.5 GB | code, 0.1.10; Metal and the processor use Vulkan's figures, Extra light 2.0 GB (CUDA) / 3.0 GB, unmeasured |

### Backends side by side

| | CUDA (3090 Ti) | Vulkan (3090 Ti) | Processor (Ryzen 9 9900X) | Metal |
|---|---|---|---|---|
| Encoder p50 / p95 | 19.7 / 30.6 ms | 50.8 / 75.6 ms | int8: 17.7 ms per second of audio | not measured |
| Cleanup p50 / p95 | 250 / 387 ms (Quality) | 274 / 417 ms (Quality) | Extra light 308 / 786 ms, Light 391 / 1,057 ms | not measured |
| Key up to text p50 / p95 | Quality 180 / 361 ms | Quality 234 / 429, Light 201 / 381 ms | Extra light 0.39 / 0.81 s (published int8 model) | not measured |
| Date | 2026-10-13 | 2026-10-13 | 2026-10-18 | |

Vulkan on a real AMD or Intel card has not been measured: the Vulkan column is NVIDIA hardware
running the Vulkan build. Detail and history: [platforms.md](platforms.md).

---

## 3. Log

Every run from the old notebook, grouped by topic, newest first within each. "Superseded" marks
conclusions later reversed. Runs after 2026-10-12 on backends, the processor, Linux and the phone
are logged in [platforms.md](platforms.md) and [android.md](android.md).

Naming: before 2026-10-11 the E4B tier was called "light" and the 26B "quality". ft1-ft6 are
training runs; shipped files are `utter-<tier>-v<N>-<quant>.gguf` (Tai, 2026-10-11): ft6 shipped
as `utter-quality-v1`, and the 2026-10-12 entry labels Light v1 as run ft5.

Removed tools the old entries named: `tools/eval/finaltext/judge.py` -> `utter-eval`;
`finaltext/dictaug.py` -> dictionary lines in dictation-to-text (`data/pairs.py`);
`boost_eval.py` -> `utter-eval`'s dictionary runs and `utter-cli bench --phrases`;
`cleanup_score.py` and `finaltext/prompts/cleanup-v4.md` -> removed; `tools/export/*.py` ->
`utter-speech`; `eval.fetch_sets` -> `utter-bench fetch`; `messages.py --directed` -> `utter-data
passages NAME --source directed`; `goldfix --grammar` -> `utter-data fix`.

### 3.1 Cleanup models

#### 2026-10-12: Cleanup v2, trained on the published dataset

Quality (E4B) and Light (E2B) v2: one recipe for both (LoRA rank 16, alpha 32, lr 2e-4, 1 epoch)
on the train split of dictation-to-text (79,246 pairs), merged, Q4_0. Run on the GPU cluster:
cleanup through `utter-cli llm-bench` with the app's prompt and server settings, exact match,
then the vLLM 26B judge. v2's scores are the first table of section 2; v1 on the same five rows
(realistic Kokoro, realistic Chatterbox, directed, realistic with dictionary, directed with
dictionary): Quality v1 88.5, 81.6, 86.0, 87.3, 88.0; Light v1 86.3, 77.3, 84.2, 86.8, 85.6.

The text test split (9,995 rows) does not compare the versions: v1 was trained before the
dataset was split by passage, and its training pairs hold most of today's test passages (Quality
v1, ft6: 3,543 of 5,400; Light v1, ft5: 5,292). v1 reproduces details the dictation never said
(an arXiv "Bose--Einstein", a quoted "major"). v2 scores 70.3% (Quality) and 64.5% (Light), the
first clean number for this split.

**Decision:** Quality ties, Light trails by 1-2 points on every set. v2 ships (0.1.2): every
shipped model is reproducible from public data with `utter-train` (Tai: "the point of v2 is
standardization").

#### 2026-10-11: Quality v1 (ft6), and the dictionary

Shipped as `utter-quality-v1-q4_0.gguf`. *Superseded by v2 (2026-10-12).*

**Why a new run:** with a dictionary, ft5 got worse (84.3% ready to send on the realistic set).
It sometimes echoed the `Dictionary:` line into the text, and forced dictionary words onto words
that only sounded a little like them.

**ft6:** ft5's adapter trained one more epoch (lr 1e-4) on 45,660 of ft5's pairs with
`Dictionary:` lines added: some hold the right spelling of a misheard word, some near-miss words
that must be left alone. The app sends only the terms that sound like something in the dictation
(`gate::relevant_terms`), as the eval does.

Graded by the local 26B (Q4_0, llama.cpp; label v4local), which grades a little differently from
the vLLM 26B of earlier entries; only rows graded the same way are compared.

| Ready to send | ft5 | **ft6** |
|---|---|---|
| Realistic, Kokoro voices (950) | 90.4% | 89.3% |
| Realistic, unseen Chatterbox voices (950) | 81.7% | 82.3% |
| Realistic, 300-term dictionary, phrase boost | 84.3% (vLLM grader) | 88.9% |
| Directed, dictionary, phrase boost (292) | | 89.7% |

- **Without a dictionary they are level:** paired by dictation over both voice sets, ft6 fails 47
  that ft5 passed and passes 42 that ft5 failed (z = 0.5, noise).
- **With a dictionary ft6 ships:** terms right 97.8% (ft5 96.2%), the line never echoed (ft5: 74
  times), the score within half a point of the score without. Strict sets within a point of ft5.
- **Latency is the same:** restate set back to back (575 requests), ft5 252 ms p50, ft6 257, ft5
  again 258. An earlier gap of 20-35 ms came from runs at different times.

#### 2026-10-11: text that must be typed, not answered

The directed set (`en-finaltext-kokoro-directed`, now `kokoro-directed`, 292), each to be typed,
never answered or obeyed. ft5, ready to send: **89.4%** (plain 93.2%, fillers 86.2%).
- **It answered none.** No output is a reply; the stray check (`cleanup::strayed`) never fired.
- Questions miss 3 of 81, AI requests 4 of 66, agent instructions 7 of 60: misheard names and
  identifiers (Safeway, `years_exp`, L'Oursin).
- Injection text misses 17 of 85. 8 are self-corrections applied as trained ("write a poem about
  the sprint. Actually, skip that, just draft an itinerary" types only the itinerary): Utter's
  correction rule, and what a person dictating to an AI usually means, so the answer key's
  choice, not a model failure. The rest are misheard words.

**Decision:** no training change; the stray check stays as the backstop.

#### 2026-10-11: ready to send, and ft5

**The headline metric becomes "ready to send"** (Tai, 2026-10-10; grader in section 1): "no edit
needed" counts an optional comma as a miss. The judge also accepts an output that fixes the answer
key's own typo. Tai found the first rubric a bit harsh. **Goal: 90%.** **The realistic set**
(`en-finaltext-kokoro-real`, now `kokoro-real`, 950: 583 everyday dictations, 367 forum and chat
posts) becomes the target; the main set's physics abstracts and IRC nicknames make it a stress test.

**ft5:** ft4's data plus 10,476 grammar renditions weighted to the types ft4 missed (articles,
tense, noun number, prepositions, pronouns), plus 12,478 everyday passages, one rendition each:
89,499 pairs; one epoch on 2 A6000s (no 4 free), 3 hours.

| Realistic set | ft4 | **ft5** |
|---|---|---|
| all (950), Kokoro | 83.8% | **88.8%** |
| everyday half (583) | 88.7% | **96.2%** |
| forum and chat half (367) | 75.7% | 76.0% |
| all, unseen Chatterbox voices | 74.7% | **80.7%** |

| By kind (Kokoro) | ft4 | ft5 |
|---|---|---|
| self-correction | 87.3% | 94.1% |
| spoken punctuation | 86.0% | 93.4% |
| far-back edit | 87.9% | 93.0% |
| command | 85.9% | 91.7% |
| plain | 89.3% | 90.0% |
| fillers | 79.1% | 79.7% |
| broken grammar | 67.0% | 79.0% |

Strict scores of ft5, no edit needed / characters to fix (ft4's are in the ft4 table): main 18.8 /
3.5, main Chatterbox 16.5 / 4.8, restate 15.9 / 2.4, grammar 14.5 / 3.5, lists 47.4 / 3.6,
far-back edits 40.9 / 3.5.

**Decision:** ft5 regressed nowhere and became the light (E4B) tier. *Superseded by ft6, then
v2.* Left: forum and chat posts stay at 76% (names, nicknames and product names Parakeet
mishears, which only the dictionary or context can fix); "me and him went" is not fixed; a
leading "Uh, so..." no longer survives (none of ft5's 32 filler misses keep one: they are
misheard words and garbled restarts).

#### 2026-10-10: ft4, grammar, lists and far-back edits

**Data:** 66,981 pairs: the earlier kinds plus 28k new passages with one rendition each;
grammar mistakes typed by ERRANT category from speaker profiles; lists (real ones from Stack
Exchange and GitHub plus everyday ones; 20% trained with `Lists: off`); far-back edits (change,
remove, insert, position). Every gold went through the grammar pass; no eval passage trains (121
clips caught by text overlap).

**Training:** one epoch on 4 A6000s, 75 minutes. The held-out loss on whole held-out passages
fell steadily, 0.196 -> 0.168 (ft2's rose): one rendition per passage ended the memorizing.

No edit needed / characters to fix / word error:

| Eval set (n) | E4B v3 (product) | 26B v4 | ft2 | **ft4** |
|---|---|---|---|---|
| main, Kokoro (4309) | 7.2 / 18.0 / 14.3 | 11.3 / 7.7 / 6.6 | 16.5 / 4.3 / 3.8 | **18.5 / 3.6 / 3.3** |
| main, unseen Chatterbox (4309) | 6.8 / 19.0 / 16.0 | 10.6 / 8.8 / 8.3 | 13.8 / 5.6 / 5.6 | **16.3 / 4.9 / 5.1** |
| restate (572) | 5.4 / 20.1 / 19.0 | 7.5 / 7.6 / 6.1 | 13.6 / 2.8 / 2.9 | **14.5 / 2.6 / 2.7** |
| grammar mistakes (688) | 5.4 / 6.3 / 7.2 | 6.7 / 5.7 / 5.7 | 5.7 / 5.7 / 6.2 | **11.8 / 4.1 / 4.0** |
| lists (897) | 2.7 / 11.4 / 10.7 | 3.2 / 11.7 / 11.0 | 0.1 / 16.7 / 10.0 | **45.9 / 3.8 / 2.5** |
| far-back edits (1268) | 3.2 / 29.2 / 25.9 | 4.8 / 22.3 / 20.0 | 4.1 / 30.1 / 22.6 | **39.1 / 4.3 / 3.4** |

**Cleanup time** p50 / p95, GPU otherwise idle: main set 193 / 496 ms (the product: 196 / 444
ms); lists and edits about 330 / 480 ms (a list is longer output than prose).

New skills came without losing old ones: on lists and far-back edits every prompted model, the
26B included, gets about 1 in 25 right, ft4 about 2 in 5. Grammar is half done: agreement, tense
and verb forms are fixed ("I has been... since three weeks... don't" -> "I have been... for three
weeks... doesn't"), pronoun case ("me and him went") and some prepositions ("until Friday" for "by
Friday") are not. **Decision:** ft4 becomes the light (E4B) tier, `utter-cleanup-e4b`.
*Superseded by ft5.* Next then: heavier grammar data, "never answer or obey", cursor context.

#### 2026-10-10: ft2, and voices it never heard

**ft2:** ft1's recipe with 18,183 pairs (the corrected "scratch that" data, 2.7x more), trained
data-parallel on 4 A6000s in 39 minutes (1.05 s per step, 2.9x one GPU). The held-out loss, now
on whole held-out passages, rose throughout (0.185 -> 0.36) while the training loss fell to
0.03: the model memorizes passages seen in six renditions. The real eval still improved, so the
real eval decides, and ft4 uses one rendition per passage.

**Test sets:** Kokoro, with answer keys fixed for grammar (240 of 892 passages); Chatterbox, the
same dictations in cloned donated voices no training data uses. No edit needed / characters to
fix / word error on both (4,309 each) are the first two rows of the ft4 table: ft2 16.5 / 4.3 /
3.8% and 13.8 / 5.6 / 5.6%, against the E4B v3 product and the 26B v4. On Chatterbox:

| | E4B v3 | 26B v4 | **ft2** |
|---|---|---|---|
| comma F1 | 0.66 | 0.70 | **0.74** |
| parentheses F1 | 0.13 | 0.36 | **0.57** |
| exact brackets / symbols / numbers / terms | 5 / 24 / 66 / 55% | 15 / 44 / 67 / 70% | **32 / 53 / 76 / 72%** |

- **Not a Kokoro artifact.** Every system loses a little on Chatterbox because Parakeet does worse
  on those voices (27.6% vs 25.8% characters to fix on its own); ranking and margins stay.
- **Gaps the eval did not measure:** ft2 writes lists inline, keeps the speaker's grammar
  mistakes (the product's prompt fixed them), and misses far-back edits. Tai set lists and
  grammar fixing on by default ([cleanup.md](cleanup.md)), so ft4 trains them and the eval gained
  grammar, list and amend sets.

#### 2026-10-10: fine-tuning the cleanup model (ft1)

**Model:** LoRA on Gemma 4 E4B (Google's unquantized QAT checkpoint), exported to the Q4_0 GGUF
the product runs; a 28-word system prompt (`crates/utter-llm/prompts/cleanup-ft.md`), the rules
in the weights. **Data:** 6,643 pairs from the final-text train split (real passages rendered by
the 26B in six kinds, Kokoro, Parakeet), split by source document. **Training:** one RTX A6000
per run, bf16, 2 epochs in 42 minutes (about 3 s per step of 16 sequences).

Sweep, eval loss on 135 held-out pairs (two seeds of one setting differ by 0.0006), as rank / lr
/ epochs: 16 / 1e-4 / 2: 0.0287; 16 / 1e-4 / 3: 0.0231; 32 / 1e-4 / 2: 0.0229; **16 / 2e-4 / 2:
0.0190** (seed 1: 0.0196); 32 / 2e-4 / 2: 0.0189; 16 / 3e-4 / 2: 0.0205.

Corrected eval (4,318 dictations, fixed gold):

| | E4B v3 | E4B v4 | 26B v4 | **ft1 r16 2e-4** | ft1 r32 1e-4 |
|---|---|---|---|---|---|
| no edit needed | 7.4% | 8.2% | 11.5% | **14.0%** | 12.7% |
| characters to fix | 17.9% | 12.3% | 7.6% | **6.3%** | 8.8% |
| word error | 14.0% | 10.1% | 6.4% | **4.7%** | 6.2% |
| comma F1 | 0.67 | 0.67 | 0.70 | **0.73** | 0.72 |
| parentheses F1 | 0.21 | 0.37 | 0.40 | **0.56** | 0.54 |
| exact brackets / symbols / terms | 7 / 25 / 56% | 15 / 30 / 62% | 19 / 44 / 69% | **29 / 46 / 72%** | 28 / 45 / 71% |
| restate set: characters to fix | 20.1% | 16.8% | 7.5% | **4.2%** | 5.7% |

- **The 4-bit E4B, fine-tuned, beats the prompted 26B** on every headline number, at the E4B's
  size and speed (tokens per second matched the base model once it had the GPU to itself).
- **Its weak spot was the data's:** "scratch that" and "delete that" (13.9% and 13.6% characters
  to fix, against 8.7% and 5.6% for the 26B); their renditions taught the wrong scope. ft2 trains
  on corrected data.

Eval fixes along the way (baselines re-scored): 7 of 901 eval passages (27 train) were German,
Italian, Dutch, French or Finnish IRC lines read by English voices (the passage filter, now
`data/passages.py`, keeps English only); "scratch that" renditions had a two-sentence false start
the gold dropped whole, contradicting the one-sentence scope (401 eval and 1,268 train renditions
re-rendered with one sentence); the rendition checks rejected faithful readings (decades,
"sixteen point ten", "nineteen oh five", regnal numerals): half failed, about a fifth now, for good
reasons (dates read as words, typos the gold keeps).

#### 2026-10-10: final-text baselines (eval v1)

Gold: 1,000 real written passages, 200 each of technical, everyday, science, chat and prose.
Spoken forms: 5 kinds rendered by Gemma 4 26B with the gold known by construction (4,756
renditions after checks); Kokoro (12 voices); Parakeet through Utter's runtime. Scored: 3,683
dictations whose gold the screen did not flag. Prompt v4 adds punctuation from meaning, spoken
symbols, acronyms and correction scope to the product's v3.

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
| exact numbers / symbols / terms | 65 / 14 / 51% | 73 / 27 / 56% | 70 / 33 / 63% | 72 / 33 / 58% | 73 / 45 / 69% |
| cleanup time p50 / p95 | | 196 / 444 ms | 209 / 488 ms | 212 / 457 ms | 238 / 561 ms |

The product then got about 1 dictation in 13 exactly right, the best prompted model about 1 in 8;
prompting helps the 26B more. Retraction commands and disfluencies fail most ("Never mind, let's
start over" drops one sentence, not everything). Most misses are 2-3 small differences:
punctuation conventions, misheard names, quirks of the gold. **Decision:** fine-tune next. Gaps
then: no sentence-scope corrections, a noisy gold screen (gold fixes replaced it), one TTS engine
(Chatterbox followed).

#### 2026-10-09: cleanup model, prompted

*Superseded by fine-tuning (2026-10-10) and the tiers of 2026-10-11.*

**Method:** rules then in the old cleanup spec (now [cleanup.md](cleanup.md)); the product's
prompt (`crates/utter-llm/prompts/cleanup.md`, still the 26B reference's). 112 curated cases scored
by code (each case's checks, exact match, WER, words added); 516 real-speech utterances (Parakeet
on AMI and Monsoon, no expected text) scored by rules (fillers left, words added, speaker's words
kept, outputs that look like answers). Every model through Utter's llama-server (`utter-cli
llm-bench`), one request at a time, thinking off, temperature 0, system prompt cached; wall time
on the 3090 Ti at 30% load from desktop apps.

Candidates (all 4-bit; prompt v1):

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

"Added" is mostly grammar repair ("a", "makes", "fewer", "He and I") and, for E4B, style changes
("gonna" -> "going to"); the rules then forbade both. Higher-precision quantizations were not
pursued: slower, and latency comes first (Tai).

Making it fast, Gemma 4 26B-A4B, p50 curated / real: baseline 215 / 284 ms; `--swa-full` (a
full-size cache for the sliding-window layers; without it every request rebuilds that cache from a
checkpoint; prompt processing about 95 -> 25 ms) 107 / 182 ms; + n-gram drafting from the prompt
(3-token match, 6-token draft, acceptance about 60%) 98 / 136 ms. Not used: n-gram drafting on the Qwen models (no gain at 62% acceptance: their recurrent layers
verify drafted tokens one by one); llama-server's default n-gram sizes (12/48, 5% acceptance,
slower); prefilling the prompt while the user speaks (a raw-prefix prefill is reusable but saved
2-3 ms with `--swa-full`). Drafting is effectively lossless: all curated outputs identical, 97%
of real ones (the rest differ by float rounding of batched verification). `--swa-full` and the
3/6 drafting are still the app's settings (`crates/utter-llm/src/cleanup.rs`).

**Prompt v2** (keep the speaker's grammar, slang, contractions, spellings): 26B checks 98% ->
**99%**, real-speech added words 16.3% -> **9.1%**, latency unchanged; the one failure left "6
chairs" for "six chairs". **Prompt v3** (two examples: grammar and slang kept; an "I mean"
correction): E4B checks 96% -> **97%**, added words 20.0% -> 15.5%, 124 ms p50; 26B 99%
unchanged, added words 9.1% -> 8.1%.

**Decision then:** Gemma 4 26B-A4B QAT with `--swa-full` and drafting, prompt v2: 99% of checks,
101 ms p50 curated, 136 ms real, p95 about 255 ms; 14.4 GB of weights; component
`gemma4-26b-a4b-qat`. E4B the fallback for smaller GPUs (96%, 126 ms, 4.9 GB). The 26B remains an
optional component, used only as the judge and a reference.

### 3.2 Speech recognition

#### 2026-10-10: phrase boosting

`crates/utter-asr/src/boost.rs` adds a bonus to dictionary phrases' tokens during greedy
decoding; weights unchanged. Method (`utter-cli bench --phrases`): 250 terms sampled from the
final-text golds as a user's dictionary would hold them (MySQL, TF32, Kerr-Newman; no paths,
sizes or capitalized everyday words), the 115 unsampled terms as a control. Recall: a term in the
gold written exactly, case and all. Inserted: a term written more often than the gold has it.
Harm test: the same dictionary on VoxPopuli and Earnings-22.

| | term recall | inserted | other terms | WER | clips changed |
|---|---|---|---|---|---|
| no boost | 50.1% | 136 | 37.6% | 6.86% | |
| continue +3, depth +1 (default) | 56.2% | 146 | 37.8% | 6.87% | 231 of 4756 |
| continue +5, depth +1 | **59.6%** | 162 | 37.7% | 6.90% | 370 |

Harm test, WER (clips changed), VoxPopuli (300) / Earnings-22 (300): no boost 3.03% / 9.90%; +3
3.03% (0) / 9.92% (2); +5 3.02% (1) / 10.00% (6), 3 of Earnings-22's own terms lost.

- **The first version did harm.** A bonus on any non-blank token is the same as lowering blank,
  so the decoder wrote words nobody said ("Period.", "<unk>", "a" for "uh"); a phrase-start bonus
  pulled common words toward dictionary starts ("um" -> "UM"). At +1.5 it changed 1,353 clips,
  cost the control 1.9 points and added 48 insertions, for the recall of the fix at +3.
- **The fix:** whether to emit stays the model's decision on unboosted scores; the bonus only
  chooses among non-blank tokens. No start bonus by default (+1 added nothing).
- **Cost:** per-frame decoding, about 6-8 ms more decode at p50 (17 -> 24 ms on Earnings-22),
  only with a dictionary.
- **Decision:** +3, depth +1 (`BoostConfig::default`). "Not wired into dictation yet" then; it is
  now: the pipeline boosts the user's dictionary and sends the heard terms to cleanup.

#### 2026-10-09: parakeet-ultra in Utter's Rust runtime

**Built:** the export into ONNX graphs (encoder with projection folded in, decoder step, joint,
fused decoder-plus-joint), now `utter-speech export`; the check (70 of 70 clips give the same
tokens and text as Transformers), now `utter-speech check`; `crates/utter-asr` with the audio
front end in Rust (matches Transformers' features; `crates/utter-asr/tests/frontend.rs`) and
greedy TDT decoding on onnxruntime; `utter-cli bench` writing results `utter-bench` scores.

**Accuracy:** the same as Transformers, pooled English WER difference +0.00 points (95% interval
+0.00 to +0.01) over 2,095 clips; 7.3% mean English WER.

**Latency** (per clip, batch 1, back to back), median of the sets' p50 / p95: **Utter, Rust 33 /
46 ms** (all clips 34.3 / 59.4 ms), Photon (Moondream's engine, Python bench) 31 / 40 ms. Per
stage, all clips, p50: front end 1.0 ms, encoder 20.5 ms, decode 12.5 ms; short AMI clips
(1-5 s) 27.5 ms in total. About 2 ms behind Photon's target on medians, with the GPU at 31-35%
utilization from other applications during the Rust runs.

| Change | Effect |
|---|---|
| First working runtime | 75.8 ms p50 |
| Encoder convolutions rewritten (`tools/utter_tools/speech/conv_free.py`) | onnxruntime's cuDNN Conv redid its setup on every new input length (2.3 ms on a repeated length, about 42 ms on a new one; every utterance is new). Pointwise convs became MatMuls; each depthwise conv runs padded to a fixed 512 frames (41 s) and is sliced back: exact, and cuDNN's setup happens once. A 9-shifted-multiply-add version removed the setup but launched about 650 more kernels: slower. |
| Sparse mel filterbank | Front end 7-19 ms -> 0.3-1.9 ms |
| Decoder and joint on the CPU | 12.6 ms decode p50, against 20.1 ms for the fused GPU step (per-call GPU overhead outweighs the small graphs). The GPU path stays an option (`--decoder-device`). |
| Not used | cuDNN's default convolution algorithm (750 ms per clip); memory-pattern planning off (no effect on latency then); one CPU thread (decode 20 ms); no spinning (encoder faster, decode slower). *The thread and spinning choice was reversed on 2026-10-12.* |

**Idle GPU, the finding that shaped the pipeline:** after 1.5 s idle the GPU drops to P5 (525 of
2,115 MHz) and the next encoder pass takes about 108 ms instead of 20 (AMI). Keep-alive passes
every 100 ms bring short clips back to 28.7 ms but long ones only to 62 ms (against 32). So the
pipeline warms on key down and the live preview's passes keep the GPU busy until key up; the
clock hold followed (2026-10-10). Left on the table then: CUDA graphs per length bucket (not
built); FP16 encoder weights (done 2026-10-09, 8-bit on 2026-10-11); the subsampling
convolutions' 9-tap rewrite (about 135 small launches).

#### 2026-10-09: speech recognition, first bench

**Method:** 11 public sets, 300 clips each (now `utter-bench fetch`); each model in its official
runtime, batch 1, after 3 warm-up clips; the language left to the model where it can detect it
(Cohere cannot and is told, which favours it); WER as in section 1; paired bootstrap of pooled
WER. Latency is the Python runtime's: it ranks models, Utter's own runtime is measured separately.

WER %, lower is better:

| Model | ami | earn22 | giga | libri-o | monsoon | urgent | voxpop | **en mean** | bud500 | fleurs | giga2 | vimd | **vi mean** | lang-ID miss | p50 / p95 ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **parakeet-ultra-0.6b** (Photon) | 9.9 | 10.0 | 10.6 | 3.3 | 4.8 | 9.3 | 3.0 | **7.3** | - | - | - | - | - | - | **31 / 40** |
| parakeet-ultra-0.6b (Transformers) | 9.8 | 10.0 | 10.7 | 3.3 | 4.8 | 9.3 | 3.0 | **7.3** | - | - | - | - | - | - | 74 / 111 |
| qwen3-asr-1.7b | 10.8 | 9.4 | 9.9 | 3.4 | 5.0 | 9.2 | 3.1 | **7.3** | 4.7 | 6.6 | 8.2 | 9.4 | **7.2** | 0.8% | 671 / 1090 |
| cohere-transcribe (told the language) | 7.9 | 10.5 | 11.1 | 2.4 | 5.5 | 9.8 | 5.6 | 7.5 | 3.8 | 7.0 | 9.7 | 9.2 | 7.4 | - | 162 / 230 |
| parakeet-tdt-0.6b-v3 | 10.4 | 10.8 | 11.1 | 3.3 | 5.5 | 10.4 | 3.0 | 7.8 | - | - | - | - | - | - | 72 / 92 |
| qwen3-asr-0.6b | 11.4 | 10.8 | 10.0 | 4.3 | 5.5 | 10.4 | 2.9 | 7.9 | 7.5 | 9.1 | 11.5 | 10.4 | 9.6 | 0.9% | 709 / 1109 |
| whisper-large-v3-turbo | 15.5 | 11.3 | 11.2 | 3.7 | 6.0 | 13.7 | 10.3 | 10.2 | 64.9 | 9.3 | 18.0 | 14.6 | 26.7 | 1.0% | 189 / 233 |
| nemotron-3.5-streaming-0.6b | 18.0 | 19.3 | 13.6 | 6.4 | 55.9 | 20.7 | 4.3 | 19.7 | 10.5 | 15.7 | 12.9 | 16.6 | 13.9 | 17.4% | 184 / 267 |
| parakeet-ctc-0.6b-vi | ≈100 on every English set | | | | | | | 101.9 | 7.5 | 13.3 | 12.2 | 10.2 | 10.8 | - | 55 / 65 |

Paired bootstrap, pooled WER over the same clips (English 2,095; Vietnamese 1,124):

| Comparison | Difference (A - B) | 95% interval | Verdict |
|---|---|---|---|
| ultra vs qwen3-asr-1.7b, en | +0.06 | -0.21 to +0.32 | tie |
| ultra vs parakeet v3, en | -0.51 | -0.77 to -0.28 | ultra better |
| ultra vs cohere, en | -0.61 | -0.90 to -0.32 | ultra better |
| ultra in Transformers vs in Photon, en | +0.00 | -0.04 to +0.04 | the same model |
| qwen3-asr-1.7b vs cohere, vi | -0.29 | -0.96 to +0.34 | tie |
| parakeet-ctc-vi vs qwen3-asr-1.7b, vi | +2.72 | +2.22 to +3.25 | qwen better |

1. **English: parakeet-ultra.** Ties the most accurate model (Qwen3-ASR-1.7B), beats Parakeet v3
   and Cohere, at a fraction of Qwen's latency. Same architecture and tokenizer as Parakeet TDT
   0.6B v3, CC-BY-4.0.
2. **Its weights are portable.** In plain Transformers (`ParakeetForTDT`) it is exactly as
   accurate as in Photon, so Utter does not need Photon, which **sends usage telemetry to
   api.moondream.ai every minute** (hostname, GPU, request counts; no audio or text found) with no
   setting to turn it off. The bench ran it with the network blocked.
3. **The runtime matters as much as the model:** Ultra 31 ms in Photon, 74 ms in Transformers;
   Parakeet v3 72 ms in onnx-asr. 4. **Vietnamese: Qwen3-ASR-1.7B or Cohere** (a tie; about 0.7
   s and 0.16 s per clip in Python); deferred, English first (Tai). 5. **Nemotron 3.5 streaming
   loses:** it heard Indian-accented English as Hindi on Monsoon (55.9%) and missed the language
   on 17% of clips. 6. The Vietnamese Parakeet turns any English into Vietnamese words. 7. Not
   investigated: Whisper's 64.9% on Bud500 (probably hallucinations on short clips).

**Decisions:** English is parakeet-ultra in Utter's own ONNX runtime (done the same day, above).
Vietnamese: revisit after English ships, with Qwen3-ASR-1.7B and Cohere Transcribe; `utter-bench`
keeps them as its candidates.

### 3.3 Latency and clocks

#### 2026-10-12: a pipeline that does not starve itself

In the app (Quality, a test instance with its own data folder), `UTTER_TEST_AUDIO` feeding a 5.7
s recording through the whole pipeline in real time: insertion into Notepad, History, the Bar,
the clipboard put back; no sound played. Every onnxruntime session ran one thread per core (12) with spin-waiting. Back to back that won
the speech bench (decode 12.5 against 20 ms on one thread, 2026-10-09); in the app it starved
everything else while the person spoke:

| 3 dictations each | Mic stream gaps | Speech after key up | Cleanup | Key up to text in the app |
|---|---|---|---|---|
| 12 threads, spinning | 26 (audio lost) | 61-79 ms | 169-191 ms | 282-320 ms |
| **2 threads, no spinning** | **0** | **48-71 ms** | **131-158 ms** | **231-260 ms** |

The paste is 47-61 ms: 25-30 ms the quiet wait for clipboard managers, about 20 ms SendInput
passing each key through every keyboard hook on the machine. **The first dictation is as fast as
the rest** (260 ms); in the unoptimized development build it was not (paste 667 ms the first
time, speech 388 ms), so development builds are now optimized (`Cargo.toml` `[profile.dev]`):
what `npx tauri dev` shows is what ships.

**Decision:** GPU sessions run 2 CPU threads, no spinning (`ParakeetOptions::default`; still so).
When the whole speech model runs on the processor the thread count is a share of the logical
processors instead (2026-10-18, [platforms.md](platforms.md)).

#### 2026-10-10: GPU clocks while the user speaks

The first `e2e-bench` (clips replayed in real time, 1 s gaps) had cleanup at 140-240 ms on short
texts, against 95-124 ms back to back. `llm-bench --gap-ms` (Gemma 4 E4B, product settings, 30
curated cases):

| Idle before the request | p50 | prompt (19 tokens) | per generated token |
|---|---|---|---|
| none | 95 ms | 19 ms | 7.3 ms |
| 0.5 s | 101 ms | 18 ms | 7.6 ms |
| 1 s | 110 ms | 20 ms | 7.8 ms |
| 3 s | 229 ms | 107 ms | 12.3 ms |

`nvidia-smi` every 100 ms: under light load the GPU sits in **P3, memory clock 5,001 MHz, half of
P2's 10,251**, and decoding is bound by memory bandwidth. The speech keep-alive passes keep it out
of P5 but not out of P3.

| Keeping it warm through a 3 s gap | p50 | GPU state, mean power |
|---|---|---|
| nothing | 229 ms | P3/P5 |
| one-token llama-server request every 250 / 500 / 1000 ms | 187 / 171 / 205 ms | mostly P3 |
| ... every 50 / 100 ms | 142 / 166 ms | P3 two thirds of the time, 114 / 101 W |
| ... back to back | 89 ms | P2, 262 W |
| a spin kernel in another process, full / half duty | 249 / 121 ms | P2, 157 W: time-slicing between processes slows the request |
| **one GPU thread in a `nanosleep` kernel, in Utter's process, stopped at key up** | **86 ms** (p95 143) | **P2, 157 W while the key is down** |

The last is `utter_runtime::gpu::GpuWarmer`: the clocks stay up about a second after it stops, so
cleanup runs at full speed with nothing competing. CUDA driver API from the driver, no component.
A first version ended each kernel on a `%globaltimer` deadline and hung the pipeline twice in the
first long clip after short ones; each kernel now ends after a fixed count of naps (at most 4 ms).
`UTTER_NO_GPU_WARM=1` turns it off. NVIDIA only; nothing holds the clocks on Vulkan yet.

End to end (40 clips per set, 1 s gaps, the E4B "light" tier of the time), key up to text ready:

| | before | with the warmer |
|---|---|---|
| AMI p50 / p95 | 55 / 253 ms | **31 / 163 ms** |
| VoxPopuli p50 / p95 | 74 / 496 ms | **50 / 368 ms** |
| transcribed (AMI) p50 / p95 | 36 / 99 ms | **26 / 49 ms** |
| cleanup, the 10 AMI clips the gate sent to it | 141-237 ms | **89-167 ms** |

These ran with the gate deciding which dictations to clean; `e2e-bench` now cleans every one
(`--gated` restores the old behavior), so later key-up figures are not comparable. No hang in the
80 clips. Tail: two VoxPopuli clips spoken as one 20 s segment took 205 and 244 ms to transcribe
after key up, the final pass waiting behind a preview pass of nearly the same 20 s.

So a segment over 8 s is now cut at a 128 ms pause instead of 384 ms (`crates/utter-asr/src/vad.rs`).
VoxPopuli, 40 clips, 384 ms pauses only -> soft cut after 8 s: key up to text p50 / p95 50 / 368
-> **28 / 315 ms**; transcribed 47 / 108 -> **27 / 73 ms**; WER +0.10 points (95% CI -0.21 to
+0.43, noise). Two clips still arrive as one segment (no 128 ms pause in 18 s) and take 114 and 191 ms.

### 3.4 Startup

#### 2026-10-12: cold starts

A start after a reboot reads about 7.2 GB from disk. Reproduced by copying the Quality components
with unbuffered writes (robocopy /J); launch to ready, two runs each:

| | Cold | Warm |
|---|---|---|
| The loaders read on their own | 7.34 s | 3.78 s |
| One read-ahead stream per component | 6.26-6.59 s | 3.79-4.03 s |
| 64 MB ranges, 8 threads | 5.46-6.28 s | 4.10-4.41 s (it read the cached 7 GB again) |
| PrefetchVirtualMemory on mapped files | 7.27-7.56 s (synchronous, 1.3 GB/s) | 3.54-3.59 s |
| **8 threads, after timing the model's first 64 MB (cached: read nothing)** | **5.59-6.01 s** | **3.54-3.57 s** |

The disk gives these files under 2 GB/s whatever the method, so a cold start cannot go far under
4 s here; the rest is the encoder's session (2 s) and the server. A cache probe reads 7.5-9 GB/s,
the disk about 1.1 GB/s to one stream: the 4 GB/s line between them is wide. Not Windows
Defender: a fresh copy read in 0.11 s the first time, and starts right after Repair re-downloaded
the runtime took 3.3-4.2 s. The one 22 s start seen was the machine busy at that moment (every
stage slow at once). **Decision:** the timed 8-thread read-ahead ships.

#### 2026-10-12: starting in under 4 seconds

Launch to ready, measured from process start (the splash's wait; it was measured from the
pipeline thread's start before, which hid the launch), Quality:

| | Launch to ready |
|---|---|
| Before: everything in turn, development build unoptimized | 7.9 s warm, 13-36 s under load |
| Speech model and cleanup server load at once | 4.7-5.7 s |
| Voice detection and the microphone open meanwhile; the splash window after loading starts | 4.7-5.2 s |
| The small decoder sessions beside the encoder's (one stalled 2.1 s once) | **3.55-3.75 s** (5 runs) |

Left: the encoder's session takes about 2.0 s (graph optimization at load; lower optimization
levels loaded slower, 5-7.7 s, and saving the optimized graph failed in onnxruntime 1.31); the
cleanup server about 2.9 s (vocabulary 1.1-1.5 s, weights 1.3-1.5 s), side by side. Under load
from the other, the server's weights once took 8 s: one reason the CPU threads (3.3) matter.

### 3.5 Memory and tiers

#### 2026-10-11: Light on the GPU, and the speech model's memory

**Light runs everything on the GPU** (Tai): speech on the CPU took 736 ms p50 a clip (the 8-bit
encoder alone 705 ms, Ryzen 9 9900X, all cores), against about 45 ms on the GPU; transcripts were
fine (1 of 200 AMI clips differs from full precision), the speed was not. *Supersedes the
speech-on-the-CPU plan for Light.* (The processor backend of 2026-10-18 uses an int8 model built
for the CPU instead.)

**Light's cleanup model (E2B) is already lean:** 1,342 MiB of weights on the GPU (its 2,152 MiB of
per-layer embeddings stay in system RAM), 72 MiB cache, 122 MiB compute buffers, about 1.77 GB
with its CUDA context. **The speech model's memory was mostly the pool, not the weights.** Peak over loading plus 240
utterances (60 eval clips including the 10 longest, up to 24.8 s, 4 shuffled passes), each run
twice (within 25 MiB):

| Speech build, settings | GPU memory | Encoder p50 |
|---|---|---|
| float16, as it was | 2,326 MiB | 22.6 ms |
| float16, weights outside the pool, no memory pattern | **1,960 MiB** | 22.7 ms |
| 8-bit weights, as it was | 2,315 MiB | 28.6 ms |
| 8-bit weights, no memory pattern | **1,295 MiB** | 26.9 ms |
| 8-bit weights, weights outside the pool, no memory pattern | 1,410 MiB | 26.1 ms |
| 8-bit weights, pool grown by exact sizes | 4,067 MiB | 26.9 ms |
| 8-bit weights, cuDNN kept to a small workspace | 2,322 MiB | 27.7 ms |

- **Memory-pattern planning** keeps a plan per input length, and every utterance is a new length;
  off, the 8-bit build stays flat at its load size. **Weights outside the pool**
  (`session.use_device_allocator_for_initializers`) helps float16, whose 1.2 GB the power-of-two
  pool rounds up to 2 GB; 8-bit weights do better inside, leaving room the activations use.
  **Exact-size growth fragments**, as on 2026-10-09. Every setting: 240 of 240 transcripts identical.
- **What is left** of the 8-bit build's 1.3 GB: 650 MB of weights and about 450 MB of CUDA context
  and cuDNN/cuBLAS kernels, which every CUDA process pays (lazy module loading already on; eager
  takes 2.0 GB). One process with one context for speech and cleanup is the next saving (not
  built: llama-server is still a child process).
- **The 4-bit encoder** would save another 300 MB but costs accuracy (+0.08 WER points, worse in
  90% of resamples): out.

**Then both models on the 8-bit speech build** (Tai: cut memory as far as quality allows, small
latency costs are fine), **and cleanup prompts processed 128 tokens at a time** (`-ub 128`): the
compute buffer shrinks 109 -> 27 MiB (Quality) and 122 -> 30 MiB (Light), outputs identical (200
real cases at -ub 256, 128 and 64 all equal 512), cleanup time the same (Quality 111 -> 114 ms
p50, Light 81 -> 82 ms). 64 saves only 11 MB more and would slow long prompts.

End to end (120 clips of AMI, VoxPopuli and Earnings-22 in real time; peak GPU memory of Utter and
its llama-server together):

| | GPU memory | Key up to text, p50 / p95 |
|---|---|---|
| Quality, as it was | 5,306 MiB (5.6 GB) | 192 / 396 ms |
| **Quality, 8-bit speech, -ub 128** | **4,572 MiB (4.8 GB)** | 209 / 406 ms |
| Light, all on the GPU | 3,153 MiB | 170 / 337 ms |
| **Light, -ub 128** | **3,085 MiB (3.2 GB)** | 167 / 323 ms |

The context stays 4,096 tokens: its whole cache is 72 MiB for Light (flash attention keeps memory
linear; only 3 full and 12 sliding-window layers keep a cache), so halving it would save 36 MB. A
dictation too long for it is typed raw, never lost. **Decision:** 8-bit speech on CUDA for both
tiers, memory pattern off, `-ub 128`; `gpu_need` 4.8 GB and 3.25 GB (still the code's values).

#### 2026-10-09: a 5 GB footprint (tiers)

*Superseded: the tiers became Quality (fine-tuned E4B) and Light (fine-tuned E2B) on 2026-10-11,
both on the 8-bit encoder.* Tai: the quality setup (about 21 GB of GPU memory, 18 GB to download) is far too heavy; aim for
about 5 GB, the heavy model optional. `nvidia-smi`, above the idle desktop (2.8 GB), each model
under load:

| Piece | GPU memory | Notes |
|---|---|---|
| Speech model, encoder float32 | 2.3 GB | 2.4 GB weights file |
| **Speech model, encoder float16** | **2.3 GB** | 1.2 GB file; WER the same as the reference (-0.02 points, noise); encoder 18-21 ms p50, a little faster |
| Speech model, encoder weights 8-bit (MatMulNBits) | 1.9 GB | WER the same; encoder about 26 ms, 6-7 ms slower (weights unpacked every pass) |
| Speech model, encoder weights 4-bit | 1.6 GB | +0.08 WER points (not significant, worse in 90% of resamples) |
| **Gemma 4 E4B (llama-server, 4096 context)** | **3.3 GB** | its 2.7 GB of per-layer embeddings stay in system RAM (llama.cpp's default) |
| Gemma 4 E4B, 2048 context | 3.2 GB | no latency change; Tai kept 4096 |
| Gemma 4 26B-A4B | about 15 GB | |

The speech model's footprint does not depend on clip length (2.3 GB for 3 s and 25 s clips):
about 1.1 GB above its weights is fixed per-process cost (CUDA context, cuBLAS, cuDNN). Tried and
dropped then: the pool growing by exact requests (3.1 GB: blocks of every new length fragment)
and weights outside the pool (2.4 GB; it helped float16 once memory-pattern planning was off,
2026-10-11).

Tai chose the float16 encoder over 8-bit weights: 0.4 GB more for about 7 ms less. *Reversed on
2026-10-11.*

**Decision then:** light tier (default) = float16 encoder + prompted Gemma 4 E4B, about 5.6 GB, 97%
of the cleanup checks, 124 ms p50 cleanup on real speech, loads in 2.5 s; quality tier, optional =
Gemma 4 26B-A4B (about 15 GB more, 99%, 137 ms). Components `parakeet-ultra-onnx-fp16`,
`gemma4-e4b-qat`, `gemma4-26b-a4b-qat` (optional); all three remain in `components.json`, the last
two optional and used only by the eval tools.
