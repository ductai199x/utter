# Cleanup

The cleanup model turns what the speech model heard into what the speaker meant to type. This
file is the reference for it: the rules, how a dictation reaches it in the app, the models and
how they are trained, how they are evaluated, the use cases, and what is next. True for Utter
0.1.11 (2026-10-20). Numbers live in [evals.md](evals.md); this file states only the current
headline figures, with their dates.

A change of cleanup behavior changes section 1 first, then the data, then the model.

## 1. What cleanup does today

Cleanup keeps the speaker's words. It is not a rewriter: changing tone is a Style (DESIGN.md
section 9, not built), not cleanup. The rules below are what the training data encodes
(`tools/utter_tools/data/`) and what the eval grades. The fine-tuned models get no rules in
their prompt: [cleanup-ft.md](../crates/utter-llm/prompts/cleanup-ft.md) is one sentence, and
the rules are in the weights.

### It does

| # | Rule | Example (said -> typed) |
|---|---|---|
| 1 | Removes fillers: um, uh, er, ah, hmm; and "like", "you know", "I mean", "sort of", "kind of", "so", "well" when they carry no meaning | "I was, like, really surprised, you know" -> "I was really surprised." "I like it" keeps "like". |
| 2 | Applies the speaker's corrections, the cue and what it replaces both dropped. Cues: "no", "no wait", "sorry", "I mean", "actually", "or rather", "wait", "never mind", or a bare restatement | "at 2, no, 3" -> "at 3"; "the parser, never mind, it's in the lexer" -> "it's in the lexer" |
| 3 | Retraction commands: "scratch that" and "delete that" drop the last sentence; "start over", "never mind, let's start over", "let's try that from the beginning" drop everything said before them | "I'll be there around noon. Scratch that, I'll be there at one." -> "I'll be there at 1." |
| 4 | Applies far-back edits by reference, a sentence or more later | "...one box of eggs... actually, change the eggs to three boxes" -> "three boxes of eggs" |
| 5 | Removes stutters, repeats and false starts | "the the", "I w- I want" -> "I want" |
| 6 | Formats spoken lists (on by default; the `lists` setting turns it off). Items listed -> bullets ("- "); steps or an order ("first..., second..., then...") -> numbered ("1. "). An intro sentence stays before the list. Two items, or a series inside a sentence, stay prose. With lists off, the same content stays inline prose | "shopping list eggs milk bread" -> "Shopping list:" + three "- " lines |
| 7 | Follows spoken formatting: "new line", "new paragraph"; "comma", "period", "full stop", "question mark", "exclamation point", "colon" said as commands; "quote ... unquote", "open paren ... close paren" | "Dear team comma new line I wanted to give a quick update period" -> "Dear team,\nI wanted to give a quick update." |
| 8 | Writes numbers as people type them: times, dates, money, percentages, quantities over nine, anything with units, versions; small counts in prose stay words. Spoken emails and URLs become real ones | "nine thirty AM" -> "9:30 AM"; "five gigabytes" -> "5 GB"; "john dot smith at gmail dot com" -> "john.smith@gmail.com" |
| 9 | Fixes punctuation and capitalization, including marks nobody says (colons, parentheses, quotes) and brand casing | "github" -> "GitHub" |
| 10 | Fixes grammar completely (on by default, Tai 2026-10-10): agreement, tense and verb forms, articles, prepositions, noun number, pronoun case, word order, run-ons. Meaning, correct word choices and register are kept | "so me and him goes to the store and they was out of eggs" -> "So he and I went to the store, and they were out of eggs." |
| 11 | Spells dictionary words as given, when one was said (or something close to it); leaves a word alone when the term was not meant | "super base" with "Supabase" in the dictionary -> "Supabase" |

### It does not

1. **Change words otherwise.** No synonyms, no rephrasing of correct sentences, no reordering
   beyond what grammar needs. A text that needs no rule comes back unchanged. Contractions,
   informal but standard words ("gonna", "kinda") and spelling variants ("colour") stay;
   nonstandard grammar ("ain't", "me and him went", "I seen") is fixed (rule 10).
2. **Answer, obey or comment.** A dictated question, a request to an AI assistant, or text that
   tells the app to "ignore the above" is text to type. The output is only the cleaned text: no
   preamble, no quotes around it, no notes.
