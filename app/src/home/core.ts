// The Home window's link to the core (app/src-tauri/src/home.rs). The page renders what the
// core says and sends intent; the core owns the file and applies it.
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';

export type Model = 'quality' | 'light';

export type KeepHistory = 'forever' | '30-days' | '24-hours' | 'off';
export type PasteWith = 'ctrl-v' | 'ctrl-shift-v' | 'shift-insert' | 'type';
export type PasteKeep = 'normal' | '3s' | '5s' | 'leave';

export interface PasteOverride {
  app: string;
  with: PasteWith;
  keep: PasteKeep;
}

/** settings.toml, every key: the core fills a missing one with its default, so a page that
 *  left one out would reset it on every save. */
export interface Settings {
  dictionary: string[];
  lists: boolean;
  start_at_login: boolean;
  /** The key held to dictate, by its name in settings.toml ("right-ctrl"). */
  dictation_key: string;
  microphone: string;
  model: Model;
  keep_history: KeepHistory;
  debug_log: boolean;
  keep_logs: boolean;
  paste: PasteOverride[];
}

export interface HomeState {
  settings: Settings | null;
  error: string | null;
  microphones: string[];
  qualityFits: boolean;
  lightAvailable: boolean;
  /** What choosing each model would download now, in bytes (0: installed). */
  downloadBytes: Record<Model, number>;
  /** GPU memory each model needs, speech included, and the smallest card it runs on (bytes). */
  gpuNeed: Record<Model, number>;
  cardNeed: Record<Model, number>;
  /** The keys one can dictate with: the id settings.toml uses, the name people read. */
  dictationKeys: { id: string; name: string }[];
  version: string;
  dataDir: string;
  settingsPath: string;
}

export interface Applied {
  what: 'dictionary' | 'lists' | 'microphone' | 'model' | 'key';
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
    dictation_key: 'right-ctrl',
    microphone: '',
    model: 'quality',
    keep_history: 'forever',
    debug_log: false,
    keep_logs: true,
    paste: [],
  },
  error: null,
  microphones: ['Microphone (Shure MV7)', 'Headset (Jabra Evolve2 65)'],
  qualityFits: true,
  lightAvailable: true,
  downloadBytes: { quality: 0, light: 4.14e9 },
  gpuNeed: { quality: 4.8e9, light: 3.25e9 },
  cardNeed: { quality: 5.8e9, light: 4.25e9 },
  dictationKeys: [
    { id: 'right-ctrl', name: 'Right Ctrl' },
    { id: 'left-ctrl', name: 'Left Ctrl' },
    { id: 'right-shift', name: 'Right Shift' },
    { id: 'scroll-lock', name: 'Scroll Lock' },
    { id: 'f13', name: 'F13' },
  ],
  version: '0.0.1',
  dataDir: 'C:\\Users\\you\\AppData\\Local\\Utter',
  settingsPath: 'C:\\Users\\you\\AppData\\Local\\Utter\\settings.toml',
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
export const openPath = (which: 'settings' | 'data' | 'logs') =>
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

// History (docs/ux/history.md) ---------------------------------------------------------------

export interface HistoryEntry {
  id: number;
  /** Milliseconds since 1970. */
  at: number;
  appExe: string;
  appName: string;
  text: string;
  heard: string;
  inserted: boolean;
  notInserted?: 'focus-moved' | 'no-window' | 'refused' | 'failed' | 'unconfirmed';
  words: number;
  speechS: number;
  readyMs: number;
  favorite: boolean;
}

export interface HistoryQuery {
  search: string;
  favorites: boolean;
  inserted: boolean | null;
  app: string | null;
  since: number | null;
  until: number | null;
  offset: number;
  limit: number;
}

export interface HistoryPage {
  entries: HistoryEntry[];
  matched: number;
  total: number;
  notInserted: number;
  favorites: number;
  apps: { exe: string; name: string; count: number }[];
}

