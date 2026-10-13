<script lang="ts">
  // The Home window (docs/ux/home.md, docs/ux/history.md): a rail of sections, History first.
  // The frame owns what the sections share: the core's state, the settings and saving them.
  import { onMount } from 'svelte';
  import { cubicOut } from 'svelte/easing';
  import { fade } from 'svelte/transition';
  import AnimatedMark from '../lib/AnimatedMark.svelte';
  import Dictionary from './Dictionary.svelte';
  import History from './History.svelte';
  import Settings from './Settings.svelte';
  import './page.css';
  import {
    homeState,
    onApplied,
    onState,
    saveSettings,
    type Applied,
    type HomeState,
    type Settings as SettingsData,
  } from './core';

  type Section = 'history' | 'dictionary' | 'settings';
  const SECTIONS: { id: Section; label: string; key: string }[] = [
    { id: 'history', label: 'History', key: '1' },
    { id: 'dictionary', label: 'Dictionary', key: '2' },
    { id: 'settings', label: 'Settings', key: '3' },
  ];

  let section = $state<Section>(loadSection());
  let home = $state<HomeState | null>(null);
  let settings = $state<SettingsData | null>(null);
  let saveError = $state('');
  // Per setting: "loading" while a model loads, or the error that kept the old value.
  let applied = $state<Partial<Record<Applied['what'], Applied>>>({});
  let dictionary = $state<ReturnType<typeof Dictionary>>();

  function loadSection(): Section {
    try {
      const s = localStorage.getItem('home.section');
      if (s === 'history' || s === 'dictionary' || s === 'settings') return s;
    } catch {
      // Storage can be unavailable; History is the place to start.
    }
    return 'history';
  }

  function go(s: Section) {
    section = s;
    try {
      localStorage.setItem('home.section', s);
    } catch {
      // Remembering the section is a convenience only.
    }
  }

  function take(s: HomeState) {
    home = s;
    if (s.settings) settings = s.settings;
    // A broken file opens Settings on its banner (docs/ux/home.md).
    if (s.error) section = 'settings';
  }

  async function save(next: SettingsData) {
    try {
      settings = await saveSettings(next);
      saveError = '';
    } catch (e) {
      saveError = String(e);
    }
  }

  function onKey(e: KeyboardEvent) {
    const s = SECTIONS.find((x) => x.key === e.key);
    if (e.ctrlKey && s) {
      e.preventDefault();
      go(s.id);
    } else if (e.ctrlKey && e.key.toLowerCase() === 'f' && section === 'dictionary') {
      e.preventDefault();
      dictionary?.focusAdd();
    }
  }

  function railKey(e: KeyboardEvent) {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    const i = SECTIONS.findIndex((s) => s.id === section);
    const next = SECTIONS[(i + (e.key === 'ArrowDown' ? 1 : SECTIONS.length - 1)) % SECTIONS.length]!;
    go(next.id);
    document.getElementById(`rail-${next.id}`)?.focus();
  }

  onMount(() => {
    void homeState().then(take);
    const offs = [
      onState(take),
      onApplied((a) => {
        applied = { ...applied, [a.what]: a };
        if (a.status === 'ok') {
          setTimeout(() => {
            if (applied[a.what] === a) applied = { ...applied, [a.what]: undefined };
          }, 1500);
        }
      }),
    ];
    return () => offs.forEach((p) => void p.then((off) => off()));
  });
</script>

<svelte:window onkeydown={onKey} />

<div class="frame">
  <nav class="rail" aria-label="Sections">
    <div class="mark" title="Utter"><AnimatedMark size={30} color="var(--home-accent)" live={false} /></div>
    {#each SECTIONS as s (s.id)}
      <button
        id="rail-{s.id}"
        class="section"
        class:bottom={s.id === 'settings'}
        class:selected={section === s.id}
        aria-current={section === s.id ? 'page' : undefined}
        aria-label={s.label}
        title="{s.label} (Ctrl+{s.key})"
        onclick={() => go(s.id)}
        onkeydown={railKey}
      >
        {#if s.id === 'history'}
          <!-- Lines of text, with a hand of time at their end. -->
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <path d="M3.5 5.5h9M3.5 9.5h6M3.5 13.5h4.5" />
            <circle cx="14.25" cy="13.75" r="3.25" />
            <path d="M14.25 12.25v1.6l1.1.7" />
          </svg>
        {:else if s.id === 'dictionary'}
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <path d="M4.5 3.5h9A2.5 2.5 0 0 1 16 6v10.5H7A2.5 2.5 0 0 1 4.5 14z" />
            <path d="M4.5 14A2.5 2.5 0 0 1 7 11.5h9M7.5 6.75h5" />
          </svg>
        {:else}
          <svg viewBox="0 0 20 20" aria-hidden="true">
            <path d="M3.5 6h8M14.5 6h2M3.5 14h2M8.5 14h8" />
            <circle cx="13" cy="6" r="1.75" />
            <circle cx="7" cy="14" r="1.75" />
          </svg>
        {/if}
      </button>
    {/each}
  </nav>

  <main>
    {#key section}
      <div class="section-body" in:fade={{ duration: 160, easing: cubicOut }}>
        {#if section === 'history'}
          <History
            historyOff={settings?.keep_history === 'off'}
            keyName={home?.dictationKeys.find((k) => k.id === settings?.dictation_key)?.name ?? 'Right Ctrl'}
          />
        {:else if section === 'dictionary'}
          <Dictionary bind:this={dictionary} {settings} {save} />
        {:else}
          <Settings {home} {settings} {applied} {saveError} {save} />
        {/if}
      </div>
    {/key}
  </main>
</div>

<style>
  .frame {
    display: flex;
    height: 100%;
  }

  .rail {
    width: 60px;
    flex: none;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 14px 0 12px;
    background: var(--home-rail);
    border-right: 1px solid var(--home-border);
  }
  .mark {
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    margin-bottom: 14px;
    filter: drop-shadow(0 0 10px var(--home-accent-soft));
  }
  .section {
    position: relative;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    padding: 0;
    border: none;
    border-radius: 11px;
    background: none;
    color: var(--home-muted);
    transition:
      background-color 140ms var(--ease-out),
      color 140ms var(--ease-out);
  }
  .section.bottom {
    margin-top: auto;
  }
  .section:hover:not(:disabled) {
    background: var(--home-hover);
    color: var(--home-text);
  }
  .section.selected,
  .section.selected:hover:not(:disabled) {
    background: var(--home-accent-soft);
    color: var(--home-accent);
  }
  /* The rail's selection: a short rule at its left edge, like the mark's lines. */
  .section.selected::before {
    content: '';
    position: absolute;
    left: -10px;
    top: 11px;
    bottom: 11px;
    width: 3px;
    border-radius: 0 2px 2px 0;
    background: var(--home-accent);
  }
  .section svg {
    width: 20px;
    height: 20px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  main {
    flex: 1;
    min-width: 0;
    position: relative;
  }
  .section-body {
    position: absolute;
    inset: 0;
  }
</style>