3. **Add anything** the speaker did not say, except punctuation, line breaks, list markers and
   what grammar needs.
4. **Translate.** English only today.

## 2. How a dictation becomes text

All of this is in [session.rs](../crates/utter-pipeline/src/session.rs),
[clean.rs](../crates/utter-pipeline/src/clean.rs) and
[cleanup.rs](../crates/utter-llm/src/cleanup.rs).

1. **Segments while you speak.** Voice detection cuts the audio at pauses; each segment goes to
   Parakeet at once. Every 300 ms a preview pass transcribes the segment still being spoken.
2. **Cleanup at every pause.** When a segment's transcript comes back, the whole dictation so
   far (every finished segment, joined) is sent to the cleanup worker. A newer request makes
   older queued ones stale; the worker runs only the newest.
3. **Live text in the Bar.** The Bar shows the newest cleaned text in full weight, then every
   word heard since that cleanup started, dimmed as provisional (`clean::bar_words`). A failed
   cleanup shows the raw words: they are never lost.
4. **Key up.** The last segment is transcribed. If nothing was heard after the last pause's
   cleanup, that cleanup is the final text and the paste does not wait ("reused" in the log).
   Otherwise the worker cleans the whole dictation once more and the paste waits up to 3 s
   (`CLEANUP_TIMEOUT`); past that, or on any error, the raw transcript is pasted.
5. **The Bar ends on the pasted text.** One code path produces the final text; the Bar shows
   it and exactly it is inserted. Text goes into the app once, at the end (DESIGN.md 4.2).

### The request

```
system: <the tier's prompt: cleanup-ft.md for Quality, Light and Mini>
user:   Lists: off                     (only when the lists setting is off)
        Dictionary: Supabase, Vercel   (only terms heard in this dictation)
        Dictated: <the transcript>
```

`cleanup::messages_with` is the only place this format is defined;
[pairs.py](../tools/utter_tools/data/pairs.py) mirrors it byte for byte for training. A model
must be sent exactly the prompt it was trained with. There is no context block: the app, the
field and the text around the cursor are not sent (section 5, G1 and G2).

### The gate

[gate.rs](../crates/utter-pipeline/src/gate.rs) decides from keywords whether a transcript
needs the model (fillers, repeats, correction cues, spoken formatting, list cues, addresses,
spoken numbers, dictionary terms heard). **The app cleans every dictation** (`always_clean`,
default true): with cleanup done while you speak, the model's cost is mostly hidden. The gate's
reason is still logged for every dictation, and `utter-cli e2e-bench --gated` runs with it.

### Guards

| Guard | What it catches | What happens |
|---|---|---|
| Reply limit (`max_tokens`) | A reply longer than half the input's characters in tokens (at least 64): the model is answering or looping | The reply is cut off and the retry guard runs |
| Retry guard (`cleanup::chat`, shipped 0.1.7) | A cut-off reply, usually a bullet list opened on plain prose, or a loop | One second try with a grammar that forbids a leading list marker and a mild repeat penalty (1.15 over the last 32 tokens); if that also runs away, the raw text goes in |
| Strayed guard (`cleanup::strayed`) | More than 70% of the output's content words (4 or more) were never said: the model answered, obeyed or invented | The raw transcript goes in, logged as an error. Calibrated on 14,382 outputs (2026-10-11): no false alarm on English, every sample reply caught |
| `tidy_output` | A leading "Cleaned:" or quotes around the whole reply | Removed |
| Server health | llama-server exited (crash, driver reset) | Restarted with backoff (1, 5, 30, 60 s); meanwhile every request fails at once and the raw text goes in |

### Dictionary terms

A term reaches the model only when it was heard: letter for letter (case aside), or within a
small edit distance as one to three joined words, for terms of five letters or more
(`gate::relevant_terms`; "super base" -> "Supabase"). Sending the whole dictionary every time
cost prefill time and invited the model to write terms nobody said. The same dictionary is
boosted in the recognizer (phrase boosting in the TDT decoder), so most terms come out right
before cleanup sees them.

### Server settings

`--swa-full`, `-ub 128`, n-gram drafting from the prompt (3-token match, 6-token draft),
context 4,096 tokens. Drafting works because cleanup copies most of its input (about 60% of
drafted tokens accepted). The reasons and measurements are in the `SERVER_ARGS` comment and
[evals.md](evals.md).

