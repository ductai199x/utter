<script lang="ts">
  // The splash (docs/ux/splash.md): a card while dictation loads. The core reports each
  // loading stage (splash.rs); this page lights it, and fades the card out once dictation is
  // ready and the card has been up three seconds.
  import { onMount } from 'svelte';
  import AnimatedMark from '../lib/AnimatedMark.svelte';
  import { Scene } from './scene';

  type StageId = 'speech' | 'cleanup' | 'microphone' | 'ready';
  const STAGES: { id: StageId; label: string; status: string }[] = [
    { id: 'speech', label: 'Speech', status: 'Loading the speech model...' },
    { id: 'cleanup', label: 'Cleanup', status: 'Loading the cleanup model...' },
    { id: 'microphone', label: 'Microphone', status: 'Opening the microphone...' },
    { id: 'ready', label: 'Ready', status: 'Ready. Hold {key} to dictate.' },
  ];
  const SHOWN_MS = 3000;
  const READY_HOLD_MS = 1200;
  const FADE_MS = 750;

  const inTauri = '__TAURI_INTERNALS__' in window;
  const params = new URLSearchParams(location.search);
  const demo = params.has('demo');
  // A demo stays up for review; ?demo&fade plays the fade too.
  const demoFade = params.has('fade');

  let current = $state(-1); // index of the stage loading now; STAGES.length - 1 when ready
  let leaving = $state(false);
  let version = $state('');
  let canvas = $state<HTMLCanvasElement>();
  const startedAt = performance.now();
  // The key held to dictate, named in the ready line.
  let keyName = $state('Right Ctrl');
  const status = $derived(current < 0 ? 'Starting...' : STAGES[current]!.status.replace('{key}', keyName));

  const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches;
  // The card's drawing area: 640 x 400 inside a window with room for its shadow.
  const scene = new Scene({ width: 640, height: 400, reducedMotion: reduced });

  function onStage(id: StageId) {
    const index = STAGES.findIndex((s) => s.id === id);
    if (index <= current) return;
    current = index;
    scene.pulse(performance.now());
    if (id === 'ready') {
      // At least three seconds up, and the ready line read, then the fade.
      const wait = Math.max(READY_HOLD_MS, SHOWN_MS - (performance.now() - startedAt));
      setTimeout(leave, wait);
    }
  }

  async function close() {
    if (inTauri) {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('splash_close').catch(() => {});
    }
  }

  function leave() {
    if (leaving || (demo && !demoFade)) return;
    leaving = true;
    if (!demo) setTimeout(close, FADE_MS);
  }

  /** Esc or a click: the card goes now; loading carries on, and the tray says when it is ready. */
  function dismiss() {
    leaving = true;
    setTimeout(close, 160);
  }

  onMount(() => {
    let raf = 0;
    const frame = (now: number) => {
      if (canvas) scene.draw(canvas, now);
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    const offs: (() => void)[] = [];
    if (inTauri) {
      // Loading starts before this page does: once listening, it asks where loading is, so a
      // stage reported before it was ready (even "ready") is never missed.
      void import('@tauri-apps/api/event').then(({ listen }) =>
        listen<{ stage: StageId }>('splash-stage', (e) => onStage(e.payload.stage)).then(async (off) => {
          offs.push(off);
          const { invoke } = await import('@tauri-apps/api/core');
          const now = await invoke<StageId | null>('splash_stage').catch(() => null);
          if (now) onStage(now);
        }),
      );
      void import('@tauri-apps/api/app').then(({ getVersion }) => getVersion().then((v) => (version = v)));
      void import('@tauri-apps/api/core').then(({ invoke }) =>
        invoke<string>('dictation_key').then((k) => (keyName = k), () => {}),
      );
    } else {
      version = '0.0.1';
    }
    if (demo) {
      // A browser review: the stages on a timer, as a real start reports them.
      const times: [number, StageId][] = [
        [400, 'speech'],
        [2600, 'cleanup'],
        [6200, 'microphone'],
        [7200, 'ready'],
      ];
      const timers = times.map(([ms, id]) => setTimeout(() => onStage(id), ms));
      offs.push(() => timers.forEach(clearTimeout));
    }
    return () => {
      cancelAnimationFrame(raf);
      offs.forEach((off) => off());
    };
  });
</script>

<svelte:window onkeydown={(e) => e.key === 'Escape' && dismiss()} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<div class="window">
  <div class="card" class:leaving role="presentation" onclick={dismiss}>
    <canvas bind:this={canvas} aria-hidden="true"></canvas>
    <div class="brand">
      <div class="mark"><AnimatedMark size={48} color="#6b7cff" /></div>
      <div>
        <div class="name">Utter</div>
        <div class="version">{version ? `Version ${version}` : ''}</div>
      </div>
    </div>
    <div class="progress">
      <ol class="stages" aria-label="Getting ready">
        {#each STAGES as s, i (s.id)}
          <li class:done={i < current || (s.id === 'ready' && current === i)} class:now={i === current && s.id !== 'ready'}>
            <span class="dot" aria-hidden="true"></span>{s.label}
          </li>
        {/each}
      </ol>
      <p class="status" aria-live="polite">{status}</p>
    </div>
  </div>
</div>

<style>
  :global(html, body) {
    margin: 0;
    height: 100%;
    background: transparent;
    overflow: hidden;
    user-select: none;
    cursor: default;
  }
  /* The window has 20 px of room around the card for its shadow. */
  .window {
    display: grid;
    place-items: center;
    width: 100%;
    height: 100%;
  }
  .card {
    position: relative;
    width: 640px;
    height: 400px;
    border-radius: 16px;
    overflow: hidden;
    background:
      radial-gradient(70% 60% at 62% 40%, #19224a 0%, rgba(25, 34, 74, 0) 70%),
      linear-gradient(160deg, #11152a 0%, #0b0e1b 100%);
    border: 1px solid rgba(255, 255, 255, 0.07);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.45);
    font-family: var(--font-ui);
    color: #eef1ff;
    animation: enter 320ms var(--ease-out) both;
    transition:
      opacity 750ms var(--ease-in),
      transform 750ms var(--ease-in);
  }
  .card.leaving {
    opacity: 0;
    transform: scale(0.985);
  }
  @keyframes enter {
    from {
      opacity: 0;
      transform: scale(0.985);
    }
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 640px;
    height: 400px;
  }
  /* The scene has the upper card; the name and the progress share the bottom edge. */
  .brand {
    position: absolute;
    left: 32px;
    bottom: 30px;
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .mark {
    display: flex;
    filter: drop-shadow(0 0 14px rgba(107, 124, 255, 0.45));
  }
  .progress {
    position: absolute;
    right: 36px;
    bottom: 32px;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
  }
  .name {
    font-size: 24px;
    line-height: 30px;
    font-weight: 600;
    letter-spacing: -0.01em;
  }
  .version {
    font-size: 12px;
    line-height: 16px;
    color: rgba(238, 241, 255, 0.5);
  }
  .stages {
    display: flex;
    gap: 14px;
    white-space: nowrap;
    list-style: none;
    margin: 0 0 8px;
    padding: 0;
    font-size: 12px;
    line-height: 16px;
    color: rgba(238, 241, 255, 0.38);
  }
  .stages li {
    display: flex;
    align-items: center;
    gap: 6px;
    transition: color 300ms var(--ease-out);
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    transition:
      background-color 300ms var(--ease-out),
      box-shadow 300ms var(--ease-out);
  }
  .stages li.done {
    color: rgba(238, 241, 255, 0.92);
  }
  .stages li.done .dot {
    background: #6b8cff;
    box-shadow: 0 0 8px rgba(107, 140, 255, 0.8);
  }
  .stages li.now {
    color: rgba(238, 241, 255, 0.85);
  }
  .stages li.now .dot {
    background: #6b8cff;
    animation: pulse 1.1s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .status {
    margin: 0;
    min-height: 20px;
    font-size: 14px;
    line-height: 20px;
    color: rgba(238, 241, 255, 0.72);
  }
  @media (prefers-reduced-motion: reduce) {
    .stages li.now .dot {
      animation: none;
    }
  }
</style>
