<script lang="ts">
  // First run (docs/ux/first-run.md): system check, microphone, download, a first dictation.
  // The core does the work and reports; this page shows it and sends intent.
  import { onDestroy, onMount, tick } from 'svelte';
  import { cubicOut } from 'svelte/easing';
  import { fade, fly } from 'svelte/transition';
  import * as core from './core';
  import type { Checks, Progress } from './core';
  import Logo from '../lib/Logo.svelte';
  import { Waveform } from '../lib/waveform';

  const STEPS = 4;
  const SENTENCE = "Hi team, the meeting moved to three, no, four o'clock. Bring the slides.";

  let step = $state(0);
  // +1 forward, -1 back: which side the next step slides in from.
  let dir = $state(1);
  let primary = $state<HTMLButtonElement>();

  // Step 1.
  let checks = $state<Checks | null>(null);
  const allOk = $derived(checks !== null && checks.checks.every((c) => c.ok));

  // Step 2.
  let mics = $state<string[]>([]);
  let device = $state('');
  let micError = $state('');
  const wave = new Waveform({ bars: 64, barW: 3, gap: 3, min: 3, max: 40 }, 90);
  let meter = $state<HTMLCanvasElement>();

  // Step 3.
  let progress = $state<Progress | null>(null);
  let started = false;
  let samples: { t: number; done: number }[] = [];
  let eta = $state('');

  // Step 4.
  let typed = $state('');
  let box = $state<HTMLTextAreaElement>();
  const arrived = $derived(typed.trim().length > 0);
  // The sample has a self-correction in it; when cleanup applied it, say so.
  const corrected = $derived(/\bfour\b/i.test(typed) && !/\bthree\b/i.test(typed));

  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  const slide = (node: Element) => fly(node, { x: reduced ? 0 : 8 * dir, duration: 120, easing: cubicOut });
  const leave = (node: Element) => fade(node, { duration: reduced ? 0 : 120 });

  async function go(next: number) {
    dir = next > step ? 1 : -1;
    if (step === 1 && next !== 1) await core.micStop();
    step = next;
    if (next === 1) void openMic();
    if (next === 2 && !started) void startDownload();
    await tick();
    // The main action takes the focus; on the last step, the text box does.
    setTimeout(() => (step === 3 ? box : primary)?.focus(), 130);
  }

  async function runChecks() {
    checks = null;
    checks = await core.check();
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
    if (meter) wave.draw(meter, accent, dt, step === 1);
    raf = requestAnimationFrame(frame);
  }

  async function startDownload() {
    started = true;
    samples = [];
    eta = '';
    // A retry resumes: the bar stays where it was until the core reports.
    progress = {
      done: progress?.done ?? 0,
      total: progress?.total || (checks?.downloadBytes ?? 0),
      stage: 'downloading',
      label: progress?.label ?? '',
    };
    await core.download();
  }

  function onProgress(p: Progress) {
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
    if (p.stage === 'ready') setTimeout(() => step === 2 && go(3), 500);
  }

  const gb = (b: number) => (b / 1e9).toFixed(1);
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
    void runChecks();
    void core.microphones().then((m) => (mics = m));
    const offs = [core.onLevel((l) => wave.push(l)), core.onProgress(onProgress)];
    raf = requestAnimationFrame(frame);
    void tick().then(() => primary?.focus());
    return () => {
      cancelAnimationFrame(raf);
      offs.forEach((p) => void p.then((off) => off()));
    };
  });
  onDestroy(() => void core.micStop());
</script>

<svelte:window onkeydown={onKey} />

<div class="frame">
  <div class="dots" aria-label="Step {step + 1} of {STEPS}" role="img">
    {#each Array(STEPS) as _, i (i)}
      <span class:done={i < step} class:current={i === step}></span>
    {/each}
  </div>

  <div class="stage">
    {#key step}
      <section in:slide out:leave>
        {#if step === 0}
          <div class="logo"><Logo size={36} /></div>
          <h1>Welcome to Utter</h1>
          <p class="lede">
            Hold a key, speak, and clean text appears where you type. Everything runs on this computer; nothing you
            say leaves it.
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
        {:else if step === 1}
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
        {:else if step === 2}
          <h1>Downloading the models</h1>
          <p class="lede">
            About {gb(checks?.downloadBytes ?? 0)} GB. You can use your computer meanwhile; Utter will let you know when
            it is ready.
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
            <span>{progress ? `${gb(progress.done)} of ${gb(progress.total)} GB` : ''}</span>
            <span>{eta}</span>
          </div>
          {#if progress?.stage === 'error'}
            <p class="error">Download interrupted. Retry picks up where it stopped.</p>
          {:else}
            <p class="muted">{stageLine}</p>
          {/if}
        {:else}
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
        {/if}
      </section>
    {/key}
  </div>

  <footer>
    {#if step === 0}
      {#if checks?.noGpu}
        <button class="primary" bind:this={primary} onclick={core.quit}>Quit</button>
      {:else if checks && !allOk}
        <button class="primary" bind:this={primary} onclick={runChecks}>Check again</button>
      {:else}
        <button class="primary" bind:this={primary} disabled={!checks} onclick={() => go(1)}>Continue</button>
      {/if}
    {:else if step === 1}
      <button class="text" onclick={() => go(0)}>Back</button>
      <button class="primary" bind:this={primary} onclick={() => go(2)}>Continue</button>
    {:else if step === 2}
      {#if progress?.stage !== 'error'}
        <p class="note">Closing this window pauses the download; it resumes next time.</p>
      {/if}
      {#if progress?.stage === 'error'}
        <button class="primary" bind:this={primary} onclick={startDownload}>Retry</button>
      {/if}
    {:else}
      <button class="text" onclick={core.finish}>Skip</button>
      <button class="primary" bind:this={primary} disabled={!arrived} onclick={core.finish}>Finish</button>
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
</style>
