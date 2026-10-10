# Plan: the text you meant, live (draft for Tai's review, 2026-10-10)

Evidence for every decision here is in `docs/research/2026-10-10-final-text.md`.

## What went wrong

Tai's first real dictation failed in two ways:

1. **The Bar showed raw words and a different text was pasted.** Cleanup ran after key up, and
   the Bar never saw it.
2. **Cleanup was built and graded on a few dozen examples I wrote myself.** It scored 97% on
   them and failed the first real sentence. The method was wrong, not just the examples:
   - A general model was prompted to rewrite the whole transcript. The literature measures
     this as the worst way to punctuate.
   - A keyword gate decided when to clean at all.
   - No test came from text I hadn't written.

## The target

Tai speaks naturally, without thinking about punctuation or commands. The Bar shows, as he
speaks, the text he would have typed, and exactly that text is inserted. That means getting
all of these right:
- **Every mark nobody says:** commas, periods, colons, semicolons, question marks, dashes,
  parentheses, quotes, apostrophes, capitals, paragraphs, lists.
- **Spoken symbols and formatting**, in all the ways people say them, including the
  ambiguous ones ("parenthesis ... parenthesis", "four x faster" -> "(4× faster)").
- **Numbers, units, versions and approximations** as people type them ("~240", "7.5
  hours", "72k", "TF32").
- **Names and jargon** spelled right.
- **Corrections:** what was retracted is dropped, and only that. Commands are told apart from
  the same words used as content: "scratch that", "let's start over", "actually".

The measure is what Tai would feel: **the share of dictations that need no edit at all.**
Category metrics (below) show where the misses come from.

## How the best systems do it (summary)

- **Wispr Flow:**
  - fine-tuned cleanup model, not a prompted one;
  - output about the length of the input, decoded speculatively;
  - recognizer biased with the user's vocabulary and on-screen context;
  - the whole dictation as context for corrections;
  - learns from the edits users make after the paste;
  - raw transcript kept for undo and as a fallback;
  - 700 ms p99 budget.
- **Mature dictation systems:**
  - keep spoken, written and display forms apart;
  - do formatting with constrained edits, never free rewriting;
  - tell commands from content by form (long vs short spoken forms), by pauses, and with
    escape words;
  - handle vocabulary in layers: recognizer biasing, a dictionary, fuzzy correction, the
    same list given to the model.
- **Research:**
  - punctuation from the audio beats text-only;
  - fine-tuned or constrained models beat prompted rewriting by a wide margin (comma F1 0.84
    vs 0.46);
  - phrase boosting in the recognizer roughly doubles jargon recall.
- **Live text:**
  - nobody shows raw words styled as final;
  - live cleaned text exists (per pause, per sentence);
  - it has to be stable: change shown text only when the meaning changes, and measure flicker.

## Design

### 1. One document, cleaned as you speak

First version (built 2026-10-10, `crates/utter-pipeline/src/clean.rs`): at every pause the whole
dictation so far is cleaned, so a correction can reach back anywhere; if nothing was heard after
the last pause's cleanup, it is the final text and the paste does not wait for the model. Most
dictations are short, so this is the window below with no settled part. The settled part comes
when long dictations need it (measured by the e2e bench).

While the key is down, Utter keeps one document with three parts:

| Part | What it is | Can it change? |
|---|---|---|
| **Settled** | cleaned text more than a few sentences back | only by an explicit command ("scratch that" reaching it, "let's start over") |
| **Window** | the last few sentences, cleaned | yes: each new segment re-cleans the window, so a correction or a later word can fix it |
| **Tail** | the words being spoken now, raw | replaced by cleaned text when its segment ends |

- **When a segment ends:** at each pause, the cleanup model gets the window's raw text, the
  new segment, the settled text as read-only context, the dictionary and the app. It
  returns the cleaned window. The window is bounded, about 2-4 sentences: the literature's
  best segment size, and enough for corrections, 96% of which reach back fewer than 7 words.
- **Commands with a larger scope** ("let's start over", "scratch that" after a whole
  sentence) come back from the model as explicit operations on the document, not as text.
- **At key up:** only the last segment is new. The same step runs once more, and the result
  is the final text. **The Bar shows it and exactly it is pasted:** one code path, so the two
  cannot disagree.
- **Latency:** key-up latency no longer grows with length: last segment plus one window
  cleanup, whether the dictation is 3 seconds or 3 minutes.
- **Failure:** if cleanup fails or misses its deadline, the raw text is pasted, and the
  History keeps both versions (as Wispr does).
- **No gate:** every dictation is cleaned. The cost moves into the time you are speaking.

### 2. The Bar

- **What it shows:**
  - settled and window text: the real output, in full weight;
  - the tail: dimmed, clearly provisional;
  - changes to shown text animate in place: a short crossfade on the changed words, no jumps.
- **At key up:** the tail turns into its cleaned form, and Done shows over the final text
  that was pasted.
- **Setting:** waveform-only Bar (Wispr's choice) for later.
- **Flicker:** measured as normalized erasure (words erased per final word), and kept low.
  Window text only changes when cleanup actually changes it.

### Look and feel (as important as accuracy)

The Bar and the app must look elegant and feel calm. That is a requirement with its own
process, not a polish pass at the end.

- **References first:** study how the best-regarded desktop overlays look and move (Wispr's
  Flow Bar, Apple's dictation and Dynamic Island, Raycast, Arc), the same way the pipeline
  was researched.
- **Iterate visually:** every Bar change is reviewed in the demo page (`bar.html?demo`) and
  in the real app, from screenshots and recordings:
  - over white and black, at 100% and 150% scaling;
  - provisional words, corrections cross-fading, wrapping, Done, errors.
- **Motion:** every transition uses the motion tokens and is checked frame by frame (no jump,
  no reflow, no flicker).
- **Gate:** the acceptance checks in `docs/ux/bar.md`, plus Tai's eye: visuals go to Tai as
  screenshots and recordings before they are called done.

### 3. The cleanup model

- **Who owns what:**
  - The recognizer (Parakeet) owns the words and the punctuation it hears.
  - The cleanup model owns everything a recognizer can't know:
    - structure (colons, lists, parentheses, paragraphs);
    - symbols and spoken formatting;
    - numbers and units as typed;
    - corrections and commands;
    - fixes to punctuation the recognizer got wrong.
- **Output:** the text, about as long as its input, so n-gram drafting stays effective (it
  already is: 60% of drafted tokens accepted).
- **Guard:** an alignment check flags any change to a content word that no allowed
  transformation explains. We measure it first, then decide whether it blocks.
- **How we get there, decided by measurements (step 2 of the work):**
  1. **Baselines:** Parakeet alone; the current prompt; a rewritten prompt that covers every
     category; Gemma 26B (quality tier) as a reference ceiling.
  2. **Fine-tune.** What Wispr does and what the research supports. Train a small model on
     pairs from the same pipeline as the eval, built from different source text, so the eval
     stays held out.
     - Base: Gemma 4 E4B, and others if they measure better.
     - Method: LoRA on this machine, then 4-bit for llama.cpp.
     - Inputs: Parakeet's actual output on synthesized speech, so the model learns the
       recognizer's real errors and habits.
  3. **Pause markers:** test whether marking long pauses (from word timings) helps commas
     and paragraphs. Keep them only if they measure better.
- **Deterministic rules** only where they are exact (a spoken "new paragraph" at a pause, the
  user's own snippets), never as a substitute for the model's judgment.

### 4. Vocabulary

Parakeet is never fine-tuned (Tai, 2026-10-10); only the cleanup model is. Recognition improves
without touching its weights:

1. **Phrase boosting in the recognizer:** port NeMo's boosting tree into our greedy TDT
   decoder. Dictionary words then come out right in the first place.
2. **Dictionary:** written form plus optional sounds-like, given to the cleanup model too.
3. **Learning from edits:** after a paste, read the field back; words Tai corrects become
   dictionary entries (as Wispr and VoiceInk do).
4. **Context:** names and terms on screen and in the text around the cursor (phase 2 of the
   product, already planned).

### 5. Logging

Every dictation writes one record to `C:\Users\Tai\.cache\utter\dictations\`:
- the audio (setting, on by default in development);
- every recognizer pass;
- every Bar state with its time;
- every cleanup input and output;
- the final text, the target app, and every stage's time.

The log stays on this machine. Tai can mark any dictation as a test case; nothing becomes
one unless he does.

## How it is tested

### The eval set

- **Ground truth is real writing, not my examples.** Text people actually typed, across:
  - technical notes and docs (GitHub READMEs, Stack Exchange, arXiv);
  - chat and email style;
  - prose.

  Licenses are checked per source and recorded.
- **Spoken forms, several per passage:**
  1. Natural speech with no punctuation said.
  2. Symbols and formatting spoken in the documented ways and the ambiguous ones.
  3. Numbers read the way people read them.
  4. Disfluencies, corrections of every scope (word -> sentence -> "start over"), and
     commands, with the gold result known by construction.
  5. Commands used as content ("write 'scratch that' in quotes", dialogue).
- **Audio:** a local TTS model with several voices, then Parakeet. So every test goes through
  the real recognizer, and Parakeet's own punctuation is measured. Nobody has published it.
- **Real speech** for what TTS can't fake:
  - Libriheavy test sets (full book punctuation aligned with audio);
  - Earnings-22 (formatting, jargon);
  - VoxPopuli raw text;
  - Tai's own dictations he marks.
- **Held out:** training data is built from different source text, and the eval set is never
  used to tune prompts by hand.
- **Real dictations:** Tai's sentence goes into the real-dictation set like any other.
  Nothing is tuned to it.

### Measured on every change

| Category | Metric |
|---|---|
| Whole dictation | share needing no edit; characters to fix |
| Punctuation not spoken | precision, recall, F1 per mark (. , ? ! : ; — ( ) " ' …), capitals, paragraphs |
| Spoken symbols and formatting | span accuracy |
| Numbers, units, versions | span accuracy |
| Names and jargon | recall, and false insertions, with and without the dictionary |
| Corrections | dropped correctly; scope right; over-deletion; under-deletion |
| Commands vs content | executed when meant; kept when literal |
| Faithfulness | words the speaker meant that were changed or added (hallucination) |
| Live | Bar final = pasted (must be 100%); flicker; time to clean each segment |
| Latency | key up -> text p50 and p99, by dictation length |

### Gates before anything goes back to Tai

- Bar final text equals the pasted text in 100% of dictations.
- Key up -> text: p50 at most 150 ms, p99 at most 400 ms, at any dictation length.
- Faithfulness: at most 1 in 200 meant words changed.
- Every category above clearly better than the baseline, with the no-edit rate on held-out
  technical writing reported honestly. The bar for each is set from the baselines in step 1,
  and written down before step 2 starts.

## Order of work

1. **Logging, and the Bar ends on the pasted text** (small; removes the mismatch Tai hit,
   and every later step uses the log).
2. **The eval set and baselines.** A report: where Parakeet, the current cleanup and the
   26B model actually stand, per category.
3. **The cleanup model:** prompt and format experiments, then the fine-tune, chosen by the
   eval.
4. **Cleaning as you speak, and the live Bar.**
5. **Vocabulary:** phrase boosting, dictionary, learning from edits.
6. **Back to Tai,** once the gates pass, with the report.

## Needs Tai's OK

- **Downloads:**
  - a local TTS model (chosen by a live survey);
  - corpora for the eval and training sets (tens of GB on C:);
  - the Python TTS and normalization tools (dev only, never shipped).
- **Fine-tuning on this machine:** several hours of GPU time per run.
- **"Let's start over" default:** clear everything before it when it stands alone at a
  pause; keep it as text when it's part of a sentence. (Wispr documents no behaviour here.)
