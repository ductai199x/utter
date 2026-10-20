<script lang="ts">
  // The Paste rules window (docs/ux/insertion.md, "Paste rules"): every rule in a table to
  // search, sort and filter, edited in place, and adding one for any app. It edits the same
  // settings.toml as the Home window; the core keeps both in step (home-state).
  import { onMount, tick } from 'svelte';
  import { cubicOut } from 'svelte/easing';
  import { fly } from 'svelte/transition';
  import '../lib/chips.css';
  import { commandKey, isWindows } from '../lib/keys';
  import {
    KEEP_LABELS,
    WITH_CHOICES,
    WITH_LABELS,
    groupRules,
    homeState,
    onState,
    pasteRules,
    ruleSource,
    saveSettings,
    withApp,
    withRule,
    withoutRule,
    type AppRow,
    type PasteKeep,
    type PasteRules,
    type PasteWith,
    type RuleGroup,
    type Settings,
  } from '../home/core';

  type Source = 'default' | 'changed' | 'yours';
  type SortKey = 'name' | 'with' | 'keep' | 'dictations' | 'source';
  const SOURCES: [Source | 'all', string][] = [
    ['all', 'All'],
    ['yours', 'Yours'],
    ['changed', 'Changed'],
    ['default', 'Defaults'],
  ];
  const SOURCE_LABELS: Record<Source, string> = { default: 'Default', changed: 'Changed', yours: 'Yours' };
  const WITHS = WITH_CHOICES;
  const KEEPS = Object.entries(KEEP_LABELS) as [PasteKeep, string][];

  let settings = $state<Settings | null>(null);
  let rules = $state<PasteRules | null>(null);
  let error = $state('');

  let search = $state('');
  let source = $state<Source | 'all'>('all');
  let withFilter = $state<PasteWith | null>(null);
  let keepFilter = $state<PasteKeep | null>(null);
  let menu = $state<'source' | 'with' | 'keep' | null>(null);
  let sort = $state<{ key: SortKey; up: boolean }>({ key: 'dictations', up: false });

  let picker = $state(false);
  let pick = $state('');
  let pickEl = $state<HTMLInputElement>();
  let fileEl = $state<HTMLInputElement>();
  let searchEl = $state<HTMLInputElement>();
  let added = $state('');

  const groups = $derived(groupRules(rules?.rules ?? []));
  const shown = $derived.by(() => {
    const q = search.trim().toLowerCase();
    const rows = groups.filter(
      (g) =>
        (source === 'all' || ruleSource(g) === source) &&
        (!withFilter || g.with === withFilter) &&
        (!keepFilter || g.keep === keepFilter) &&
        (!q || g.name.toLowerCase().includes(q) || g.apps.some((a) => a.toLowerCase().includes(q))),
    );
    const order: Record<Source, number> = { yours: 0, changed: 1, default: 2 };
    const key = (g: RuleGroup): string | number =>
      sort.key === 'name'
        ? g.name.toLowerCase()
        : sort.key === 'with'
          ? WITH_LABELS[g.with]
          : sort.key === 'keep'
            ? KEEPS.findIndex(([k]) => k === g.keep)
            : sort.key === 'source'
              ? order[ruleSource(g)]
              : g.dictations;
    return [...rows].sort((a, b) => {
      const [x, y] = [key(a), key(b)];
      const c = x < y ? -1 : x > y ? 1 : 0;
      // Ties read A to Z whichever way the column sorts.
      return (sort.up ? c : -c) || a.name.localeCompare(b.name);
    });
  });
  const filtering = $derived(!!search.trim() || source !== 'all' || !!withFilter || !!keepFilter);

  /** Apps for the picker: open now first, then the ones dictated into, each once. */
  const choices = $derived.by(() => {
    const q = pick.trim().toLowerCase();
    const has = (exe: string) => groups.some((g) => g.apps.some((a) => a.toLowerCase() === exe.toLowerCase()));
    const match = (a: AppRow) => !q || a.name.toLowerCase().includes(q) || a.exe.toLowerCase().includes(q);
    const open = (rules?.openApps ?? []).filter(match);
    const recent = (rules?.recentApps ?? []).filter(
      (a) => match(a) && !open.some((o) => o.exe.toLowerCase() === a.exe.toLowerCase()),
    );
    return { open, recent, has };
  });

  async function refresh() {
    if (settings) rules = await pasteRules($state.snapshot(settings.paste));
  }

  async function save(paste: Settings['paste']) {
    if (!settings) return;
    try {
      settings = await saveSettings({ ...settings, paste });
      error = '';
      await refresh();
    } catch (e) {
      error = String(e);
    }
  }

  function setSort(key: SortKey) {
    // Counts read best from the most; everything else from the top of the alphabet.
    sort = sort.key === key ? { key, up: !sort.up } : { key, up: key !== 'dictations' };
  }

  async function openPicker() {
    picker = true;
    pick = '';
    // The apps open right now, asked for each time the picker opens.
    await refresh();
    await tick();
    pickEl?.focus();
  }

  /** Whether `exe` has a rule already: Utter's default or the person's. */
  const hasRule = (exe: string) => rules?.rules.some((r) => r.app.toLowerCase() === exe.toLowerCase()) ?? false;

  /** Adds a rule for `exe` pasting `how`, or shows the rule it has: never one over a default. */
  async function addApp(exe: string, how: PasteWith = 'ctrl-v') {
    if (!settings || !exe) return;
    picker = false;
    if (!hasRule(exe)) await save(withApp(settings.paste, exe, how));
    added = exe.toLowerCase();
    await tick();
    document.querySelector('.tr.added')?.scrollIntoView({ block: 'nearest' });
    setTimeout(() => (added = ''), 1600);
  }

  function chooseFile(e: Event) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    // Only the program's file name matters: rules match on it.
    if (file && /\.exe$/i.test(file.name)) void addApp(file.name);
    (e.currentTarget as HTMLInputElement).value = '';
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      if (picker) picker = false;
      else if (menu) menu = null;
      else if (search) search = '';
    } else if (commandKey(e) && e.key.toLowerCase() === 'f') {
      e.preventDefault();
      searchEl?.focus();
      searchEl?.select();
    }
  }

  function pickKey(e: KeyboardEvent) {
    if (e.key !== 'Enter') return;
    const first = [...choices.open, ...choices.recent].find((a) => !choices.has(a.exe));
    if (first) void addApp(first.exe);
  }

  onMount(() => {
    void homeState().then((s) => {
      settings = s.settings;
      void refresh();
    });
    const off = onState((s) => {
      if (!s.settings) return;
      settings = s.settings;
      void refresh();
    });
    return () => void off.then((f) => f());
  });
