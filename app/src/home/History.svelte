<script lang="ts">
  // The History section (docs/ux/history.md): every dictation, newest first, beside the selected
  // one in full. The core filters and searches (history.rs); this page asks a page at a time.
  import { onMount, tick } from 'svelte';
  import { cubicOut } from 'svelte/easing';
  import { fade, fly } from 'svelte/transition';
  import AnimatedMark from '../lib/AnimatedMark.svelte';
  import '../lib/chips.css';
  import {
    historyChanges,
    historyCopy,
    historyDelete,
    historyFavorite,
    historyQuery,
    historyRestore,
    onHistoryChanged,
    type Change,
    type HistoryEntry,
    type HistoryPage,
    type HistoryQuery,
  } from './core';

  let { historyOff = false }: { historyOff?: boolean } = $props();

  const PAGE = 60;
  type Status = 'all' | 'inserted' | 'not-inserted';
  type Time = 'all' | 'today' | 'yesterday' | 'week' | 'month';
  const TIMES: [Time, string][] = [
    ['all', 'All time'],
    ['today', 'Today'],
    ['yesterday', 'Yesterday'],
    ['week', 'Last 7 days'],
    ['month', 'Last 30 days'],
  ];
  const STATUSES: [Status, string][] = [
    ['all', 'All'],
    ['inserted', 'Inserted'],
    ['not-inserted', 'Not inserted'],
  ];
  const REASONS: Record<string, string> = {
    'focus-moved': 'The focus moved to another window before it was ready.',
    'no-window': 'No window had the focus.',
    refused: 'The app runs as administrator, so Windows would not let Utter type into it.',
    failed: 'The paste did not go through.',
  };

  let search = $state('');
  let favorites = $state(false);
  let status = $state<Status>('all');
  let app = $state<string | null>(null);
  let time = $state<Time>('all');
  let menu = $state<'status' | 'app' | 'time' | null>(null);

  let page = $state<HistoryPage | null>(null);
  let entries = $state<HistoryEntry[]>([]);
  let selectedId = $state<number | null>(null);
  let loading = false;
  // Ids already shown, so only new arrivals fade in.
  let seen = new Set<number>();
  let fresh = $state(new Set<number>());

  let changes = $state<Change[]>([]);
  let showChanges = $state(true);
  let copied = $state<'text' | 'heard' | null>(null);
  let undo = $state<HistoryEntry | null>(null);
  let undoTimer: ReturnType<typeof setTimeout> | undefined;

  let searchEl = $state<HTMLInputElement>();
  let listEl = $state<HTMLDivElement>();
  let sentinel = $state<HTMLDivElement>();
  // The selection rule: one element that glides to the selected row and glints as it lands.
  let rule = $state({ top: 0, height: 0, shown: false });
  let ruleEl = $state<HTMLSpanElement>();
  const reduceMotion = matchMedia('(prefers-reduced-motion: reduce)').matches;

  function glint() {
    if (reduceMotion || !ruleEl) return;
    ruleEl.animate(
      [
        { boxShadow: '0 0 0 0 transparent', filter: 'brightness(1)' },
        { boxShadow: '0 0 10px 1px var(--home-glint)', filter: 'brightness(1.35)', offset: 0.35 },
        { boxShadow: '0 0 0 0 transparent', filter: 'brightness(1)' },
      ],
      { duration: 700, easing: 'cubic-bezier(0.2, 0, 0, 1)' },
    );
  }

  const selected = $derived(entries.find((e) => e.id === selectedId) ?? null);
  const filtering = $derived(!!search.trim() || favorites || status !== 'all' || !!app || time !== 'all');
  const appName = $derived(page?.apps.find((a) => a.exe === app)?.name ?? '');

  function bounds(t: Time): { since: number | null; until: number | null } {
    const midnight = new Date();
    midnight.setHours(0, 0, 0, 0);
    const m = midnight.getTime();
    const day = 24 * 3600 * 1000;
    switch (t) {
      case 'today':
        return { since: m, until: null };
      case 'yesterday':
        return { since: m - day, until: m };
      case 'week':
        return { since: Date.now() - 7 * day, until: null };
      case 'month':
        return { since: Date.now() - 30 * day, until: null };
      default:
        return { since: null, until: null };
    }
  }

  function query(offset: number): HistoryQuery {
    return {
      search: search.trim(),
      favorites,
      inserted: status === 'all' ? null : status === 'inserted',
      app,
      ...bounds(time),
      offset,
      limit: PAGE,
    };
  }

  /** Loads the first page again (a filter changed, or a dictation arrived). */
  async function reload() {
    const p = await historyQuery(query(0));
    page = p;
    const arrived = p.entries.filter((e) => seen.size > 0 && !seen.has(e.id)).map((e) => e.id);
    fresh = new Set(arrived);
    p.entries.forEach((e) => seen.add(e.id));
    const wasFirst = selectedId === null || selectedId === entries[0]?.id;
    entries = p.entries;
    // The selection stays where it was, unless it was on the first row or is gone.
    if (wasFirst || !entries.some((e) => e.id === selectedId)) selectedId = entries[0]?.id ?? null;
    await placeRule();
  }

  async function more() {
    if (loading || !page || entries.length >= page.matched) return;
    loading = true;
    const p = await historyQuery(query(entries.length));
    p.entries.forEach((e) => seen.add(e.id));
    entries = [...entries, ...p.entries.filter((e) => !entries.some((x) => x.id === e.id))];
    loading = false;
  }

  // Filters and search apply as they change; typing waits a breath.
  let searchTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    void [favorites, status, app, time];
    void reload();
  });
  function onSearch() {
    clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void reload(), 90);
  }

  function clearFilters() {
    search = '';
    favorites = false;
    status = 'all';
    app = null;
    time = 'all';
  }

  // What Utter heard, for the selected entry.
  $effect(() => {
    const e = selected;
    changes = [];
    if (e && e.heard && e.heard !== e.text) {
      void historyChanges(e.heard, e.text).then((c) => {
        if (selectedId === e.id) changes = c;
      });
    }
  });

  async function placeRule() {
    await tick();
    const row = listEl?.querySelector<HTMLElement>(`[data-id="${selectedId}"]`);
    if (!row) {
      rule = { ...rule, shown: false };
      return;
    }
    rule = { top: row.offsetTop + 10, height: row.offsetHeight - 20, shown: true };
  }

  async function select(id: number, scroll = false) {
    if (id === selectedId) return;
    selectedId = id;
    copied = null;
    await placeRule();
    glint();
    if (scroll) listEl?.querySelector(`[data-id="${id}"]`)?.scrollIntoView({ block: 'nearest' });
  }

  function step(by: number) {
    const i = entries.findIndex((e) => e.id === selectedId);
    const next = entries[Math.max(0, Math.min(entries.length - 1, i + by))];
    if (next) void select(next.id, true);
    if (i + by >= entries.length - 5) void more();
  }

  async function copy(what: 'text' | 'heard') {
    if (!selected) return;
    await historyCopy(what === 'text' ? selected.text : selected.heard);
    copied = what;
    setTimeout(() => {
      if (copied === what) copied = null;
    }, 1200);
  }

  async function toggleFavorite() {
    const e = selected;
    if (!e) return;
    await historyFavorite(e.id, !e.favorite);
    entries = entries.map((x) => (x.id === e.id ? { ...x, favorite: !e.favorite } : x));
    if (favorites && e.favorite) void reload();
  }

  async function remove() {
    const e = selected;
    if (!e) return;
    const i = entries.findIndex((x) => x.id === e.id);
    const gone = await historyDelete(e.id);
    entries = entries.filter((x) => x.id !== e.id);
    selectedId = (entries[i] ?? entries[i - 1])?.id ?? null;
    if (page) page = { ...page, matched: page.matched - 1, total: page.total - 1 };
    undo = gone;
    clearTimeout(undoTimer);
    undoTimer = setTimeout(() => (undo = null), 6000);
    await placeRule();
    glint();
  }

  async function restore() {
    if (!undo) return;
    const e = undo;
    undo = null;
    await historyRestore(e);
    await reload();
    await select(e.id, true);
  }

  function typing(e: KeyboardEvent) {
    const t = e.target as HTMLElement;
    return t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable;
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      if (menu) menu = null;
      else if (document.activeElement === searchEl && search) {
        search = '';
        void reload();
      }
      return;
    }
    if ((e.ctrlKey && e.key.toLowerCase() === 'f') || (e.key === '/' && !typing(e))) {
      e.preventDefault();
      searchEl?.focus();
      searchEl?.select();
      return;
    }
    if (e.ctrlKey && e.key.toLowerCase() === 'z' && undo && !typing(e)) {
      e.preventDefault();
      void restore();
      return;
    }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      if (typing(e) && e.target !== searchEl) return;
      e.preventDefault();
      step(e.key === 'ArrowDown' ? 1 : -1);
      return;
    }
    if (typing(e) || e.ctrlKey || e.altKey || e.metaKey || window.getSelection()?.toString()) return;
    const k = e.key.toLowerCase();
    if (k === 'c') void copy('text');
    else if (k === 'f') void toggleFavorite();
    else if (k === 'd' || e.key === 'Delete') void remove();
  }

  // Dates ------------------------------------------------------------------------------------

  const timeOf = (at: number) => new Date(at).toLocaleTimeString([], { hour: 'numeric', minute: '2-digit' });

  function dayOf(at: number): string {
    const d = new Date(at);
    const today = new Date();
    today.setHours(0, 0, 0, 0);
    const day = new Date(d);
    day.setHours(0, 0, 0, 0);
    const ago = Math.round((today.getTime() - day.getTime()) / (24 * 3600 * 1000));
    if (ago === 0) return 'Today';
    if (ago === 1) return 'Yesterday';
    if (ago < 7) return d.toLocaleDateString([], { weekday: 'long' });
    return d.toLocaleDateString([], {
      month: 'short',
      day: 'numeric',
      year: d.getFullYear() === today.getFullYear() ? undefined : 'numeric',
    });
  }

  function fullDate(at: number): string {
    const day = dayOf(at);
    const t = timeOf(at);
    if (day === 'Today' || day === 'Yesterday') return `${day} at ${t}`;
    return `${new Date(at).toLocaleDateString([], { weekday: 'long', month: 'long', day: 'numeric' })} at ${t}`;
  }

  /** The list, with a day header before the first entry of each day. */
  const rows = $derived.by(() => {
    const out: ({ kind: 'day'; label: string } | { kind: 'entry'; entry: HistoryEntry })[] = [];
    let last = '';
    for (const e of entries) {
      const d = dayOf(e.at);
      if (d !== last) out.push({ kind: 'day', label: d });
      last = d;
      out.push({ kind: 'entry', entry: e });
    }
    return out;
  });

  /** The text split around search matches, for highlighting. */
  function marks(text: string): { t: string; hit: boolean }[] {
    const q = search.trim();
    if (!q) return [{ t: text, hit: false }];
    const fold = (s: string) =>
      s
        .normalize('NFD')
        .replace(/\p{M}/gu, '')
        .toLowerCase();
    // Folding keeps one character per character for Latin text, so indexes line up.
    const hay = fold(text);
    const needle = fold(q);
    if (hay.length !== text.length) return [{ t: text, hit: false }];
    const out: { t: string; hit: boolean }[] = [];
    let i = 0;
    for (let at = hay.indexOf(needle); at >= 0 && needle; at = hay.indexOf(needle, i)) {
      if (at > i) out.push({ t: text.slice(i, at), hit: false });
      out.push({ t: text.slice(at, at + needle.length), hit: true });
      i = at + needle.length;
    }
    if (i < text.length) out.push({ t: text.slice(i), hit: false });
    return out;
  }

  onMount(() => {
    const off = onHistoryChanged(() => void reload());
    const io = new IntersectionObserver((seen) => seen.some((s) => s.isIntersecting) && void more(), {
      root: listEl,
      rootMargin: '400px',
    });
    if (sentinel) io.observe(sentinel);
    const ro = new ResizeObserver(() => void placeRule());
    if (listEl) ro.observe(listEl);
    return () => {
      io.disconnect();
      ro.disconnect();
      void off.then((f) => f());
    };
  });