export interface Change {
  kind: 'same' | 'removed' | 'added';
  text: string;
}

export const historyQuery = (query: HistoryQuery) =>
  inTauri ? invoke<HistoryPage>('history_query', { query }) : Promise.resolve(mockQuery(query));
export const historyFavorite = (id: number, favorite: boolean) => {
  if (inTauri) return invoke<void>('history_favorite', { id, favorite });
  const e = mockHistory.find((e) => e.id === id);
  if (e) e.favorite = favorite;
  return Promise.resolve();
};
export const historyDelete = (id: number): Promise<HistoryEntry | null> => {
  if (inTauri) return invoke<HistoryEntry | null>('history_delete', { id });
  const i = mockHistory.findIndex((e) => e.id === id);
  return Promise.resolve(i < 0 ? null : mockHistory.splice(i, 1)[0]!);
};
export const historyRestore = (entry: HistoryEntry) => {
  if (inTauri) return invoke<void>('history_restore', { entry });
  mockHistory.push(clone(entry));
  mockHistory.sort((a, b) => a.id - b.id);
  return Promise.resolve();
};
export const historyCopy = (text: string) =>
  inTauri ? invoke<void>('history_copy', { text }) : navigator.clipboard.writeText(text).catch(() => {});
export const historyChanges = (heard: string, text: string) =>
  inTauri ? invoke<Change[]>('history_changes', { heard, text }) : Promise.resolve(mockChanges(heard, text));
export const historyCountOlder = (keep: KeepHistory) =>
  inTauri
    ? invoke<number>('history_count_older', { keep })
    : Promise.resolve(keep === 'forever' ? 0 : keep === 'off' ? mockHistory.length : Math.floor(mockHistory.length / 3));
export const historyClear = () => {
  if (inTauri) return invoke<void>('history_clear');
  mockHistory.length = 0;
  return Promise.resolve();
};
export const onHistoryChanged = (f: () => void): Promise<UnlistenFn> =>
  inTauri ? listen('history-changed', () => f()) : Promise.resolve(() => {});

// Paste rules (docs/ux/insertion.md) ---------------------------------------------------------

export interface RuleRow {
  app: string;
  name: string;
  with: PasteWith;
  keep: PasteKeep;
  /** Utter's default for this program: [with, keep]. */
  default: [PasteWith, PasteKeep] | null;
  /** Dictations into this program (History). */
  dictations: number;
}

export interface AppRow {
  exe: string;
  name: string;
  dictations: number;
}

export interface PasteRules {
  rules: RuleRow[];
  openApps: AppRow[];
  /** Apps dictated into, most used first. */
  recentApps: AppRow[];
  lastApp: AppRow | null;
  /** The last dictation did not paste in `lastApp`: a rule may fix that app. */
  lastFailed: boolean;
}

export const pasteRules = (overrides: PasteOverride[]) =>
  inTauri ? invoke<PasteRules>('paste_rules', { overrides }) : Promise.resolve(mockRules(overrides));

/** Opens the Paste rules window (in a browser review, the page in a new tab). */
export const openPasteRules = () =>
  inTauri ? invoke<void>('open_paste_rules') : Promise.resolve(void window.open('rules.html', 'rules'));

export const WITH_LABELS: Record<PasteWith, string> = {
  'ctrl-v': 'Ctrl+V',
  'ctrl-shift-v': 'Ctrl+Shift+V',
  'shift-insert': 'Shift+Insert',
  type: 'Type it out',
};
export const KEEP_LABELS: Record<PasteKeep, string> = {
  normal: 'Normal',
  '3s': '3 seconds',
  '5s': '5 seconds',
  leave: 'Leave it there',
};

/** One row per app: programs of the same app (Remote Desktop's two) share a rule. */
export interface RuleGroup {
  name: string;
  apps: string[];
  with: PasteWith;
  keep: PasteKeep;
  default: [PasteWith, PasteKeep] | null;
  dictations: number;
}

