<script lang="ts">
  // A review page for the mark (dev only, `npm run dev`, then brand.html): the animated mark
  // large, and the static one at the sizes it ships at.
  import AnimatedMark from '../lib/AnimatedMark.svelte';
  import Logo from '../lib/Logo.svelte';

  let run = $state(0);
</script>

<main>
  <section class="stage">
    {#key run}<AnimatedMark size={280} color="#6B7CFF" />{/key}
    <button onclick={() => run++}>Replay</button>
  </section>
  <section class="sizes">
    {#each [128, 64, 32, 16] as s (s)}<Logo size={s} />{/each}
    {#each [128, 64, 32, 16] as s (s)}<Logo size={s} tile={false} />{/each}
  </section>
</main>

<style>
  :global(body) {
    margin: 0;
    background: #0b0e1b;
    font-family: var(--font-ui);
    color: #eef1ff;
  }
  :global(:root) {
    --home-accent: #6b7cff;
    --home-accent-text: #0e1330;
  }
  main {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 32px;
    padding: 40px;
  }
  .stage {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 16px;
    padding: 24px 48px;
    border-radius: 20px;
    background: radial-gradient(70% 70% at 50% 45%, #182148 0%, #0e1330 70%);
  }
  .stage :global(svg) {
    filter: drop-shadow(0 0 22px rgba(107, 124, 255, 0.45));
  }
  button {
    font: inherit;
    color: #c8cef5;
    background: rgba(255, 255, 255, 0.06);
    border: 1px solid rgba(255, 255, 255, 0.12);
    border-radius: 8px;
    padding: 6px 16px;
    cursor: pointer;
  }
  .sizes {
    display: grid;
    grid-template-columns: repeat(4, auto);
    align-items: center;
    justify-items: center;
    gap: 20px 32px;
  }
</style>
