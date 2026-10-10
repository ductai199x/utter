# Research: how dictation becomes the text you meant (2026-10-10)

Four research passes, run after Tai's first live test failed: the Bar showed raw words
("parenthesis four ex faster parenthesis") while a different, cleaned text was pasted, and the
cleanup was built from a handful of hand-written examples. Sources are linked inline; [P] is a
primary source (the vendor's own docs, paper, code), [3P] third party, [U] unknown.

## 1. Wispr Flow

**What the user sees**
- [P] Desktop shows no words while speaking: "Desktop dictation inserts the completed text
  after you stop, rather than displaying a live word-by-word transcript"
  ([What is Flow](https://docs.wisprflow.ai/articles/2772472373-what-is-flow)). Deliberate:
  "What you say later can clarify what you said earlier"
  ([interaction challenges](https://wisprflow.ai/post/interaction-challenges-with-voice)).
- [P] The Flow Bar shows only state: resting, recording, processing
  ([Flow Bar](https://docs.wisprflow.ai/articles/1790396454-move-and-dock-the-flow-bar-on-desktop)).
- [P] After the paste: History keeps the raw transcript, "Undo AI edit" restores it, and a diff
  viewer shows what cleanup changed
  ([Auto Cleanup](https://docs.wisprflow.ai/articles/4283510616-auto-cleanup-control-how-much-flow-edits-your-dictation-beta)).
- [P] If formatting fails, the raw transcript is pasted.

**How it works**
- [P/3P] Audio streams to the server while the key is held (gRPC), with context updates
  during the dictation; an LLM call extracts proper nouns from the ASR words mid-stream
  ([teardown](https://wensenwu.com/thoughts/wispr-flow-investigation)).
- [P] Speech recognition: in-house Canto (~2B), "provided with specialized vocabulary from a
  person's dictionary", conditioned on app context, names and terms, trained with RL on user
  corrections ([Canto](https://wisprflow.ai/canto)).
- [P] Cleanup: a **fine-tuned** Llama, not a prompted general model
  ([Baseten](https://www.baseten.co/resources/customers/wispr-flow/)). 50-100 words in 250 ms;
  smaller or quantized models were worse; output must be about the size of the input;
  speculative decoding that assumes the output repeats the input cut latency "by over half"
  ([SE Radio 703](https://se-radio.net/2026/01/se-radio-703-sahaj-garg-on-low-latency-ai/)).
- [P] Budget: 700 ms from end of speech (ASR 200, LLM 200, network 200), measured at p99
  ([technical challenges](https://wisprflow.ai/post/technical-challenges)). Third parties
  measure 1.5-1.8 s in practice.
- [P] Personalization: edits after the paste are captured and learned (dictionary auto-learn;
  RL on the user's style). Writing styles change only punctuation, capitalization and spacing.

**Formatting and corrections**
- [P] Formatting: fillers, grammar, punctuation, capitalization; spoken punctuation names
  (period, comma, colon, semicolon, question mark, em dash), new line/paragraph, lists from
  "one... two...". **Symbols, units, code are not documented**
  ([Smart Formatting & Backtrack](https://docs.wisprflow.ai/articles/5373093536-how-do-i-use-smart-formatting-backtrack)).
- [P] Backtrack: "Flow uses your full dictation as context to decide what to change. Only clear
  self-corrections are removed." "'actually' alone is insufficient: ambiguous corrections,
  unfinished fragments and 'never mind' used as part of the message remain." Scope is decided
  by meaning, not a fixed rule; every documented example replaces a phrase. "Start over" is
  not documented. Editing text already in the field is Command Mode, not Backtrack.
- [P] Context awareness: app and website, text before/after the cursor and the selection,
  on-screen text, names in conversations, variable and file names in IDEs
  ([Context awareness](https://docs.wisprflow.ai/articles/4678293671-feature-context-awareness)).
- [P] Known failures they fixed by A/B tests and benchmarks: models "too conservative on course
  correction" and "too aggressive about changing words" ([changelog](https://wisprflow.ai/whats-new)).

## 2. Other products

- **Mature systems keep forms apart:** spoken (lexical) -> written (ITN) -> display
  (punctuation, casing, disfluency). Azure has four output forms; Speechmatics
  spoken_form/written_form; Amazon Phrase/DisplayAs; Dragon `written\spoken` vocabulary
  ([Azure](https://learn.microsoft.com/en-us/azure/ai-services/speech-service/display-text-format)).
- **Constrained, not free, generation for formatting:** Apple's ITN labels each spoken token
  with edit operations (seq2seq rejected for "particularly egregious and jarring errors",
  [Apple](https://machinelearning.apple.com/research/inverse-text-normal)); NVIDIA's Thutmose
  tags tokens (seq2seq "prone to hallucinations").
- **Command or content:** Talon tags each spoken symbol as command-only, dictation or both:
  long forms ("question mark", "left paren") always work, short ambiguous ones ("dot", "dash",
  "tilde", "x") only in command context ([talonhub/community](https://github.com/talonhub/community));
  pauses: Google needs at least 0.5 s around a command; Dragon treats only pause-delimited
  phrases as commands; escapes: "literal comma" (Windows), "escape ..." (Talon).
- **Vocabulary in layers:** biasing inside the recognizer; written form plus spoken form or
  sounds-like; fuzzy correction after recognition (Handy: Levenshtein plus Soundex); the same
  list in the LLM prompt; learning from the user's edits (VoiceInk AutoLearn).
- **Spoken symbol vocabulary (union of documented lists):** Dragon, Apple, Windows and Talon
  all document open/close paren, bracket, brace, quote; tilde, slash, backslash, underscore,
  pipe, caret, backtick, asterisk, ampersand, at, hash, percent; plus/minus/equals/
  multiplication sign; currency; new line/paragraph; caps/no space modifiers. None documents a
  bare repeated "parenthesis" or "four x faster" -> "4× faster": these need inference.
- **LLM cleanup apps** (VoiceInk, OpenWhispr, Handy, AssemblyAI Dictation, Windows fluid
  dictation): the only family that handles implicit corrections; all keep the raw transcript
  as a fallback.

## 3. Literature

- **Free rewriting is the worst punctuator.** Llama-3.2-1B on IWSLT: prompted rewrite comma F1
  0.457 against 0.752 for a small fine-tuned tagger and 0.840 for scoring each word boundary
  with LoRA ([2606.05179](https://arxiv.org/abs/2606.05179)). AssemblyAI Universal-2-TF: a tagger
  plus span rewriting beats a full seq2seq model on punctuation (PER 29.0 vs 35.0) and ITN, 20x
  faster ([2501.05948](https://arxiv.org/abs/2501.05948)).
- **Punctuation from audio beats text-only:** end-to-end ASR punctuation beats an ASR -> text
  punctuator cascade (LibriSpeech-PC, [2310.02943](https://arxiv.org/abs/2310.02943)); commas are
  the weakest mark everywhere. Parakeet's punctuation accuracy is not published by anyone.
  Pause markers from word timings: weak, mixed evidence; untested with LLMs.
- **Rich punctuation (colon, semicolon, dash, parentheses, quotes) is almost never evaluated.**
  Libriheavy keeps the original book text with full punctuation aligned to 50k h of audio
  ([2309.08105](https://arxiv.org/abs/2309.08105)).
- **Self-repair:** reparandum -> interregnum -> repair (Levelt 1983); in Switchboard, reparanda
  under 7 words cover 96% of edited regions; restarts are almost undetectable by taggers
  (recall 0.09, [2311.00867](https://arxiv.org/abs/2311.00867)). LLM disfluency removal splits
  into over- and under-deleting regimes; fine-tuning lifts a 3B model from the high 60s to over
  90 F1; segmenting input to about 4 sentences helps ([DRES 2509.20321](https://arxiv.org/abs/2509.20321)).
  **No dataset covers the scope of "scratch that" or "start over".**
- **Command vs content:** TERTiUS (Microsoft, ACL 2023, [2307.04008](https://arxiv.org/abs/2307.04008)):
  segmenting dictation from commands, F1 90.9 with T5.
- **Contextual biasing for Parakeet TDT:** NeMo GPU phrase boosting works with greedy TDT; on
  tech talks, phrase F-score 42.5 -> 70.4 greedy, 2-5% overhead
  ([2508.07014](https://arxiv.org/abs/2508.07014),
  [NeMo docs](https://docs.nvidia.com/nemo/speech/latest/asr/asr_customization/word_boosting.html)).
  Keyword lists in an LLM prompt doubled hard-keyword recall in SWAB
  ([2408.09688](https://arxiv.org/abs/2408.09688)).
- **Building eval data:** verbalize real written text, synthesize speech, recognize, score against
  the original. Four-in-One trained this way ([2210.15063](https://arxiv.org/abs/2210.15063)); an
  LLM -> TTS -> label pipeline for numerics worked on real audio
  ([2408.00004](https://arxiv.org/abs/2408.00004)). Metrics: per-mark F1, ITN accuracy,
  key-phrase F with false accepts, over- and under-deletion, preservation of untouched words.

## 4. Live text

- No shipping product shows raw words styled as final and then pastes different words.
  Pattern A: no words, cleaned paste (Wispr, Willow, Typeless, Gboard Rambler). Pattern B:
  provisional text in a distinct style, visibly replaced (Apple volatile results at 40% opacity,
  [WWDC25](https://developer.apple.com/videos/play/wwdc2025/277/); Dragon's preliminary results).
  Pattern C: committed prefix plus an unstable tail.
- Live cleaned text exists: Google AI Edge Eloquent cleans at each pause; Aqua's Realtime Mode
  refines live; open-source per-sentence cleanup cut the wait after release from 1.3 s to 0.16 s
  (fn-flow) and 4.6 s to 1.1 s (undertone); both fall back to a full re-clean when a later
  correction reaches into earlier text.
- Stability: flicker correlates with distraction and fatigue; append freely, change shown text
  only when meaning changes; users preferred stability over accuracy for text already shown
  ([Google CHI 2023](https://research.google/blog/modeling-and-improving-text-stability-in-live-captions/)).
  Re-running on each prefix with masking cut erasure 2.1 -> 0.1 at no quality cost
  ([Google Translate](https://research.google/blog/stabilizing-live-speech-translation-in-google-translate/)).
  Metric: normalized erasure (words erased per final word).