</script>

<svelte:window
  onkeydown={onKey}
  onclick={() => {
    menu = null;
    picker = false;
  }}
/>

<div class="window">
  <header class="head">
    <div>
      <h1>Paste rules</h1>
      <p class="lede">How each app pastes a dictation. Every other app: {WITH_LABELS['ctrl-v']}, and the clipboard back after 0.5 s.</p>
    </div>
    <div class="add-wrap">
      <button
        class="primary add"
        aria-haspopup="dialog"
        aria-expanded={picker}
        onclick={(e) => {
          e.stopPropagation();
          if (picker) picker = false;
          else void openPicker();
        }}
      >
        <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M8 3.5v9M3.5 8h9" /></svg>
        Add an app
      </button>
      {#if picker}
        <div
          class="picker"
          role="dialog"
          aria-label="Add an app"
          tabindex="-1"
          transition:fly={{ y: -6, duration: 140, easing: cubicOut }}
          onclick={(e) => e.stopPropagation()}
          onkeydown={() => {}}
        >
          <label class="search-field">
            <svg viewBox="0 0 20 20" aria-hidden="true"><circle cx="9" cy="9" r="5.5" /><path d="M13.2 13.2L17 17" /></svg>
            <input bind:this={pickEl} bind:value={pick} onkeydown={pickKey} placeholder="Find an app" spellcheck="false" />
          </label>
          <div class="choices">
            {#if choices.open.length}
              <p class="group">Open now</p>
              {#each choices.open as a (a.exe)}
                {@render choice(a)}
              {/each}
            {/if}
            {#if choices.recent.length}
              <p class="group">Dictated into</p>
              {#each choices.recent as a (a.exe)}
                {@render choice(a)}
              {/each}
            {/if}
            {#if !choices.open.length && !choices.recent.length}
              <p class="none">No open app matches.</p>
            {/if}
          </div>
          {#if isWindows}
            <button class="browse" onclick={() => fileEl?.click()}>
              <svg viewBox="0 0 20 20" aria-hidden="true"
                ><path d="M3.5 6A1.5 1.5 0 0 1 5 4.5h3.2l1.6 1.8H15A1.5 1.5 0 0 1 16.5 7.8v6.7A1.5 1.5 0 0 1 15 16H5a1.5 1.5 0 0 1-1.5-1.5z" /></svg
              >
              Choose a program…
            </button>
            <input type="file" accept=".exe" bind:this={fileEl} onchange={chooseFile} hidden />
          {/if}
        </div>
      {/if}
    </div>
  </header>

  {#snippet choice(a: AppRow)}
    {@const taken = choices.has(a.exe)}
    <button class="choice" disabled={taken} onclick={() => void addApp(a.exe)}>
      <span class="choice-name">{a.name}<span class="choice-exe">{a.exe}</span></span>
      <span class="choice-note">{taken ? 'Has a rule' : a.dictations ? `${a.dictations} ${a.dictations === 1 ? 'dictation' : 'dictations'}` : ''}</span>
    </button>
  {/snippet}

  <div class="toolbar">
    <label class="search-field">
      <svg viewBox="0 0 20 20" aria-hidden="true"><circle cx="9" cy="9" r="5.5" /><path d="M13.2 13.2L17 17" /></svg>
      <input bind:this={searchEl} bind:value={search} placeholder="Search apps" aria-label="Search apps" spellcheck="false" />
      <span class="count">{filtering ? `${shown.length} of ${groups.length}` : groups.length}</span>
    </label>
    <div class="chips" role="toolbar" aria-label="Filters">
      {#each [['source', source !== 'all', source === 'all' ? 'Source' : SOURCES.find(([s]) => s === source)![1]], ['with', !!withFilter, withFilter ? WITH_LABELS[withFilter] : 'Paste with'], ['keep', !!keepFilter, keepFilter ? KEEP_LABELS[keepFilter] : 'Keep']] as const as [which, on, label] (which)}
        <div class="chip-wrap">
          <button
            class="chip"
            class:on
            aria-haspopup="menu"
            aria-expanded={menu === which}
            onclick={(e) => {
              e.stopPropagation();
              menu = menu === which ? null : which;
            }}
          >
            {label}
            {#if on}
              <span
                class="clear"
                role="button"
                tabindex="-1"
                aria-label="Clear {which}"
                onclick={(e) => {
                  e.stopPropagation();
                  if (which === 'source') source = 'all';
                  else if (which === 'with') withFilter = null;
                  else keepFilter = null;
                  menu = null;
                }}
                onkeydown={() => {}}>×</span
              >
            {:else}
              <svg class="chev" viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 6.5l3.5 3.5 3.5-3.5" /></svg>
            {/if}
          </button>
          {#if menu === which}
            <div
              class="menu"
              role="menu"
              tabindex="-1"
              transition:fly={{ y: -4, duration: 120, easing: cubicOut }}
              onclick={(e) => e.stopPropagation()}
              onkeydown={() => {}}
            >
              {#if which === 'source'}
                {#each SOURCES as [v, l] (v)}
                  <button role="menuitemradio" aria-checked={source === v} onclick={() => ((source = v), (menu = null))}
                    >{l}<span class="n">{v === 'all' ? groups.length : groups.filter((g) => ruleSource(g) === v).length}</span
                    ></button
                  >
                {/each}
              {:else if which === 'with'}
                <button role="menuitemradio" aria-checked={!withFilter} onclick={() => ((withFilter = null), (menu = null))}
                  >Any</button
                >
                {#each WITHS as [v, l] (v)}
                  <button role="menuitemradio" aria-checked={withFilter === v} onclick={() => ((withFilter = v), (menu = null))}
                    >{l}</button
                  >
                {/each}
              {:else}
                <button role="menuitemradio" aria-checked={!keepFilter} onclick={() => ((keepFilter = null), (menu = null))}
                  >Any</button
                >
                {#each KEEPS as [v, l] (v)}
                  <button role="menuitemradio" aria-checked={keepFilter === v} onclick={() => ((keepFilter = v), (menu = null))}
                    >{l}</button
                  >
                {/each}
              {/if}
            </div>
          {/if}
        </div>
      {/each}
    </div>
  </div>

  {#if error}<p class="error" role="alert">{error}</p>{/if}
  {#if rules?.lastFailed && rules.lastApp}
    {@const app = rules.lastApp}
    <div class="banner" role="status">
      <span>
        Your last dictation, in {app.name}, did not paste.
        {hasRule(app.exe) ? 'Try another way to paste in its rule.' : 'Typing it out works in most apps that ignore the paste keys.'}
      </span>
      <button onclick={() => void addApp(app.exe, 'type')}>
        {hasRule(app.exe) ? 'Show its rule' : `Type it out in ${app.name}`}
      </button>
    </div>
  {/if}

  <div class="table" role="table" aria-label="Paste rules">
    <div class="tr th" role="row">
      {#each [['name', 'App'], ['with', 'Paste with'], ['keep', 'Keep on clipboard'], ['dictations', 'Dictations'], ['source', 'Source']] as const as [key, label] (key)}
        <button
          class="sort"
          class:active={sort.key === key}
          class:num={key === 'dictations'}
          role="columnheader"
          aria-sort={sort.key === key ? (sort.up ? 'ascending' : 'descending') : 'none'}
          onclick={() => setSort(key)}
        >
          {label}
          <svg viewBox="0 0 16 16" aria-hidden="true" class:up={sort.key === key && sort.up}
            ><path d="M4.5 6.5l3.5 3.5 3.5-3.5" /></svg
          >
        </button>
      {/each}
      <span role="columnheader" aria-label="Actions"></span>
    </div>
    <div class="tbody">
      {#each shown as g (g.apps.join())}
        {@const src = ruleSource(g)}
        <div class="tr" class:added={g.apps.some((a) => a.toLowerCase() === added)} data-app={g.apps[0]!.toLowerCase()} role="row">
          <div class="app" role="cell">
            <span class="name">{g.name}</span>
            <span class="exe">{g.apps.join(', ')}</span>
          </div>
          <div role="cell">
            <select
              aria-label="{g.name}: paste with"
              value={g.with}
              onchange={(e) => settings && save(withRule(settings.paste, g, { with: e.currentTarget.value as PasteWith, keep: g.keep }))}
            >
              {#each WITHS as [v, l] (v)}<option value={v}>{l}</option>{/each}
            </select>
          </div>
          <div role="cell">
            <select
              aria-label="{g.name}: keep on the clipboard"
              value={g.keep}
              onchange={(e) => settings && save(withRule(settings.paste, g, { with: g.with, keep: e.currentTarget.value as PasteKeep }))}
            >
              {#each KEEPS as [v, l] (v)}<option value={v}>{l}</option>{/each}
            </select>
          </div>
          <div class="num" role="cell">{g.dictations}</div>
          <div role="cell"><span class="source {src}">{SOURCE_LABELS[src]}</span></div>
          <div class="act" role="cell">
            {#if src === 'changed'}
              <button class="link" onclick={() => settings && save(withoutRule(settings.paste, g))}>Reset</button>
            {:else if src === 'yours'}
              <button class="remove" aria-label="Remove {g.name}" onclick={() => settings && save(withoutRule(settings.paste, g))}>
                <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" /></svg>
              </button>
            {/if}
          </div>
        </div>
      {:else}
        {#if rules}<p class="empty">No rules match.</p>{/if}
      {/each}
    </div>
  </div>
</div>

<style>
  .window {
    display: flex;
    flex-direction: column;
    height: 100%;
    padding: 28px 32px 0;
    box-sizing: border-box;
    background: radial-gradient(80% 45% at 20% -10%, var(--home-accent-soft) 0%, transparent 60%), var(--home-bg);
  }
  .head {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: 24px;
  }
  h1 {
    font: 600 24px/32px var(--font-display);
    letter-spacing: -0.01em;
    margin: 0;
  }
  .lede {
    color: var(--home-muted);
    margin: 2px 0 0;
  }
  .add-wrap {
    position: relative;
    flex: none;
  }
  .add {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    min-width: 0;
    padding: 0 16px;
  }
  .add svg {
    width: 14px;
    height: 14px;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
  }
  .picker {
    position: absolute;
    right: 0;
    top: calc(100% + 8px);
    width: 340px;
    padding: 10px;
    border-radius: 14px;
    background: var(--home-surface);
    border: 1px solid var(--home-border);
    box-shadow: var(--home-shadow);
    z-index: 20;
  }
  .choices {
    max-height: 300px;
    overflow-y: auto;
    margin: 6px -4px 0;
    padding: 0 4px;
  }
  .group {
    margin: 10px 8px 4px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--home-faint);
  }
  .choice {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
    height: 40px;
    padding: 0 8px;
    border: none;
    border-radius: 8px;
    background: none;
    text-align: left;
  }
  .choice-name {
    display: flex;
    flex-direction: column;
    min-width: 0;
    line-height: 17px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .choice-exe {
    font: 11px/14px 'Cascadia Mono', Consolas, monospace;
    color: var(--home-faint);
  }
  .choice-note {
    flex: none;
    font-size: 12px;
    color: var(--home-faint);
  }
  .none {
    margin: 14px 8px;
    color: var(--home-muted);
  }
  .browse {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    margin-top: 8px;
    padding: 0 10px;
    border-style: dashed;
    background: none;
    color: var(--home-muted);
  }
  .browse svg {
    width: 16px;
    height: 16px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linejoin: round;
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 14px;
    margin: 22px 0 14px;
  }
  .toolbar .search-field {
    width: 300px;
    flex: none;
  }
  .error {
    color: var(--home-error);
    margin: 0 0 10px;
  }

  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 10px 14px;
    margin: 0 0 10px;
    border-radius: 8px;
    background: var(--home-error-bg);
    color: var(--home-error);
  }

  .banner button {
    flex: none;
  }

  .table {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border: 1px solid var(--home-border);
    border-bottom: none;
    border-radius: 12px 12px 0 0;
    background: var(--home-surface);
    overflow: hidden;
  }
  .tbody {
    flex: 1;
    overflow-y: auto;
  }
  .tr {
    display: grid;
    grid-template-columns: minmax(180px, 1fr) 160px 150px 90px 92px 64px;
    align-items: center;
    gap: 8px;
    min-height: 52px;
    padding: 0 12px 0 18px;
    border-bottom: 1px solid var(--home-border);
    transition: background-color 120ms var(--ease-out);
  }
  .tbody .tr:hover {
    background: var(--home-hover);
  }
  .tr.added {
    animation: added 1.6s var(--ease-out);
  }
  @keyframes added {
    15% {
      background: var(--home-accent-soft);
    }
  }
  .th {
    min-height: 40px;
    background: var(--home-rail);
  }
  .sort {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 28px;
    padding: 0;
    border: none;
    background: none;
    font-size: 11.5px;
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--home-faint);
    justify-self: start;
  }
  .sort.num {
    justify-self: end;
  }
  .sort:hover:not(:disabled) {
    background: none;
    color: var(--home-muted);
  }
  .sort.active {
    color: var(--home-text);
  }
  .sort svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    stroke-linecap: round;
    opacity: 0;
    transition: transform 160ms var(--ease-out);
  }
  .sort.active svg {
    opacity: 1;
  }
  .sort svg.up {
    transform: rotate(180deg);
  }
  .app {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .exe {
    font: 11.5px/16px 'Cascadia Mono', Consolas, monospace;
    color: var(--home-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tr select {
    width: 100%;
    height: 32px;
    font-size: 13px;
    padding-left: 10px;
    background-color: transparent;
    border-color: transparent;
    transition:
      border-color 120ms var(--ease-out),
      background-color 120ms var(--ease-out);
  }
  .tr:hover select,
  .tr select:focus-visible {
    border-color: var(--home-border);
    background-color: var(--home-bg);
  }
  .num {
    text-align: right;
    font-variant-numeric: tabular-nums;
    color: var(--home-muted);
  }
  .source {
    display: inline-block;
    padding: 1px 8px;
    border-radius: 10px;
    font-size: 11.5px;
    line-height: 18px;
  }
  .source.default {
    color: var(--home-faint);
  }
  .source.changed {
    color: var(--home-accent);
    background: var(--home-accent-soft);
  }
  .source.yours {
    color: var(--home-ok);
    background: var(--home-ok-soft);
  }
  .act {
    display: flex;
    justify-content: flex-end;
  }
  .remove {
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    background: none;
    display: grid;
    place-items: center;
    color: var(--home-muted);
  }
  .remove svg {
    width: 14px;
    height: 14px;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
  }
  .empty {
    padding: 40px;
    text-align: center;
    color: var(--home-muted);
  }
</style>