export function groupRules(rows: RuleRow[]): RuleGroup[] {
  const out: RuleGroup[] = [];
  for (const r of rows) {
    const g = out.find(
      (g) => g.name === r.name && g.with === r.with && g.keep === r.keep && `${g.default}` === `${r.default}`,
    );
    if (g) {
      g.apps.push(r.app);
      g.dictations += r.dictations;
    } else out.push({ name: r.name, apps: [r.app], with: r.with, keep: r.keep, default: r.default, dictations: r.dictations });
  }
  return out;
}

/** Where a rule comes from: Utter's default as it is, a default the person changed, or theirs. */
export function ruleSource(g: RuleGroup): 'default' | 'changed' | 'yours' {
  if (!g.default) return 'yours';
  return g.default[0] !== g.with || g.default[1] !== g.keep ? 'changed' : 'default';
}

const sameApp = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** The person's rules with `g` set to `rule`; back at Utter's default it has no entry, so a
 *  better default in a later version still arrives. */
export function withRule(paste: PasteOverride[], g: RuleGroup, rule: Pick<PasteOverride, 'with' | 'keep'>): PasteOverride[] {
  const others = paste.filter((o) => !g.apps.some((a) => sameApp(o.app, a)));
  const isDefault = g.default && g.default[0] === rule.with && g.default[1] === rule.keep;
  return isDefault ? others : [...others, ...g.apps.map((app) => ({ app, ...rule }))];
}

/** The person's rules without `g`: a default goes back to Utter's, a rule of theirs goes. */
export const withoutRule = (paste: PasteOverride[], g: RuleGroup) =>
  paste.filter((o) => !g.apps.some((a) => sameApp(o.app, a)));

/** The person's rules with a new one for `exe` (Ctrl+V, Normal until they change it). */
export const withApp = (paste: PasteOverride[], exe: string, how: PasteWith = 'ctrl-v'): PasteOverride[] =>
  paste.some((o) => sameApp(o.app, exe)) ? paste : [...paste, { app: exe, with: how, keep: 'normal' }];

// The stand-in core's History: a believable week of dictations, so the page can be reviewed
// in a browser. Mock only; the app's History comes from history.rs.
const DAY = 24 * 3600 * 1000;
const mockHistory: HistoryEntry[] = (() => {
  const now = Date.now();
  const rows: [number, string, string, string, string, boolean?, HistoryEntry['notInserted']?][] = [
    [0.004, 'slack.exe', 'Slack', 'um so the deploy finished and the error rate is back under point one percent', 'The deploy finished, and the error rate is back under 0.1%.'],
    [0.03, 'chrome.exe', 'Google Chrome', 'can you summarize this thread and pull out the action items for me', 'Can you summarize this thread and pull out the action items for me?'],
    [0.09, 'Code.exe', 'Visual Studio Code', 'rename the function to parse config and make it return a result instead of panicking', 'Rename the function to parse_config and make it return a Result instead of panicking.'],
    [0.2, 'OUTLOOK.EXE', 'Outlook (classic)', 'hi maria thanks for the notes i will send the revised draft by thursday no actually friday morning', 'Hi Maria, thanks for the notes. I will send the revised draft by Friday morning.', false, 'focus-moved'],
    [0.45, 'slack.exe', 'Slack', 'lunch at twelve thirty works for me', 'Lunch at 12:30 works for me.'],
    [0.9, 'WINWORD.EXE', 'Microsoft Word', 'the results suggest that the model generalizes well to accents it never heard during training although the sample is small', 'The results suggest that the model generalizes well to accents it never heard during training, although the sample is small.'],
    [1.1, 'Notion.exe', 'Notion', 'groceries eggs milk sourdough and coffee beans', 'Groceries:\n- Eggs\n- Milk\n- Sourdough\n- Coffee beans'],
    [1.3, 'chrome.exe', 'Google Chrome', 'whats the difference between vulkan and cuda for running llama cpp on amd cards', "What's the difference between Vulkan and CUDA for running llama.cpp on AMD cards?"],
    [1.6, 'slack.exe', 'Slack', 'sounds good ill take a look after standup', "Sounds good, I'll take a look after standup."],
    [2.2, 'WindowsTerminal.exe', 'Windows Terminal', 'git log dash dash oneline dash twenty', 'git log --oneline -20'],
    [2.8, 'ms-teams.exe', 'Microsoft Teams', 'running about five minutes late start without me', 'Running about five minutes late, start without me.'],
    [3.5, 'Code.exe', 'Visual Studio Code', 'add a test for the case where the history file ends with a torn line', 'Add a test for the case where the history file ends with a torn line.'],
    [4.4, 'chrome.exe', 'Google Chrome', 'sam lee at example dot com', 'Sam Lee at example.com', false, 'no-window'],
    [5.2, 'slack.exe', 'Slack', 'congrats on the launch the new onboarding flow looks great', 'Congrats on the launch! The new onboarding flow looks great.'],
    [6.1, 'WINWORD.EXE', 'Microsoft Word', 'in summary we recommend shipping the light model as the default for cards with four gigabytes', 'In summary, we recommend shipping the Light model as the default for cards with 4 GB.'],
  ];
  // The rows are newest first; ids follow time, so the newest has the highest. Kept oldest first.
  return rows
    .map(([days, appExe, appName, heard, text, inserted = true, notInserted], i) => ({
    id: rows.length - i,
    at: now - days * DAY,
    appExe,
    appName,
    text,
    heard,
    inserted,
    notInserted,
    words: text.split(/\s+/).length,
    speechS: Math.round(heard.split(' ').length * 3.2) / 10,
    readyMs: 140 + ((i * 37) % 120),
    favorite: i === 5 || i === 2,
  }))
    .reverse();
})();

