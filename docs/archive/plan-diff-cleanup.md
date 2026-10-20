# Plan: cleanup that writes only its edits

Status: design, 2026-10-19, branch `cleanup-diffs`. Nothing here is built yet.

## Why

Today the cleanup model takes a dictation and writes all of it back, cleaned. The reply is as
long as the dictation, and decoding the reply is nearly all of the time: on the 3090 Ti, Quality
reads a 500-word dictation (about 650 tokens) in 0.13 s and takes 1.6-2.5 s to write it back
(measured 2026-10-19, n-gram drafting on). That is fine for a 50-word dictation and wrong for a
long one: the model rewrites hundreds of words that did not change, on every pause.

What Tai asked for: the model cleans new speech as it comes, almost instantly, and can still
edit anything in the last ~500 words ("change the eggs to three boxes", "scratch that",
"let's start over", a list that only becomes a list at its second item). The window of text it
can edit slides forward in chunks as the dictation grows.

So the model writes **diffs**: the cleaned new words, plus edits to text it already cleaned, and
nothing else. A reply's length is the size of what changed, not the size of the window.

## The model's job

Input, every request (a pause in speech, and key up):

- **Text**: the cleaned document's editable tail, the last ~500 words, exactly as it stands (the
  model's own earlier output, after its earlier edits). Read-only context unless an edit names it.
- **Dictated**: the new words the recognizer heard since the last request, raw.
- The dictionary and the lists setting, as today.

Output: zero or more edits to the Text, then the cleaned new words.

- New speech that refers to nothing earlier (most requests): no edits, just the new words cleaned.
  The reply is about as long as the new speech, a sentence or two, the same as a short dictation
  today (0.15-0.3 s on CUDA).
- New speech that changes earlier text: one edit per change, then whatever is left of the new
  speech to append (often nothing: "actually, make that Thursday" is all edit).

The model decides what a correction reaches, as today; the code only applies what it says.

## Output format

Constraints: a 2B-4B model must produce it reliably; it is cheap in tokens; it anchors edits by
quoting text, never by counting words or characters (small models cannot count); it can delete
300 words without writing them out; a parser can check every part of it.

```
<edit><from>exact text from the Text</from><to>its replacement</to></edit>
<edit><from>first words</from><through>last words</through><to>replacement</to></edit>
<new>the cleaned new words</new>
```

- `from` alone: replace exactly that span. `from` + `through`: replace the whole range from the
  start of `from` to the end of `through` (a long deletion or rewrite costs a few words of
  anchors, not the range). `to` may be empty (delete).
- Anchors are quoted from the Text as it stands before this reply's edits; edits do not overlap
  and come in document order.
- An anchor matches the **last** occurrence in the Text (corrections are about recent words). The
  model adds words to an anchor until it is unique; training targets are built that way.
