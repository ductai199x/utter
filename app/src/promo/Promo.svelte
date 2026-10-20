<script lang="ts">
  // The README's demo: one dictation into a chat, played through the real Bar (bar.html in a
  // frame the size of its window, sent the events the core sends). Timings follow the app's
  // measured ones (docs/evals.md): words show as they are heard, each pause brings back
  // the cleaned text, and the paste lands about a quarter second after the key goes up.
  // tools/media/record.py calls `window.promo.start()` and records until `done`.
  import { onMount } from 'svelte';

  // What is said, segment by segment, and the dictation cleaned up to the end of each.
  const SAID: [string, string][] = [
    ['hey sarah um yes I can take the release review', 'Hey Sarah, yes, I can take the release review'],
    ['on thursday no wait friday at ten', 'Hey Sarah, yes, I can take the release review on Friday at 10.'],
  ];
  const WORD_MS = 190;
  const PAUSE_MS = 320;
  const KEY_UP_TO_TEXT_MS = 250;

  let frame = $state<HTMLIFrameElement>();
  let caption = $state('Hold a key and speak.');
  let keyDown = $state(false);
  let typed = $state('');
  let landed = $state(false);

  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const emit = (name: string, detail: unknown) =>
    frame?.contentWindow?.dispatchEvent(new CustomEvent(name, { detail }));

  /** Microphone levels while speaking: syllables rising and falling, each one a little different
   *  (from a fixed seed, so every recording is the same). */
  function levels(): () => void {
    const start = performance.now();
    let seed = 7;
    const next = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
    const id = setInterval(() => {
      const t = (performance.now() - start) / 1000;
      const syllable = Math.max(0, Math.sin(t * 2 * Math.PI * 3.1));
      // The waveform draws decibels (lib/waveform.ts): between syllables speech drops far.
      emit('bar:levels', [0.002 + 0.15 * syllable ** 3 * (0.3 + 0.7 * next())]);
    }, 16);
    return () => clearInterval(id);
  }

  async function start() {
    await sleep(700);
    keyDown = true;
    emit('bar:state', { state: 'listening', handsFree: false });
    const stop = levels();
    let cleaned = '';
    for (const [raw, clean] of SAID) {
      const heard: string[] = [];
      for (const word of raw.split(' ')) {
        heard.push(word);
        emit('bar:words', { committed: cleaned, preview: heard.join(' '), final: false });
        await sleep(WORD_MS);
      }
      await sleep(PAUSE_MS);
      cleaned = clean;
      emit('bar:words', { committed: cleaned, preview: '', final: false });
      await sleep(240);
    }
    stop();
    keyDown = false;
    emit('bar:state', { state: 'finishing' });
    await sleep(KEY_UP_TO_TEXT_MS);
    emit('bar:words', { committed: cleaned, preview: '', final: true });
    emit('bar:state', { state: 'done' });
    typed = cleaned;
    landed = true;
    caption = 'Clean text where you type, a quarter second after you let go.';
    await sleep(1500);
    emit('bar:state', { state: 'exit' });
    await sleep(500);
    emit('bar:state', { state: 'hidden' });
    landed = false;
    caption = 'Utter. It all runs on your computer.';
    await sleep(1900);
  }

  onMount(() => {
    let finished = false;
    (window as unknown as { promo: object }).promo = {
      start: () => start().then(() => (finished = true)),
      get done() {
        return finished;
      },
    };
  });
</script>

