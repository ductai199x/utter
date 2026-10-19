<script lang="ts">
  // The Settings section (docs/ux/home.md, docs/ux/insertion.md "Paste rules", docs/ux/history.md
  // "Settings → History"). Every change saves at once through `save`; the core applies it on
  // the next dictation and reports back (`applied`).
  import Confirm from './Confirm.svelte';
  import {
    KEEP_LABELS,
    WITH_LABELS,
    groupRules,
    historyClear,
    historyCountOlder,
    historyQuery,
    openPasteRules,
    openPath,
    pasteRules,
    quit,
    restart,
    ruleSource,
    withRule,
    withoutRule,
    type Applied,
    type CpuThreads,
    type HomeState,
    type KeepHistory,
    type Model,
    type Runtime,
    type PasteKeep,
    type PasteRules,
    type PasteWith,
    type Settings,
  } from './core';

  let {
    home,
    settings,
    applied,
    saveError,
    save,
  }: {
    home: HomeState | null;
    settings: Settings | null;
    applied: Partial<Record<Applied['what'], Applied>>;
    saveError: string;
    save: (next: Settings) => Promise<void>;
  } = $props();

  /** A card size people know: bytes in whole GB, rounded up. */
  const cardGb = (bytes: number) => Math.ceil(bytes / 2 ** 30);
  /** GPU memory a model needs, to one decimal. */
  const needGb = (model: Model) => ((home?.gpuNeed[model] ?? 0) / 1e9).toFixed(1);
  // Why the quality model cannot be chosen, if it cannot.
  const qualityBlocked = $derived(
    home && !home.qualityFits
      ? home.running === 'cpu'
        ? 'needs a graphics card.'
        : `needs a graphics card with ${cardGb(home.cardNeed.quality)} GB of memory.`
      : '',
  );
  // The two models: Quality, the bigger one, and Light, once its model is published.
  const MODELS = $derived(
    (
      [
        ['quality', 'Quality', `The bigger model, most accurate: about ${needGb('quality')} GB of GPU memory.`],
        [
          'light',
          'Light',
          home?.running === 'cpu'
            ? `The smaller model, on the processor: about ${needGb('light')} GB of memory.`
            : `The smaller model, for ${cardGb(home?.cardNeed.light ?? 0)} GB graphics cards: about ${needGb('light')} GB.`,
        ],
        ['mini', 'Extra light', `The smallest model, fastest on the processor: about ${needGb('mini')} GB of memory.`],
      ] as const
    ).filter(([id]) => (id !== 'light' || home?.lightAvailable) && (id === 'quality' || home?.models.includes(id))),
  );
  /** "Downloads 3.9 GB" when choosing `model` needs a download first. */
  function downloadNote(model: Model): string | undefined {
    const bytes = home?.downloadBytes[model] ?? 0;
    return bytes > 0 ? `Downloads ${(bytes / 1e9).toFixed(1)} GB` : undefined;
  }

  // Paste rules: here the person's own and changed ones; every rule in the Paste rules window.

  const WITH = Object.entries(WITH_LABELS) as [PasteWith, string][];
  const KEEP = Object.entries(KEEP_LABELS) as [PasteKeep, string][];
  let rules = $state<PasteRules | null>(null);
  $effect(() => {
    const overrides = settings?.paste ?? [];
    void pasteRules($state.snapshot(overrides)).then((r) => (rules = r));
  });
  const groups = $derived(groupRules(rules?.rules ?? []));
  const mine = $derived(groups.filter((g) => ruleSource(g) !== 'default'));
  const defaults = $derived(groups.length - mine.length);
  // The model that runs (runtime_choice::effective_model): Extra light on the processor in place
  // of Quality, which needs a graphics card; Light with a graphics card in place of Extra light.
  const shownModel = $derived(
    settings && home && !home.models.includes(settings.model)
      ? home.running === 'cpu'
        ? 'mini'
        : 'light'
      : settings?.model,
  );
  // The runtime: "auto" shows as the one it picks (the first, best one).
  const chosenRuntime = $derived(
    settings?.runtime && settings.runtime !== 'auto' && home?.runtimes.some((r) => r.id === settings?.runtime)
      ? settings.runtime
      : (home?.runtimes[0]?.id ?? 'cpu'),
  );
  const pendingRuntime = $derived(
    home && chosenRuntime !== home.running ? home.runtimes.find((r) => r.id === chosenRuntime) : undefined,
  );
  const runtimeDesc = $derived(home?.runtimes.find((r) => r.id === chosenRuntime)?.detail ?? '');
  // Processor threads apply at start, like the runtime.
  const pendingThreads = $derived(!!settings && !!home && settings.cpu_threads !== home.runningThreads);
  const loginLabel = $derived(home?.os === 'windows' ? 'Start with Windows' : 'Start at login');
  // GNOME shows no tray icons without an extension, so elsewhere it is "in the background".
  const loginDesc = $derived(
    home?.os === 'windows' ? 'Utter starts in the tray when you sign in.' : 'Utter starts in the background when you sign in.',
  );

  // History ------------------------------------------------------------------------------

  const KEEP_HISTORY: [KeepHistory, string][] = [
    ['forever', 'Forever'],
    ['30-days', '30 days'],
    ['24-hours', '24 hours'],
    ['off', 'Off'],
  ];
  let confirm = $state<{ title: string; message: string; action: string; run: () => void } | null>(null);
  // Why a History row could not do what was asked (History could not be read).
  let historyError = $state<string | null>(null);

  async function changeKeep(keep: KeepHistory, select: HTMLSelectElement) {
    if (!settings) return;
    const current = settings.keep_history;
    const order = KEEP_HISTORY.map(([k]) => k);
    const apply = () => settings && void save({ ...settings, keep_history: keep });
    // Keeping longer deletes nothing; keeping less asks first, if anything would go.
    let n = 0;
    try {
      n = order.indexOf(keep) > order.indexOf(current) ? await historyCountOlder(keep) : 0;
      historyError = null;
    } catch (e) {
      select.value = current;
      historyError = String(e);
      return;
    }
    if (n === 0 && keep !== 'off') return apply();
    // Until confirmed the control shows what is stored.
    select.value = current;
    const plural = `${n} ${n === 1 ? 'dictation' : 'dictations'}`;
    confirm =
      keep === 'off'
        ? {
            title: 'Turn History off?',
            message: n ? `This deletes all ${plural} and keeps no new ones.` : 'New dictations will not be kept.',
            action: n ? 'Delete and turn off' : 'Turn off',
            run: apply,
          }
        : {
            title: 'Keep less history?',
            message: `Delete ${plural} older than ${KEEP_HISTORY.find(([k]) => k === keep)![1]}?`,
            action: 'Delete',
            run: apply,
          };
  }

  async function clearHistory() {
    let total: number;
    try {
      ({ total } = await historyQuery({
        search: '',
        favorites: false,
        inserted: null,
        app: null,
        since: null,
        until: null,
        offset: 0,
        limit: 1,
      }));
      historyError = null;
    } catch (e) {
      historyError = String(e);
      return;
    }
    if (total === 0) return;
    confirm = {
      title: 'Clear history?',
      message: `Delete all ${total} ${total === 1 ? 'dictation' : 'dictations'}? This cannot be undone.`,
      action: 'Delete all',
      run: () => void historyClear(),
    };
  }
