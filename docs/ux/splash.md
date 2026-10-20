# The splash

What shows while Utter gets ready after a person starts it: the models load, and without it
nothing on screen says anything is happening. Status for 0.1.11: **built**
(`app/src/splash`, `app/src-tauri/src/splash.rs`).

The requirements come from the splash Tai shaped in a sibling project (a chess coaching
companion): the same window behaviour, timing and care. The picture is Utter's own.

## How long loading takes

The card is up for at least 3 s, so a warm start (about 3.6 s) shows it for the whole load.
Measured launch to ready, Quality, RTX 3090 Ti, release-like build (docs/evals.md, 2026-10-12):

| Start | Launch to ready |
|---|---|
| Warm (files in the file cache) | 3.55-3.75 s (5 runs) |
| Cold (after a reboot, about 7.2 GB read from disk) | 5.59-6.01 s |

Other tiers and GPUs are not measured here. The splash exists to cover this wait, not to
excuse a slower one: the target is a start of 5 s or less.

## The idea: the mark, told at the size of the card

Utter's mark is a voice that lays itself down into lines of text (`app/src/lib/mark.ts`). The
splash tells that story across the whole card. Five fine ribbons of light come in from the left
edge the way a voice does, louder and quieter in phrases, crossing and drifting like silk. As
they reach the middle of the card they settle and flatten into the faint ruled lines of a page.
When a phrase arrives at the page, a glowing write-head writes it out on the line as words
(rounded strokes, like the mark's lines, never real letters); in a pause between phrases the
head waits. The ribbon feeding the line being written is a touch brighter. When the page is
full it rests, fades, and the next page begins.

Calm is the rule: slow waves (wavelengths of 118 to 170 px, a phrase travelling at 120 px/s),
one palette (the accent's blues and violets, with light in it), additive light on a deep
near-black blue.

## Window

- A frameless card, 640 x 400 logical pixels, 16 px corners. The window is 680 x 440: 20 px
  around the card for the card's own soft shadow. Transparent window background, no OS shadow
  (an OS shadow leaves a dark frame during the fade). Centered on the screen by Tauri's
  `center()`.
- An ordinary window: in the taskbar and Alt+Tab, never always-on-top (Tai rejected that in the
  sibling project: it sat on top of everything and could not be put behind). Not resizable.
- Dark in both OS themes: a deep near-black blue (`#11152a` to `#0b0e1b` with a lighter
  radial glow) so the points of light read.
- The card fades in over 320 ms (opacity, scale 0.985 to 1).

## What it shows

- Bottom left: the animated mark (`lib/AnimatedMark.svelte`), 48 px with a soft accent glow,
  drawing itself in as the card appears and then living as it does everywhere. "Utter" beside
  it, 24 px semibold, and "Version 0.1.10" under it, small and muted.
- Bottom right: the stages of getting ready as a short row of words, each lighting up as it
  finishes:
  `Speech · Cleanup · Microphone · Ready`
  The one loading pulses softly; finished ones are full brightness; the others are dim. Each
  stage report sends a pulse of light along the ribbons into the page.
- One status line under the row, right-aligned. Before the first report: "Starting...". Then
  "Loading the speech model...", "Loading the cleanup model...", "Opening the microphone...",
  and "Ready. Hold Right Ctrl to dictate." (it names the key the person chose, from
  `dictation_key`). It never shows a raw error.
- The scene has the upper card; the name and the progress share the bottom edge, so nothing
  is drawn behind text.

The core reports each stage (`splash::stage`, called from `Pipeline::start_reporting`). A card
that opens after loading began asks where loading is (`splash_stage`), so a stage reported
early is never missed.

## Timing

- It should appear within 150 ms of launch. In 0.1.10 the core starts loading first and opens
  the card after (`main.rs`: "Loading first: building the card's window takes a moment it need
  not wait"). The time to the first frame is not measured: see the checks.
- It stays at least 3 seconds (a splash that comes and goes in a blink reads as a glitch), and
  until dictation is ready.
- When ready: the status line says "Ready. Hold Right Ctrl to dictate." for 1.2 s (longer if
  the 3 s minimum is not reached yet), then the card fades out over 750 ms (opacity to 0, scale
  to 0.985), and Utter is in the tray. The tray tooltip reads "Utter: hold Right Ctrl to
  dictate".

## When it shows, and when it does not

| Start | Splash |
|---|---|
| The person starts Utter (Start menu, desktop, the installer's last page) and dictation needs loading | yes |
| Utter starts at sign-in (Start with Windows, the `--autostart` flag) | no: tray only, quiet while the person logs in |
| First run, update, repair or runtime download is needed | no: the setup window shows instead (docs/ux/first-run.md) |
| Everything is installed but the person never finished or skipped Try it | no: the setup window opens on Try it alone |
| Dictation cannot start | the problem screen opens first, then the splash closes (same place, no blank moment between) |
| Launched again while running | no: the Home window opens |

## Escape hatches

- Esc, or a click on the card, hides it early (the card fades in 160 ms); loading carries on
  and the tray says when it is ready.
- Closing it does not quit Utter.

## Motion and performance

- The canvas runs at the display's refresh rate, its resolution capped at 2x (the models load
  at the same time, on the same machine).
- The write-head moves at up to 120 px/s, so a page takes 12 to 15 s; a full page rests 1.8 s
  and fades over 1.2 s. A warm start ends before the first page is full.
- With "reduce motion" set: the ribbons are still, the page is written, the mark is drawn, and
  the stage pulses do not run; the progress row and status line work as above. The pulsing dot
  on the loading stage stops.

## Review

`npm run dev` in `app/`, then `splash.html?demo` plays the stages on a timer in a browser
(`?demo&fade` plays the fade too). Tai reviews it there live; after two misses at his taste,
ask plainly what he wants.

## Acceptance checks

"Code" means the behavior is in the source; "unverified" means no test or measurement in the
repo shows it.

| # | Check | Status |
|---|---|---|
| 1 | Starting Utter shows the card within 150 ms; nothing else flashes first | Unverified; the card opens after loading starts, so the budget needs a measurement |
| 2 | The stages light in order as the real loading reports them; the status line follows | Met in code (`stage("speech")`, `"cleanup"`, `"microphone"`, then `"ready"`) |
| 3 | It stays at least 3 s, then fades out smoothly; no dark frame or flicker during the fade | 3 s minimum: met in code (`SHOWN_MS`). Fade quality: unverified |
| 4 | At sign-in it does not appear | Met in code (`started_at_sign_in`) |
| 5 | It never covers a window the person brings to the front | Met in code (not always-on-top). Not taking the focus from another app when it opens: unverified |
| 6 | A start failure turns into the problem screen without a blank moment between | Met in code (problem window first, then the card closes); unverified on screen |
| 7 | Smooth at 100% and 150% scaling; no dropped frames while the models load | Unverified |
