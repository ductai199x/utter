<script lang="ts">
  // Utter's mark (app-icon.svg's shape) in the theme's accent.
  let { size = 24 }: { size?: number } = $props();
</script>

<svg class="logo" viewBox="0 0 40 40" width={size} height={size} aria-hidden="true">
  <rect class="tile" width="40" height="40" rx="10" />
  <g class="wave">
    <rect x="9" y="17" width="3" height="6" rx="1.5" />
    <rect x="14.5" y="13" width="3" height="14" rx="1.5" />
    <rect x="20" y="9" width="3" height="22" rx="1.5" />
    <rect x="25.5" y="14" width="3" height="12" rx="1.5" />
    <rect x="31" y="18" width="3" height="4" rx="1.5" />
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
  .wave {
    fill: var(--home-accent-text);
  }
</style>
