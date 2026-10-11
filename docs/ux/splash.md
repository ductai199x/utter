# The splash

What shows while Utter gets ready after a person starts it: the models load for 10 to 20
seconds, and without it nothing on screen says anything is happening. Status: **spec, not
built**.

The requirements come from the splash Tai shaped in the chess coaching companion
(the sibling project, desktop/src/routes/splash): the same window behaviour, timing and
care. The picture is Utter's own.

## The idea: voice becoming text

Utter turns speech into clean text, so the splash shows exactly that. A field of fine points of
light moves as a voice does: a living waveform across the card, louder and quieter in phrases.
Every few seconds a phrase gathers: the points stream out of the wave and settle into a word,
crisp and still for a moment, like text arriving where you type. Then the word lets go, its
points flow back into the wave, and the next phrase begins.

The words are Utter's own vocabulary, chosen to read well at a glance: "utter", "hello",
"clean text", "just speak". Never more than two words, set in the UI font, weight 600.

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

- The Utter mark, 72 px, top left of the card's content column, with a soft accent glow that
  breathes (scale 1.0 to 1.03 over 2.6 s).
- "Utter" beside it, 28 px semibold, and the version under it, small and muted.
- The progress: the stages of getting ready, as a short row of words under the title, each
  lighting up as it finishes:
  `Speech · Cleanup · Microphone · Ready`
  The one loading pulses softly; finished ones are full brightness with a small check; the
  others are dim. Each finish sends a gentle ripple through the waveform.
- One status line under the row: "Loading the speech model...", then the next stage, then
  "Ready. Hold Right Ctrl to dictate." It never shows a raw error.
- The animation (the waveform and the words of light) fills the card behind the text, its
  center of mass on the right so the text column on the left stays readable.

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
- Each frame is drawn to an off-screen buffer and put on the card in one go.
- About 2,400 points. Forming a word takes 1.6 s (eased), holding it 1.4 s, releasing it 0.9 s.
- With "reduce motion" set: the waveform is still, no words form, the progress row and status
  line work as above.

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