</script>

<div class="page">
  <div class="column">
    <h1>Settings</h1>
    <p class="lede">Changes apply to your next dictation.</p>

    {#if home?.error}
      <div class="banner" role="alert">
        <span>{home.error}</span>
        <button onclick={() => openPath('settings')}>Open file</button>
      </div>
    {/if}
    {#if saveError && !(home?.error && saveError.includes(home.error))}<div class="banner" role="alert"><span>{saveError}</span></div>{/if}

    {#if settings && home}
      <h2>General</h2>
      <div class="row">
        <div class="label">
          <label for="login">{loginLabel}</label>
          <span class="desc">{loginDesc}</span>
        </div>
        <button
          id="login"
          class="switch"
          role="switch"
          aria-checked={settings.start_at_login}
          aria-label={loginLabel}
          onclick={() => settings && save({ ...settings, start_at_login: !settings.start_at_login })}
          ><span></span></button
        >
      </div>
      {#if !home.tray}
        <div class="row">
          <div class="label">
            <span class="label-text">Quit Utter</span>
            <span class="desc">This desktop shows no tray icons. Opening Utter again brings back this window.</span>
          </div>
          <button onclick={() => void quit()}>Quit</button>
        </div>
      {/if}

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
          {#if applied.microphone?.status === 'error'}<span class="row-error">{applied.microphone.error}</span>
          {:else if home.microphones.length === 0}<span class="row-error"
              >No microphone connected. Plug one in; Utter uses it right away.</span
            >{/if}
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
      {#if home.runtimes.length > 1}
        <div class="row">
          <div class="label">
            <label for="runtime">Runs on</label>
            <span class="desc">{runtimeDesc}</span>
          </div>
          <select
            id="runtime"
            value={chosenRuntime}
            onchange={(e) => settings && save({ ...settings, runtime: e.currentTarget.value as Runtime })}
          >
            {#each home.runtimes as r (r.id)}
              <option value={r.id}>{r.name}{r.id === home.runtimes[0]?.id ? ' (recommended)' : ''}</option>
            {/each}
          </select>
        </div>
        {#if pendingRuntime}
          <div class="row">
            <div class="label">
              <span class="label-text">Utter restarts to switch to {pendingRuntime.name}</span>
              <span class="desc"
                >{pendingRuntime.downloadBytes > 0
                  ? `It downloads ${(pendingRuntime.downloadBytes / 1e9).toFixed(1)} GB first; dictation is off until that is done.`
                  : 'It takes a few seconds.'}{pendingRuntime.id === 'cpu' && settings.model === 'quality'
                ? ' On the processor it runs Extra light.'
                : ''}</span
              >
            </div>
            <button class="primary" onclick={() => void restart()}>Restart now</button>
          </div>
        {/if}
      {/if}
      {#if home.running === 'cpu' && !pendingRuntime}
        <div class="row">
          <div class="label">
            <label for="cpu-threads">Processor threads</label>
            <span class="desc"
              >{settings.cpu_threads === 'high'
                ? 'Three quarters of the processor, for the models. Can leave other apps slower while you dictate.'
                : "Half of the processor, for the models. Leaves room for your other apps."}</span
            >
          </div>
          <select
            id="cpu-threads"
            value={settings.cpu_threads}
            onchange={(e) => settings && save({ ...settings, cpu_threads: e.currentTarget.value as CpuThreads })}
          >
            <option value="default">Default ({home.threadCounts.default} threads)</option>
            <option value="high">High ({home.threadCounts.high} threads)</option>
          </select>
        </div>
        {#if pendingThreads}
          <div class="row">
            <div class="label">
              <span class="label-text">Utter restarts to use {home.threadCounts[settings.cpu_threads]} threads</span>
              <span class="desc">It takes a few seconds.</span>
            </div>
            <button class="primary" onclick={() => void restart()}>Restart now</button>
          </div>
        {/if}
      {/if}
      <div class="row">
        <div class="label">
          <span class="label-text" id="model-label">Model</span>
          <span class="desc">{MODELS.find(([id]) => id === shownModel)?.[2] ?? ''}</span>
          {#if applied.model?.status === 'loading'}
            <span class="row-status"><span class="spinner" aria-hidden="true"></span>Loading...</span>
          {:else if applied.model?.status === 'error'}
            <span class="row-error">{applied.model.error}</span>
          {/if}
        </div>
        <div class="segmented" role="radiogroup" aria-labelledby="model-label">
          {#each MODELS as [id, label] (id)}
            {@const blocked = id === 'quality' ? qualityBlocked : ''}
            <button
              role="radio"
              aria-checked={shownModel === id}
              disabled={!!blocked}
              title={blocked ? `Quality ${blocked}` : downloadNote(id)}
              onclick={() => settings && save({ ...settings, model: id })}>{label}</button
            >
          {/each}
        </div>
      </div>
      {#if qualityBlocked}<p class="hint">Quality {qualityBlocked}</p>{/if}

      <h2>Typing</h2>
      <div class="row">
        <div class="label">
          <span class="label-text">Paste rules</span>
          <span class="desc">
            Most apps paste with Ctrl+V and read the clipboard at once. Utter knows {defaults}
            {defaults === 1 ? 'app' : 'apps'} that don't{mine.length ? `, and you have ${mine.length} of your own` : ''}.
          </span>
        </div>
        <button onclick={() => void openPasteRules()}>All paste rules…</button>
      </div>
      {#if mine.length}
        <ul class="rules">
          {#each mine as g (g.apps.join())}
            <li>
              <div class="rule-app">
                <span class="rule-name">{g.name}</span>
                <span class="rule-exe">{g.apps.join(', ')}</span>
              </div>
              <select
                aria-label="{g.name}: paste with"
                value={g.with}
                onchange={(e) =>
                  settings &&
                  save({ ...settings, paste: withRule(settings.paste, g, { with: e.currentTarget.value as PasteWith, keep: g.keep }) })}
              >
                {#each WITH as [v, label] (v)}<option value={v}>{label}</option>{/each}
              </select>
              <select
                aria-label="{g.name}: keep on the clipboard"
                value={g.keep}
                onchange={(e) =>
                  settings &&
                  save({ ...settings, paste: withRule(settings.paste, g, { with: g.with, keep: e.currentTarget.value as PasteKeep }) })}
              >
                {#each KEEP as [v, label] (v)}<option value={v}>{label}</option>{/each}
              </select>
              <span class="rule-tail">
                {#if ruleSource(g) === 'changed'}
                  <button class="link" onclick={() => settings && save({ ...settings, paste: withoutRule(settings.paste, g) })}
                    >Reset</button
                  >
                {:else}
                  <button
                    class="remove"
                    aria-label="Remove {g.name}"
                    onclick={() => settings && save({ ...settings, paste: withoutRule(settings.paste, g) })}
                  >
                    <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" /></svg>
                  </button>
                {/if}
              </span>
            </li>
          {/each}
        </ul>
      {/if}

      <h2>History</h2>
      <div class="row">
        <div class="label">
          <label for="keep">Keep history</label>
          <span class="desc">Your dictations, kept on this computer only.</span>
        </div>
        <select
          id="keep"
          value={settings.keep_history}
          onchange={(e) => void changeKeep(e.currentTarget.value as KeepHistory, e.currentTarget)}
        >
          {#each KEEP_HISTORY as [v, label] (v)}<option value={v}>{label}</option>{/each}
        </select>
      </div>
      <div class="row">
        <div class="label">
          <span class="label-text">Clear history</span>
          <span class="desc">Deletes every dictation, favorites too.</span>
        </div>
        <button onclick={clearHistory}>Clear…</button>
      </div>
      {#if historyError}<p class="row-error" role="alert">History could not be read: {historyError}</p>{/if}

      <h2>Shortcuts</h2>
      <div class="row">
        <div class="label">
          <label for="key">Hold to talk</label>
          <span class="desc">Hold to dictate. Double-tap for hands-free; Esc cancels.</span>
          {#if applied.key?.status === 'error'}<span class="row-error">{applied.key.error}</span>{/if}
        </div>
        <select
          id="key"
          value={settings.dictation_key}
          onchange={(e) => settings && save({ ...settings, dictation_key: e.currentTarget.value })}
        >
          {#each home?.dictationKeys ?? [] as k (k.id)}<option value={k.id}>{k.name}</option>{/each}
        </select>
      </div>
      <div class="row">
        <div class="label">
          <span class="label-text">Paste last dictation</span>
          <span class="desc">Into the app you are in. Also in the tray menu.</span>
        </div>
        <kbd>Alt+Shift+Z</kbd>
      </div>
      <div class="row">
        <div class="label">
          <span class="label-text">Copy last dictation</span>
          <span class="desc">To the clipboard, without pasting.</span>
        </div>
        <kbd>Alt+Shift+X</kbd>
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
      {#if settings}
      <div class="row">
        <div class="label">
          <label for="logs">Keep logs</label>
          <span class="desc">Timings and errors, never what you say. They help find out what went wrong; off deletes them.</span>
        </div>
        <span class="row-actions">
          {#if settings.keep_logs}<button class="link" onclick={() => openPath('logs')}>Open</button>{/if}
          <button
            id="logs"
            class="switch"
            role="switch"
            aria-checked={settings.keep_logs}
            aria-label="Keep logs"
            onclick={() => settings && save({ ...settings, keep_logs: !settings.keep_logs })}
            ><span></span></button
          >
        </span>
      </div>
      {/if}
    {/if}
  </div>
</div>

{#if confirm}
  <Confirm
    title={confirm.title}
    message={confirm.message}
    action={confirm.action}
    onconfirm={() => {
      confirm?.run();
      confirm = null;
    }}
    oncancel={() => (confirm = null)}
  />
{/if}

<style>
  .row-actions {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .rules {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .rules li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 150px 140px 64px;
    align-items: center;
    gap: 10px;
    min-height: 50px;
    border-bottom: 1px solid var(--home-border);
  }
  .rules select {
    height: 32px;
    font-size: 13px;
    padding-left: 10px;
    background-color: transparent;
    border-color: transparent;
    transition:
      border-color 120ms var(--ease-out),
      background-color 120ms var(--ease-out);
  }
  .rules li:hover select,
  .rules select:focus-visible {
    border-color: var(--home-border);
    background-color: var(--home-surface);
  }
  .rule-app {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .rule-name {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .rule-exe {
    font: 11.5px/16px 'Cascadia Mono', Consolas, monospace;
    color: var(--home-faint);
  }
  .rule-tail {
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
</style>