**Long dictations.** Every request carries the whole dictation, and the reply is as long as it.
Nothing windows it, so cleanup time grows with length, and a dictation whose request and reply
pass the 4,096-token context fails to the raw text. Section 6 is the fix.

## 3. The models

| Tier (UI name) | File | Base | Training | GPU memory |
|---|---|---|---|---|
| Quality (default) | `utter-quality-v2-q4_0.gguf` | Gemma 4 E4B | LoRA | about 3.3 GB; 4.8 GB with the speech model |
| Light | `utter-light-v2-q4_0.gguf` | Gemma 4 E2B | LoRA | about 1.8 GB; 3.2 GB with the speech model |
| Mini ("Extra light") | `utter-mini-v1-q4_0.gguf` | Gemma 3 1B | full fine-tune | for computers without a graphics card, and the phone |
| Reference (never offered) | `gemma-4-26B_q4_0-it.gguf` | Gemma 4 26B-A4B | prompted | about 15 GB; eval reference and judge only |

All are Q4_0 GGUF on llama.cpp's server, pinned with SHA-256 in
[components.json](../crates/utter-runtime/components.json) and published at
[huggingface.co/ductai199x/utter](https://huggingface.co/ductai199x/utter) with their adapters.
Bases are Google's unquantized QAT checkpoints, so the result quantizes back to Q4_0 the way the
base was trained to.

**Naming.** Shipped files are `utter-<tier>-v<N>-<quant>.gguf` (Tai, 2026-10-11). Training run
labels (ft1 to ft6) are internal and never appear in a file name. `utter-train` enforces the
name, but its pattern allows only `quality` and `light`: exporting a Mini needs
`export_gguf.py` directly until the pattern includes `mini`.

### Published data

| Repo | What it holds |
|---|---|
| [dictation-to-text](https://huggingface.co/datasets/ductai199x/dictation-to-text) | About 89k pairs (train 79,246 at v2's revision; text test 9,995), each in the exact chat the app sends: system prompt, user message, final text. Split by passage, so no test passage is in train |
| [dictation-speech](https://huggingface.co/datasets/ductai199x/dictation-speech) | The audio behind the pairs (16 kHz FLAC; `corpus*` collections), plus every speech eval set, all test, including Chatterbox voices no training row uses |

Both are CC BY-SA, with each row's source and license. They grow only through `utter-data`
(section 4).

### Training recipe

From [train_lora.py](../tools/envs/train/train_lora.py), whose defaults are the shipped recipe:

| | Quality and Light (v2) | Mini (v1) |
|---|---|---|
| Method | LoRA rank 16, alpha 32, dropout 0.05 | every weight (`--full`), float32 weights, bf16 math |
| Schedule | 1 epoch, lr 2e-4, cosine, 20 warm-up steps | 2 epochs, lr 2e-5 |
| Batch | 8 per GPU, max length 1,024 tokens | same |
| Loss | on the completion's tokens only | same |

Why the 1B is fully trained: a rank-16 LoRA left it its pretrained habit of opening bullet lists
on prose, and its replies ran away (806 cut-offs in 25,469 against 39 with `--full`,
2026-10-14). Training targets are always the dataset's golds; no model's output is a target.

### Retraining

From `tools/` (see [development.md](development.md) for the machine setup):

```
uv run utter-train --name quality-v3 --base e4b --gpus 2 --gguf utter-quality-v3-q4_0.gguf
uv run utter-train --name light-v3 --base e2b --gpus 2 --gguf utter-light-v3-q4_0.gguf
uv run utter-train --merged RUN/merged --gguf utter-quality-v3-q4_0.gguf     # export only
```

`utter-train` writes the dataset's splits at one revision (`--ref`, default main) into
`<data dir>/train/<name>/data/` with `source.json` recording the revision, runs `train_lora.py`
under `torchrun` in `tools/envs/train`, and exports Q4_0 with llama.cpp at the runtime's build,
printing size and SHA-256 for components.json. Mini's recipe needs flags `utter-train` does not
pass:

```
cd tools/envs/train
uv run torchrun --nproc-per-node 1 train_lora.py DATA OUT --eval-data EVAL --base g3-1b --full --epochs 2 --lr 2e-5
```

A new model ships only after `utter-eval` (section 4) against the current one, then through
components.json and a release.

## 4. How it is evaluated

### The one number: ready to send

The share of dictations a careful writer would send exactly as typed (Tai, 2026-10-10). An
output identical to the gold (spacing aside) passes. Every other output goes to a judge, Gemma
4 26B-A4B with [judge.md](../tools/utter_tools/prompts/judge.md): it lists every difference
from the intended text, then grades "acceptable" (only equally valid choices differ, or the
output fixes the gold's own typo) or "needs_edit" with a cause (style, recognition,
punctuation, capitalization, grammar, correction, list, numbers-symbols, words, other). It reads
outputs; it writes none. The rubric was calibrated on Tai's review of 20 verdicts. Exact match
alone was dropped as the headline because it counts an optional comma as a miss. A reply cut
off after the retry counts as a failure.

### Eval sets

`utter-eval` builds every set from the datasets' test splits at one revision:

| Set | What it is |
|---|---|
| `text-test` | The text test split's transcripts (9,995 rows) |
| Realistic, Kokoro voices (950) | What people dictate: everyday messages, emails, notes, AI prompts, plus short real forum and chat posts; each spoken one way |
| Realistic, Chatterbox voices (950) | The same, in cloned voices no training data uses |
| Directed (292) | Questions, AI requests, agent instructions, and text addressed to the app itself: must be typed, never answered |
| Focused sets | main, restate, grammar, lists, far-back edits (amend), each a speech collection |
| Realistic and directed, with a dictionary | Run again with a 300- or 231-term dictionary, boosted in the recognizer and sent to cleanup as the app does |

Speech sets are recognized by `utter-cli bench` (the app's Parakeet), and every model runs
through `utter-cli llm-bench` with the app's prompt and server settings. Every step resumes.

```
cd tools
uv run utter-eval --model quality-v3=PATH.gguf --model quality-v2=PATH.gguf --judge-url URL
```

Without `--judge-url` the judge is the local 26B; it grades a little differently from the vLLM
26B, so only rows graded the same way are compared.

### How golds are made

Every gold is real writing, and every spoken form is built so its gold is known by
construction (`utter-data`, [tools/utter_tools/data/](../tools/utter_tools/data/)):

1. **Passages** (`passages`): paragraphs people typed, from openly licensed corpora (Stack
   Exchange, arXiv abstracts, Ubuntu IRC, Wikipedia), filtered to dictatable prose; plus
   everyday messages and directed text an LLM writes from seeded situations; plus real and
   everyday lists.
2. **Gold fixes** (`fix`): clear typos and grammar errors in the passage are fixed, each
   proposed by an LLM as data and applied by code only where it matches exactly once (grammar
   only if ERRANT confirms it). Otherwise a gold with a grammar error would teach keeping it.
3. **Renditions** (`render`): a script is built in written form. Code inserts what must be
   thrown away (fillers, repeats, a wrong word before a correction cue, a false start before
   "scratch that", a grammar error of a requested ERRANT type, a wrong detail amended later).
   The LLM only proposes data; code places it. An LLM then reads the script aloud, numbers and
   symbols in words; a reading is kept only if its words and digits match the script. Kinds:
   natural, explicit (spoken punctuation), disfluent, correction, command, restate, grammar,
   list, amend.
4. **Speech** (`speak`): Kokoro-82M (twelve voices) or Chatterbox clones.
5. **Pairs** (`pairs`): Parakeet's actual output on the speech, so the model learns the
   recognizer's real errors, with the passage as the gold. A share of list rows is trained with
   "Lists: off" and a prose gold; a share gets a dictionary with true terms, unrelated ones and
   near misses.
6. `build`, `check` (everything the datasets promise), `upload --yes` after a clean check.

### Current numbers

Ready to send %, held-out speech, 26B judge on vLLM (2026-10-12):

| Set | Quality v2 | Light v2 |
|---|---|---|
| Realistic, Kokoro voices | 88.1 | 85.4 |
| Realistic, unseen Chatterbox voices | 80.1 | 75.5 |
| Directed | 87.3 | 82.5 |
| Realistic, with a dictionary | 87.8 | 85.7 |
| Text test split | 70.3 | 64.5 |

Mini v1 (2026-10-14, the same judge on all 25,469 held-out dictations, retry guard on): 79.7
realistic Kokoro, 67.0 unseen voices, 51.9 text test, against 85.7, 74.6 and 64.3 for Light v2
in the same run; 1 reply cut off after the retry. Dictionary terms spelled right: 97.8%
(Quality v1, 2026-10-11; not re-measured for v2). The directed set: no output has ever been a
reply (2026-10-11). Every run, with its setup, is in [evals.md](evals.md).

## 5. Use cases

From the use-case plan (2026-10-10). Built means trained and measured; partly means part of the
case is covered; not built means neither.

| # | Use case | Status |
|---|---|---|
| A1, A2 | Unspoken punctuation, all marks; capitals and brand casing | Built |
| A3 | Grammar fixed completely | Built (grammar kind, gold fixes) |
| A4 | Misheard words fixed from context | Partly: dictionary only; misheard names are the main remaining miss |
| A5 | Paragraphs in long dictations | Not built |
| A6 | Correct text comes back unchanged | Built (natural kind) |
| B1, B2 | Fillers, stutters, false starts; immediate corrections of any scope | Built |
| B3 | Retraction commands with scope | Built |
| B4 | Far-back edits by reference | Built (amend kind) |
| B5 | Thinking aloud, trailing off | Partly |
| B6 | Pauses joined across segments | Built (the whole dictation is cleaned at each pause) |
| C1, H1 | Lists, bullets or numbered; lists off as a setting | Built |
| C2 | Spoken layout commands ("new paragraph", "bullet point") | Partly |
| C3, C4, F4 | Email layout, chat style, very short utterances by app | Not built: deferred to per-app Styles (Tai, 2026-10-10); Utter formats only what was said |
| C5 | Quotes and dialogue | Built (explicit kind) |
| D1, D2, D5 | Numbers, money, units, versions, times, dates, symbols | Built through renditions; NeMo's deterministic written-to-spoken grammars (recipe R5) not built |
| D3, D4 | Phone numbers, addresses, IDs; emails, URLs, paths | Not targeted: few in the data |
| D6, F3 | Spelling out ("Smith with a Y"), emoji by name | Not built |
| D7 | Names and jargon through the dictionary | Built (phrase boosting, relevant terms, dictionary rows) |
| E1, E2 | Identifiers, file names, CLI commands, shortcuts | Not built: needs screen context |
| E3, F2 | Prompts, questions and instructions typed, never answered | Built (directed set, strayed guard) |
| F1 | Command words used as content ("he told me to start over") | Partly |
| F5 | Noise and recognizer hallucinations -> nothing | Not built in the model; voice detection gates silence |
| G1 | Dictating into the middle of a sentence | Partly: the app adds a leading space from the text before the cursor (`spaced`); the model does not see it |
| G2 | App-appropriate output | Not built |
| G3 | Other languages, code-switching | Not built (Vietnamese is planned) |
| H2 | Snippets | Not built |

## 6. What is next

### Cleanup that writes only its edits (not built)

Design of 2026-10-19 (branch `cleanup-diffs`, which holds only the plan and a reverted
attempt). Nothing of it is in the code: the pipeline still sends the whole dictation and takes
the whole text back.

**Problem.** Decoding the reply is nearly all of cleanup's time. Quality reads a 500-word
dictation (about 650 tokens) in 0.13 s and takes 1.6-2.5 s to write it back (3090 Ti, drafting
on, 2026-10-19). Every pause rewrites hundreds of unchanged words. Tai asked for new speech
cleaned almost at once, with anything in the last ~500 words still editable ("change the eggs
to three boxes", "scratch that", a list that becomes a list at its second item).

**Design.**

- Input per request: **Text**, the cleaned document's editable tail (the last ~500 words, from
  a sentence boundary, as the model left it), and **Dictated**, the raw words since the last
  request; plus the dictionary and lists lines.
- Output: zero or more edits, then the new words cleaned. Edits anchor by quoting text, never
  by counting:

  ```
  <edit><from>exact text</from><to>replacement</to></edit>
  <edit><from>first words</from><through>last words</through><to>replacement</to></edit>
  <new>the cleaned new words</new>
  ```

  An anchor matches its last occurrence; the model grows it until unique. Edits come in
  document order and do not overlap. `<new>` is last and may be empty.
- Rust parses strictly, resolves anchors (exact, then ignoring case, whitespace and edge
  punctuation), applies edits back to front, appends `<new>`, and runs the strayed guard on
  `<new>` and every `<to>`. Any failure: the new words go in as heard, with no edits, logged;
  no live retry.
- The tail moves in jumps (past ~500 words, its start jumps ~200 words), so the prompt prefix
  stays cached between jumps. Requests are serialized. Key up is one short request.
- Training: v3 of every tier, built from dictation-to-text by chaining rows into long
  dictations and splitting at sentence ends; the Text before each step is the gold as it read
  then (known by construction); the target is the edit script to the gold after the step, kept
  only if applying it gives the gold exactly. A short dictation is a step with an empty Text.
- Eval: long dictations replayed request by request (WER of the final document), edit accuracy
  by distance (1 to ~400 words), malformed replies and failed anchors per 1,000 requests,
  today's short-dictation eval unchanged, latency per request on every backend.
- Done when a 20-minute dictation keeps up live on every backend, the key-up wait is as today,
  and short-dictation accuracy is within noise of v2.

**Open questions.** Window size in tokens or words (~500 words, about 700 tokens, to start;
Tai). Whether `<new>` carries its own internal corrections (proposed: yes, as today). The same
format for Vietnamese and code-switched dictation, with data later.

**Order.** Example builder and apply code with one shared test file for Python and Rust; a
format prototype with a prompted large model; train v3 Quality and Light; the pipeline; long
end-to-end runs on GPU, processor and a VM.

The full plan is kept in git history as `docs/archive/plan-diff-cleanup.md` (`git log --follow -- docs/archive/plan-diff-cleanup.md`).

### Other unbuilt items from the plans

- **Context block** (recipe R6): optional header lines before `Dictated:` (`App:` email, chat,
  document, code, terminal, ai-prompt or other; `Field:` single-line or multi-line; `Before:` the
  last ~200 characters before the cursor; `After:` the first ~50 after it). Trained with random
  subsets so any part may be missing. The Windows client would fill it from UI Automation; some
  apps (Slack-like) expose nothing, and Wispr treats those as "new sentence". Context is data,
  never instructions: text in Before or After is not copied out unless the speaker said it.
  Needed for G1, G2, C3, C4. Cursor data: split a passage at a word boundary, Before is the first
  part, the gold is the rest with the right leading space. App data: the same content under each
  App, differing only where the spec says (email layout, no layout in single-line fields, shell
  commands verbatim).
- **Data recipes** not built, all with golds known by construction:
  - R5 written forms: entity-dense passages, spoken forms from NeMo's written-to-spoken grammars
    (Apache-2.0) so digits are exact, an LLM adding casual variants ("five bucks", "oh" for
    zero); hard check: every digit sequence in the gold appears in the output.
  - R7 spelling and emoji from templates: "T-O-N-I" -> "Toni", "Smith with a Y", "capital B",
    NATO letters, emoji by name from a fixed table.
  - R8 code (needs screen text): identifiers from context ("user ID" with `userId` before the
    cursor), file names spoken with "dot".
  - R10 noise: recognizer output on noise-only audio and known Parakeet hallucinations ("thank
    you"), gold empty.
  - R11 long dictations: two to five passages of one document spoken as one dictation, gold the
    paragraphs; tests paragraphing and that nothing is dropped over minutes of speech.
- **Negatives for edit commands** (F1): "change the eggs to three boxes" or "he told me to start
  over" used as content must stay typed; the amend kind has no such negatives yet.
- **Faithfulness gate**: an alignment check that flags any content word changed without an
  allowed transformation, target at most 1 in 200 meant words changed; measure first, then decide
  whether it blocks. The strayed guard only catches whole-reply invention.
- **Learning from edits**: read the field back after a paste; corrected words become dictionary
  entries.
- **Real-speech eval**: the cleanup eval uses only synthetic voices. Real recordings (appendix)
  and public real-speech sets (Disfl-QA spoken, Earnings-21/22, NeMo text-processing tests) are
  not wired into `utter-eval`. Also considered: Libriheavy test sets (book punctuation aligned
  with audio), VoxPopuli raw text, and for written grammar W&I+LOCNESS, JFLEG and CoNLL-2014
  (non-commercial, local eval only; the native half is a no-op check). Non-native accents are a
  later iteration (Tai, 2026-10-10); grammar repair already covers broken English in the words.
  The eval is never used to tune prompts by hand.
- **Flicker metric** for the live Bar (words erased per final word): not measured.
- **Pause markers** from word timings as a hint for commas and paragraphs: never tested.

### Tried or considered, and rejected

So these are not proposed again:

- **Prompted general model as the cleanup model:** the fine-tuned E4B beat the prompted 26B on
  every measure (2026-10-10). The 26B stays as eval reference and judge.
- **Keyword gate as the default:** cleaning while the user speaks hides the model's cost; every
  dictation is cleaned. The gate is kept for logging and `--gated` runs.
- **Re-cleaning a 500-word window** (2026-10-19, reverted the same day): it rewrote up to 500
  words on every request, too slow. Replaced by the edit design above.
- **Distillation, training the phone's 1B on Quality's outputs:** rejected by Tai. Targets are
  dataset golds only.
- **Gemma 3 270M for the phone:** 25 points behind Light even fully trained (2026-10-14).
- **LoRA for the 1B:** runaway replies; full fine-tuning instead.
- **Fine-tuning Parakeet:** ruled out (Tai, 2026-10-10). Vocabulary goes through phrase
  boosting and the dictionary.
- **Speak & Improve 2025** (real non-native speech): not used; its non-commercial agreement must
  not hold anything up (Tai, 2026-10-10).
- **Email greetings, sign-offs and chat-style trailing periods:** deferred to per-app Styles.

### History

| Date | Step |
|---|---|
| 2026-10-09 | Prompted models compared on hand-written cases: Gemma 4 26B-A4B first, then Gemma 4 E4B with the rules prompt as the default for a 5 GB footprint, the 26B optional |
| 2026-10-10 | First real dictation fails: Bar and paste disagreed, cleanup graded on hand-written cases. Eval rebuilt from real writing through TTS and Parakeet. ft1 (LoRA on E4B) beats the prompted 26B. ft2 regresses on lists and grammar, which it never saw: the use-case plan follows (every promised behavior gets a rule, an eval kind and training data). ft4 adds grammar, lists, far-back edits |
| 2026-10-11 | "Ready to send" becomes the headline. ft5 (88.8% realistic). ft6 learns the dictionary and ships as Quality v1. Strayed guard calibrated |
| 2026-10-12 | v2 of Quality and Light, trained on the published dataset by one recipe (0.1.2): every shipped model reproducible from public data |
| 2026-10-14 | Mini (Gemma 3 1B, full fine-tune) for the phone and the processor; retry guard (desktop in 0.1.7) |
| 2026-10-19 | Windowed re-clean reverted; edit-only cleanup designed |

Research behind these decisions:
[final text](research/2026-10-10-final-text.md),
[dictation products](research/2026-10-10-dictation-products.md),
[grammar, ITN and editing](research/2026-10-10-grammar-itn-editing.md).

## Appendix: real-test recordings

Real speech is the test the synthetic voices cannot replace. A few recordings are enough when
each covers several cases. They are held out: never trained on. No tool reads them yet.

1. Dictate each one with Utter into a plain text editor, at your normal pace, the way you would
   say it. Restarts and "um"s are welcome.
2. Fix the text until it is exactly what you meant. That is the gold.
3. Save it as `<dictation id>.gold.txt` next to that dictation's log in the data folder's
   `dictations/` (development builds keep the audio).

| # | Situation | Say, in your own words | Covers |
|---|---|---|---|
| 1 | Weekend grocery list | eggs, milk, bread, spinach, coffee filters, a bag of rice; later decide on three boxes of eggs, not one; then drop one item | list, far-back edit, removal |
| 2 | Message to a coworker | the sync moves from Tuesday 3 PM to Thursday 4:30 PM in room 204; get the day wrong once and correct it; the budget is about $12,500 | numbers, times, money, self-correction |
| 3 | Project update in chat | a tool with TF32 runs at about 240 videos a minute, four times faster (as an aside, in parentheses), version 2.1.3 | symbols, versions, parentheses, jargon |
| 4 | How to reset a router | four steps, in order | numbered list |
| 5 | Lunch plans | a sentence, "scratch that", a different one; later "let's start over" and say it again | retraction scope |
| 6 | Status update, about a minute | think out loud over two or three topics | fillers, paragraphs, long dictation, grammar |
| 7 | A prompt for an AI assistant | ask a question and ask for three options | typed, never answered |
| 8 | Contact details | your name spelled letter by letter, an email address, a phone number | spelling, email, phone |
