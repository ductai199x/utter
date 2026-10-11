<script lang="ts">
  // Utter's mark (lib/mark.ts) in the theme's accent: on a tile for app icons and small sizes,
  // or bare (the splash draws its own, animated).
  import { LINES, MARK_STROKE, ribbonPath } from './mark';

  let { size = 24, tile = true }: { size?: number; tile?: boolean } = $props();
</script>

<svg class="logo" viewBox="0 0 64 64" width={size} height={size} aria-hidden="true">
  {#if tile}<rect class="tile" width="64" height="64" rx="15" />{/if}
  <g class:on-tile={tile} fill="none" stroke-width={MARK_STROKE} stroke-linecap="round" stroke-linejoin="round">
    <path d={ribbonPath()} />
    {#each LINES as d (d)}<path {d} />{/each}
  </g>
</svg>

<style>
  .logo {
    display: block;
    flex: none;
  }
  .tile {
    fill: var(--home-accent);
  }
  g {
    stroke: var(--home-accent);
  }
  g.on-tile {
    stroke: var(--home-accent-text);
  }
</style>
