<script lang="ts">
  // Utter's mark, alive (lib/mark.ts). Arrival: the ribbon draws itself from its hook into the
  // first line, and the other two lines write in after it. Then the paragraph lives: each line
  // eases longer and shorter, out of step in a slow cascade from the top, and the end of a line
  // that is growing glows like a cursor writing; under it the rise breathes. Still, and
  // finished, with reduced motion.
  import { onMount } from 'svelte';
  import { LINE_END, LINE_START, LINE_Y, MARK_STROKE, linePath, ribbonPath } from './mark';

  let {
    size = 96,
    color = 'var(--home-accent)',
    glow = '#dfe4ff',
    intro = true,
    live = true,
  }: { size?: number; color?: string; glow?: string; intro?: boolean; live?: boolean } = $props();

  const INTRO_MS = 900; // the ribbon drawing itself
  const LINE_MS = 380; // each line writing in
  const LINE_AT = [640, 800]; // when lines two and three start
  const PERIOD_MS = 5200; // one line's grow-and-shrink
  const CASCADE = 0.16; // of a period, how far each line lags the one above
  const BREATH_MS = 4200;
  // How far a line shrinks: its end eases between these (grid units), the first line less,
  // since the ribbon flows into it.
  const SHORTEST = [46.5, 43, 41];

  let ribbon = $state<SVGPathElement>();
  let ribbonD = $state(ribbonPath());
  let lineD = $state(LINE_Y.slice(1).map((y) => linePath(y)));
  let ribbonDraw = $state(0); // 0..1 of the ribbon drawn
  let lineDraw = $state([0, 0]);
  let length = $state(60);
  // The write-heads: per line, where its end is and how bright its glow (0..1).
  let heads = $state(LINE_Y.map(() => ({ x: LINE_END, glow: 0 })));

  const easeOut = (p: number) => 1 - Math.pow(1 - Math.min(1, Math.max(0, p)), 3);
  // A smooth back-and-forth that lingers at both ends: 0 -> 1 -> 0 over one period.
  const swell = (phase: number) => {
    const s = 0.5 - 0.5 * Math.cos(phase * Math.PI * 2);
    return s * s * (3 - 2 * s);
  };

  onMount(() => {
    if (ribbon) length = ribbon.getTotalLength();
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
      ribbonDraw = 1;
      lineDraw = [1, 1];
      return;
    }
    const t0 = performance.now();
    let raf = 0;
    const frame = (now: number) => {
      const t = now - t0;
      ribbonDraw = intro ? easeOut(t / INTRO_MS) : 1;
      lineDraw = intro ? LINE_AT.map((at) => easeOut((t - at) / LINE_MS)) : [1, 1];
      // The living part begins once the arrival has settled, easing in over a second.
      const alive = intro ? Math.max(0, t - (LINE_AT[1]! + LINE_MS + 600)) : t;
      // A mark that only arrives (live = false) rests once it has: nothing more to draw.
      if (!live && alive > 0) {
        ribbonDraw = 1;
        lineDraw = [1, 1];
        heads = LINE_Y.map(() => ({ x: LINE_END, glow: 0 }));
        return;
      }
      const ease = Math.min(1, alive / 1000);
      const ends = LINE_Y.map((_, i) => {
        // Full length at the start of a period, so the arrival hands over without a jump.
        const s = swell(alive / PERIOD_MS - i * CASCADE);
        return LINE_END - (LINE_END - SHORTEST[i]!) * s * ease;
      });
      // The glow follows growth: the faster a line's end moves right, the brighter its head.
      heads = ends.map((x, i) => {
        const ahead = LINE_END - (LINE_END - SHORTEST[i]!) * swell((alive + 60) / PERIOD_MS - i * CASCADE) * ease;
        const speed = (ahead - x) / (60 / 1000); // grid units per second
        return { x, glow: Math.min(1, Math.max(0, speed / 9)) };
      });
      const breath = Math.sin((alive / BREATH_MS) * Math.PI * 2);
      ribbonD = ribbonPath(1 + 0.05 * breath * ease, ends[0]);
      lineD = LINE_Y.slice(1).map((y, i) => linePath(y, ends[i + 1]));
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => cancelAnimationFrame(raf);
  });

  const HEAD = 9; // the stretch behind a line's end that brightens, in grid units
  // Gradient and filter ids, unique per mark so several on one page don't share them.
  const uid = `mark-${Math.random().toString(36).slice(2, 8)}`;
</script>

<svg viewBox="0 0 64 64" width={size} height={size} aria-hidden="true">
  <defs>
    <filter id="{uid}-blur" x="-50%" y="-200%" width="200%" height="500%">
      <feGaussianBlur stdDeviation="1.6" />
    </filter>
    <!-- Each glint fades in along its line, so it reads as light gathering at the end. -->
    {#each heads as h, i (i)}
      <linearGradient id="{uid}-glint-{i}" gradientUnits="userSpaceOnUse" x1={h.x - HEAD} y1="0" x2={h.x} y2="0">
        <stop offset="0" stop-color={glow} stop-opacity="0" />
        <stop offset="0.7" stop-color={glow} stop-opacity={0.4 * h.glow} />
        <stop offset="1" stop-color={glow} stop-opacity={0.75 * h.glow} />
      </linearGradient>
    {/each}
  </defs>
  <g fill="none" stroke-width={MARK_STROKE} stroke-linecap="round" stroke-linejoin="round">
    <path
      bind:this={ribbon}
      d={ribbonD}
      stroke={color}
      stroke-dasharray={length + 20}
      stroke-dashoffset={(length + 20) * (1 - ribbonDraw)}
      opacity={ribbonDraw > 0 ? 1 : 0}
    />
    {#each lineD as d, i (i)}
      <path {d} stroke={color} pathLength="1" stroke-dasharray="1" stroke-dashoffset={1 - lineDraw[i]!} opacity={lineDraw[i]! > 0 ? 1 : 0} />
    {/each}
    <!-- The write-heads: the line itself brightening toward its growing end, with a soft halo. -->
    {#each heads as h, i (i)}
      {#if h.glow > 0.02}
        {@const d = `M ${Math.max(i === 0 ? LINE_START + 2 : LINE_START, h.x - HEAD)} ${LINE_Y[i]} L ${h.x} ${LINE_Y[i]}`}
        <path {d} stroke="url(#{uid}-glint-{i})" stroke-width={MARK_STROKE + 3} opacity="0.5" filter="url(#{uid}-blur)" />
        <path {d} stroke="url(#{uid}-glint-{i})" />
      {/if}
    {/each}
  </g>
</svg>
