<script lang="ts">
  // The setup window (docs/ux/first-run.md). One page, four modes from the URL: first run
  // (check, microphone, download, try it), update and repair (check, download, then it closes
  // itself), and problem (why dictation could not start, and what to do). The core does the
  // work and reports; this page shows it and sends intent.
  import { onDestroy, onMount, tick } from 'svelte';
  import { cubicOut } from 'svelte/easing';
  import { fade, fly } from 'svelte/transition';
  import * as core from './core';
  import type { Checks, ModelCard, ModelId, Problem, Progress } from './core';
  import Logo from '../lib/Logo.svelte';
  import { Waveform } from '../lib/waveform';

  type View = 'check' | 'model' | 'mic' | 'download' | 'try' | 'problem' | 'done';
  const SENTENCE = "Hi team, the meeting moved to three, no, four o'clock. Bring the slides.";

  const mode = core.mode();
  const flow: View[] = mode === 'first' ? ['check', 'model', 'mic', 'download', 'try'] : ['check', 'download'];
  let step = $state(0);
  // The problem and done screens stand outside the steps.
  let special = $state<'problem' | 'done' | null>(mode === 'problem' ? 'problem' : null);
  const view = $derived<View>(special ?? flow[step]!);
  // +1 forward, -1 back: which side the next view slides in from.
  let dir = $state(1);
  let primary = $state<HTMLButtonElement>();

  // Check.
  let checks = $state<Checks | null>(null);
  const allOk = $derived(checks !== null && checks.checks.every((c) => c.ok));
  const checkTitle =
    mode === 'update'
      ? 'Utter needs to download an update'
      : mode === 'repair'
        ? "Some of Utter's files are missing or damaged"
        : 'Welcome to Utter';

  // Microphone.
  let mics = $state<string[]>([]);
  let device = $state('');
  let micError = $state('');
  const wave = new Waveform({ bars: 64, barW: 3, gap: 3, min: 3, max: 40 }, 90);
  let meter = $state<HTMLCanvasElement>();

  // Download.
  let progress = $state<Progress | null>(null);
  let started = false;
  let repairing = $state(false);
  let samples: { t: number; done: number }[] = [];
  let eta = $state('');

  // Try it.
  let typed = $state('');
  let box = $state<HTMLTextAreaElement>();
  const arrived = $derived(typed.trim().length > 0);
  // The sample has a self-correction in it; when cleanup applied it, say so.
  const corrected = $derived(/\bfour\b/i.test(typed) && !/\bthree\b/i.test(typed));

  // Problem.
  let problem = $state<Problem | null>(null);
  let starting = $state(false);
  let details = $state(false);

  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const slide = (node: Element) => fly(node, { x: reduced ? 0 : 8 * dir, duration: 120, easing: cubicOut });
  const leave = (node: Element) => fade(node, { duration: reduced ? 0 : 120 });

  function focusMain() {
    // The main action takes the focus; on Try it, the text box does.
    void tick().then(() => setTimeout(() => (view === 'try' ? box : primary)?.focus(), 130));
  }

  async function go(next: number) {
    dir = next > step ? 1 : -1;
    if (flow[step] === 'mic' && flow[next] !== 'mic') await core.micStop();
    step = next;
    if (flow[next] === 'mic') void openMic();
    if (flow[next] === 'download' && !started) void startDownload();
    focusMain();
  }
  const next = () => go(step + 1);

  async function runChecks() {
    checks = null;
    checks = await core.check();
  }

  // Choose your model: the recommended card is selected when the step opens.
  let cards = $state<ModelCard[]>([]);
  let picked = $state<ModelId>('quality');
  let modelError = $state('');
  let choosing = $state(false);
  const MODEL_TEXT: Record<ModelId, { name: string; line: string }> = {
    quality: { name: 'Quality', line: 'The bigger model. Most accurate; best for most cards.' },
    light: { name: 'Light', line: 'The smaller model. Runs on 4 GB graphics cards.' },
  };

  async function loadModels() {
    cards = await core.models();
    picked = (cards.find((c) => c.recommended) ?? cards.find((c) => c.selected) ?? cards[0])?.id ?? 'quality';
  }

  /** Saves the choice, then checks the disk again for what it downloads. */
  async function confirmModel() {
    choosing = true;
    modelError = '';
    try {
      await core.choose(picked);
      checks = await core.check();
      const disk = checks.checks.find((c) => c.id === 'disk');
      if (disk && !disk.ok) modelError = disk.fix ?? 'Not enough disk space for this model.';
      else void next();
    } finally {
      choosing = false;
    }
  }

  function cardKey(e: KeyboardEvent) {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
    e.preventDefault();
    const usable = cards.filter((c) => c.fits);
    const i = usable.findIndex((c) => c.id === picked);
    const nextCard = usable[(i + (e.key === 'ArrowRight' ? 1 : usable.length - 1)) % usable.length];
    if (nextCard) {
      picked = nextCard.id;
      void tick().then(() => document.getElementById(`card-${nextCard.id}`)?.focus());
    }
  }

  async function openMic() {
    micError = '';
    try {
      await core.micStart(device);
    } catch {
      micError = 'This microphone could not be opened. Choose another.';
      if (device) {
        device = '';
        await core.micStart('').catch(() => {});
      }
    }
  }

  // The meter draws like the Bar's waveform (lib/waveform.ts), wider and over the last 1.5 s
  // (90 levels at 60 Hz).
  let raf = 0;
  let lastFrame = performance.now();
  // The accent follows the OS theme: read it again when the theme changes.
  const readAccent = () => getComputedStyle(document.documentElement).getPropertyValue('--home-accent').trim();
  let accent = readAccent();
  matchMedia('(prefers-color-scheme: dark)').addEventListener('change', () => (accent = readAccent()));
  function frame(now: number) {
    const dt = Math.min(50, now - lastFrame);
    lastFrame = now;
    if (meter) wave.draw(meter, accent, dt, view === 'mic');
    raf = requestAnimationFrame(frame);
  }

  function resetProgress() {
    samples = [];
    eta = '';
    // A retry resumes: the bar stays where it was until the core reports.
    progress = {
      done: progress?.done ?? 0,
      total: progress?.total || (checks?.downloadBytes ?? 0),
      stage: 'downloading',
      label: progress?.label ?? '',
    };
  }

  async function startDownload() {
    started = true;
    resetProgress();
    await core.download();
  }

  function onProgress(p: Progress) {
    if (p.stage === 'start-error') {
      // Wherever it happens, a failed start shows the problem screen.
      problem = p.problem ?? null;
      starting = false;
      started = false;
      dir = 1;
      special = 'problem';
      focusMain();
      return;
    }
    // Ready carries no byte counts: the bar stays full.
    progress = p.stage === 'ready' ? { ...p, done: progress?.total ?? 0, total: progress?.total ?? 0 } : p;
    if (p.stage === 'downloading') {
      const now = performance.now();
      samples.push({ t: now, done: p.done });
      while (samples.length > 2 && now - samples[0]!.t > 10_000) samples.shift();
      const first = samples[0]!;
      const span = (now - first.t) / 1000;
      // Nothing until there are three seconds to go on.
      if (span >= 3 && p.done > first.done) {
        const rate = (p.done - first.done) / span;
        const left = (p.total - p.done) / rate;
        eta = left < 60 ? 'less than a minute left' : `about ${Math.round(left / 60)} min left`;
      }
    } else eta = '';
    if (p.stage === 'error') started = false;
    if (p.stage === 'ready') {
      starting = false;
      if (mode === 'first' && !repairing) setTimeout(() => view === 'download' && go(flow.indexOf('try')), 500);
      else {
        // Update, repair or a retry: nothing more to ask. Say so briefly, then get out of the way.
        dir = 1;
        special = 'done';
        setTimeout(() => void core.finish(), 1500);
      }
    }
  }

  function retry() {
    starting = true;
    void core.retry();
  }

  function repair() {
    repairing = true;
    special = null;
    step = flow.indexOf('download');
    started = true;
    progress = null;
    resetProgress();
    void core.repair();
  }

  const failed = $derived(progress?.stage === 'error');
  const gb = (b: number) => (b / 1e9).toFixed(1);
  // GPU needs read as the round figures they are: 2.5, 6, 18.
  const gpuGb = (b: number) => String(Math.round((b / 1e9) * 10) / 10);
  const pct = $derived(progress && progress.total > 0 ? Math.min(100, (progress.done / progress.total) * 100) : 0);
  const stageLine = $derived.by(() => {
    if (!progress) return '';
    switch (progress.stage) {
      case 'downloading':
        return progress.label;
      case 'verifying':
      case 'extracting':
        return 'Checking files...';
      case 'loading':
      case 'ready':
        return 'Getting ready...';
      default:
        return '';
    }
  });
  const downloadTitle = $derived(
    repairing ? 'Repairing Utter' : mode === 'first' ? 'Downloading the models' : 'Downloading the update',
  );

  const CHECK_LABELS: Record<string, string> = {
    gpu: 'Graphics card',
    memory: 'GPU memory',
    driver: 'Graphics driver',
    disk: 'Disk space',
  };

  function onKey(e: KeyboardEvent) {
    if (e.key === 'Enter' && !(e.target instanceof HTMLTextAreaElement) && !(e.target instanceof HTMLButtonElement)) {
      primary?.click();
    }
  }

  onMount(() => {
    if (mode === 'problem') void core.lastProblem().then((p) => (problem = p));
    else void runChecks();
    if (mode === 'first') {
      void core.microphones().then((m) => (mics = m));
      void loadModels();
    }
    const offs = [core.onLevel((l) => wave.push(l)), core.onProgress(onProgress)];
    raf = requestAnimationFrame(frame);
    focusMain();
    return () => {
      cancelAnimationFrame(raf);
      offs.forEach((p) => void p.then((off) => off()));
    };
  });
  onDestroy(() => void core.micStop());
