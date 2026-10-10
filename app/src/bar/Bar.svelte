<script lang="ts">
  // The Bar (docs/ux/bar.md). The window it lives in is fixed-size, transparent and
  // click-through; the pill animates inside it, anchored at the bottom center. Rust drives the
  // states and shows/hides the native window; this page only draws.
  import { listen } from '@tauri-apps/api/event';
  import { onMount } from 'svelte';

  type BarState = 'hidden' | 'listening' | 'finishing' | 'done' | 'cancelled' | 'error' | 'exit';

  let barState = $state<BarState>('hidden');
  let handsFree = $state(false);
  let committed = $state('');
  let preview = $state('');
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

  let canvas = $state<HTMLCanvasElement>();
  let textEl = $state<HTMLSpanElement>();
  let textWidth = $state(0);

  const hasWords = $derived(committed.length + preview.length > 0);
  const pillWidth = $derived(
    hasWords && (barState === 'listening' || barState === 'finishing')
      ? Math.min(MAX_W, BASE_W + TEXT_GAP + textWidth + TEXT_END - PAD)
      : barState === 'error'
        ? MAX_W
        : BASE_W,
  );

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

  $effect(() => {
    // Re-measure the words whenever they change.
    committed;
    preview;
    if (textEl) textWidth = textEl.scrollWidth;
  });

  onMount(() => {
    raf = requestAnimationFrame(frame);
    const unlisten = [
      listen<{ state: BarState; handsFree?: boolean; message?: string }>('bar:state', (e) => {
        const p = e.payload;
        if (p.state === 'listening' && barState !== 'listening') {
          committed = '';
          preview = '';
          levels.fill(0);
        }
        barState = p.state;
        if (p.handsFree !== undefined) handsFree = p.handsFree;
        if (p.state === 'hidden') handsFree = false;
        message = p.message ?? '';
      }),
      listen<{ committed: string; preview: string }>('bar:words', (e) => {
        committed = e.payload.committed;
        preview = e.payload.preview;
      }),
      listen<number[]>('bar:levels', (e) => {
        for (const l of e.payload) pushLevel(l);
      }),
    ];
    return () => {
      cancelAnimationFrame(raf);
      unlisten.forEach((p) => p.then((f) => f()));
    };
  });
</script>

<div class="stage">
  <div
    class="pill"
    class:shown={barState !== 'hidden' && barState !== 'exit' && barState !== 'cancelled'}
    class:finishing={barState === 'finishing'}
    style:width="{pillWidth}px"
  >
    {#if handsFree && barState === 'listening'}<span class="dot"></span>{/if}
    {#if barState === 'done'}
      <svg class="check" viewBox="0 0 24 24" width="18" height="18" aria-hidden="true">
        <path d="M5 12.5l4.2 4.2L19 7" />
      </svg>
    {:else if barState === 'error'}
      <span class="error">{message}</span>
    {:else}
      <canvas bind:this={canvas} style:width="{WAVE_W}px" style:height="{BAR_MAX}px"></canvas>
    {/if}
    {#if hasWords && (barState === 'listening' || barState === 'finishing')}
      <div class="words">
        <span class="line" bind:this={textEl}
          >{committed}{#if preview}{#key preview}<span class="preview">{committed ? ' ' : ''}{preview}</span>{/key}{/if}</span
        >
      </div>
    {/if}
  </div>
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
    align-items: center;
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
      width var(--dur-resize) var(--ease-out);
  }
  .pill.shown {
    opacity: 1;
    transform: scale(1);
    transition:
      opacity var(--dur-enter) var(--ease-out),
      transform var(--dur-enter) var(--ease-out),
      width var(--dur-resize) var(--ease-out);
  }
  canvas {
    flex: none;
    display: block;
  }
  /* The line is right-aligned in a box that clips: when it is longer than the box, its start
     (the oldest words) overflows on the left and the newest words stay in view. Left to
     right throughout, so punctuation stays where it belongs. */
  .words {
    flex: 1 1 auto;
    min-width: 0;
    display: flex;
    justify-content: flex-end;
    overflow: hidden;
    color: var(--bar-words);
    font-size: 14px;
    line-height: 40px;
  }
  .words:has(.line:not(:empty)) {
    mask-image: linear-gradient(to right, transparent 0, black 32px);
  }
  .line {
    flex: none;
    white-space: nowrap;
  }
  .preview {
    color: var(--bar-word-newest);
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
    margin: 0 auto;
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
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
