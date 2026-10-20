<script lang="ts">
  // The Dictionary section (docs/ux/home.md): words Utter should spell the person's way. Every
  // change saves at once through `save`; the core applies it on the next dictation.
  import { tick } from 'svelte';
  import { cubicIn, cubicOut } from 'svelte/easing';
  import { fly, slide } from 'svelte/transition';
  import { MAX_TERMS, validTerm, type Settings } from './core';

  let { settings, save }: { settings: Settings | null; save: (next: Settings) => Promise<void> } = $props();

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

  /** Focuses the add field (Cmd+F or Ctrl+F on this section). */
  export function focusAdd() {
    addInput?.focus();
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

  function autofocus(el: HTMLInputElement) {
    el.focus();
    el.select();
  }
</script>

<div class="page">
  <div class="column">
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
  </div>
</div>

<style>
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
    height: 38px;
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
    height: 38px;
    border: none;
    background: none;
    text-align: left;
    cursor: default;
    flex: 1;
    padding: 0 12px;
    line-height: 38px;
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
    color: var(--home-muted);
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
</style>