const fold = (s: string) =>
  s
    .normalize('NFD')
    .replace(/\p{M}/gu, '')
    .toLowerCase();

function mockQuery(q: HistoryQuery): HistoryPage {
  const needle = fold(q.search.trim());
  const all = [...mockHistory].reverse();
  const matching = all.filter(
    (e) =>
      (!q.favorites || e.favorite) &&
      (q.inserted === null || e.inserted === q.inserted) &&
      (!q.app || e.appExe === q.app) &&
      (q.since === null || e.at >= q.since) &&
      (q.until === null || e.at < q.until) &&
      (!needle || fold(e.text).includes(needle) || fold(e.heard).includes(needle)),
  );
  const apps = new Map<string, { exe: string; name: string; count: number }>();
  for (const e of mockHistory) {
    const a = apps.get(e.appExe) ?? { exe: e.appExe, name: e.appName, count: 0 };
    a.count++;
    apps.set(e.appExe, a);
  }
  return clone({
    entries: matching.slice(q.offset, q.offset + (q.limit || 50)),
    matched: matching.length,
    total: mockHistory.length,
    notInserted: mockHistory.filter((e) => !e.inserted).length,
    favorites: mockHistory.filter((e) => e.favorite).length,
    apps: [...apps.values()].sort((a, b) => b.count - a.count),
  });
}

/** The mock's stand-in for history::changes (the same rule: words, case and punctuation aside). */
function mockChanges(heard: string, text: string): Change[] {
  const a = heard.split(/\s+/).filter(Boolean);
  const b = text.split(/\s+/).filter(Boolean);
  const key = (w: string) => fold(w).replace(/[^\p{L}\p{N}]/gu, '');
  const ka = a.map(key);
  const kb = b.map(key);
  const lcs = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i--)
    for (let j = b.length - 1; j >= 0; j--)
      lcs[i]![j] = ka[i] && ka[i] === kb[j] ? lcs[i + 1]![j + 1]! + 1 : Math.max(lcs[i + 1]![j]!, lcs[i]![j + 1]!);
  const out: Change[] = [];
  const push = (kind: Change['kind'], w: string) => {
    const last = out[out.length - 1];
    if (last && last.kind === kind) last.text += ` ${w}`;
    else out.push({ kind, text: w });
  };
  let i = 0;
  let j = 0;
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && ka[i] && ka[i] === kb[j]) {
      push('same', b[j]!);
      i++;
      j++;
    } else if (i < a.length && (j === b.length || lcs[i + 1]![j]! >= lcs[i]![j + 1]!)) push('removed', a[i++]!);
    else {
      push(kb[j] ? 'added' : 'same', b[j]!);
      j++;
    }
  }
  return out;
}