- `<new>` is always last and may be empty or absent.
- Tags are chosen because they cannot occur in dictated text after the text pass (spoken "less
  than" becomes words or "<" alone, never `<edit>`); the parser treats only exact tags as tags.

Example. Text: `Buy eggs and milk. Then call Sam about Tuesday.` Dictated: `oh and make that
Thursday not Tuesday and also pick up bread`

```
<edit><from>Tuesday</from><to>Thursday</to></edit>
<new>Also, pick up bread.</new>
```

A list that becomes a list at its second item. Text: `I need to buy eggs.` Dictated:
`second milk third bread`

```
<edit><from>I need to buy eggs.</from><to>I need to buy:
1. Eggs</to></edit>
<new>2. Milk
3. Bread</new>
```

## Parsing and applying (Rust, `utter-pipeline`)

1. Parse the tags strictly. Anything outside tags, an unclosed tag, `through` without `from`:
   the reply is malformed.
2. Resolve each anchor in the Text: exact match first, then a match that ignores case,
   whitespace and punctuation at the edges. No match, or the range runs backwards or overlaps
   another edit: that edit fails.
3. Apply edits back to front, then append `<new>`.
4. Guard (today's `strayed`) on `<new>` and on every `to`: content words that were never said
   mean the model answered or invented; that part is rejected.
5. Any failure: the new words go in as the text pass made them (no edits, never lost), and the
   failure is logged with the reply. A failed request is not retried live; the next pause sends
   the next words against the document as it stands.

Every request and its edits go to the trace, so a run can be replayed and audited.

## The document and the window

- The pipeline holds the dictation's cleaned document. The Bar shows it; the target app gets it
  once, at key up (DESIGN.md 4.2 is unchanged).
- The editable tail is the last ~500 words, starting at a sentence boundary. Older text is final.
- The tail moves forward in chunks, not word by word: when it passes ~500 words, its start jumps
  forward ~200 words to a sentence boundary. Between jumps the prompt (system, dictionary, Text)
  is unchanged except at its end, so the server reuses its cache and reads only the new tokens.
  A jump costs one full read of the tail (0.13 s on CUDA).
- Requests are serialized: request k+1 is built from the document after request k is applied.
  Words that arrive while a request runs wait for the next one (as today, newest wins).
- Key up: one request with the last words. Its wait is a short reply, as today, however long
  the dictation.
- Memory and audio stay bounded the same way as in the reverted work (drain what is handed over,
  keep 10 minutes of audio, the Bar shows the tail); that part is redone on this design.

## Training data

The model must be retrained (v3, both tiers, phone model later) on this task. The examples are
built from `ductai199x/dictation-to-text`, so the dataset stays the standard; the chat format
changes, the rows do not.

A training example is one step of a dictation: (Text, Dictated) -> edits + new.

1. **Chain and split.** Chain rows into long dictations (as `long-bench` did) and split each
   row's dictated text at its sentence ends into the requests a speaker's pauses would make.
2. **The Text before a step** is the gold final text of everything before it, as it read at that
   point: for most rows, the gold final text cut at the aligned sentence boundary. Where a later
   step changes earlier text, the earlier state is known by construction: the amend generator
   places the wrong detail itself (`data/amend.py`), corrections and "scratch that" keep what was
   said before the command, and a list's first item reads as prose until the second arrives.
   No model's output is ever a target.
3. **The target** is the edit script from that Text to the gold final text after the step:
   word-level diff, hunks merged when close, anchors grown until unique, ranges over ~12 words
   written as `from`/`through`. Applying the target to the Text must give the gold exactly; an
   example where it does not is dropped. This check runs on every example.
4. **Mix.** Most steps have no edits (as in real speech). Steps with edits are over-weighted so
   the model learns them; exact proportions are set on the eval.
5. The single-request case (a short dictation) is a step with an empty Text: the reply is just
   `<new>`. One format for short and long dictations.

## Eval

- **Long dictations, end to end:** chained dictations from `text-test` replayed request by
  request through the real pipeline code; WER of the final document against the chained gold.
- **Edits:** the amend, correction, command and list categories split so the change lands in a
  later request than what it changes, at distances from 1 to ~400 words; edit applied correctly,
  wrong edit, missed edit.
- **Format:** malformed replies, failed anchors, guard rejections, per 1,000 requests.
- **Short dictations:** today's cleanup eval, unchanged, so v3 is no worse than v2 on what most
  people do.
- **Latency:** per request p50/p95 and output tokens, CUDA, Vulkan and processor; key-up wait.

Done (DESIGN.md 12): a 20-minute dictation keeps up live on every backend, key-up wait as today,
short-dictation accuracy within noise of v2, edit accuracy and format failure numbers in
docs/eval-results.md.

## Order of work

1. Example builder and the apply/check code (Python for data, Rust for the app, one shared
   test file of Text/reply/expected cases so both parse the same way).
2. Prototype the format with a prompted large model on FLAME to find where the format is hard,
   before spending a training run.
3. Train v3 Quality and Light on FLAME; eval.
4. Pipeline: document, tail, serialized requests, apply, fallbacks, bounded memory.
5. End-to-end long-dictation runs (GPU, processor, VM), then release.

## Open questions

- Window size in tokens or words: ~500 words (~700 tokens) is the starting point (Tai).
- Whether `<new>` should also carry the dictated words' own internal corrections (yes, as today:
  "at 2, no, 3" inside one request needs no edit).
- Vietnamese and code-switched dictations: same format; data comes later with the Vietnamese set.
