# The splash

What shows while Utter gets ready after a person starts it: the models load for 10 to 20
seconds, and without it nothing on screen says anything is happening. Status: **built** (v2,
app/src/splash/scene.ts).

The requirements come from the splash Tai shaped in the chess coaching companion
(the sibling project, desktop/src/routes/splash): the same window behaviour, timing and
care. The picture is Utter's own.

## The idea: the mark, told at the size of the card

Utter's mark is a voice that lays itself down into lines of text (app/src/lib/mark.ts). The
splash tells that story across the whole card. Five fine ribbons of light come in from the left
edge the way a voice does, louder and quieter in phrases, crossing and drifting like silk. As
they reach the middle of the card they settle and flatten into the faint ruled lines of a page.
When a phrase arrives at the page, a glowing write-head writes it out on the line as words
(rounded strokes, like the mark's lines, never real letters); in a pause between phrases the
head waits. The ribbon feeding the line being written is a touch brighter. When the page is
full it rests, fades, and the next page begins.

Calm is the rule: slow waves (wavelengths of 120 to 170 px, a phrase travelling at 120 px/s),
one palette (the accent's blues and violets, with light in it), additive light on a deep
near-black blue.

## Window

- A frameless card, 640 x 400 logical pixels, 16 px corners, centered on the monitor with the
  cursor. Transparent window background, no OS shadow (the card draws its own soft shadow;
  an OS shadow leaves a dark frame during the fade).
- An ordinary window: in the taskbar and Alt+Tab, never always-on-top (Tai rejected that in the
  chess app: "it's basically on top of everything else and I can't bring something else in
  front"). Not resizable. It does not take the focus from what the person is doing if they
  have moved on.
- Dark in both OS themes: a deep near-black blue (#0d1020) with the accent's light in it, so the
  points of light read.

## What it shows

- Bottom left: the animated mark (lib/AnimatedMark.svelte), 48 px with a soft accent glow,
  drawing itself in as the card appears and then living as it does everywhere. "Utter" beside
  it, 24 px semibold, and the version under it, small and muted.
- Bottom right: the progress, the stages of getting ready as a short row of words, each
  lighting up as it finishes:
  `Speech · Cleanup · Microphone · Ready`
  The one loading pulses softly; finished ones are full brightness; the others are dim. Each
  finish sends a pulse of light along the ribbons into the page, and the ruled lines brighten
  as it arrives.
- One status line under the row, right-aligned: "Loading the speech model...", then the next stage, then
  "Ready. Hold Right Ctrl to dictate." It never shows a raw error.
- The scene has the upper card; the name and the progress share the bottom edge, so nothing
  is drawn behind text.

## Timing

- It appears within 150 ms of launch: before the models start loading.
- It stays at least 3 seconds (a splash that comes and goes in a blink reads as a glitch), and
  until dictation is ready.
- When ready: the status line says "Ready. Hold Right Ctrl to dictate." for 1.2 s, then the card
  fades out over 750 ms (opacity to 0, scale to 0.985), and Utter is in the tray.

## When it shows, and when it does not

| Start | Splash |
|---|---|
| The person starts Utter (Start menu, desktop, the installer's last page) and dictation needs loading | yes |
| Utter starts at sign-in (Start with Windows) | no: tray only, quiet while the person logs in |
| First run, update or repair is needed | no: the setup window shows instead (docs/ux/first-run.md) |
| Dictation cannot start | the splash hands over to the problem screen (same place, no flash between) |
| Launched again while running | no: the Home window opens |

## Escape hatches

- Esc, or a click on the card, hides it early; loading carries on and the tray says when it is
  ready.
- Closing it does not quit Utter.

## Motion and performance

- The canvas runs at the display's refresh rate, its resolution capped at 2x (the models load
  at the same time, on the same machine).
- Five ribbons, each a core, a soft glow and two faint strands: about 30 strokes a frame.
- The write-head moves at up to 120 px/s, so a page takes 12 to 15 s; a full page rests 1.8 s
  and fades over 1.2 s.
- With "reduce motion" set: the ribbons are still, the page is written, the mark is drawn;
  the progress row and status line work as above.

## Review

`npm run dev`, then `splash.html?demo` plays the stages on a timer in a browser. Tai reviews it
there live; after two misses at his taste, ask plainly what he wants.

## Acceptance checks

1. Starting Utter shows the card within 150 ms; nothing else flashes first.
2. The stages light in order as the real loading reports them; the status line follows.
3. It stays at least 3 s, then fades out smoothly; no dark frame or flicker during the fade.
4. At sign-in it does not appear.
5. It never covers a window the person brings to the front.
6. A start failure turns into the problem screen without a blank moment between.
7. Smooth at 100% and 150% scaling; no dropped frames while the models load.
