// Dev only (bar.html?demo): plays a dictation through the Bar as the core would, so the
// Bar can be looked at and iterated on without the native app. Not in the production build.
import { emitLocal } from './events';

const SENTENCE =
  'WaveRep with TF32 is now running at about 240 videos per minute, four times faster, with the 72k protocol-eligible videos projected to finish in about five hours and all 108k in about seven and a half hours; AIGVDBench level extraction will wrap in about six minutes, freeing more GPU capacity.';
const FINAL =
  'WaveRep with TF32 is now running at ~240 videos/min (4× faster), with the 72k protocol-eligible videos projected to finish in ~5 hours and all 108k in ~7.5 hours; AIGVDBench level extraction will wrap in ~6 minutes, freeing more GPU capacity.';

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function play() {
  for (;;) {
    emitLocal('bar:state', { state: 'listening', handsFree: false });
    const levels = setInterval(() => emitLocal('bar:levels', [0.02 + Math.random() * 0.12]), 16);
    const words = SENTENCE.split(' ');
    let committed: string[] = [];
    for (let i = 0; i < words.length; i++) {
      const preview = words.slice(committed.length, i + 1).join(' ');
      emitLocal('bar:words', { committed: committed.join(' '), preview, final: false });
      // A segment ends every 7 words.
      if ((i + 1) % 7 === 0) committed = words.slice(0, i + 1);
      await sleep(160);
    }
    clearInterval(levels);
    emitLocal('bar:state', { state: 'finishing' });
    await sleep(150);
    emitLocal('bar:words', { committed: FINAL, preview: '', final: true });
    emitLocal('bar:state', { state: 'done' });
    await sleep(1200);
    emitLocal('bar:state', { state: 'exit' });
    await sleep(600);
    emitLocal('bar:state', { state: 'hidden' });
    await sleep(800);
  }
}

void play();
