<script lang="ts">
  // The Bar (docs/ux/bar.md). The window it lives in is fixed-size, transparent and
  // click-through; the pill animates inside it, anchored at the bottom center. Rust drives the
  // states and shows/hides the native window; this page only draws.
  import { on } from './events';
  import { onMount } from 'svelte';

  type BarState = 'hidden' | 'listening' | 'finishing' | 'done' | 'cancelled' | 'error' | 'exit';

  let barState = $state<BarState>('hidden');
  let handsFree = $state(false);
  let committed = $state('');
  let preview = $state('');
  // `committed` is the dictation cleaned up to the last pause (shown as text); `preview` is what
  // was heard after it, not cleaned yet (provisional, dimmed). `isFinal`: the words are the
  // text being pasted.
  let isFinal = $state(false);
  let message = $state('');

  // Geometry, logical px (docs/ux/bar.md "Geometry").
  const BARS = 16;
  const BAR_W = 3;
  const BAR_GAP = 3;
  const BAR_MIN = 3;
  const BAR_MAX = 22;
  const PAD = 14;
  const TEXT_GAP = 12;
  const TEXT_END = 16;
  const WAVE_W = BARS * BAR_W + (BARS - 1) * BAR_GAP;
  const BASE_W = WAVE_W + 2 * PAD;
  const MAX_W = 520;
  const LINE_H = 20;
  const MAX_LINES = 5;
  const BASE_H = 40;

  // Everything in the pill but the words' own width: border 2, padding 28 less the words' 2 px
  // end padding (so 16 at the end), the waveform slot and the gap, and 1 px of slack so a
  // line that fits is never wrapped by rounding.
  const CHROME_W = 2 + 2 * PAD + WAVE_W + TEXT_GAP + (TEXT_END - PAD) + 1;
  const MAX_TEXT_W = MAX_W - CHROME_W;

  let canvas = $state<HTMLCanvasElement>();
  // Hidden copies measure the words at the pill's target size, not its animated one: one line
  // gives the natural width, one wrapped at the target width gives the line count.
  let measureEl = $state<HTMLSpanElement>();
  let wrapEl = $state<HTMLDivElement>();
  let textWidth = $state(0);
  let lines = $state(1);
  const textAreaW = $derived(Math.min(MAX_TEXT_W, Math.ceil(textWidth)));

  const hasWords = $derived(committed.length + preview.length > 0);
  // Exactly the text the pill shows, for the measuring copies.
  const shownText = $derived(committed && preview ? `${committed} ${preview}` : committed || preview);
  // Words show while listening and finishing, and stay under the Done check once they are
  // the pasted text.
  const showWords = $derived(
    hasWords && (barState === 'listening' || barState === 'finishing' || (barState === 'done' && isFinal)),
  );
  const pillWidth = $derived(
    showWords
      ? CHROME_W + textAreaW
      : barState === 'error'
        ? MAX_W
        : BASE_W,
  );
  const pillHeight = $derived(showWords ? BASE_H + (Math.min(lines, MAX_LINES) - 1) * LINE_H : BASE_H);

  // Waveform: levels arrive at about 60 Hz; each bar shows the peak of its slice of the last
  // 600 ms, newest on the right, with a fast attack and a gentle release.
  const HISTORY = 36; // 600 ms at 60 Hz
  const levels = new Float32Array(HISTORY);
  let head = 0;
  const heights = new Float32Array(BARS).fill(BAR_MIN);
  const ATTACK_MS = 30;
  const RELEASE_MS = 140;

  function pushLevel(rms: number) {
    levels[head] = rms;
    head = (head + 1) % HISTORY;
  }

  /** RMS (0..1) to a 0..1 bar height: speech sits around 0.02-0.2 RMS, so a log scale. */
  function shape(rms: number): number {
    const db = 20 * Math.log10(Math.max(rms, 1e-5));
    return Math.min(1, Math.max(0, (db + 55) / 40));
  }

  let last = performance.now();
  let raf = 0;
  // Read once: the token does not change at run time.
  const waveColor =
    getComputedStyle(document.documentElement).getPropertyValue('--bar-wave').trim() || 'rgba(255,255,255,0.92)';
  function frame(now: number) {
    const dt = Math.min(50, now - last);
    last = now;
    const cv = canvas;
    const ctx = cv?.getContext('2d');
    if (cv && ctx) {
      const dpr = window.devicePixelRatio || 1;
      const w = WAVE_W, h = BAR_MAX;
      if (cv.width !== Math.round(w * dpr)) {
        cv.width = Math.round(w * dpr);
        cv.height = Math.round(h * dpr);
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      ctx.fillStyle = waveColor;
      const per = HISTORY / BARS;
      const live = barState === 'listening';
      for (let i = 0; i < BARS; i++) {
        let peak = 0;
        for (let k = 0; k < per; k++) {
          const idx = (head + Math.floor(i * per) + k) % HISTORY;
          peak = Math.max(peak, levels[idx] ?? 0);
        }
        const target = live ? BAR_MIN + shape(peak) * (BAR_MAX - BAR_MIN) : BAR_MIN;
        const tau = target > heights[i]! ? ATTACK_MS : RELEASE_MS;
        heights[i] = heights[i]! + (target - heights[i]!) * (1 - Math.exp(-dt / tau));
        const bh = heights[i]!;
        const x = i * (BAR_W + BAR_GAP);
        const y = (h - bh) / 2;
        ctx.beginPath();
        ctx.roundRect(x, y, BAR_W, bh, BAR_W / 2);
        ctx.fill();
      }
    }
    raf = requestAnimationFrame(frame);
  }

  onMount(() => {
    // The measuring copies report their own size after every layout, before paint: the
    // one-line width sets the pill's width, and the wrapped height at that width (which
    // changes only once the new width is laid out) sets its line count.
    const sizes = new ResizeObserver(() => {
      if (measureEl) textWidth = measureEl.getBoundingClientRect().width;
      if (wrapEl) lines = Math.max(1, Math.round(wrapEl.getBoundingClientRect().height / LINE_H));
    });
    if (measureEl) sizes.observe(measureEl);
    if (wrapEl) sizes.observe(wrapEl);
    raf = requestAnimationFrame(frame);
    const unlisten = [
      on<{ state: BarState; handsFree?: boolean; message?: string }>('bar:state', (e) => {
        const p = e.payload;
        if (p.state === 'listening' && barState !== 'listening') {
          committed = '';
          preview = '';
          isFinal = false;
          levels.fill(0);
        }
        barState = p.state;
        if (p.handsFree !== undefined) handsFree = p.handsFree;
        if (p.state === 'hidden') handsFree = false;
        message = p.message ?? '';
      }),
      on<{ committed: string; preview: string; final: boolean }>('bar:words', (e) => {
        committed = e.payload.committed;
        preview = e.payload.preview;
        isFinal = e.payload.final;
      }),
      on<number[]>('bar:levels', (e) => {
        for (const l of e.payload) pushLevel(l);
      }),
    ];
    return () => {
      sizes.disconnect();
      cancelAnimationFrame(raf);
      unlisten.forEach((p) => p.then((f) => f()));
    };
  });
</script>

<div class="stage" style:--wave-w="{WAVE_W}px" style:--line-h="{LINE_H}px">
  <div
    class="pill"
    class:shown={barState !== 'hidden' && barState !== 'exit' && barState !== 'cancelled'}
    class:finishing={barState === 'finishing'}
    style:width="{pillWidth}px"
    style:height="{pillHeight}px"
  >
    {#if handsFree && barState === 'listening'}<span class="dot"></span>{/if}
    {#if barState === 'error'}
      <span class="error">{message}</span>
    {:else}
      <div class="lead" class:done={barState === 'done'}>
        {#if barState === 'done'}
          <svg class="check" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
            <path d="M5 12.5l4.2 4.2L19 7" />
          </svg>
        {:else}
          <canvas bind:this={canvas} style:width="{WAVE_W}px" style:height="{BAR_MAX}px"></canvas>
        {/if}
      </div>
    {/if}
    {#if showWords}
      <div class="words" class:overflowing={lines > MAX_LINES}>
        <!-- Laid out at the pill's target width, not its animated one: as the pill grows it
             reveals the text instead of reflowing it, so lines never spill past the height. -->
        <div class="text" style:width="{textAreaW + 1}px">
          {committed}{#if preview}{#key preview}<span class="preview">{committed ? ' ' : ''}{preview}</span>{/key}{/if}
        </div>
      </div>
    {/if}
  </div>
  <span class="measure" bind:this={measureEl} aria-hidden="true">{shownText}</span>
  <div class="measure wrap" bind:this={wrapEl} style:width="{textAreaW + 1}px" aria-hidden="true">{shownText}</div>
</div>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    background: transparent;
    overflow: hidden;
    user-select: none;
    font-family: var(--font-ui);
  }
  .stage {
    position: fixed;
    inset: 0;
    display: flex;
    align-items: flex-end;
    justify-content: center;
    padding-bottom: 12px;
  }
  .pill {
    position: relative;
    box-sizing: border-box;
    height: 40px;
    border-radius: 20px;
    padding: 0 14px;
    display: flex;
    /* The waveform sits beside the newest line, at the bottom, as the pill grows upward. */
    align-items: flex-end;
    gap: 12px;
    overflow: hidden;
    background: var(--bar-bg);
    border: 1px solid var(--bar-border);
    box-shadow: var(--bar-shadow);
    opacity: 0;
    transform: scale(0.97);
    transform-origin: 50% 100%;
    transition:
      opacity var(--dur-exit) var(--ease-in),
      transform var(--dur-exit) var(--ease-in),
      width var(--dur-resize) var(--ease-out),
      height var(--dur-resize) var(--ease-out);
  }
  .pill.shown {
    opacity: 1;
    transform: scale(1);
    transition:
      opacity var(--dur-enter) var(--ease-out),
      transform var(--dur-enter) var(--ease-out),
      width var(--dur-resize) var(--ease-out),
      height var(--dur-resize) var(--ease-out);
  }
  /* A fixed-width slot, so the words never move when the waveform becomes the check. */
  .lead {
    flex: none;
    width: var(--wave-w);
    height: 38px;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  /* The Done check alone (no words) is centered in the pill. */
  .lead.done:last-child {
    flex: 1;
    justify-content: center;
  }
  canvas {
    display: block;
  }
  /* The words wrap at the pill's width. The newest line is the bottom one; when there are more
     lines than fit, the oldest overflow the top, under a fade. */
  .words {
    flex: 1 1 auto;
    align-self: stretch;
    min-width: 0;
    box-sizing: border-box;
    padding: 9px 2px 9px 0;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    overflow: hidden;
    color: var(--bar-words);
    font-size: var(--bar-text-size);
    line-height: var(--line-h);
    transition: color var(--dur-word) var(--ease-out);
  }
  /* Nothing shows in the top padding (no sliced letters); the oldest full line fades in. */
  .words.overflowing {
    mask-image: linear-gradient(to bottom, transparent 9px, rgba(0, 0, 0, 0.35) 9px, black 29px);
  }
  .text {
    flex: none;
    overflow-wrap: anywhere;
  }
  .measure {
    position: absolute;
    display: inline-block;
    left: -10000px;
    top: 0;
    visibility: hidden;
    white-space: nowrap;
    font-size: var(--bar-text-size);
  }
  .measure.wrap {
    white-space: normal;
    line-height: var(--line-h);
    overflow-wrap: anywhere;
  }
  /* Heard but not cleaned yet: provisional until the next cleanup replaces it. */
  .preview {
    color: var(--bar-words-provisional);
    animation: word-in var(--dur-word) var(--ease-out);
  }
  @keyframes word-in {
    from {
      opacity: 0.35;
    }
  }
  .dot {
    position: absolute;
    left: 6px;
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--bar-hands-free);
  }
  .finishing::after {
    content: '';
    position: absolute;
    inset: 0;
    background: linear-gradient(100deg, transparent 30%, rgba(255, 255, 255, 0.07) 50%, transparent 70%);
    background-size: 220% 100%;
    animation: shimmer 900ms linear 150ms infinite;
    opacity: 0;
  }
  @keyframes shimmer {
    from {
      opacity: 1;
      background-position: 120% 0;
    }
    to {
      opacity: 1;
      background-position: -120% 0;
    }
  }
  .check {
    flex: none;
    fill: none;
    stroke: var(--bar-wave);
    stroke-width: 2.4;
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-dasharray: 24;
    stroke-dashoffset: 24;
    animation: draw 220ms var(--ease-out) forwards;
  }
  @keyframes draw {
    to {
      stroke-dashoffset: 0;
    }
  }
  .error {
    color: var(--bar-error);
    font-size: 13px;
    line-height: 38px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
