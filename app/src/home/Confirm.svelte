<script lang="ts">
  // A confirm over the Home window, for what cannot be undone (docs/ux/history.md: clearing
  // History, shortening how long it keeps). Esc or a click outside cancels. The focus starts
  // on Cancel, so a stray Enter never deletes anything.
  import { cubicOut } from 'svelte/easing';
  import { fade, scale } from 'svelte/transition';

  let {
    title,
    message,
    action,
    onconfirm,
    oncancel,
  }: { title: string; message: string; action: string; onconfirm: () => void; oncancel: () => void } = $props();

  function focus(el: HTMLButtonElement) {
    el.focus();
  }
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && oncancel()} />

<div class="scrim" transition:fade={{ duration: 140 }} role="presentation" onclick={oncancel}></div>
<div
  class="dialog"
  role="alertdialog"
  aria-modal="true"
  aria-labelledby="confirm-title"
  aria-describedby="confirm-message"
  transition:scale={{ start: 0.97, duration: 160, easing: cubicOut }}
>
  <h2 id="confirm-title">{title}</h2>
  <p id="confirm-message">{message}</p>
  <div class="actions">
    <button use:focus onclick={oncancel}>Cancel</button>
    <button class="danger" onclick={onconfirm}>{action}</button>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: rgba(8, 10, 20, 0.36);
    z-index: 20;
  }
  .dialog {
    position: fixed;
    left: 50%;
    top: 38%;
    translate: -50% -50%;
    width: min(400px, calc(100vw - 48px));
    padding: 22px 24px 18px;
    border-radius: 14px;
    background: var(--home-surface);
    border: 1px solid var(--home-border);
    box-shadow: var(--home-shadow);
    z-index: 21;
  }
  h2 {
    font: 600 16px/22px var(--font-display);
    margin: 0 0 6px;
  }
  p {
    margin: 0 0 20px;
    color: var(--home-muted);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .danger {
    border-color: transparent;
    background: #c4383a;
    color: #fff;
    font-weight: 600;
  }
  .danger:hover:not(:disabled) {
    background: #b13033;
  }
</style>