</script>

<svelte:window onkeydown={onKey} />

<div class="frame">
  <div class="dots" class:hidden={special !== null} aria-label="Step {step + 1} of {flow.length}" role="img">
    {#each flow as _, i (i)}
      <span class:done={i < step} class:current={i === step}></span>
    {/each}
  </div>

  <div class="stage">
    {#key view}
      <section in:slide out:leave>
        {#if view === 'check'}
          <div class="logo"><Logo size={36} /></div>
          <h1>{checkTitle}</h1>
          <p class="lede">
            {#if mode === 'first'}
              Hold a key, speak, and clean text appears where you type. Everything runs on this computer; nothing you
              say leaves it.
            {:else if checks}
              {gb(checks.downloadBytes)} GB to download. Dictation starts again as soon as it is done.
            {:else}
              Checking this computer...
            {/if}
          </p>
          <ul class="checks">
            {#each checks?.checks ?? [{ id: 'gpu' }, { id: 'memory' }, { id: 'driver' }, { id: 'disk' }] as c (c.id)}
              {@const r = checks?.checks.find((x) => x.id === c.id)}
              <li>
                <span class="mark" class:ok={r?.ok} class:bad={r && !r.ok}>
                  {#if !r}
                    <span class="spinner" aria-label="Checking"></span>
                  {:else if r.ok}
                    <svg viewBox="0 0 16 16" aria-label="Passed"><path d="M3.5 8.5l3 3 6-7" /></svg>
                  {:else}
                    <svg viewBox="0 0 16 16" aria-label="Failed"><path d="M4.5 4.5l7 7M11.5 4.5l-7 7" /></svg>
                  {/if}
                </span>
                <span class="name">{CHECK_LABELS[c.id]}</span>
                <span class="value">{r?.value ?? ''}</span>
                {#if r?.fix}<span class="fix">{r.fix}</span>{/if}
              </li>
            {/each}
          </ul>
          {#if checks?.noGpu}
            <p class="panel">
              Utter needs an NVIDIA graphics card with 8 GB of memory or more. Support for other cards is planned.
            </p>
          {/if}
        {:else if view === 'model'}
          <h1>Choose your model</h1>
          <p class="lede">Bigger models write more carefully. You can change this later in Settings.</p>
          <div class="cards" role="radiogroup" aria-label="Model">
            {#each cards as c (c.id)}
              <button
                id="card-{c.id}"
                class="card"
                class:picked={picked === c.id}
                role="radio"
                aria-checked={picked === c.id}
                disabled={!c.fits}
                tabindex={picked === c.id ? 0 : -1}
                onclick={() => (picked = c.id)}
                onkeydown={cardKey}
              >
                <span class="card-head">
                  <span class="card-name">{MODEL_TEXT[c.id].name}</span>
                  {#if picked === c.id}
                    <span class="card-check" aria-hidden="true"
                      ><svg viewBox="0 0 16 16"><path d="M4 8.5l2.5 2.5 5.5-6" /></svg></span
                    >
                  {/if}
                </span>
                {#if c.recommended}<span class="pill">Recommended</span>{/if}
                <span class="card-line">{MODEL_TEXT[c.id].line}</span>
                <span class="card-facts">
                  {#if c.fits}
                    <span>{gpuGb(c.gpuBytes)} GB GPU memory</span>
                    <span>{c.downloadBytes > 0 ? `${gb(c.downloadBytes)} GB download` : 'Installed'}</span>
                  {:else}
                    <span class="card-why">Needs a {Math.ceil(c.cardBytes / 2 ** 30)} GB graphics card</span>
                  {/if}
                </span>
              </button>
            {/each}
          </div>
          <p class="error" aria-live="polite">{modelError}</p>
        {:else if view === 'mic'}
          <h1>Your microphone</h1>
          <p class="lede">Say something. The bars should move.</p>
          <select bind:value={device} onchange={openMic} aria-label="Microphone">
            <option value="">System default</option>
            {#each mics as m (m)}<option value={m}>{m}</option>{/each}
          </select>
          <p class="error" aria-live="polite">{micError}</p>
          <div class="meter" aria-hidden="true">
            <canvas bind:this={meter} style:width="{wave.width}px" style:height="40px"></canvas>
          </div>
        {:else if view === 'download'}
          <h1>{downloadTitle}</h1>
          <p class="lede">
            {#if repairing}
              Checking every file and downloading again the ones that are damaged.
            {:else}
              About {gb(progress?.total || checks?.downloadBytes || 0)} GB. You can use your computer meanwhile; Utter will
              let you know when it is ready.
            {/if}
          </p>
          <div
            class="bar"
            class:loading={progress?.stage === 'loading' || progress?.stage === 'ready'}
            role="progressbar"
            aria-valuemin="0"
            aria-valuemax="100"
            aria-valuenow={Math.round(pct)}
          >
            <span style:width="{pct}%"></span>
          </div>
          <div class="bar-text">
            <span>{progress && progress.total > 0 ? `${gb(progress.done)} of ${gb(progress.total)} GB` : ''}</span>
            <span>{eta}</span>
          </div>
          {#if failed}
            <p class="error">{progress?.error}</p>
          {:else}
            <p class="muted">{stageLine}</p>
          {/if}
        {:else if view === 'try'}
          <h1>Try it</h1>
          <p class="lede">Hold <kbd>Right Ctrl</kbd>, say this, then let go:</p>
          <blockquote>{SENTENCE}</blockquote>
          <textarea bind:this={box} bind:value={typed} rows="3" aria-label="Your first dictation"></textarea>
          <div class="success" class:shown={arrived} aria-live="polite">
            {#if arrived}
              <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M3.5 8.5l3 3 6-7" /></svg>
              <div>
                {#if corrected}<p>You said "three, no, four", and Utter wrote "four".</p>{/if}
                <p>Hold Right Ctrl in any app to dictate. Double-tap it to keep talking hands&#8209;free.</p>
              </div>
            {/if}
          </div>
        {:else if view === 'problem'}
          <div class="badge warn" aria-hidden="true">
            <svg viewBox="0 0 20 20"><path d="M10 5.5v5.5" /><circle cx="10" cy="14.25" r="0.4" /></svg>
          </div>
          <h1>{problem?.title ?? 'Utter could not start'}</h1>
          <p class="lede">{problem?.message ?? ''}</p>
          {#if problem?.detail}
            <button class="disclosure" aria-expanded={details} onclick={() => (details = !details)}>
              <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M6 4l4 4-4 4" /></svg>
              Details
            </button>
            {#if details}
              <pre class="detail" transition:fade={{ duration: 120 }}>{problem.detail}</pre>
            {/if}
          {/if}
        {:else}
          <div class="badge ok" aria-hidden="true">
            <svg viewBox="0 0 20 20"><path d="M5.5 10.5l3 3 6-7" /></svg>
          </div>
          <h1>Utter is ready</h1>
          <p class="lede">Hold Right Ctrl in any app to dictate.</p>
        {/if}
      </section>
    {/key}
  </div>

  <footer>
    {#if view === 'check'}
      {#if checks?.noGpu}
        <button class="primary" bind:this={primary} onclick={core.quit}>Quit</button>
      {:else if checks && !allOk}
        <button class="primary" bind:this={primary} onclick={runChecks}>Check again</button>
      {:else}
        <button class="primary" bind:this={primary} disabled={!checks} onclick={next}>Continue</button>
      {/if}
    {:else if view === 'model'}
      <button class="text" onclick={() => go(step - 1)}>Back</button>
      <button class="primary" bind:this={primary} disabled={choosing || cards.length === 0} onclick={confirmModel}>
        Continue
      </button>
    {:else if view === 'mic'}
      <button class="text" onclick={() => go(step - 1)}>Back</button>
      <button class="primary" bind:this={primary} onclick={next}>Continue</button>
    {:else if view === 'download'}
      {#if failed}
        <button class="primary" bind:this={primary} onclick={startDownload}>Retry</button>
      {:else}
        <p class="note">Closing this window pauses the download; it resumes next time.</p>
      {/if}
    {:else if view === 'try'}
      <button class="text" onclick={core.finish}>Skip</button>
      <button class="primary" bind:this={primary} disabled={!arrived} onclick={core.finish}>Finish</button>
    {:else if view === 'problem'}
      {#if problem?.actions.includes('logs')}
        <button class="text" onclick={() => core.openLogs()}>Open logs</button>
      {/if}
      {#if problem?.actions.includes('close')}
        <!-- Dictation keeps running on the previous model: closing is enough. -->
        <button class="text" onclick={core.finish}>Close</button>
      {:else}
        <button class="text" onclick={core.quit}>Quit Utter</button>
      {/if}
      {#if problem?.actions.includes('repair')}
        <button class="primary" bind:this={primary} onclick={repair}>Repair</button>
      {:else}
        <button class="primary" bind:this={primary} disabled={starting} onclick={retry}>
          {#if starting}<span class="spinner on-accent" aria-hidden="true"></span>Starting...{:else}Retry{/if}
        </button>
      {/if}
    {/if}
  </footer>
</div>

<style>
  .frame {
    position: relative;
    height: 100%;
    display: flex;
    flex-direction: column;
    box-sizing: border-box;
    padding: 0 48px 24px;
  }

  .dots {
    display: flex;
    justify-content: center;
    gap: 8px;
    height: 6px;
    margin: 28px 0 38px;
  }
  .dots span {
    width: 6px;
    height: 6px;
    border-radius: 3px;
    background: var(--home-border);
    transition:
      width var(--dur-resize) var(--ease-out),
      background-color var(--dur-resize) var(--ease-out);
  }
  .dots span.done {
    background: var(--home-accent);
  }
  .dots span.current {
    width: 18px;
    background: var(--home-accent);
  }

  /* Outgoing and incoming steps share one cell while they cross-fade. */
  .stage {
    flex: 1;
    display: grid;
    min-height: 0;
  }
  section {
    grid-area: 1 / 1;
    min-width: 0;
  }
  h1 {
    font-size: 24px;
    line-height: 32px;
    font-weight: 600;
    margin: 0;
  }
  .lede {
    color: var(--home-muted);
    margin: 8px 0 28px;
    max-width: 480px;
  }
  .muted {
    color: var(--home-muted);
    margin: 10px 0 0;
    min-height: 20px;
  }
  .error {
    color: var(--home-error);
    margin: 10px 0 0;
    min-height: 20px;
  }

  .checks {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .checks li {
    display: grid;
    grid-template-columns: 24px 1fr auto;
    align-items: center;
    column-gap: 10px;
    padding: 11px 0;
    border-bottom: 1px solid var(--home-border);
  }
  .checks li:first-child {
    border-top: 1px solid var(--home-border);
  }
  .mark {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
  }
  .mark.ok {
    background: var(--home-ok-soft);
    color: var(--home-ok);
  }
  .mark.bad {
    background: var(--home-error-bg);
    color: var(--home-error);
  }
  .mark svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .value {
    color: var(--home-muted);
    text-align: right;
  }
  .fix {
    grid-column: 2 / 4;
    color: var(--home-error);
    font-size: 13px;
    line-height: 18px;
    margin-top: 4px;
  }
  .panel {
    margin: 20px 0 0;
    padding: 12px 14px;
    border-radius: 6px;
    background: var(--home-error-bg);
    color: var(--home-error);
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

  select {
    width: 100%;
  }
  .meter {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 64px;
    margin-top: 8px;
    border-radius: 8px;
    background: var(--home-surface);
    border: 1px solid var(--home-border);
    overflow: hidden;
  }
  .logo {
    margin-bottom: 16px;
  }
  .note {
    flex: 1;
    align-self: center;
    margin: 0;
    color: var(--home-muted);
    font-size: 13px;
    line-height: 18px;
  }

  .bar {
    position: relative;
    height: 8px;
    border-radius: 4px;
    background: var(--home-selected);
    overflow: hidden;
  }
  .bar span {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: 4px;
    background: var(--home-accent);
    transition: width 300ms var(--ease-out);
  }
  .bar.loading span::after {
    content: '';
    position: absolute;
    inset: 0;
    background: linear-gradient(90deg, transparent, rgba(255, 255, 255, 0.45), transparent);
    animation: sweep 1.6s var(--ease-out) infinite;
  }
  @keyframes sweep {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(100%);
    }
  }
  .bar-text {
    display: flex;
    justify-content: space-between;
    margin-top: 10px;
    font-variant-numeric: tabular-nums;
  }
  .bar-text span:last-child {
    color: var(--home-muted);
  }

  kbd {
    font: 600 12px var(--font-ui);
    padding: 2px 6px;
    border: 1px solid var(--home-border);
    border-bottom-width: 2px;
    border-radius: 5px;
    background: var(--home-surface);
    color: var(--home-text);
  }
  blockquote {
    margin: -8px 0 16px;
    padding: 12px 16px;
    border-left: 3px solid var(--home-accent);
    border-radius: 0 6px 6px 0;
    background: var(--home-surface);
    font-size: 15px;
    line-height: 22px;
  }
  textarea {
    width: 100%;
    padding: 10px 12px;
    resize: none;
  }
  .success {
    display: flex;
    gap: 8px;
    align-items: flex-start;
    min-height: 40px;
    margin: 12px 0 0;
    opacity: 0;
    transition: opacity var(--dur-enter) var(--ease-out);
  }
  .success.shown {
    opacity: 1;
  }
  .success p {
    margin: 0;
  }
  .success p + p {
    color: var(--home-muted);
  }
  .success svg {
    flex: none;
    width: 16px;
    height: 16px;
    margin-top: 2px;
    fill: none;
    stroke: var(--home-ok);
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }

  footer {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    min-height: 36px;
  }

  /* Choose your model: one card per tier, the picked one outlined in the accent. */
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(0, 1fr));
    gap: 12px;
  }
  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    height: auto;
    min-height: 168px;
    padding: 14px 14px 12px;
    text-align: left;
    border: 1px solid var(--home-border);
    border-radius: 10px;
    background: var(--home-surface);
    transition:
      border-color var(--dur-resize) var(--ease-out),
      box-shadow var(--dur-resize) var(--ease-out),
      transform 120ms var(--ease-out);
  }
  .card:hover:not(:disabled) {
    background: var(--home-surface);
    border-color: color-mix(in srgb, var(--home-accent) 45%, var(--home-border));
  }
  .card.picked {
    border-color: var(--home-accent);
    box-shadow: 0 0 0 1px var(--home-accent);
  }
  .card:disabled {
    opacity: 0.5;
  }
  .card-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    width: 100%;
  }
  .card-name {
    font-size: 16px;
    font-weight: 600;
  }
  .card-check {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    border-radius: 50%;
    background: var(--home-accent);
    color: var(--home-accent-text);
  }
  .card-check svg {
    width: 12px;
    height: 12px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2.2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  /* On the card's top edge, so every card's text starts at the same height. */
  .pill {
    position: absolute;
    top: -9px;
    left: 12px;
    padding: 1px 8px;
    border-radius: 10px;
    background: var(--home-accent);
    color: var(--home-accent-text);
    font-size: 11px;
    line-height: 16px;
    font-weight: 600;
    letter-spacing: 0.02em;
  }
  .card-line {
    color: var(--home-muted);
    font-size: 13px;
    line-height: 18px;
  }
  .card-facts {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin-top: auto;
    padding-top: 8px;
    width: 100%;
    border-top: 1px solid var(--home-border);
    font-size: 12px;
    line-height: 17px;
    color: var(--home-muted);
    font-variant-numeric: tabular-nums;
  }
  .card-why {
    color: var(--home-error);
  }

  /* Outside the steps (problem, done) the dots keep their space but do not show. */
  .dots.hidden {
    visibility: hidden;
  }
  .badge {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    margin-bottom: 16px;
    border-radius: 10px;
  }
  .badge svg {
    width: 20px;
    height: 20px;
    fill: none;
    stroke: currentColor;
    stroke-width: 2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .badge.warn {
    background: var(--home-error-bg);
    color: var(--home-error);
  }
  .badge.ok {
    background: var(--home-ok-soft);
    color: var(--home-ok);
  }
  .disclosure {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 28px;
    padding: 0 6px 0 2px;
    margin-left: -2px;
    border: none;
    background: none;
    color: var(--home-muted);
  }
  .disclosure svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: transform var(--dur-resize) var(--ease-out);
  }
  .disclosure[aria-expanded='true'] svg {
    transform: rotate(90deg);
  }
  .detail {
    max-height: 120px;
    overflow: auto;
    margin: 6px 0 0;
    padding: 10px 12px;
    border: 1px solid var(--home-border);
    border-radius: 6px;
    background: var(--home-surface);
    font: 12px/18px 'Cascadia Mono', Consolas, monospace;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    user-select: text;
  }
  button.primary {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
  }
  .spinner.on-accent {
    border-color: color-mix(in srgb, var(--home-accent-text) 30%, transparent);
    border-top-color: var(--home-accent-text);
  }
</style>
