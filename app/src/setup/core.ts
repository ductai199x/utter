// First run's link to the core (app/src-tauri/src/setup.rs). In a plain browser (`npm run dev`,
// then setup.html) a stand-in core plays every state so the page can be reviewed without the
// app: add ?fail=driver, ?fail=gpu, ?fail=download or ?fail=mic to see the failures.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export interface Check {
  id: 'gpu' | 'memory' | 'driver' | 'disk';
  ok: boolean;
  value: string;
  fix: string | null;
}

export interface Checks {
  checks: Check[];
  noGpu: boolean;
  downloadBytes: number;
}

export interface Progress {
  done: number;
  total: number;
  stage: 'downloading' | 'verifying' | 'extracting' | 'loading' | 'ready' | 'error';
  label: string;
  error?: string;
}

const inTauri = '__TAURI_INTERNALS__' in window;
const fail = new URLSearchParams(location.search).get('fail');

export function check(): Promise<Checks> {
  if (inTauri) return invoke<Checks>('setup_check');
  const checks: Check[] =
    fail === 'gpu'
      ? [{ id: 'gpu', ok: false, value: 'None found', fix: null }]
      : [
          { id: 'gpu', ok: true, value: 'NVIDIA GeForce RTX 3090 Ti', fix: null },
          { id: 'memory', ok: true, value: '24 GB', fix: null },
          fail === 'driver'
            ? {
                id: 'driver',
                ok: false,
                value: 'CUDA 12.4',
                fix: 'Update the NVIDIA driver to version 580 or newer, then check again.',
              }
            : { id: 'driver', ok: true, value: 'CUDA 13.4', fix: null },
          { id: 'disk', ok: true, value: '186.2 GB free, 7.8 GB needed', fix: null },
        ];
  return new Promise((r) => setTimeout(() => r({ checks, noGpu: fail === 'gpu', downloadBytes: 7.84e9 }), 600));
}

export const microphones = (): Promise<string[]> =>
  inTauri
    ? invoke<string[]>('setup_microphones')
    : Promise.resolve(['Microphone (Shure MV7)', 'Headset (Jabra Evolve2 65)']);

let mockLevel: ReturnType<typeof setInterval> | undefined;
const levelHandlers: ((l: number) => void)[] = [];

export function micStart(device: string): Promise<void> {
  if (inTauri) return invoke('setup_mic_start', { device });
  if (fail === 'mic' && device) return Promise.reject(new Error('no such device'));
  clearInterval(mockLevel);
  let t = 0;
  // Speech-like: syllables (about 5 a second) of varying loudness inside phrases, with pauses
  // between phrases and a little room noise.
  let syllable = 0;
  // 60 levels a second, as the core sends them.
  mockLevel = setInterval(() => {
    t += 16;
    if (t % 200 < 16) syllable = 0.004 + 0.1 * Math.random() ** 2;
    const phrase = Math.sin(t / 900) + Math.sin(t / 370) * 0.4 > 0.15 ? 1 : 0;
    const envelope = Math.sin(((t % 200) / 200) * Math.PI) ** 2;
    const level = 0.0015 * Math.random() + phrase * syllable * envelope;
    levelHandlers.forEach((f) => f(level));
  }, 16);
  return Promise.resolve();
}

export function micStop(): Promise<void> {
  if (inTauri) return invoke('setup_mic_stop');
  clearInterval(mockLevel);
  return Promise.resolve();
}

export function onLevel(f: (l: number) => void): Promise<UnlistenFn> {
  if (inTauri) return listen<number>('setup-level', (e) => f(e.payload));
  levelHandlers.push(f);
  return Promise.resolve(() => levelHandlers.splice(levelHandlers.indexOf(f), 1));
}

const progressHandlers: ((p: Progress) => void)[] = [];
let mockRun: ReturnType<typeof setInterval> | undefined;
let mockDone = 0;

export function download(): Promise<void> {
  if (inTauri) return invoke('setup_download');
  const total = 7.84e9;
  const parts: [number, string][] = [
    [1.32e9, 'GPU runtime'],
    [1.36e9, 'Speech model'],
    [5.16e9, 'Cleanup model'],
  ];
  const labelAt = (d: number) => {
    let sum = 0;
    for (const [size, label] of parts) {
      sum += size;
      if (d < sum) return label;
    }
    return 'Cleanup model';
  };
  const emit = (p: Progress) => progressHandlers.forEach((f) => f(p));
  clearInterval(mockRun);
  mockRun = setInterval(() => {
    // About 12 s for the whole download, with uneven bursts like a real connection.
    mockDone = Math.min(total, mockDone + total / 120 * (0.4 + Math.random() * 1.2));
    if (fail === 'download' && mockDone > total * 0.4) {
      clearInterval(mockRun);
      emit({ done: mockDone, total, stage: 'error', label: labelAt(mockDone), error: 'connection reset' });
      history.replaceState(null, '', location.pathname);
      return;
    }
    emit({ done: mockDone, total, stage: 'downloading', label: labelAt(mockDone) });
    if (mockDone >= total) {
      clearInterval(mockRun);
      emit({ done: total, total, stage: 'verifying', label: 'Cleanup model' });
      setTimeout(() => emit({ done: total, total, stage: 'loading', label: '' }), 1200);
      setTimeout(() => emit({ done: 0, total: 0, stage: 'ready', label: '' }), 4000);
    }
  }, 100);
  return Promise.resolve();
}

export function onProgress(f: (p: Progress) => void): Promise<UnlistenFn> {
  if (inTauri) return listen<Progress>('setup-progress', (e) => f(e.payload));
  progressHandlers.push(f);
  return Promise.resolve(() => progressHandlers.splice(progressHandlers.indexOf(f), 1));
}

export const finish = (): Promise<void> => (inTauri ? invoke('setup_finish') : Promise.resolve());

/** Quits Utter (no usable GPU): closing the window does it. */
export async function quit(): Promise<void> {
  if (!inTauri) return;
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().close();
}
