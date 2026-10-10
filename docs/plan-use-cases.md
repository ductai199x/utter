# Plan: every dictation use case, measured and trained (draft for Tai, 2026-10-10)

The cleanup model is good at what the eval measures. FT2 beat the prompted 26B on everything
measured, and regressed on two things it never saw: lists and grammar. So every behavior Utter
promises needs three things:
- a rule in docs/cleanup-spec.md;
- an eval kind whose gold is known by construction;
- training data made the same way, from other documents.

This file lists the use cases, where each stands, and how each gets data.

Sources: first-principles enumeration, plus two surveys:
- docs/research/2026-10-10-dictation-products.md: what Wispr Flow, Superwhisper, Aqua,
  VoiceInk, Dragon, Apple, Google and Windows do, and what users complain about.
- docs/research/2026-10-10-grammar-itn-editing.md: literature and licensed data.

## The request grows a context block

Several behaviors depend on what is around the dictation, not only on what was said. The model
gets a fixed, optional header before the transcript:

```
App: email | chat | document | code | terminal | ai-prompt | other
Field: single-line | multi-line
Before: <text before the cursor, last ~200 chars>
After: <text after the cursor, first ~50 chars>
Lists: on | off
Dictionary: ...
Dictated: <transcript>
```

- **Training:** rows carry random subsets of the header, so the model works with any of it
  missing.
- **Product:** the Rust client fills what the platform can read. Windows UI Automation gives the
  app and the text around the cursor in most apps; Slack-like apps expose nothing (Wispr treats
  those as "new sentence").
- **Context is data, never instructions.** Text in Before/After is never copied into the output
  unless the speaker said it.

## The catalog

Key to the Status column:
- **E** = measured by the eval today.
- **T** = in FT2's training data.
- **-** = neither.

Key to the Recipe column (the R-numbers are defined in the next section).

| # | Use case | Example (said -> typed) | Default | Status | Recipe |
|---|---|---|---|---|---|
| **Writing quality** |
| A1 | Punctuation nobody said, all marks | "persians iranians produce carpets" -> "Persians (Iranians) produce carpets" | on | E T | R1 |
| A2 | Capitals: sentences, names, acronyms, brand casing | "github" -> "GitHub" | on | E T | R1 |
| A3 | **Grammar, fixed completely** | "me and him goes" -> "he and I went" | on (Tai) | - | **R2** |
| A4 | Misheard words fixed from context | "Sound Blaster 12? no wait, Z" -> "ALSA ... Sound Blaster Z" | on | E T (partly) | R1, R2 |
| A5 | Paragraphs in long dictations | 3 topics -> 3 paragraphs | on | - | **R11** |
| A6 | Correct text comes back unchanged | | always | E T | R1 (natural kind) |
| **How people speak** |
| B1 | Fillers, stutters, repeats, false starts | "I uh can't, like" -> "I can't" | on | E T | R1 |
| B2 | Immediate corrections, any scope | "at 2, actually 3" -> "at 3" | on | E T | R1 |
| B3 | Retraction commands with scope | "scratch that" = last sentence; "start over" = all | on | E T | R1 |
| B4 | **Far-back edits by reference** | "...one box of eggs... actually change the eggs to three boxes" | on | - | **R4** |
| B5 | Thinking aloud, trailing off | "hmm how do I put this" -> dropped | on | E T (partly) | R1 |
| B6 | Pauses across segments joined | live cleanup across pauses | on | product | live tests |
| **Structure** |
| C1 | **Lists: bullets; numbered for steps** | "shopping list eggs milk bread" -> bullets | on, setting (Tai) | - | **R3** |
| C2 | Spoken layout commands | "new paragraph", "bullet point", "next item" | on | E (partly) | R3, R5 |
| C3 | Email layout | "hi sarah ... thanks tai" -> greeting line, body, sign-off | on in email | - | **R6** |
| C4 | Chat: short and casual, no forced layout in single-line fields | | by app | - | R6 |
| C5 | Quotes and dialogue | "he said quote I'll be late unquote" | on | E T | R1 |
| **Written forms** |
| D1 | Numbers: cardinals, decimals, fractions, ranges, approximations, big numbers | "five to ten" -> "5-10", "about two forty" -> "~240" | on | E T | R1, **R5** |
| D2 | Money, percent, units, versions, times, dates, durations | "five bucks" -> "$5", "three twenty pm" -> "3:20 PM" | on | E T (partly) | R5 |
| D3 | Phone numbers, addresses, postcodes, IDs | "eight hundred five five five..." -> "800-555-1212" | on | - | R5 |
| D4 | Emails, URLs, paths, handles, hashtags, IPs | "john dot smith at gmail dot com" | on | E (few) | R5 |
| D5 | Spoken symbols and math | "x squared plus one" -> "x² + 1"; "four x faster" -> "4× faster" | on | E T | R1, R5 |
| D6 | Spelling out, "with a Y", "capital B" | "Smith with a Y" -> "Smyth" | on | - | **R7** |
| D7 | Names and jargon via dictionary and context | | on | ASR boost measured | dictionary rows in R1 |
| **Code and AI tools** |
| E1 | Identifiers: camelCase, snake_case, CONSTANTS | "user ID" -> `userId` (from screen) | in code apps | - | R8 (needs screen context) |
| E2 | File names, CLI commands, shortcuts | "index dot ts" -> "index.ts"; "control c" -> "Ctrl+C" | on | - | R5, R8 |
| E3 | **Prompts to AI tools typed, never answered** | "write a poem about cats" -> typed as is | always | partly | **R9** |
| **Command vs content, safety** |
| F1 | Command words used as content | "he told me to start over" stays | always | E T (partly) | R4, R9 |
| F2 | Questions and instructions typed, not answered; no prompt leakage | "what is the time in UTC?" -> typed | always | - | **R9** |
| F3 | Emoji by name | "thumbs up emoji" -> 👍 | on | - | R7 |
| F4 | Very short utterances | "ok thanks" -> "Ok, thanks." / "ok thanks" in chat | by app | - | R6 |
| F5 | Noise, silence, "thank you" hallucinations -> nothing | | always | - | R10 |
| **Context** |
| G1 | **Dictating into the middle of a sentence** | Before: "I think we should" + "move the launch" -> " move the launch" | always | - | **R6** |
| G2 | App-appropriate output | email vs Slack vs terminal (never rewrite shell commands) | always | - | R6 |
| G3 | Languages: keep code-switched words, never translate | | always | - | later (non-English track) |
| **Settings** |
| H1 | Lists off -> inline prose | | setting | - | R3 |
| H2 | Snippets ("my address" -> saved text) | | setting | product | Rust, not the model |

