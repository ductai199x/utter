<script lang="ts">
  // What Utter does with your data, in plain words, on the welcome step and in Settings > About.
  // Every line is a fact of the code: the only network requests are the component and model
  // downloads (utter-runtime) and the update check and download (update.rs, which the person can
  // turn off and which installs only on Update); the cleanup model is served on this computer (127.0.0.1); the
  // only text read from other apps is up to 2 characters before the cursor, for spacing
  // (caret::BEFORE_CHARS), and the end of a paste, to see that it arrived. Change the words
  // with the code.
  let { compact = false }: { compact?: boolean } = $props();
  const here = navigator.userAgent.includes('Macintosh') ? 'Mac' : 'computer';
</script>

<section class="privacy" class:compact aria-label="Your privacy">
  <p class="head">
    <svg viewBox="0 0 16 16" aria-hidden="true"
      ><path d="M8 1.5l5 2v4c0 3.2-2.2 5.8-5 7-2.8-1.2-5-3.8-5-7v-4z" /><path d="M5.8 8l1.6 1.6 3-3.2" /></svg
    >
    Private by design
  </p>
  <ul>
    <li>Everything happens <b>on your {here}</b>. Nothing you say is <b>ever sent anywhere</b>.</li>
    <li>Utter only uses the internet <b>to download its models and updates</b>. After that, it <b>works offline</b>.</li>
    <li>It <b>never</b> reads your files or records your screen.</li>
    <li>Your history <b>stays on your {here}</b>. You can turn it off anytime.</li>
    <li>It's <b>open source</b>: anyone can check the code at github.com/ductai199x/utter.</li>
  </ul>
</section>

<style>
  .privacy {
    margin: 18px 0 0;
    padding: 12px 14px;
    border-radius: 8px;
    background: var(--home-accent-soft);
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--home-muted);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 6px;
    font-weight: 600;
    color: var(--home-text);
  }
  svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: var(--home-accent);
    stroke-width: 1.4;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  ul {
    margin: 0;
    padding-left: 18px;
  }
  li + li {
    margin-top: 3px;
  }
  b {
    font-weight: 600;
    color: var(--home-text);
  }
  .compact {
    margin-top: 14px;
    padding: 10px 12px;
    font-size: 12px;
  }
</style>
