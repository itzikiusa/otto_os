// Pure helpers for the floating "Type or speak… ⌘K" bar (FloatingBar.svelte,
// layout.md §7): spaces + their persistence, the result rows (commands, the
// "Ask Otto" row, search hits), key handling and when the in-app pill shows.
// No Svelte, no DOM, no imports beyond types — unit-tested under Node
// (unit/floatingBar.test.ts).

// ─── Spaces ─────────────────────────────────────────────────────────────────
// Four pinned contexts (01–04, ⌃1–⌃4 inside the bar). Each remembers a name,
// the workspace it asks in, a provider + model, and its last thread. Stored
// per device in localStorage and shared by both hosts (the main window's
// in-app bar and the ⌥Space panel run on the same origin).

export const SPACE_COUNT = 4;
export const SPACES_KEY = 'otto_bar_spaces';
/** Turns kept per space thread. */
export const MAX_THREAD = 20;
export const MAX_SPACE_NAME = 24;
export const DEFAULT_SPACE_NAMES = ['Personal', 'Work', 'Research', 'Home'] as const;

export type TurnTone = 'pending' | 'ok' | 'info' | 'warn' | 'error';

export interface BarTurn {
  id: string;
  /** What the user typed. */
  q: string;
  /** The answer (plain text; the bar never renders HTML from a reply). */
  a: string;
  tone: TurnTone;
  /** Secondary line (optimized prompt, counts, error detail). */
  detail?: string;
  /** Epoch ms. */
  at: number;
  /** Route "Open in Otto" jumps to (`agents/<id>`, `git/<repo>` …). */
  route?: string;
  /** Who answered — agent output is always attributed. */
  source?: string;
  /** Planned actions awaiting a person (never persisted: a reload drops them). */
  plan?: unknown[];
  /** Sessions a permanent delete would remove (confirm first; never persisted). */
  closeIds?: string[];
}

export interface BarSpace {
  name: string;
  /** Workspace the space asks in; null = whatever Otto has selected. */
  workspaceId: string | null;
  /** '' = Otto's default agent provider. */
  provider: string;
  /** '' = the provider's default model. */
  model: string;
  thread: BarTurn[];
}

export interface BarSpaces {
  /** 0-based active space. */
  active: number;
  spaces: BarSpace[];
}

export function defaultSpaces(): BarSpaces {
  return {
    active: 0,
    spaces: DEFAULT_SPACE_NAMES.map((name) => ({
      name,
      workspaceId: null,
      provider: '',
      model: '',
      thread: [],
    })),
  };
}

function str(v: unknown, max = 4000): string {
  return typeof v === 'string' ? v.slice(0, max) : '';
}

const TONES: readonly TurnTone[] = ['pending', 'ok', 'info', 'warn', 'error'];

function normalizeTurn(raw: unknown): BarTurn | null {
  if (!raw || typeof raw !== 'object') return null;
  const r = raw as Record<string, unknown>;
  const q = str(r.q);
  const a = str(r.a);
  if (q === '' && a === '') return null;
  let tone = (TONES as readonly unknown[]).includes(r.tone) ? (r.tone as TurnTone) : 'info';
  let detail = str(r.detail, 1000) || undefined;
  // A turn that was still waiting (a pending plan / confirm / in-flight ask)
  // when the page went away can't be resumed — say so instead of spinning.
  if (tone === 'pending') {
    tone = 'info';
    detail = 'Not run — the bar was closed before this finished.';
  }
  return {
    id: str(r.id, 64) || `t${Math.random().toString(36).slice(2, 10)}`,
    q,
    a,
    tone,
    detail,
    at: typeof r.at === 'number' && Number.isFinite(r.at) ? r.at : 0,
    route: str(r.route, 512) || undefined,
    source: str(r.source, 80) || undefined,
  };
}

function normalizeSpace(raw: unknown, i: number): BarSpace {
  const d = defaultSpaces().spaces[i];
  if (!raw || typeof raw !== 'object') return d;
  const r = raw as Record<string, unknown>;
  const name = str(r.name).trim().slice(0, MAX_SPACE_NAME) || d.name;
  const thread = Array.isArray(r.thread)
    ? r.thread.map(normalizeTurn).filter((t): t is BarTurn => t !== null).slice(-MAX_THREAD)
    : [];
  return {
    name,
    workspaceId: typeof r.workspaceId === 'string' && r.workspaceId !== '' ? r.workspaceId : null,
    provider: str(r.provider, 64),
    model: str(r.model, 128),
    thread,
  };
}

