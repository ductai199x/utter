// The Home window's link to the core (app/src-tauri/src/home.rs). The page renders what the
// core says and sends intent; the core owns the file and applies it.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type Model = 'light' | 'quality';

export interface Settings {
  dictionary: string[];
  lists: boolean;
  start_at_login: boolean;
  microphone: string;
  model: Model;
}

export interface HomeState {
  settings: Settings | null;
  error: string | null;
  microphones: string[];
  qualityInstalled: boolean;
  qualityFits: boolean;
  version: string;
  dataDir: string;
  settingsPath: string;
}

export interface Applied {
  what: 'dictionary' | 'lists' | 'microphone' | 'model';
  status: 'loading' | 'ok' | 'error';
  error?: string;
}

const inTauri = '__TAURI_INTERNALS__' in window;

// In a plain browser (`npm run dev`, then home.html), a stand-in core keeps the page
// reviewable without the app: settings live in memory, a model switch takes two seconds.
const mock: HomeState = {
  settings: {
    dictionary: ['Supabase', 'Kubernetes', 'Nguyen'],
    lists: true,
    start_at_login: true,
    microphone: '',
    model: 'light',
  },
  error: null,
  microphones: ['Microphone (Shure MV7)', 'Headset (Jabra Evolve2 65)'],
  qualityInstalled: true,
  qualityFits: true,
  version: '0.0.1',
  dataDir: 'C:\\Users\\you\\.cache\\utter',
  settingsPath: 'C:\\Users\\you\\.cache\\utter\\settings.toml',
};
const appliedHandlers: ((a: Applied) => void)[] = [];
// JSON, not structuredClone: the page passes Svelte state proxies, which cannot be cloned.
const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v)) as T;

export const homeState = () => (inTauri ? invoke<HomeState>('home_state') : Promise.resolve(clone(mock)));
export const saveSettings = (settings: Settings) => {
  if (inTauri) return invoke<Settings>('save_settings', { settings });
  if (settings.model !== mock.settings!.model) {
    appliedHandlers.forEach((f) => f({ what: 'model', status: 'loading' }));
    setTimeout(() => appliedHandlers.forEach((f) => f({ what: 'model', status: 'ok' })), 2000);
  }
  mock.settings = clone(settings);
  return Promise.resolve(clone(settings));
};
export const openPath = (which: 'settings' | 'data') =>
  inTauri ? invoke<void>('open_path', { which }) : Promise.resolve();

export const onState = (f: (s: HomeState) => void): Promise<UnlistenFn> =>
  inTauri ? listen<HomeState>('home-state', (e) => f(e.payload)) : Promise.resolve(() => {});
export const onApplied = (f: (a: Applied) => void): Promise<UnlistenFn> => {
  if (inTauri) return listen<Applied>('home-applied', (e) => f(e.payload));
  appliedHandlers.push(f);
  return Promise.resolve(() => appliedHandlers.splice(appliedHandlers.indexOf(f), 1));
};

/** The rule the core enforces (settings::clean_terms), so the page can say why it refused. */
export function validTerm(t: string): boolean {
  return [...t].length <= 64 && [...t].filter((c) => /[\p{L}\p{N}]/u.test(c)).length >= 2;
}

export const MAX_TERMS = 2000;