const MOCK_DEFAULTS: [string, string, PasteWith, PasteKeep][] = [
  ['mintty.exe', 'Git Bash (mintty)', 'shift-insert', 'normal'],
  ['putty.exe', 'PuTTY', 'shift-insert', 'normal'],
  ['alacritty.exe', 'Alacritty', 'ctrl-shift-v', 'normal'],
  ['wezterm-gui.exe', 'WezTerm', 'ctrl-shift-v', 'normal'],
  ['WINWORD.EXE', 'Microsoft Word', 'ctrl-v', '3s'],
  ['OUTLOOK.EXE', 'Outlook (classic)', 'ctrl-v', '3s'],
  ['olk.exe', 'Outlook', 'ctrl-v', '3s'],
  ['WhatsApp.exe', 'WhatsApp', 'ctrl-v', '3s'],
  ['mstsc.exe', 'Remote Desktop', 'ctrl-v', '5s'],
  ['msrdc.exe', 'Remote Desktop', 'ctrl-v', '5s'],
  ['wfica32.exe', 'Citrix Workspace', 'ctrl-v', '5s'],
  ['CDViewer.exe', 'Citrix Workspace', 'ctrl-v', '5s'],
  ['TeamViewer.exe', 'TeamViewer', 'ctrl-v', '5s'],
  ['vncviewer.exe', 'VNC Viewer', 'ctrl-v', '5s'],
];

function mockRules(overrides: PasteOverride[]): PasteRules {
  const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
  const rules: RuleRow[] = MOCK_DEFAULTS.map(([app, name, w, k]) => {
    const mine = overrides.find((o) => same(o.app, app));
    return { app, name, with: mine?.with ?? w, keep: mine?.keep ?? k, default: [w, k], dictations: 0 };
  });
  const used = (exe: string) => mockHistory.filter((e) => same(e.appExe, exe)).length;
  rules.forEach((r) => (r.dictations = used(r.app)));
  const named = new Map(mockHistory.map((e) => [e.appExe.toLowerCase(), e.appName]));
  for (const o of overrides) {
    if (!rules.some((r) => same(r.app, o.app))) {
      const name = named.get(o.app.toLowerCase()) ?? o.app.replace(/\.exe$/i, '');
      rules.push({ app: o.app, name, with: o.with, keep: o.keep, default: null, dictations: used(o.app) });
    }
  }
  const recent = [...named.entries()]
    .map(([exe, name]) => ({ exe: mockHistory.find((e) => e.appExe.toLowerCase() === exe)!.appExe, name, dictations: used(exe) }))
    .sort((a, b) => b.dictations - a.dictations);
  return {
    rules,
    openApps: [
      { exe: 'chrome.exe', name: 'Google Chrome', dictations: used('chrome.exe') },
      { exe: 'slack.exe', name: 'Slack', dictations: used('slack.exe') },
      { exe: 'Code.exe', name: 'Visual Studio Code', dictations: used('Code.exe') },
      { exe: 'WindowsTerminal.exe', name: 'Windows Terminal', dictations: used('WindowsTerminal.exe') },
      { exe: 'Spotify.exe', name: 'Spotify', dictations: 0 },
    ],
    recentApps: recent,
    lastApp: recent.find((a) => a.exe === 'slack.exe') ?? null,
    lastFailed: true,
  };
}