/** Parse persisted spaces, tolerating anything (old shapes, hand edits). */
export function parseSpaces(raw: string | null): BarSpaces {
  if (!raw) return defaultSpaces();
  let v: unknown;
  try {
    v = JSON.parse(raw);
  } catch {
    return defaultSpaces();
  }
  if (!v || typeof v !== 'object') return defaultSpaces();
  const r = v as Record<string, unknown>;
  const list = Array.isArray(r.spaces) ? r.spaces : [];
  const spaces = Array.from({ length: SPACE_COUNT }, (_, i) => normalizeSpace(list[i], i));
  const active = typeof r.active === 'number' && Number.isInteger(r.active) ? r.active : 0;
  return { active: clampSpace(active), spaces };
}

/** Serialize for storage: transient fields (pending plans, confirms) dropped. */
export function serializeSpaces(s: BarSpaces): string {
  return JSON.stringify({
    active: clampSpace(s.active),
    spaces: s.spaces.slice(0, SPACE_COUNT).map((sp) => ({
      name: sp.name,
      workspaceId: sp.workspaceId,
      provider: sp.provider,
      model: sp.model,
      thread: sp.thread.slice(-MAX_THREAD).map((t) => ({
        id: t.id,
        q: t.q,
        a: t.a,
        tone: t.tone,
        detail: t.detail,
        at: t.at,
        route: t.route,
        source: t.source,
      })),
    })),
  });
}

export function clampSpace(i: number): number {
  return Math.min(SPACE_COUNT - 1, Math.max(0, Math.trunc(i) || 0));
}

/** "01" … "04". */
export function spaceLabel(i: number): string {
  return String(clampSpace(i) + 1).padStart(2, '0');
}

/** Append a turn, keeping the last MAX_THREAD. */
export function pushTurn(thread: readonly BarTurn[], turn: BarTurn): BarTurn[] {
  return [...thread, turn].slice(-MAX_THREAD);
}

/** Replace a turn by id (merge), leaving the rest untouched. */
export function patchTurn(thread: readonly BarTurn[], id: string, patch: Partial<BarTurn>): BarTurn[] {
  return thread.map((t) => (t.id === id ? { ...t, ...patch } : t));
}

// ─── Result rows ────────────────────────────────────────────────────────────

export interface CommandLike {
  id: string;
  title: string;
}
export interface HitLike {
  kind: string;
  id: string;
  title: string;
}

export type BarRow<C, H> =
  | { kind: 'cmd'; key: string; cmd: C }
  | { kind: 'ask'; key: 'ask'; text: string }
  | { kind: 'hit'; key: string; hit: H };

/** True when `query` reads as a command name rather than a sentence for Otto:
 *  every word of it starts a word of the title ("go to home", "git", "new
 *  sess"). Fuzzy subsequence hits ("fix the tests" → "Focus Session: test…")
 *  don't count — free text defaults to asking Otto. */
export function looksLikeCommand(query: string, title: string): boolean {
  const split = (t: string): string[] => t.toLowerCase().split(/[\s:/·•—–-]+/).filter(Boolean);
  const q = split(query);
  if (q.length === 0) return false;
  const words = split(title);
  return q.every((w) => words.some((t) => t.startsWith(w)));
}

/** Rows for a query, in display order, with the Enter default first:
 *  - empty query → the ranked commands only (recents);
 *  - the top command reads like the query → commands, then "Ask Otto";
 *  - otherwise (free text) → "Ask Otto" first, then commands.
 *  Search hits always trail. Keys are unique (listbox option ids). */
export function buildRows<C extends CommandLike, H extends HitLike>(
  query: string,
  ranked: readonly C[],
  hits: readonly H[] = [],
): BarRow<C, H>[] {
  const q = query.trim();
  const cmds: BarRow<C, H>[] = ranked.map((cmd) => ({ kind: 'cmd', key: `cmd:${cmd.id}`, cmd }));
  if (q === '') return cmds;
  const ask: BarRow<C, H> = { kind: 'ask', key: 'ask', text: q };
  const hitRows: BarRow<C, H>[] = hits.map((hit) => ({ kind: 'hit', key: `hit:${hit.kind}:${hit.id}`, hit }));
  const commandFirst = ranked.length > 0 && looksLikeCommand(q, ranked[0].title);
  return commandFirst ? [...cmds, ask, ...hitRows] : [ask, ...cmds, ...hitRows];
}

/** Move a listbox selection by `delta`, wrapping; -1 when there are no rows. */
export function moveSelection(current: number, delta: number, count: number): number {
  if (count <= 0) return -1;
  if (current < 0) return delta > 0 ? 0 : count - 1;
  return (((current + delta) % count) + count) % count;
}

