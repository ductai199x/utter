// The setup window's link to the core (app/src-tauri/src/setup.rs). In a plain browser
// (`npm run dev`, then setup.html) a stand-in core plays every state so the page can be reviewed
// without the app:
// - ?mode=first|update|repair|problem picks the mode (the core opens the window with it);
// - ?problem=gpu-memory|no-microphone|damaged|driver|other picks the problem shown;
// - ?fail=driver|gpu|download|start|mic plays a failure once.
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
  stage: 'downloading' | 'verifying' | 'extracting' | 'loading' | 'ready' | 'error' | 'start-error';
  label: string;
  error?: string;
  problem?: Problem;
}

export interface Problem {
  kind: 'gpu-memory' | 'no-microphone' | 'damaged' | 'driver' | 'other';
  title: string;
  message: string;
  detail: string;
  actions: ('retry' | 'repair' | 'logs' | 'quit' | 'close')[];
}

export type Mode = 'first' | 'update' | 'model' | 'repair' | 'problem';

const params = new URLSearchParams(location.search);
/** Opened to carry on what the person already started (a repair that restarted Utter). */
export const carryOn = params.get('go') === '1';
export const mode = (): Mode => {
  const m = params.get('mode');
  return m === 'update' || m === 'model' || m === 'repair' || m === 'problem' ? m : 'first';
};

const inTauri = '__TAURI_INTERNALS__' in window;
// Cleared once a failure has played, so Retry then succeeds.
let fail = new URLSearchParams(location.search).get('fail');

// beforeChoice: first run's check, before the model step: the disk is checked against the
// smallest model on offer, which the person can still choose.
export function check(beforeChoice = false): Promise<Checks> {
  if (inTauri) return invoke<Checks>('setup_check', { beforeChoice });
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
          { id: 'disk', ok: true, value: '186.2 GB free, 6.0 GB needed', fix: null },
        ];
  return new Promise((r) => setTimeout(() => r({ checks, noGpu: fail === 'gpu', downloadBytes: 5.95e9 }), 600));
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
  const total = 5.95e9;
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
      fail = null;
      return;
    }
    emit({ done: mockDone, total, stage: 'downloading', label: labelAt(mockDone) });
    if (mockDone >= total) {
      clearInterval(mockRun);
      emit({ done: total, total, stage: 'verifying', label: 'Cleanup model' });
      setTimeout(() => emit({ done: total, total, stage: 'loading', label: '' }), 1200);
      setTimeout(mockStart, 4000);
    }
  }, 100);
  return Promise.resolve();
}

export function onProgress(f: (p: Progress) => void): Promise<UnlistenFn> {
  if (inTauri) return listen<Progress>('setup-progress', (e) => f(e.payload));
  progressHandlers.push(f);
  return Promise.resolve(() => progressHandlers.splice(progressHandlers.indexOf(f), 1));
}

const MOCK_PROBLEMS: Record<Problem['kind'], Problem> = {
  'gpu-memory': {
    kind: 'gpu-memory',
    title: 'Not enough free GPU memory',
    message:
      'Utter needs about 4.8 GB free on the graphics card; 2.1 GB are free now. Close games or other apps using the GPU, then retry.',
    detail: 'onnxruntime: Failed to allocate memory for requested buffer of size 52428800',
    actions: ['retry', 'quit'],
  },
  'no-microphone': {
    kind: 'no-microphone',
    title: 'No microphone found',
    message: 'Plug in a microphone or headset, then retry.',
    detail: 'no input device',
    actions: ['retry', 'quit'],
  },
  damaged: {
    kind: 'damaged',
    title: "Some of Utter's files are damaged",
    message: 'Repair checks every file and downloads again the ones that are damaged.',
    detail: 'model: Protobuf parsing failed',
    actions: ['repair', 'quit'],
  },
  driver: {
    kind: 'driver',
    title: 'The graphics driver could not run Utter',
    message: 'Update the NVIDIA driver, restart the computer, then retry.',
    detail: 'CUDA driver version is insufficient for CUDA runtime version',
    actions: ['retry', 'logs', 'quit'],
  },
  other: {
    kind: 'other',
    title: 'Utter could not start',
    message: 'llama-server exited during start (exit code: 3221225477)',
    detail: 'llama-server exited during start (exit code: 3221225477); see C:\\Users\\you\\.cache\\utter\\logs\\llama-server.log',
    actions: ['retry', 'logs', 'quit'],
  },
};
const mockProblem = (): Problem =>
  MOCK_PROBLEMS[(params.get('problem') as Problem['kind']) ?? 'gpu-memory'] ?? MOCK_PROBLEMS['gpu-memory'];

/** The stand-in's start: fails once with ?fail=start, then succeeds. */
function mockStart() {
  const emit = (p: Progress) => progressHandlers.forEach((f) => f(p));
  if (fail === 'start') {
    fail = null;
    const problem = mockProblem();
    emit({ done: 0, total: 0, stage: 'start-error', label: '', error: problem.message, problem });
  } else emit({ done: 0, total: 0, stage: 'ready', label: '' });
}

export type ModelId = 'quality' | 'light';

export interface ModelCard {
  id: ModelId;
  gpuBytes: number;
  cardBytes: number;
  downloadBytes: number;
  fits: boolean;
  recommended: boolean;
  selected: boolean;
}

// ?gpu=4 plays a 4 GB card (Quality does not fit, Light is recommended).
const mockGpuGb = Number(params.get('gpu') ?? 24);

export function models(): Promise<ModelCard[]> {
  if (inTauri) return invoke<ModelCard[]>('setup_models');
  const quality = mockGpuGb >= 5.4;
  return Promise.resolve([
    { id: 'quality', gpuBytes: 4.8e9, cardBytes: 5.8e9, downloadBytes: 5.95e9, fits: quality, recommended: quality, selected: true },
    { id: 'light', gpuBytes: 3.25e9, cardBytes: 4.25e9, downloadBytes: 4.14e9, fits: mockGpuGb >= 3.96, recommended: !quality, selected: false },
  ]);
}

export const choose = (model: ModelId): Promise<void> =>
  inTauri ? invoke('setup_choose', { model }) : Promise.resolve();

export const lastProblem = (): Promise<Problem | null> =>
  inTauri ? invoke<Problem | null>('setup_problem') : Promise.resolve(mockProblem());

export function retry(): Promise<void> {
  if (inTauri) return invoke('setup_retry');
  setTimeout(mockStart, 1800);
  return Promise.resolve();
}

export function repair(): Promise<void> {
  if (inTauri) return invoke('setup_repair');
  mockDone = 5.9e9;
  return download();
}

export const openLogs = (): Promise<void> =>
  inTauri ? invoke('open_path', { which: 'logs' }) : Promise.resolve();

export const finish = (): Promise<void> => (inTauri ? invoke('setup_finish') : Promise.resolve());
/** The name of the key held to dictate ("Right Ctrl"). */
export const dictationKey = (): Promise<string> =>
  inTauri ? invoke<string>('dictation_key') : Promise.resolve('Right Ctrl');

/** Quits Utter (no usable GPU): closing the window does it. */
export async function quit(): Promise<void> {
  if (!inTauri) return;
  const { getCurrentWindow } = await import('@tauri-apps/api/window');
  await getCurrentWindow().close();
}
