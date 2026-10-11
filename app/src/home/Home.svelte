<script lang="ts">
  // The Home window (docs/ux/home.md): the dictionary and settings. Every change saves at once;
  // the core applies it on the next dictation and reports back (onApplied).
  import { onMount, tick } from 'svelte';
  import { cubicIn, cubicOut } from 'svelte/easing';
  import { fly, slide } from 'svelte/transition';
  import Logo from '../lib/Logo.svelte';
  import {
    MAX_TERMS,
    homeState,
    onApplied,
    onState,
    openPath,
    saveSettings,
    validTerm,
    type Applied,
    type HomeState,
    type Settings,
  } from './core';

  type Page = 'dictionary' | 'settings';
  const PAGES: { id: Page; label: string }[] = [
    { id: 'dictionary', label: 'Dictionary' },
    { id: 'settings', label: 'Settings' },
  ];

  let page = $state<Page>(loadPage());
  let home = $state<HomeState | null>(null);
  let settings = $state<Settings | null>(null);
  let saveError = $state('');
  // Per setting: "loading" while a model loads, or the error that kept the old value.
  let applied = $state<Partial<Record<Applied['what'], Applied>>>({});

  // Dictionary page.
  let draft = $state('');
  let message = $state('');
  let undo = $state<{ term: string; index: number } | null>(null);
  let messageTimer: ReturnType<typeof setTimeout> | undefined;
  let flashed = $state('');
  let editing = $state<number | null>(null);
  let editText = $state('');
  let addInput = $state<HTMLInputElement>();
  let fileInput = $state<HTMLInputElement>();

  const terms = $derived(settings?.dictionary ?? []);
  const filter = $derived(draft.includes(',') || draft.includes('\n') ? '' : draft.trim().toLowerCase());
  const shown = $derived(
    terms.map((t, i) => ({ t, i })).filter(({ t }) => !filter || t.toLowerCase().includes(filter)),
  );

  function loadPage(): Page {
    try {
      const p = localStorage.getItem('home.page');
      if (p === 'dictionary' || p === 'settings') return p;
    } catch {
      // Storage can be unavailable; the first page is fine.
    }
    return 'dictionary';
  }

  function go(p: Page) {
    page = p;
    try {
      localStorage.setItem('home.page', p);
    } catch {
      // Remembering the page is a convenience only.
    }
  }

  function take(s: HomeState) {
    home = s;
    if (s.settings) settings = s.settings;
    // A broken file opens Settings on its banner (docs/ux/home.md).
    if (s.error) page = 'settings';
  }

  async function save(next: Settings) {
    try {
      settings = await saveSettings(next);
      saveError = '';
    } catch (e) {
      saveError = String(e);
    }
  }

  function say(text: string, withUndo: { term: string; index: number } | null = null) {
    message = text;
    undo = withUndo;
    clearTimeout(messageTimer);
    messageTimer = setTimeout(() => {
      message = '';
      undo = null;
    }, 6000);
  }

  function flash(term: string) {
    flashed = '';
    void tick().then(() => (flashed = term));
    setTimeout(() => {
      if (flashed === term) flashed = '';
    }, 600);
  }

  /** Adds entries, newest first; returns how many were new and how many were already there. */
  function addTerms(raw: string[]): { added: number; existing: string[]; invalid: number } {
    if (!settings) return { added: 0, existing: [], invalid: 0 };
    const have = new Set(settings.dictionary);
    const fresh: string[] = [];
    const existing: string[] = [];
    let invalid = 0;
    for (const r of raw) {
      const t = r.trim();
      if (!t) continue;
      if (!validTerm(t)) invalid++;
      else if (have.has(t)) existing.push(t);
      else {
        have.add(t);
        fresh.push(t);
      }
    }
    const room = MAX_TERMS - settings.dictionary.length;
    const added = fresh.slice(0, Math.max(0, room));
    // Newest first, and several added at once keep the order they were typed in.
    if (added.length) void save({ ...settings, dictionary: [...added, ...settings.dictionary] });
    if (fresh.length > added.length) say(`The dictionary is full (${MAX_TERMS} words)`);
    return { added: added.length, existing, invalid };
  }

  function submitDraft() {
    const parts = draft.split(/[,\n]/);
    const { added, existing, invalid } = addTerms(parts);
    if (added === 0 && existing.length === 1 && invalid === 0) flash(existing[0]!);
    else if (parts.length > 1 && (added || existing.length)) {
      say(`Added ${added} ${added === 1 ? 'word' : 'words'}${existing.length ? ` (${existing.length} already here)` : ''}`);
    } else if (invalid) say('A word needs at least two letters or digits, and at most 64 characters');
    if (added || existing.length) draft = '';
  }

  function onPaste(e: ClipboardEvent) {
    const text = e.clipboardData?.getData('text') ?? '';
    if (text.includes('\n')) {
      e.preventDefault();
      draft = text;
      submitDraft();
    }
  }

  function remove(index: number) {
    if (!settings) return;
    const term = settings.dictionary[index]!;
    void save({ ...settings, dictionary: settings.dictionary.filter((_, i) => i !== index) });
    say(`Removed ${term}.`, { term, index });
  }

  function restore() {
    if (!settings || !undo) return;
    const d = [...settings.dictionary];
    if (!d.includes(undo.term)) d.splice(Math.min(undo.index, d.length), 0, undo.term);
    void save({ ...settings, dictionary: d });
    message = '';
    undo = null;
  }

  function startEdit(index: number) {
    editing = index;
    editText = terms[index] ?? '';
  }

  function commitEdit() {
    if (!settings || editing === null) return;
    const t = editText.trim();
    const index = editing;
    editing = null;
    if (t === settings.dictionary[index]) return;
    if (!validTerm(t)) return say('A word needs at least two letters or digits, and at most 64 characters');
    if (settings.dictionary.includes(t)) return flash(t);
    const d = [...settings.dictionary];
    d[index] = t;
    void save({ ...settings, dictionary: d });
  }

  async function importFile(e: Event) {
    const file = (e.currentTarget as HTMLInputElement).files?.[0];
    if (!file) return;
    const text = await file.text();
    // A CSV's first column; a quoted cell loses its quotes.
    const lines = text.split(/\r?\n/).map((l) =>
      file.name.toLowerCase().endsWith('.csv') ? (l.split(',')[0] ?? '').replace(/^"|"$/g, '') : l,
    );
    const { added, existing } = addTerms(lines);
    say(`Added ${added} ${added === 1 ? 'word' : 'words'}${existing.length ? ` (${existing.length} were already here)` : ''}`);
    (e.currentTarget as HTMLInputElement).value = '';
  }

  function rowKey(e: KeyboardEvent, index: number) {
    if (e.key === 'Enter') startEdit(index);
    else if (e.key === 'Delete') remove(index);
  }

  function onKey(e: KeyboardEvent) {
    if (e.ctrlKey && (e.key === '1' || e.key === '2')) {
      e.preventDefault();
      go(e.key === '1' ? 'dictionary' : 'settings');
    } else if (e.ctrlKey && e.key.toLowerCase() === 'f' && page === 'dictionary') {
      e.preventDefault();
      addInput?.focus();
    }
  }

  function sidebarKey(e: KeyboardEvent) {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    e.preventDefault();
    const i = PAGES.findIndex((p) => p.id === page);
    const next = PAGES[(i + (e.key === 'ArrowDown' ? 1 : PAGES.length - 1)) % PAGES.length]!;
    go(next.id);
    void tick().then(() => document.getElementById(`nav-${next.id}`)?.focus());
  }

  function autofocus(el: HTMLInputElement) {
    el.focus();
    el.select();
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
  <nav class="sidebar" aria-label="Pages">
    <div class="brand">
      <Logo size={22} />
      Utter
    </div>
    <ul>
      {#each PAGES as p (p.id)}
        <li>
          <a
            id="nav-{p.id}"
            href="#{p.id}"
            class:selected={page === p.id}
            aria-current={page === p.id ? 'page' : undefined}
            onclick={(e) => {
              e.preventDefault();
              go(p.id);
            }}
            onkeydown={sidebarKey}
          >
            {#if p.id === 'dictionary'}
              <svg viewBox="0 0 20 20" aria-hidden="true"
                ><path d="M4 3.5h9.5A2.5 2.5 0 0 1 16 6v10.5H6.5A2.5 2.5 0 0 1 4 14z" /><path d="M7 7.5h6M7 10.5h4" /></svg
              >
            {:else}
              <svg viewBox="0 0 20 20" aria-hidden="true"
                ><path d="M3.5 6h8M14.5 6h2M3.5 14h2M8.5 14h8" /><circle cx="13" cy="6" r="1.75" /><circle
                  cx="7"
                  cy="14"
                  r="1.75"
                /></svg
              >
            {/if}
            {p.label}
          </a>
        </li>
      {/each}
    </ul>
  </nav>

  <main>
    <div class="column">
      {#if page === 'dictionary'}
        <h1>Dictionary</h1>
        <p class="lede">Words Utter should spell your way: names, products, terms.</p>

        <div class="add-row">
          <input
            class="add"
            bind:this={addInput}
            bind:value={draft}
            placeholder="Add a word or name"
            aria-label="Add a word or name"
            disabled={!settings}
            onkeydown={(e) => e.key === 'Enter' && submitDraft()}
            onpaste={onPaste}
          />
          <button class="primary" disabled={!settings || !draft.trim()} onclick={submitDraft}>Add</button>
        </div>
        <p class="message" aria-live="polite">
          {#if message}
            {message}
            {#if undo}<button class="link" onclick={restore}>Undo</button>{/if}
          {/if}
        </p>

        <div class="list-head">
          <span>{terms.length} {terms.length === 1 ? 'word' : 'words'}</span>
          <button class="link" onclick={() => fileInput?.click()} disabled={!settings}>Import</button>
          <input type="file" accept=".txt,.csv,text/plain,text/csv" bind:this={fileInput} onchange={importFile} hidden />
        </div>

        {#if terms.length === 0}
          <p class="empty">
            No words yet. Add names and terms Utter mishears, like your company, your teammates or the tools you use.
          </p>
        {:else}
          <ul class="terms">
            {#each shown as { t, i } (t)}
              <li
                class:flash={flashed === t}
                in:fly|local={{ y: -4, duration: 120, easing: cubicOut }}
                out:slide|local={{ duration: 160, easing: cubicIn }}
              >
                {#if editing === i}
                  <input
                    class="edit"
                    bind:value={editText}
                    aria-label="Edit {t}"
                    use:autofocus
                    onkeydown={(e) => {
                      if (e.key === 'Enter') commitEdit();
                      else if (e.key === 'Escape') editing = null;
                    }}
                    onblur={commitEdit}
                  />
                {:else}
                  <button
                    class="term"
                    title="Double-click or Enter to edit"
                    ondblclick={() => startEdit(i)}
                    onkeydown={(e) => rowKey(e, i)}>{t}</button
                  >
                  <button class="remove" aria-label="Remove {t}" onclick={() => remove(i)}>
                    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" /></svg>
                  </button>
                {/if}
              </li>
            {/each}
          </ul>
        {/if}
      {:else}
        <h1>Settings</h1>
        <p class="lede">Changes apply to your next dictation.</p>

        {#if home?.error}
          <div class="banner" role="alert">
            <span>{home.error}</span>
            <button onclick={() => openPath('settings')}>Open file</button>
          </div>
        {/if}
        {#if saveError}<div class="banner" role="alert"><span>{saveError}</span></div>{/if}

        {#if settings && home}
          <h2>General</h2>
          <div class="row">
            <div class="label">
              <label for="login">Start with Windows</label>
              <span class="desc">Utter starts in the tray when you sign in.</span>
            </div>
            <button
              id="login"
              class="switch"
              role="switch"
              aria-checked={settings.start_at_login}
              aria-label="Start with Windows"
              onclick={() => settings && save({ ...settings, start_at_login: !settings.start_at_login })}
              ><span></span></button
            >
          </div>

          <h2>Dictation</h2>
          <div class="row">
            <div class="label">
              <label for="lists">Format lists</label>
              <span class="desc">Spoken lists become bullet points.</span>
            </div>
            <button
              id="lists"
              class="switch"
              role="switch"
              aria-checked={settings.lists}
              aria-label="Format lists"
              onclick={() => settings && save({ ...settings, lists: !settings.lists })}><span></span></button
            >
          </div>
          <div class="row">
            <div class="label">
              <label for="mic">Microphone</label>
              <span class="desc">Utter listens to this one.</span>
              {#if applied.microphone?.status === 'error'}<span class="row-error">{applied.microphone.error}</span>{/if}
            </div>
            <select
              id="mic"
              value={settings.microphone}
              onchange={(e) => settings && save({ ...settings, microphone: e.currentTarget.value })}
            >
              <option value="">System default</option>
              {#each home.microphones as m (m)}<option value={m}>{m}</option>{/each}
              {#if settings.microphone && !home.microphones.includes(settings.microphone)}
                <option value={settings.microphone}>{settings.microphone} (not connected)</option>
              {/if}
            </select>
          </div>

          <h2>Cleanup model</h2>
          <div class="row">
            <div class="label">
              <span class="label-text" id="model-label">Model</span>
              <span class="desc">
                {settings.model === 'light'
                  ? 'Fastest. Runs on most GPUs (about 6 GB).'
                  : 'Slower and larger (about 18 GB of GPU memory).'}
              </span>
              {#if applied.model?.status === 'loading'}
                <span class="row-status"><span class="spinner" aria-hidden="true"></span>Loading...</span>
              {:else if applied.model?.status === 'error'}
                <span class="row-error">{applied.model.error}</span>
              {/if}
            </div>
            <div class="segmented" role="radiogroup" aria-labelledby="model-label">
              <button
                role="radio"
                aria-checked={settings.model === 'light'}
                onclick={() => settings && save({ ...settings, model: 'light' })}>Light</button
              >
              <button
                role="radio"
                aria-checked={settings.model === 'quality'}
                disabled={!home.qualityInstalled}
                title={home.qualityInstalled ? undefined : 'Not installed'}
                onclick={() => settings && save({ ...settings, model: 'quality' })}>Quality</button
              >
            </div>
          </div>
          {#if !home.qualityInstalled}<p class="hint">Quality: not installed.</p>{/if}

          <h2>Shortcut</h2>
          <div class="row">
            <div class="label">
              <span class="label-text">Hold to talk</span>
              <span class="desc">Hold to dictate. Double-tap for hands-free; Esc cancels.</span>
            </div>
            <kbd>Right Ctrl</kbd>
          </div>
        {/if}

        {#if home}
          <h2>About</h2>
          <div class="row">
            <div class="label"><span class="label-text">Version</span></div>
            <span class="value">{home.version}</span>
          </div>
          <div class="row">
            <div class="label">
              <span class="label-text">Data folder</span>
              <span class="desc path">{home.dataDir}</span>
            </div>
            <button onclick={() => openPath('data')}>Open</button>
          </div>
          <div class="row">
            <div class="label">
              <span class="label-text">Settings file</span>
              <span class="desc path">{home.settingsPath}</span>
            </div>
            <button onclick={() => openPath('settings')}>Open</button>
          </div>
        {/if}
      {/if}
    </div>
  </main>
</div>

<style>
  .frame {
    display: flex;
    height: 100%;
  }

  .sidebar {
    width: 200px;
    flex: none;
    background: var(--home-sidebar);
    border-right: 1px solid var(--home-border);
    padding: 16px 8px;
    box-sizing: border-box;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
    font-size: 15px;
    padding: 4px 12px 20px;
  }
  .sidebar ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .sidebar a {
    position: relative;
    display: flex;
    align-items: center;
    gap: 10px;
    height: 36px;
    padding: 0 12px;
    border-radius: 6px;
    color: inherit;
    text-decoration: none;
  }
  .sidebar a svg {
    width: 18px;
    height: 18px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .sidebar a:hover {
    background: var(--home-hover);
  }
  .sidebar a.selected {
    background: var(--home-selected);
    font-weight: 600;
  }
  .sidebar a.selected::before {
    content: '';
    position: absolute;
    left: 0;
    top: 10px;
    bottom: 10px;
    width: 3px;
    border-radius: 2px;
    background: var(--home-accent);
  }

  main {
    flex: 1;
    overflow-y: auto;
  }
  .column {
    max-width: 640px;
    padding: 32px;
  }
  h1 {
    font-size: 24px;
    line-height: 32px;
    font-weight: 600;
    margin: 0;
  }
  .lede {
    color: var(--home-muted);
    margin: 4px 0 24px;
  }
  h2 {
    font-size: 13px;
    line-height: 16px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--home-muted);
    margin: 28px 0 4px;
  }

  .add-row {
    display: flex;
    gap: 8px;
  }
  .add {
    flex: 1;
    min-width: 0;
  }
  .add-row button {
    min-width: 72px;
  }
  .message {
    min-height: 20px;
    margin: 6px 0 10px;
    color: var(--home-muted);
  }

  /* Text in the field, the count and the rows share one left edge (12 px in). */
  .list-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    color: var(--home-muted);
    padding: 0 12px 6px;
    border-bottom: 1px solid var(--home-border);
  }
  .empty {
    color: var(--home-muted);
    margin: 24px 0;
  }
  .terms {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .terms li {
    display: flex;
    align-items: center;
    height: 36px;
    padding-right: 4px;
    border-bottom: 1px solid var(--home-border);
    transition: background-color 120ms var(--ease-out);
  }
  .terms li:hover {
    background: var(--home-hover);
  }
  .terms li.flash {
    animation: flash var(--dur-flash) var(--ease-out);
  }
  .term {
    height: 36px;
    border: none;
    background: none;
    text-align: left;
    cursor: default;
    flex: 1;
    padding: 0 12px;
    line-height: 36px;
    border-radius: 0;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* The row carries the hover; the term itself stays flat. */
  .term:hover:not(:disabled) {
    background: none;
  }
  /* Editing keeps the text where it was: 12 px in, less the field's border. */
  .edit {
    flex: 1;
    height: 30px;
    margin-left: 1px;
    padding: 0 10px;
  }
  .remove {
    width: 28px;
    height: 28px;
    padding: 0;
    border: none;
    background: none;
    opacity: 0;
    display: grid;
    place-items: center;
  }
  .remove svg {
    width: 14px;
    height: 14px;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
  }
  .terms li:hover .remove,
  .terms li:focus-within .remove {
    opacity: 1;
  }
  @keyframes flash {
    30% {
      background: var(--home-accent-soft);
    }
  }

  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 24px;
    min-height: 56px;
    padding: 8px 0;
    border-bottom: 1px solid var(--home-border);
  }
  .label {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .desc {
    color: var(--home-muted);
    font-size: 13px;
    line-height: 18px;
  }
  .path {
    font-family: 'Cascadia Mono', Consolas, monospace;
    font-size: 12px;
    overflow-wrap: anywhere;
  }
  .row-error {
    color: var(--home-error);
    font-size: 13px;
  }
  .row-status {
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--home-muted);
    font-size: 13px;
  }
  .spinner {
    width: 10px;
    height: 10px;
    border: 2px solid var(--home-border);
    border-top-color: var(--home-accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .hint {
    color: var(--home-muted);
    font-size: 13px;
    margin: 4px 0 0;
  }
  select {
    max-width: 280px;
  }
  .value {
    color: var(--home-muted);
  }
  kbd {
    font: 600 12px var(--font-ui);
    padding: 4px 8px;
    border: 1px solid var(--home-border);
    border-bottom-width: 2px;
    border-radius: 5px;
    background: var(--home-surface);
  }

  .switch {
    position: relative;
    flex: none;
    width: 40px;
    height: 22px;
    padding: 0;
    border-radius: 11px;
    border: 1px solid var(--home-border);
    background: var(--home-selected);
  }
  .switch span {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--home-muted);
    transition: transform var(--dur-resize) var(--ease-out);
  }
  .switch[aria-checked='true'] {
    background: var(--home-accent);
    border-color: var(--home-accent);
  }
  .switch[aria-checked='true'] span {
    transform: translateX(18px);
    background: var(--home-accent-text);
  }

  .segmented {
    display: flex;
    flex: none;
    border: 1px solid var(--home-border);
    border-radius: 6px;
    overflow: hidden;
  }
  .segmented button {
    border: none;
    border-radius: 0;
    height: 30px;
  }
  .segmented button + button {
    border-left: 1px solid var(--home-border);
  }
  .segmented button[aria-checked='true'] {
    background: var(--home-accent);
    color: var(--home-accent-text);
  }

  .banner {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 10px 14px;
    margin-bottom: 8px;
    border-radius: 6px;
    background: var(--home-error-bg);
    color: var(--home-error);
  }
</style>