<div class="stage">
  <p class="caption">{caption}</p>

  <div class="key" class:down={keyDown} aria-hidden="true">
    <span>Right Ctrl</span>
  </div>

  <div class="window">
    <header>
      <span class="avatar">S</span>
      <span class="who">Sarah Chen</span>
    </header>
    <div class="thread">
      <p class="bubble">Could you take the release review next week?</p>
    </div>
    <div class="compose" class:landed>
      {#if typed}<span>{typed}</span>{:else}<span class="placeholder">Message Sarah</span>{/if}<i class="caret"></i>
    </div>
  </div>

  <iframe bind:this={frame} src="/bar.html" title="The Bar"></iframe>
</div>

<style>
  :global(html),
  :global(body) {
    margin: 0;
    background: #0b0e18;
  }
  .stage {
    position: relative;
    width: 960px;
    height: 540px;
    overflow: hidden;
    font-family: var(--font-ui);
    background:
      radial-gradient(60% 70% at 18% 10%, rgba(74, 92, 240, 0.35), transparent 70%),
      radial-gradient(50% 60% at 90% 100%, rgba(127, 143, 255, 0.22), transparent 70%),
      linear-gradient(160deg, #10131f, #0b0e18 60%);
  }
  .caption {
    position: absolute;
    top: 30px;
    left: 0;
    right: 0;
    margin: 0;
    text-align: center;
    font-family: var(--font-display);
    font-size: 26px;
    font-weight: 600;
    letter-spacing: -0.01em;
    color: #e8ebf7;
  }
  .key {
    position: absolute;
    top: 96px;
    right: 64px;
    width: 112px;
    height: 48px;
    border-radius: 10px;
    display: grid;
    place-items: center;
    color: #c9cdea;
    font-size: 14px;
    background: linear-gradient(#262b44, #1b1f33);
    border: 1px solid rgba(150, 165, 255, 0.18);
    box-shadow:
      0 4px 0 #0d1020,
      0 8px 18px rgba(0, 0, 0, 0.4);
    transition:
      transform 90ms var(--ease-out),
      box-shadow 90ms var(--ease-out),
      color 90ms;
  }
  .key.down {
    transform: translateY(3px);
    color: #ffffff;
    box-shadow:
      0 1px 0 #0d1020,
      0 0 0 3px rgba(127, 143, 255, 0.45),
      0 6px 14px rgba(0, 0, 0, 0.4);
  }
  .window {
    position: absolute;
    top: 96px;
    left: 50%;
    transform: translateX(-50%);
    width: 560px;
    height: 300px;
    border-radius: 14px;
    background: #f6f7fb;
    box-shadow: 0 24px 60px rgba(0, 0, 0, 0.45);
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 14px 18px;
    border-bottom: 1px solid rgba(30, 40, 110, 0.1);
    background: #ffffff;
  }
  .avatar {
    width: 28px;
    height: 28px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    background: #4a5cf0;
    color: #ffffff;
    font-size: 13px;
    font-weight: 600;
  }
  .who {
    color: #161a2c;
    font-weight: 600;
    font-size: 14px;
  }
  .thread {
    flex: 1;
    padding: 18px;
  }
  .bubble {
    margin: 0;
    max-width: 70%;
    padding: 10px 14px;
    border-radius: 14px 14px 14px 4px;
    background: #ffffff;
    color: #161a2c;
    font-size: 14px;
    box-shadow: 0 1px 2px rgba(20, 30, 90, 0.08);
  }
  .compose {
    margin: 0 14px 14px;
    min-height: 22px;
    padding: 10px 14px;
    border-radius: 10px;
    background: #ffffff;
    border: 1px solid rgba(30, 40, 110, 0.14);
    color: #161a2c;
    font-size: 14px;
    line-height: 22px;
    transition: box-shadow 300ms var(--ease-out);
  }
  .compose.landed {
    box-shadow: 0 0 0 3px rgba(74, 92, 240, 0.25);
  }
  .placeholder {
    color: #9297b0;
  }
  .caret {
    display: inline-block;
    width: 1.5px;
    height: 17px;
    margin-left: 1px;
    vertical-align: -3px;
    background: #4a5cf0;
  }
  iframe {
    position: absolute;
    left: 50%;
    bottom: 4px;
    transform: translateX(-50%);
    width: 576px;
    height: 168px;
    border: 0;
    background: transparent;
  }
</style>
