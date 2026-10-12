# Eval results

Every number here was measured on the development machine (RTX 3090 Ti, Windows 11) by
`tools/eval`. Raw outputs are under `%LOCALAPPDATA%\Utter_dev\eval\results\` (see
`docs/local-setup.md`). Newest first.

---

## 2026-10-12: Cold starts

A start right after a reboot (or with the model files long out of the file cache) reads about
7.2 GB from the disk. Reproduced by copying the Quality components with unbuffered writes
(robocopy /J), so nothing of them is cached; launch to ready, two runs each:

| | Cold | Warm |
|---|---|---|
| The loaders read on their own | 7.34 s | 3.78 s |
| One read-ahead stream per component | 6.26-6.59 s | 3.79-4.03 s |
| 64 MB ranges, 8 threads | 5.46-6.28 s | 4.10-4.41 s (it read the cached 7 GB again) |
| PrefetchVirtualMemory on mapped files | 7.27-7.56 s (it ran synchronously, at 1.3 GB/s) | 3.54-3.59 s |
| **8 threads, after timing the model's first 64 MB (cached: read nothing)** | **5.59-6.01 s** | **3.54-3.57 s** |

The disk gives these files under 2 GB/s whatever the method, so a cold start cannot go far
under 4 s here; the rest is the encoder's session (2 s) and the server. A cache probe reads
7.5-9 GB/s, the disk about 1.1 GB/s to one stream: the 4 GB/s line between them is wide.

Not Windows Defender: a model file copied fresh read in 0.11 s the first time, the same as
after, and the starts right after Repair re-downloaded the runtime took 3.3-4.2 s. The one
22 s start seen was the machine busy at that moment (every stage slow at once).

---

## 2026-10-12: Starting in under 4 seconds, and a pipeline that does not starve itself

Measured in the app itself (Quality, a test instance with its own data folder), with
UTTER_TEST_AUDIO feeding a 5.7 s recording through the whole pipeline in real time: the
insertion into Notepad, History, the Bar, the person's clipboard put back. No sound is played.

**Startup, launch to ready** (the splash's wait; it was measured from the pipeline thread's
start before, which hid the launch):

| | Launch to ready |
|---|---|
| Before: everything in turn, development build unoptimized | 7.9 s warm, 13-36 s under load |
| Speech model and cleanup server load at once | 4.7-5.7 s |
| Voice detection and the microphone open meanwhile; the splash window after loading starts | 4.7-5.2 s |
| The small decoder sessions beside the encoder's (one stalled 2.1 s once) | **3.55-3.75 s** (5 runs) |

What is left: the encoder's session takes about 2.0 s (graph optimization at load; lower
optimization levels loaded slower here, 5-7.7 s, and saving the optimized graph failed in
onnxruntime 1.31), and the cleanup server about 2.9 s (vocabulary 1.1-1.5 s, weights 1.3-1.5
s), side by side. Under load from the other, the server's weights once took 8 s: the reason
the CPU threads below matter.

**The CPU threads**: every onnxruntime session ran one thread per core (12) with spin-waiting.
Back to back that won the speech bench (decode 12.5 ms against 20 ms on one thread, the
2026-10-11 section); in the app it starved everything else while the person spoke:

| 3 dictations each | Mic stream gaps | Speech after key up | Cleanup | Key up to text in the app |
|---|---|---|---|---|
| 12 threads, spinning | 26 (audio lost) | 61-79 ms | 169-191 ms | 282-320 ms |
| **2 threads, no spinning** | **0** | **48-71 ms** | **131-158 ms** | **231-260 ms** |

The paste itself is 47-61 ms: 25-30 ms of it is the quiet wait for clipboard managers, about
20 ms is SendInput passing each key through every keyboard hook on the machine.

**The first dictation is as fast as the rest** (260 ms here). In the unoptimized development
build it was not (the paste 667 ms the first time, speech 388 ms), which is why development
builds are now optimized (Cargo.toml): what `npx tauri dev` shows is what ships.

---

## 2026-10-11: Light on the GPU, and the speech model's memory

**Light runs everything on the GPU** (Tai): the speech model on the CPU took 736 ms p50 a clip
(the 8-bit encoder alone 705 ms, Ryzen 9 9900X, all cores), against about 45 ms on the GPU.
Its transcripts were fine (1 of 200 AMI clips differs from full precision); the speed was not.

**Light's cleanup model (E2B) is already lean:** 1,342 MiB of weights on the GPU (its 2,152 MiB
of per-layer embeddings stay in system RAM), a 72 MiB cache, 122 MiB of compute buffers, about
1.77 GB with its CUDA context.

**The speech model's GPU memory was mostly the memory pool, not the weights.** Measured as the
peak over loading plus 240 utterances (60 eval clips, the 10 longest included, up to 24.8 s, in
4 shuffled passes), each run twice (they agree within 25 MiB):

| Speech build, settings | GPU memory | Encoder p50 |
|---|---|---|
| float16, as it was | 2,326 MiB | 22.6 ms |
| float16, weights outside the pool, no memory pattern | **1,960 MiB** | 22.7 ms |
| 8-bit weights, as it was | 2,315 MiB | 28.6 ms |
| 8-bit weights, no memory pattern | **1,295 MiB** | 26.9 ms |
| 8-bit weights, weights outside the pool, no memory pattern | 1,410 MiB | 26.1 ms |
| 8-bit weights, pool grown by exact sizes | 4,067 MiB | 26.9 ms |
| 8-bit weights, cuDNN kept to a small workspace | 2,322 MiB | 27.7 ms |

- **Memory-pattern planning** plans for each new input length and keeps the plan, and every
  utterance has a new length. Off, the 8-bit build stays flat at its load size.
- **Weights outside the pool** (`session.use_device_allocator_for_initializers`) helps float16,
  whose 1.2 GB of weights the power-of-two pool rounds up to 2 GB; the 8-bit build does better
  with its weights in the pool, whose spare room the activations then use.
- **Growing the pool by exact sizes fragments** over many utterances, as on 2026-10-09.
- **Every setting gives the same transcripts:** 240 of 240 identical to the build as it was.
- **What is left** of the 8-bit build's 1.3 GB: 650 MB of weights, and about 450 MB of CUDA
  context and cuDNN/cuBLAS kernels, which every CUDA process pays (lazy module loading is
  already on; eager loading takes 2.0 GB). Running speech and cleanup in one process, with one
  context, is the next saving.
- **The 4-bit encoder** would save another 300 MB but costs accuracy (+0.08 WER points, worse in
  90% of resamples), so it stays out.

**Then both models on the 8-bit speech build** (Tai: cut memory as far as quality allows, small
latency costs are fine), **and cleanup prompts processed 128 tokens at a time** (`-ub 128`):
the compute buffer shrinks from 109 to 27 MiB (Quality) and 122 to 30 MiB (Light), with the
same outputs (200 real cases, at -ub 256, 128 and 64 all identical to 512) and the same cleanup
time (Quality 111 -> 114 ms p50, Light 81 -> 82 ms). 64 saves only 11 MB more and would slow
long prompts.

**End to end** (120 clips of AMI, VoxPopuli and Earnings-22 replayed in real time through the
whole pipeline; GPU memory of Utter and its llama-server together, at the peak):

| | GPU memory | Key up to text, p50 / p95 |
|---|---|---|
| Quality, as it was | 5,306 MiB (5.6 GB) | 192 / 396 ms |
| **Quality, 8-bit speech, -ub 128** | **4,572 MiB (4.8 GB)** | 209 / 406 ms |
| Light, all on the GPU | 3,153 MiB | 170 / 337 ms |
| **Light, -ub 128** | **3,085 MiB (3.2 GB)** | 167 / 323 ms |

The context stays 4,096 tokens: its whole cache is 72 MiB for Light (flash attention keeps
attention's memory linear, and only 3 full and 12 sliding-window layers keep a cache), so
halving it would save 36 MB. A dictation too long for it gets typed raw, never lost.

---

## 2026-10-11: Quality v1 (run ft6), and the dictionary

Shipped as **utter-quality-v1-q4_0.gguf** (huggingface.co/ductai199x/utter). Shipped models
are named `utter-<tier>-v<N>-<quantization>.gguf` (Tai, 2026-10-11); ft5, ft6 and the like are
training runs, named here only.

**Why a new run:** with a dictionary, ft5 got worse, not better (84.3% ready to send on the
realistic set). It sometimes echoed the `Dictionary:` line into the text, and it forced
dictionary words onto words that only sounded a little like them.

**ft6:** ft5's adapter trained one more epoch (lr 1e-4) on 45,660 of ft5's pairs with
`Dictionary:` lines added (tools/eval/finaltext/dictaug.py): some hold the right spelling of a
misheard word, some hold near-miss words that must be left alone. The app now sends only the
terms that sound like something in the dictation (gate::relevant_terms), as the eval does.

All graded by the local 26B (Q4_0, llama.cpp; label v4local). It grades a little differently
from the vLLM 26B the earlier entries used, so only rows graded the same way are compared.

| Ready to send | ft5 | **ft6** |
|---|---|---|
| Realistic, Kokoro voices (950) | 90.4% | 89.3% |
| Realistic, unseen Chatterbox voices (950) | 81.7% | 82.3% |
| Realistic, 300-term dictionary, phrase boost | 84.3% (vLLM grader) | 88.9% |
| Directed, dictionary, phrase boost (292) | | 89.7% |

- **Without a dictionary they are level.** Paired by dictation over both voice sets, ft6 fails
  47 that ft5 passed and passes 42 that ft5 failed: no difference beyond noise (z = 0.5).
- **With a dictionary ft6 is the one to ship.** Dictionary terms come out right 97.8% of the time
  (ft5 96.2%), it never echoes the line (ft5 did 74 times over the two sets), and the score with a dictionary is within half a
  point of the score without one.
- **The six earlier strict sets:** within a point of ft5 everywhere (spans that must come out
  exactly: brackets, numbers, quotes, symbols, terms).
- **Latency is the same.** Back to back on the restate set (575 requests): ft5 252 ms p50, ft6
  257 ms, ft5 again 258 ms. An earlier gap of 20 to 35 ms came from runs at different times.

---

## 2026-10-11: text that must be typed, not answered

**The directed set** (en-finaltext-kokoro-directed, 292 dictations): messages.py `--directed`
writes questions, requests to AI assistants, instructions to coding agents, and text that tells
the voice app itself to ignore what came before, stop transcribing or write a poem (85
"injection" passages). Each one is spoken plainly or with fillers. Cleanup must type every one
of them, never answer or obey it.

ft5, ready to send: **89.4%** (plain 93.2%, fillers 86.2%).
- **It answered none.** Not one output is a reply, and the "output strayed from the dictation"
  check (cleanup.rs) never fired.
- Questions miss 3 of 81, AI requests 4 of 66, agent instructions 7 of 60: misheard names and
  identifiers (Safeway, `years_exp`, L'Oursin), like everywhere else.
- Injection text misses 17 of 85. 8 of those are self-corrections, applied as trained: "write a
  poem about the sprint. Actually, skip that, just draft an itinerary" types only the
  itinerary. That is Utter's correction rule ("actually" replaces what came before), and a
  person dictating it to an AI would usually mean it, so these are the answer key's choice,
  not a model failure. The rest are misheard words.

No training change is needed for this; the stray check stays as the backstop.

---

## 2026-10-11: ready to send, and ft5

**The headline metric is now "ready to send"** (Tai, 2026-10-10): the share of dictations a
careful writer would send exactly as typed. "No edit needed" (exact match) counts an optional
comma as a miss, and so says little about whether the output is good. For each non-exact
output, Gemma 4 26B-A4B (tools/eval/finaltext/judge.py) lists every difference from the
intended text, then grades it:
- **acceptable:** only equally valid choices differ, or the output fixes the answer key's own
  typo;
- **needs an edit:** with a cause.

The rubric was calibrated on Tai's review of 20 verdicts (he found it a bit harsh): punctuation
is the writer's choice unless it garbles or changes the meaning. **Goal: 90%** (Tai, for this
push).

**The realistic eval set** (en-finaltext-kokoro-real, 950 dictations):
- what people dictate: 583 everyday messages, emails, notes, AI prompts and updates
  (messages.py), plus 367 short real forum and chat posts from the eval split;
- each spoken one way: plain, fillers, a self-correction, a command, a later edit, spoken
  punctuation or broken grammar;
- also spoken by Chatterbox voices no training data uses.

The physics abstracts and IRC nicknames of the main set make it a stress test, not the target.

**ft5:**
- ft4's data plus 10,476 more grammar renditions, weighted to the types ft4 missed (articles,
  tense, noun number, prepositions, pronouns);
- plus 12,478 everyday passages, one rendition each: 89,499 pairs;
- one epoch on FLAME's 2 A6000s (the cluster had no 4 free), 3 hours.

Ready to send:

| realistic set | ft4 | **ft5** |
|---|---|---|
| all (950), Kokoro | 83.8% | **88.8%** |
| everyday half (583) | 88.7% | **96.2%** |
| forum and chat half (367) | 75.7% | 76.0% |
| all, unseen Chatterbox voices | 74.7% | **80.7%** |

| by kind (Kokoro) | ft4 | ft5 |
|---|---|---|
| self-correction | 87.3% | 94.1% |
| spoken punctuation | 86.0% | 93.4% |
| far-back edit | 87.9% | 93.0% |
| command | 85.9% | 91.7% |
| plain | 89.3% | 90.0% |
| fillers | 79.1% | 79.7% |
| broken grammar | 67.0% | 79.0% |

Strict scores on every earlier set, each cell no edit needed / characters to fix:

| set | ft4 | ft5 |
|---|---|---|
| main | 18.5 / 3.6 | 18.8 / 3.5 |
| main, Chatterbox | 16.3 / 4.9 | 16.5 / 4.8 |
| restate | 14.5 / 2.6 | 15.9 / 2.4 |
| grammar | 11.8 / 4.1 | 14.5 / 3.5 |
| lists | 45.9 / 3.8 | 47.4 / 3.6 |
| far-back edits | 39.1 / 4.3 | 40.9 / 3.5 |

ft5 regressed nowhere and is the light tier (DESIGN.md decisions).

What is left:
- Forum and chat posts stay at 76%: names, nicknames and product names Parakeet mishears, which
  only the dictionary or context can fix.
- "Me and him went" is not fixed.
- (A leading "Uh, so..." no longer survives on this set: none of ft5's 32 filler misses keep
  one. Those misses are misheard words and garbled restarts.)

---

## 2026-10-10: ft4, grammar, lists and far-back edits

**Training data** (docs/plan-use-cases.md): 66,981 pairs, drawn from these kinds:
- the earlier kinds, plus 28k new passages with one rendition each;
- grammar: mistakes typed by ERRANT category, from speaker profiles;
- lists: real lists from Stack Exchange and GitHub plus everyday ones; 20% trained with
  Lists: off;
- far-back edits: change, remove, insert, position.

Every training gold went through the grammar pass. No eval passage trains (121 clips were
caught by text overlap).

**Training:** one epoch on FLAME's 4 A6000s in 75 minutes. The held-out loss on whole held-out
passages fell steadily, 0.196 → 0.168 (ft2's rose), so one rendition per passage ended the
memorizing.

Each cell gives no edit needed / characters to fix / word error:

| eval set (n) | E4B v3 (product) | 26B v4 | ft2 | **ft4** |
|---|---|---|---|---|
| main, Kokoro (4309) | 7.2 / 18.0 / 14.3 | 11.3 / 7.7 / 6.6 | 16.5 / 4.3 / 3.8 | **18.5 / 3.6 / 3.3** |
| main, unseen Chatterbox voices (4309) | 6.8 / 19.0 / 16.0 | 10.6 / 8.8 / 8.3 | 13.8 / 5.6 / 5.6 | **16.3 / 4.9 / 5.1** |
| restate (572) | 5.4 / 20.1 / 19.0 | 7.5 / 7.6 / 6.1 | 13.6 / 2.8 / 2.9 | **14.5 / 2.6 / 2.7** |
| grammar mistakes (688) | 5.4 / 6.3 / 7.2 | 6.7 / 5.7 / 5.7 | 5.7 / 5.7 / 6.2 | **11.8 / 4.1 / 4.0** |
| lists (897) | 2.7 / 11.4 / 10.7 | 3.2 / 11.7 / 11.0 | 0.1 / 16.7 / 10.0 | **45.9 / 3.8 / 2.5** |
| far-back edits (1268) | 3.2 / 29.2 / 25.9 | 4.8 / 22.3 / 20.0 | 4.1 / 30.1 / 22.6 | **39.1 / 4.3 / 3.4** |

**Cleanup time** p50 / p95, on the GPU it shared with nothing:
- main set: 193 / 496 ms (the product: 196 / 444 ms);
- lists and edits: about 330 / 480 ms, because a list is longer output than prose.

What this says:
- **The new skills were learned without costing the old ones.** On lists and far-back edits
  every prompted model, the 26B included, gets about 1 in 25 right; ft4 gets about 2 in 5. The
  main and unseen-voice sets improved too.
- **Grammar is half done.** Agreement, tense and verb forms are fixed ("I has been... since three
  weeks... don't" -> "I have been... for three weeks... doesn't"). Pronoun case ("me and him
  went") and some prepositions ("until Friday" for "by Friday") are not. In the probe cases,
  ft4 applied every edit and wrote every list.
- **Next:** more and heavier grammar data, with pronoun-case and preposition errors weighted up;
  then the remaining recipes (R9: never answer or obey a dictation; R6: cursor and app context).

---

## 2026-10-10: ft2, and voices it never heard

**ft2:**
- Same recipe as ft1, with 18,183 pairs: the corrected "scratch that" data and 2.7× more.
- Trained data-parallel on FLAME's 4 A6000s in 39 minutes (1.05 s per step, 2.9× one GPU).
- The held-out loss, now on whole held-out passages, rose throughout training (0.185 → 0.36)
  while the training loss fell to 0.03. The model memorizes passages it has seen in six
  renditions. Even so, the real eval improved, so the real eval decides, and ft4 uses one
  rendition per passage.

**Test sets:**
- **Kokoro:** the usual eval, with answer keys now fixed for grammar (goldfix --grammar, 240 of
  892 passages).
- **Chatterbox:** the same dictations spoken by Chatterbox, cloning donated voices that no
  training data uses, recognized by Parakeet.

Each cell gives no edit needed / characters to fix / word error:

| | E4B v3 (product) | 26B v4 | **ft2** |
|---|---|---|---|
| Kokoro (4309) | 7.2 / 18.0 / 14.3% | 11.3 / 7.7 / 6.6% | **16.5 / 4.3 / 3.8%** |
| Chatterbox, unseen voices (4309) | 6.8 / 19.0 / 16.0% | 10.6 / 8.8 / 8.3% | **13.8 / 5.6 / 5.6%** |

On Chatterbox:

| | E4B v3 | 26B v4 | **ft2** |
|---|---|---|---|
| comma F1 | 0.66 | 0.70 | **0.74** |
| parentheses F1 | 0.13 | 0.36 | **0.57** |
| exact brackets / symbols / numbers / terms | 5 / 24 / 66 / 55% | 15 / 44 / 67 / 70% | **32 / 53 / 76 / 72%** |

What this says:
- **The gains are not a Kokoro artifact.** Every system loses a little on Chatterbox, because
  Parakeet itself does worse on those voices (27.6% vs 25.8% characters to fix on its own). The
  ranking and the margins stay.
- **ft2 has gaps the eval did not measure until now:**
  - lists: it writes them inline;
  - grammar: it keeps the speaker's mistakes, where the product's prompt fixes them;
  - far-back edits.

  Tai set both behaviors on by default (docs/cleanup-spec.md, rules 4 and 8), so ft4 trains them
  and the eval gains grammar, list and amend sets (docs/plan-use-cases.md).

---

## 2026-10-10: fine-tuning the cleanup model (ft1)

**Model:** a LoRA on Gemma 4 E4B (Google's unquantized QAT checkpoint), exported to the same
Q4_0 GGUF the product runs. The system prompt is 28 words
(crates/utter-llm/prompts/cleanup-ft.md); the rules are in the weights.

**Training data:**
- 6643 pairs from the final-text train split: real passages, rendered by the 26B in six kinds,
  spoken by Kokoro, recognized by Parakeet.
- No eval passage is in it. The split is by source document.

**Training:**
- On FLAME, one RTX A6000 per run, bf16.
- 2 epochs in 42 minutes, at about 3 s per step of 16 sequences.

**Sweep** (eval loss on 135 held-out pairs; two seeds of one setting differ by 0.0006):

| rank | lr | epochs | eval loss |
|---|---|---|---|
| 16 | 1e-4 | 2 | 0.0287 |
| 16 | 1e-4 | 3 | 0.0231 |
| 32 | 1e-4 | 2 | 0.0229 |
| 16 | **2e-4** | 2 | **0.0190** (seed 1: 0.0196) |
| 32 | 2e-4 | 2 | 0.0189 |
| 16 | 3e-4 | 2 | 0.0205 |

**Final text** (corrected eval, below: 4318 dictations, fixed gold), comparing E4B v3 (the
product), E4B v4, 26B v4, ft1 r16 at lr 2e-4, and ft1 r32 at lr 1e-4:

| | E4B v3 | E4B v4 | 26B v4 | **ft1 r16 2e-4** | ft1 r32 1e-4 |
|---|---|---|---|---|---|
| no edit needed | 7.4% | 8.2% | 11.5% | **14.0%** | 12.7% |
| characters to fix | 17.9% | 12.3% | 7.6% | **6.3%** | 8.8% |
| word error | 14.0% | 10.1% | 6.4% | **4.7%** | 6.2% |
| comma F1 | 0.67 | 0.67 | 0.70 | **0.73** | 0.72 |
| parentheses F1 | 0.21 | 0.37 | 0.40 | **0.56** | 0.54 |
| exact brackets / symbols / terms | 7 / 25 / 56% | 15 / 30 / 62% | 19 / 44 / 69% | **29 / 46 / 72%** | 28 / 45 / 71% |
| restate set: characters to fix | 20.1% | 16.8% | 7.5% | **4.2%** | 5.7% |

What this says:
- **The 4-bit E4B, fine-tuned, beats the 26B prompted** on every headline number. It does so
  at the E4B's size and speed. Latency is to be measured on a quiet GPU; the eval run shared
  the GPU, and tokens per second matched the base model once it had it to itself.
- **Its weak spot was the data's:** "scratch that" and "delete that" (13.9% and 13.6% characters
  to fix, against 8.7% and 5.6% for the 26B). Their training renditions taught the wrong scope
  (below). ft2 trains on the corrected data.

**Eval fixes made along the way** (the baselines above are re-scored on the corrected set):
- **Other languages:** 7 of 901 eval passages (and 27 train) were German, Italian, Dutch,
  French or Finnish IRC lines, read by English voices. sources.py now keeps English only.
- **"Scratch that" scope:** command renditions put a two-sentence false start before "scratch
  that" or "delete that", and the gold dropped both sentences. These commands retract one
  sentence (docs/cleanup-spec.md), and the restate kind used the same cue for one sentence, so
  the data contradicted itself. Their false start is now one sentence, and the 401 eval and
  1268 train renditions were re-rendered.
- **The rendition checks** rejected faithful readings: decades ("the nineteen thirties"),
  versions ("sixteen point ten"), "nineteen oh five", regnal numerals ("the second"). Half of
  the new renditions failed; about a fifth do now. The rest fail for good reasons: dates read
  as words, typos the gold keeps.

---

## 2026-10-10: phrase boosting

Phrase boosting (crates/utter-asr/src/boost.rs) adds a bonus to dictionary phrases' tokens
during Parakeet's greedy decoding. The weights are unchanged.

Method (`utter-cli bench --phrases`, `tools/eval/boost_eval.py`):
- **Dictionary:** 250 terms sampled from the final-text eval golds, as a user's dictionary would
  hold them: words like MySQL, TF32 and Kerr-Newman. Paths, sizes and capitalized everyday words
  are left out. The 115 terms not sampled are a control.
- **Recall:** a dictionary term in the gold is written exactly, case and all.
- **Inserted:** a dictionary term written more often than the gold has it.
- **Harm test:** the same dictionary on VoxPopuli and Earnings-22, where these terms are almost
  never spoken.

| | term recall | inserted | other terms | WER | clips changed |
|---|---|---|---|---|---|
| no boost | 50.1% | 136 | 37.6% | 6.86% | |
| continue +3, depth +1 (default) | 56.2% | 146 | 37.8% | 6.87% | 231 of 4756 |
| continue +5, depth +1 | **59.6%** | 162 | 37.7% | 6.90% | 370 |

| harm test: WER (clips changed) | VoxPopuli (300) | Earnings-22 (300) |
|---|---|---|
| no boost | 3.03% | 9.90% |
| +3 | 3.03% (0) | 9.92% (2) |
| +5 | 3.02% (1) | 10.00% (6), 3 of its own terms lost |

What this says:
- **The first version did harm**, and the reason is general. Adding a bonus to any non-blank
  token is the same as lowering blank, so the decoder wrote words nobody said: "Period.",
  "<unk>", "a" for "uh". A bonus for starting a phrase also pulled common words toward
  dictionary starts ("um" -> "UM"). At +1.5 it changed 1353 clips, cost the control 1.9 points
  and added 48 insertions, for the same recall as the fix at +3.
- **The fix:** whether to emit stays the model's decision, made on the unboosted scores; the
  bonus only chooses among non-blank tokens. Starting a phrase gets no bonus by default, and a
  start bonus of +1 added nothing.
- **Cost:** boosting needs per-frame decoding, about 6-8 ms more decode time at p50 (17 -> 24
  ms on Earnings-22), and only when a dictionary is set. Folding it into the batched graph would
  take a joint call per emitted token, which is close to the same cost.
- **Not wired into dictation yet.** The dictionary UI and the cleanup prompt's dictionary line
  come with it (docs/plan-final-text.md, "Vocabulary").

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
- `utter-cli bench` writes results that `utter-bench` scores, so the Rust
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
- **Significance:** at the time, a paired bootstrap (10,000 resamples) of pooled WER over
  the clips both models transcribed (`asr_bench compare`, since removed).
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