/** Module route for a cross-module search hit ("Open" target). */
export function hitRoute(hit: { kind: string; id: string }): string {
  switch (hit.kind) {
    case 'repo':
      return `git/${hit.id}`;
    case 'workflow':
      return 'workflows';
    case 'story':
      return 'product';
    case 'api_request':
      return 'api';
    case 'swarm_task':
    case 'swarm_project':
      return 'swarm';
    case 'broker_cluster':
      return 'brokers';
    case 'memory':
      return 'vault';
    default:
      return 'home';
  }
}

// ─── Keys ───────────────────────────────────────────────────────────────────

export interface KeyLike {
  key: string;
  metaKey: boolean;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  isComposing?: boolean;
}

export type BarKeyAction =
  | { type: 'next' }
  | { type: 'prev' }
  /** Enter: run the selected row (a command, or Ask Otto). */
  | { type: 'run' }
  /** ⌘↵ / ⌃↵: ask Otto with the text, whatever is selected. */
  | { type: 'ask' }
  | { type: 'escape' }
  | { type: 'space'; index: number };

/** What a keydown in the bar input means. Everything else types. */
export function barKeyAction(e: KeyLike): BarKeyAction | null {
  if (e.isComposing) return null; // IME candidate selection owns the keys
  const mods = e.metaKey || e.ctrlKey || e.altKey || e.shiftKey;
  // ⌃1–⌃4 switch spaces (ctrl only: ⌘1 is the sidebar toggle).
  if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && /^[1-4]$/.test(e.key)) {
    return { type: 'space', index: Number(e.key) - 1 };
  }
  switch (e.key) {
    case 'ArrowDown':
      return mods ? null : { type: 'next' };
    case 'ArrowUp':
      return mods ? null : { type: 'prev' };
    case 'Enter':
      if (e.metaKey || e.ctrlKey) return { type: 'ask' };
      return e.shiftKey || e.altKey ? null : { type: 'run' };
    case 'Escape':
      return { type: 'escape' };
    default:
      return null;
  }
}

// ─── In-app presence ────────────────────────────────────────────────────────
// The in-app pill floats over content, so it gets out of the way whenever the
// user is working:
//   full — the whole pill (focused / open, pinned, or Home, whose front door
//          it is)
//   rest — a short "✦ Type or speak… ⌘K" pill floating above the status bar
//   dock — a small "✦ Ask ⌘K" chip INSIDE the status bar, covering nothing
//          (while scrolling, or while a terminal / editor / other field has
//          the keyboard — the bar never takes keys from them)
//   away — gone (a sheet, modal or the palette is up)
//   off  — the user hid it; ⌘K opens the classic palette instead

export type BarPref = 'auto' | 'pinned' | 'docked' | 'hidden';
export type BarPresence = 'full' | 'rest' | 'dock' | 'away' | 'off';

export interface PresenceInput {
  pref: BarPref;
  /** The bar has focus or its panel is open. */
  open: boolean;
  /** Home: the surface where the bar is the page's front door. */
  surface: boolean;
  /** A terminal, editor or another text field owns the keyboard. */
  workFocus: boolean;
  scrolling: boolean;
  /** A modal, sheet or the palette is open above everything. */
  overlay: boolean;
}

export function barPresence(i: PresenceInput): BarPresence {
  if (i.pref === 'hidden') return 'off';
  if (i.overlay) return 'away';
  if (i.open) return 'full';
  if (i.pref === 'docked') return 'dock';
  if (i.pref === 'pinned') return i.workFocus ? 'rest' : 'full';
  if (i.scrolling || i.workFocus) return 'dock';
  // Auto floats the pill only on Home (the bar's surface). On every other page
  // a floating pill covers working content (API responses, result grids,
  // diffs), so it docks as the status-bar chip; ⌘K still opens it in full.
  return i.surface ? 'full' : 'dock';
}

export function parseBarPref(v: string | null): BarPref {
  return v === 'pinned' || v === 'docked' || v === 'hidden' ? v : 'auto';
}

// ─── Geometry ───────────────────────────────────────────────────────────────

/** Tallest the bar may grow (pill + thread), in both hosts. */
export const BAR_MAX_H = 560;

/** Height budget for the panel above an in-app pill whose top edge sits at
 *  `pillTop` (viewport px): never past `topMargin` from the top of the
 *  window, never taller than BAR_MAX_H minus the pill, never negative. */
export function panelBudget(pillTop: number, pillHeight: number, topMargin = 12): number {
  return Math.max(0, Math.min(BAR_MAX_H - pillHeight, Math.floor(pillTop - topMargin)));
}

/** Window height to request for the ⌥Space panel: pill + panel content,
 *  capped at BAR_MAX_H (the shell also clamps to the screen). */
export function barWindowHeight(pillHeight: number, panelContent: number): number {
  const h = pillHeight + Math.max(0, Math.ceil(panelContent));
  return Math.min(BAR_MAX_H, Math.max(pillHeight, h));
}
