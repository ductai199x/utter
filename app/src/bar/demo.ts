// Dev only (bar.html?demo): plays a dictation through the Bar as the core would, so the
// Bar can be looked at and iterated on without the native app. Not in the production build.
//
// It plays the live flow: words are heard (provisional, dimmed), and at each pause the
// dictation so far comes back cleaned (shown as text), including a correction that reaches
// back into what was already shown.
import { emitLocal } from './events';

// [raw words of a segment, the dictation cleaned up to and including that segment]
const SEGMENTS: [string, string][] = [
  ['so um the new build is running at about two forty videos per minute', 'The new build is running at about 240 videos per minute'],
  ['which is four x faster than before', 'The new build is running at about 240 videos per minute, which is 4× faster than before.'],
  ['and the full set should finish in five no wait seven and a half hours', 'The new build is running at about 240 videos per minute, which is 4× faster than before, and the full set should finish in 7.5 hours.'],
  ['freeing up more g p u capacity', 'The new build is running at about 240 videos per minute, which is 4× faster than before, and the full set should finish in 7.5 hours, freeing up more GPU capacity.'],
];

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function play() {
  for (;;) {
    emitLocal('bar:state', { state: 'listening', handsFree: false });
    const levels = setInterval(() => emitLocal('bar:levels', [0.02 + Math.random() * 0.12]), 16);
    let cleaned = '';
    let pending: string[] = [];
    for (const [raw, clean] of SEGMENTS) {
      for (const word of raw.split(' ')) {
        pending.push(word);
        emitLocal('bar:words', { committed: cleaned, preview: pending.join(' '), final: false });
        await sleep(170);
      }
      // A pause: the cleanup of everything so far comes back a moment later.
      await sleep(260);
      cleaned = clean;
      pending = [];
      emitLocal('bar:words', { committed: cleaned, preview: '', final: false });
      await sleep(200);
    }
    clearInterval(levels);
    emitLocal('bar:state', { state: 'finishing' });
    await sleep(60);
    emitLocal('bar:words', { committed: cleaned, preview: '', final: true });
    emitLocal('bar:state', { state: 'done' });
    await sleep(1400);
    emitLocal('bar:state', { state: 'exit' });
    await sleep(600);
    emitLocal('bar:state', { state: 'hidden' });
    await sleep(900);
  }
}

void play();