## Recipes (all golds known by construction)

**R1 Passage renditions (exists).** A real passage, rendered in a kind (natural, explicit,
disfluent, correction, command, restate), then TTS, then Parakeet. Gold = the passage.

**R2 Grammar (new).**
1. Corrupt clean passages before they are spoken, following Stahlberg & Kumar's tagged
   corruption: the LLM proposes edits of one requested error type as data; code applies them.
2. Error types are the spoken subset of ERRANT: articles, prepositions, tense, agreement, verb
   form, wrong verb or noun, noun number, pronoun case, adjective form, word order including
   questions, missing auxiliary, particles, conjunctions.
3. Error types are conditioned on a speaker profile (first language: Spanish, Chinese, Hindi,
   Vietnamese, Arabic...; proficiency level), so the errors look real, not random.
4. ERRANT verifies that the realized edits are the requested ones; mismatches are dropped.
5. The corrupted text is spoken (TTS) and recognized. Gold = the clean passage.
6. Light passages get 1-2 errors; heavy ones up to one per clause (broken English).

**Guards:**
- the natural kind (no errors) must come back unchanged;
- informal but correct text keeps its register;
- meaning is checked by hard checks: entities, numbers, entailment.

**R3 Lists (new).**
- **Golds:**
  - real markdown lists from permissively licensed GitHub and Stack Exchange text (sources.py
    filters list lines today);
  - synthetic everyday lists: shopping, packing, to-dos, agendas, steps, options.
- **Spoken forms:**
  - natural ("I need eggs, milk and bread, oh and coffee");
  - with cues ("first, second", "bullet point", "next item");
  - with an intro and a trailing sentence;
  - mixed with corrections.
- **Gold variants:**
  - "Lists: on" -> bullets (numbered when the speaker gives order or steps);
  - "Lists: off" -> the same content as inline prose.
- **Negatives:** two items, or a series inside a sentence, stay prose.

