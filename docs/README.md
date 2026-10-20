# Utter's docs

Which doc answers what. The docs describe Utter 0.1.11 (2026-10-20) and live on the `dev`
branch. [DESIGN.md](../DESIGN.md) is the source of truth for scope, architecture and
decisions; everything here gives the detail behind it.

## Start here

| Doc | The question it answers |
|---|---|
| [../README.md](../README.md) | What is Utter, how do I install and use it? (the public page) |
| [../DESIGN.md](../DESIGN.md) | What is built, what is next, how does a dictation work, which models ship and why, what was decided and on what measurement? |
| [../CLAUDE.md](../CLAUDE.md) | What are the working rules (no slop, latency first, local only, standalone)? |

## Reference

| Doc | The question it answers |
|---|---|
| [cleanup.md](cleanup.md) | What does the cleanup model do and not do, what is it trained on, how is it judged, and what is next for it? |
| [evals.md](evals.md) | How is Utter measured, what are the current numbers, and what did every run show (a dated log, newest first)? |
| [platforms.md](platforms.md) | How does Utter work on Windows, Linux and macOS, and on CUDA, Vulkan, Metal and the processor, and what is the status of each? |
| [development.md](development.md) | How do I set up a dev machine, run the tools and benches, test in VMs and on a Mac, and cut a release? |
| [android.md](android.md) | What was measured on the phone, how is the Android app built, and where is it? |
| [../dev/README.md](../dev/README.md) | How does work on `dev` get published to `main`, and what does the privacy scanner check? |

## UX specs (`ux/`)

One spec per surface: layout, states, motion, copy, keyboard behavior and acceptance checks.
Each opens with its status for the current version.

| Spec | The surface |
|---|---|
| [ux/bar.md](ux/bar.md) | The Bar: the floating pill that shows listening, live text, done and errors |
| [ux/first-run.md](ux/first-run.md) | First run, and the Update, Repair and Problem screens |
| [ux/home.md](ux/home.md) | The Home window: History, Dictionary and Settings on a rail; the tray |
| [ux/history.md](ux/history.md) | History: search, filters, word-by-word changes, retention |
| [ux/insertion.md](ux/insertion.md) | Putting text in: the paste, spacing, confirmation, paste rules, failures |
| [ux/splash.md](ux/splash.md) | The splash card while dictation loads |
| [ux/android.md](ux/android.md) | The Android bubble and pill |

## Research (`research/`)

Dated reference research. Each file is kept as written, with a banner giving its date; facts
in it may have moved on since.

| File | The question it answered |
|---|---|
| [research/model-survey.md](research/model-survey.md) | Which speech and cleanup models and which eval data were candidates (2026-10-09)? |
| [research/2026-10-10-dictation-products.md](research/2026-10-10-dictation-products.md) | What do Wispr Flow, Superwhisper, Dragon and other dictation products do between the transcript and the text? |
| [research/2026-10-10-final-text.md](research/2026-10-10-final-text.md) | How does dictation become the text the speaker meant, in the best systems and the literature? |
| [research/2026-10-10-grammar-itn-editing.md](research/2026-10-10-grammar-itn-editing.md) | What do research and public data offer for grammar correction, written forms (numbers, dates) and voice edits? |

## Archive (`archive/`)

Plans whose work is done or dropped, audits and other material kept for the record. Nothing in
it describes Utter as it is now; the decisions that came out of it are in the decisions log in
[DESIGN.md](../DESIGN.md), section 14.

## How to keep docs current

1. **A doc changes in the same commit as the behavior it describes.** A change to what Utter
   does, its settings, its models or its numbers updates the doc that covers it, and
   `DESIGN.md` when scope or architecture moves. A feature is not done until its docs are
   (DESIGN.md section 12).
2. **Current state first, history after.** Each doc opens with what is true now and the version
   it is true for. Reasons and superseded results go below, dated, or into the archive.
3. **Every number has a date and a setup.** Write the machine, the model, the clip set and the
   tool beside each figure. Do not round, merge or recompute measured numbers; a figure that
   is stale keeps its date rather than being presented as current. New runs go into the dated
   log in [evals.md](evals.md).
4. **Decisions live in one place:** the decisions log in [DESIGN.md](../DESIGN.md), one line
   each with its measurement, oldest first. A decision that is replaced stays, marked
   "superseded by" the date of the one that replaced it.
5. **Check claims against the code.** Name the file or function a behavior lives in, so the next
   reader can verify it; say "not built" rather than describing a plan as if it shipped.
6. **Links are relative** and must resolve; code is linked by repository path.
7. **These files are public.** No paths on the maintainer's machine (write `<repo>`, `~`,
   `%LOCALAPPDATA%\Utter_dev`), no addresses, no emails, no credentials. Run
   `uv run --no-project python dev/release.py scan` before committing.