</script>

<svelte:window onkeydown={onKey} onclick={() => (menu = null)} />

<div class="history">
  <section class="list-col" aria-label="Dictations">
    <div class="top">
      <label class="search-field">
        <svg viewBox="0 0 20 20" aria-hidden="true"><circle cx="9" cy="9" r="5.5" /><path d="M13.2 13.2L17 17" /></svg>
        <input
          bind:this={searchEl}
          bind:value={search}
          oninput={onSearch}
          placeholder="Search dictations"
          aria-label="Search dictations"
          spellcheck="false"
        />
        {#if page}
          <span class="count">
            {#if filtering}{page.matched.toLocaleString()} of {page.total.toLocaleString()}{:else}{page.total.toLocaleString()}{/if}
          </span>
        {/if}
      </label>

      <div class="chips" role="toolbar" aria-label="Filters">
        <button
          class="chip star"
          class:on={favorites}
          aria-pressed={favorites}
          title="Favorites only"
          onclick={() => (favorites = !favorites)}
        >
          <svg viewBox="0 0 20 20" aria-hidden="true"
            ><path d="M10 3.2l2.05 4.3 4.7.6-3.45 3.25.88 4.65L10 13.75 5.82 16l.88-4.65L3.25 8.1l4.7-.6z" /></svg
          >
        </button>
        {#each [['status', status !== 'all', status === 'not-inserted' ? 'Not inserted' : status === 'inserted' ? 'Inserted' : 'Status'], ['app', !!app, app ? appName : 'App'], ['time', time !== 'all', time === 'all' ? 'Time' : TIMES.find(([t]) => t === time)![1]]] as const as [which, on, label] (which)}
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
              {#if which === 'status' && !on && page && page.notInserted > 0}<span class="dot" aria-hidden="true"></span>{/if}
              {label}
              {#if on}
                <span
                  class="clear"
                  role="button"
                  tabindex="-1"
                  aria-label="Clear {which}"
                  onclick={(e) => {
                    e.stopPropagation();
                    if (which === 'status') status = 'all';
                    else if (which === 'app') app = null;
                    else time = 'all';
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
                {#if which === 'status'}
                  {#each STATUSES as [s, l] (s)}
                    <button role="menuitemradio" aria-checked={status === s} onclick={() => ((status = s), (menu = null))}>
                      {l}
                      {#if s === 'not-inserted' && page?.notInserted}<span class="n">{page.notInserted}</span>{/if}
                    </button>
                  {/each}
                {:else if which === 'app'}
                  <button role="menuitemradio" aria-checked={app === null} onclick={() => ((app = null), (menu = null))}
                    >All apps</button
                  >
                  {#each page?.apps ?? [] as a (a.exe)}
                    <button role="menuitemradio" aria-checked={app === a.exe} onclick={() => ((app = a.exe), (menu = null))}>
                      {a.name}<span class="n">{a.count}</span>
                    </button>
                  {/each}
                {:else}
                  {#each TIMES as [t, l] (t)}
                    <button role="menuitemradio" aria-checked={time === t} onclick={() => ((time = t), (menu = null))}>{l}</button>
                  {/each}
                {/if}
              </div>
            {/if}
          </div>
        {/each}
      </div>
    </div>

    <div class="scroll" bind:this={listEl} role="listbox" aria-label="Dictations" tabindex="-1">
      <span
        class="rule"
        class:shown={rule.shown}
        bind:this={ruleEl}
        style:transform="translateY({rule.top}px)"
        style:height="{rule.height}px"
        aria-hidden="true"
      ></span>
      {#each rows as r (r.kind === 'day' ? `day-${r.label}` : r.entry.id)}
        {#if r.kind === 'day'}
          <div class="day">{r.label}</div>
        {:else}
          {@const e = r.entry}
          <div
            class="row"
            class:selected={e.id === selectedId}
            class:missed={!e.inserted}
            data-id={e.id}
            role="option"
            tabindex="-1"
            aria-selected={e.id === selectedId}
            onclick={() => void select(e.id)}
            onkeydown={() => {}}
            in:fly={{ y: fresh.has(e.id) ? -8 : 0, duration: fresh.has(e.id) ? 260 : 0, easing: cubicOut }}
          >
            <p class="text">
              {#each marks(e.text) as m, i (i)}{#if m.hit}<mark>{m.t}</mark>{:else}{m.t}{/if}{/each}
            </p>
            <p class="meta">
              {#if !e.inserted}<span class="missed-dot" title="Not inserted"></span>{/if}
              <span class="app">{e.appName || 'Unknown app'}</span>
              <span>·</span>
              <span>{timeOf(e.at)}</span>
              {#if e.favorite}
                <svg class="fav" viewBox="0 0 20 20" aria-label="Favorite"
                  ><path d="M10 3.2l2.05 4.3 4.7.6-3.45 3.25.88 4.65L10 13.75 5.82 16l.88-4.65L3.25 8.1l4.7-.6z" /></svg
                >
              {/if}
            </p>
          </div>
        {/if}
      {/each}
      <div class="sentinel" bind:this={sentinel}></div>

      {#if page && entries.length === 0}
        <div class="empty" in:fade={{ duration: 160 }}>
          {#if historyOff}
            <p>History is off. Turn it on in Settings to keep your dictations here.</p>
          {:else if page.total === 0}
            <p>Nothing here yet.</p>
          {:else}
            <p>No dictations match.</p>
            <button class="link" onclick={clearFilters}>Clear filters</button>
          {/if}
        </div>
      {/if}
    </div>
  </section>

  <section class="entry-col" aria-label="Selected dictation">
    {#if selected}
      {#key selected.id}
        <article class="entry" in:fade={{ duration: 140 }}>
          <header>
            <span class="entry-app">{selected.appName || 'Unknown app'}</span>
            <span>{fullDate(selected.at)}</span>
            <span class="stats">
              {selected.words}
              {selected.words === 1 ? 'word' : 'words'} · {selected.speechS.toFixed(1)} s of speech · ready in {Math.round(
                selected.readyMs,
              )} ms
            </span>
          </header>
          {#if !selected.inserted}
            <p class="missed-line">
              <span class="missed-dot"></span>Not inserted. {REASONS[selected.notInserted ?? 'failed']} It was left on the
              clipboard.
            </p>
          {/if}
          <p class="body">{selected.text}</p>

          {#if changes.length}
            <div class="heard">
              <div class="heard-head">
                <h3>What Utter heard</h3>
                <div class="toggle" role="radiogroup" aria-label="Show">
                  <button role="radio" aria-checked={showChanges} onclick={() => (showChanges = true)}>Changes</button>
                  <button role="radio" aria-checked={!showChanges} onclick={() => (showChanges = false)}>Plain</button>
                </div>
              </div>
              <p class="heard-text">
                {#if showChanges}
                  {#each changes as c, i (i)}{#if i > 0}{' '}{/if}<span class={c.kind}>{c.text}</span>{/each}
                {:else}
                  {selected.heard}
                {/if}
              </p>
            </div>
          {/if}
        </article>
      {/key}
      <footer class="actions">
        <span class="buttons">
          {#if undo}
            <span class="undo" transition:fade={{ duration: 120 }}
              >Deleted. <button class="link" onclick={() => void restore()}>Undo</button></span
            >
          {/if}
          {#if changes.length}
            <button class="action" onclick={() => void copy('heard')}>{copied === 'heard' ? 'Copied' : 'Copy what was heard'}</button>
          {/if}
          <button class="action" onclick={() => void copy('text')}>{copied === 'text' ? 'Copied' : 'Copy'} <kbd>C</kbd></button>
          <button class="action" onclick={() => void toggleFavorite()}
            >{selected.favorite ? 'Unfavorite' : 'Favorite'} <kbd>F</kbd></button
          >
          <button class="action" onclick={() => void remove()}>Delete <kbd>D</kbd></button>
        </span>
      </footer>
    {:else if page && page.total === 0 && !historyOff}
      <div class="welcome" in:fade={{ duration: 200 }}>
        <div class="welcome-mark"><AnimatedMark size={132} color="var(--home-accent)" /></div>
        <p>Hold <kbd>Right Ctrl</kbd> and speak in any app.<br />Your dictations show up here.</p>
      </div>
    {:else if undo}
      <footer class="actions">
        <span></span>
        <span class="undo">Deleted. <button class="link" onclick={() => void restore()}>Undo</button></span>
      </footer>
    {/if}
  </section>
</div>

<style>
  .history {
    display: grid;
    grid-template-columns: minmax(300px, 380px) minmax(0, 1fr);
    height: 100%;
    overflow: hidden;
  }

  /* The list ------------------------------------------------------------------------------- */
  .list-col {
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    border-right: 1px solid var(--home-border);
  }
  .top {
    padding: 16px 16px 8px;
  }
  .chips {
    flex-wrap: nowrap;
    margin-top: 10px;
  }
  .chip.star {
    width: 28px;
    padding: 0;
    justify-content: center;
  }
  .chip.star.on svg {
    fill: currentColor;
  }

  .scroll {
    position: relative;
    flex: 1;
    overflow-y: auto;
    padding: 0 8px 16px;
    outline: none;
  }
  /* The selection rule: one element that glides between rows, glinting as it lands, like the
     mark's write-head (lib/AnimatedMark.svelte). */
  .rule {
    position: absolute;
    left: 8px;
    top: 0;
    width: 3px;
    border-radius: 2px;
    background: var(--home-accent);
    opacity: 0;
    z-index: 2;
    transition:
      transform var(--dur-glide) var(--ease-out),
      height var(--dur-glide) var(--ease-out),
      opacity 160ms var(--ease-out);
  }
  .rule.shown {
    opacity: 1;
  }
  /* Day headers sit on a hairline, like ruled paper. */
  .day {
    position: sticky;
    top: 0;
    z-index: 3;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 10px 6px;
    background: var(--home-bg);
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--home-faint);
  }
  .day::after {
    content: '';
    flex: 1;
    height: 1px;
    background: var(--home-border);
  }
  .row {
    position: relative;
    padding: 10px 12px 10px 18px;
    border-radius: 10px;
    cursor: default;
    transition: background-color 120ms var(--ease-out);
  }
  .row:hover {
    background: var(--home-hover);
  }
  .row.selected {
    background: var(--home-selected);
  }
  /* A dictation that did not go in keeps a short amber rule while not selected. */
  .row.missed:not(.selected)::before {
    content: '';
    position: absolute;
    left: 0;
    top: 14px;
    bottom: 14px;
    width: 3px;
    border-radius: 2px;
    background: var(--home-error);
    opacity: 0.7;
  }
  .text {
    margin: 0;
    font-size: 14px;
    line-height: 21px;
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    white-space: pre-line;
    overflow-wrap: anywhere;
  }
  mark {
    background: var(--home-accent-soft);
    color: inherit;
    border-radius: 3px;
    padding: 0 1px;
  }
  .meta {
    display: flex;
    align-items: center;
    gap: 5px;
    margin: 4px 0 0;
    font-size: 12px;
    line-height: 16px;
    color: var(--home-faint);
  }
  .meta .app {
    color: var(--home-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .fav {
    width: 12px;
    height: 12px;
    margin-left: auto;
    fill: var(--home-accent);
  }
  .missed-dot {
    display: inline-block;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--home-error);
    flex: none;
  }
  .sentinel {
    height: 1px;
  }
  .empty {
    padding: 48px 20px;
    text-align: center;
    color: var(--home-muted);
  }
  .empty p {
    margin: 0 0 8px;
  }

  /* The entry ------------------------------------------------------------------------------- */
  .entry-col {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    /* A soft light from the top, the splash's ink glow in miniature. */
    background:
      radial-gradient(90% 55% at 35% -12%, var(--home-accent-soft) 0%, transparent 62%),
      var(--home-page);
  }
  .entry {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 40px 48px 24px;
  }
  .entry header {
    display: flex;
    gap: 8px;
    max-width: 680px;
    font-size: 12.5px;
    color: var(--home-faint);
  }
  .entry-app {
    color: var(--home-muted);
    font-weight: 600;
  }
  .entry-app::after {
    content: '·';
    margin-left: 8px;
    font-weight: 400;
    color: var(--home-faint);
  }
  .missed-line {
    max-width: 680px;
    box-sizing: border-box;
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 14px 0 0;
    padding: 10px 14px;
    border-radius: 10px;
    background: var(--home-error-bg);
    color: var(--home-error);
    font-size: 13px;
  }
  .missed-line .missed-dot {
    transform: translateY(-1px);
  }
  .body {
    max-width: 680px;
    margin: 20px 0 0;
    font: 400 19px/31px var(--font-ui);
    letter-spacing: 0.005em;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
    cursor: text;
  }
  .heard {
    max-width: 680px;
    margin-top: 36px;
    padding-top: 18px;
    border-top: 1px solid var(--home-border);
  }
  .heard-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 10px;
  }
  h3 {
    margin: 0;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
    color: var(--home-faint);
  }
  .toggle {
    display: flex;
    padding: 2px;
    border-radius: 7px;
    background: var(--home-selected);
  }
  .toggle button {
    height: 22px;
    padding: 0 9px;
    border: none;
    border-radius: 5px;
    background: none;
    font-size: 12px;
    color: var(--home-muted);
  }
  .toggle button[aria-checked='true'] {
    background: var(--home-surface);
    color: var(--home-text);
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.12);
  }
  .heard-text {
    margin: 0;
    font-size: 14px;
    line-height: 24px;
    color: var(--home-muted);
    user-select: text;
    white-space: pre-wrap;
  }
  .heard-text .removed {
    color: var(--home-faint);
    text-decoration: line-through;
    text-decoration-color: color-mix(in srgb, var(--home-faint) 80%, transparent);
  }
  .heard-text .added {
    color: var(--home-text);
    text-decoration: underline;
    text-decoration-color: var(--home-accent);
    text-decoration-thickness: 1.5px;
    text-underline-offset: 4px;
  }

  .actions {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 16px;
    min-height: 56px;
    padding: 0 20px 0 48px;
    border-top: 1px solid var(--home-border);
  }
  .stats {
    margin-left: auto;
    color: var(--home-faint);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .buttons {
    display: flex;
    align-items: center;
    gap: 4px;
    flex: none;
  }
  .action {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 32px;
    padding: 0 10px;
    border: none;
    background: none;
    color: var(--home-muted);
  }
  .action:hover:not(:disabled) {
    color: var(--home-text);
  }
  .action kbd {
    padding: 0 5px;
    font-size: 10.5px;
    line-height: 16px;
  }
  .undo {
    margin-right: 10px;
    font-size: 13px;
    color: var(--home-muted);
  }

  .welcome {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 24px;
    text-align: center;
    color: var(--home-muted);
    line-height: 24px;
  }
  .welcome-mark {
    opacity: 0.55;
    filter: drop-shadow(0 0 24px var(--home-accent-soft));
  }
  .welcome p {
    margin: 0;
  }
</style>