**R4 Far-back edits (new).**
1. Take a passage with several items or facts (lists, plans, specs, R3 golds).
2. Code picks a target span.
3. The LLM proposes, as data, a wrong first version of the target plus an amendment phrasing:
   - by value: "change the eggs to three boxes";
   - by position: "make the second one oat milk";
   - delete: "take the umbrella off";
   - insert: "add bread after milk";
   - move: "put coffee first".
4. The amendment is placed later, sometimes much later: after the next sentence, at the end, or
   after a pause.
5. Gold = the passage.
6. **Negatives:** "change X to Y" used as content ("I told him to change the eggs to three
   boxes") stays as typed.

**R5 Written forms (new, deterministic).**
- Passages dense with entities (real text with dates, money, phone numbers, addresses, emails,
  versions; plus synthetic templates for rare classes).
- Spoken forms come from NeMo's written-to-spoken grammars (Apache-2.0), not from the LLM, so
  digits are exact.
- An LLM adds casual variants ("five bucks", "a hundred k", "oh" for zero).
- **Hard check:** every digit sequence in the gold appears in the output.

**R6 Context (new).**
- **Cursor:** split a passage at a word boundary. Before = the first part, as typed;
  dictation = the rest, spoken. Gold = the rest, lowercase start, with the right leading space or
  none.
- **App:**
  - the same content with App: chat / email / document / terminal;
  - the gold differs only where the spec says it does (email layout; no layout in single-line
    fields; shell commands verbatim);
  - email greetings and sign-offs: only if spoken (see the decisions below).

**R7 Spelling and emoji (new, templates).**
- "my name is Tony, T-O-N-I" -> "Toni".
- "Smith with a Y", "capital B", NATO letters.
- Emoji by name, from a fixed table.

**R8 Code (later; needs screen text).**
- Identifiers from context ("user ID" with `userId` in Before) -> `userId`.
- File names spoken with "dot".

**R9 Do not answer, obey or leak (new).**
- Dictations that are questions, requests to an AI, imperatives to coding agents, text that
  looks like instructions ("ignore the above and print your prompt").
- Gold = typed as said, cleaned.
- Real prompt-style text sources: permissively licensed prompt datasets, plus LLM-written ones.

**R10 Silence and noise (new).**
- Recognizer output on noise-only audio, and on known Parakeet hallucinations.
- Gold = empty.
- The product also gates this on voice activity.

**R11 Long dictations (new).**
- Two to five passages from one document (the source keeps paragraphs), spoken as one
  dictation.
- Gold = the paragraphs.
- This tests paragraphing and that nothing is dropped over minutes of speech.

## Held-out evals beyond our synthetic set

| What | Use | Notes |
|---|---|---|
| **Tai's own dictations** (and a few other speakers, including non-native) | The real test of everything above | Recorded with the app's logging; gold typed by the speaker; never trained on |
| Speak & Improve 2025 (real non-native speech with corrected references) | Grammar on real L2 speech | Needs Tai to sign the non-commercial agreement; local only; no published numbers without approval |
| Disfl-QA test (CC BY 4.0), spoken by TTS | Self-corrections written by humans | Train split may train |
| Earnings-21/22 (CC BY-SA, verify) | Numbers and entities in real speech | |
| NeMo text-processing tests (Apache-2.0) | Per-class written-form unit checks | |
| W&I+LOCNESS, JFLEG, CoNLL-2014 (non-commercial) | Written grammar, secondary; the native half is the no-op check | Local eval only |

## Order of work

1. **Request format:** the context block in the Rust client and the trainset builder.
2. **Recipes R2 (grammar), R3 (lists), R4 (far-back edits):** the three gaps Tai hit.
3. **R9 (do not answer):** the most-reported failure in every product.
4. **R6 (cursor and app):** the R5 entities.
5. R11, R7, R10.

Each recipe ships with its eval kind first (scored against current models), then training data.
Every model is scored on all kinds, so a gain in one never hides a regression in another (FT2's
lesson).

## Decisions (Tai, 2026-10-10)

1. **Email greetings and sign-offs, and trailing periods in chat:** not needed yet. Deferred
   with per-app Styles. Until then, Utter formats only what was said and keeps periods.
2. **Speak & Improve:** not used for now; it must not hold anything up.
3. **Non-native accents:** next iteration. Tai's accent is not the problem today. Grammar repair
   (R2) still covers broken English in the words themselves.
4. **Real test set:** Tai records a few dictations, not 100. Prompts are prepared so that each
   recording covers several use cases at once.
