// Events from the core. Inside the app they are Tauri events; in a plain browser (the dev
// demo, `npm run dev` then bar.html?demo) the same names arrive as window CustomEvents.
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

const inTauri = '__TAURI_INTERNALS__' in window;

export function on<T>(name: string, handler: (e: { payload: T }) => void): Promise<UnlistenFn> {
  if (inTauri) return listen<T>(name, handler);
  const fn = (e: Event) => handler({ payload: (e as CustomEvent<T>).detail });
  window.addEventListener(name, fn);
  return Promise.resolve(() => window.removeEventListener(name, fn));
}

/** Dev only: play an event as the core would send it. */
export function emitLocal<T>(name: string, payload: T) {
  window.dispatchEvent(new CustomEvent(name, { detail: payload }));
}
