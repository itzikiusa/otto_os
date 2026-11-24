// The status WORDS every list row shows (docs/design/guidelines/patterns.md
// "Inbox rows, status words"): one vocabulary app-wide on top of lib/status.ts,
// so a row never shows a bare dot or colour alone.
//   Thread / session rows: Approval · Question (or Needs you) · Failed ·
//     Limited · Working 18s · Done · Idle · Stopped.
//   Resources (connections, accounts, clusters): Connected · Can’t connect ·
//     Needs sign-in · Not checked.
// Each word carries one tone (the semantic tokens --warning / --danger /
// --success; '' is the calm --text-dim). Pure, import-free (unit/statusWords.test.ts).

/** The tone a word is drawn in; '' = calm (no colour). */
export type WordTone = 'warning' | 'danger' | 'success' | '';

/** A word and its tone, ready for a row's status slot. */
export interface StatusWord {
  word: string;
  tone: WordTone;
}

/** Where a row belongs, from its derived state key and unread flag. */
export type InboxClass = 'needs' | 'failed' | 'done' | 'working' | 'rest';

/** `pending`: requests this client knows are waiting on the person for the
 *  thread (its runtime approvals / questions from live events, an MCP
 *  approval card — threadInbox.svelte.ts `kindsOf`) or a subagent of it
 *  that waits: any one makes it "needs you", whatever the row's own status
 *  still says (a provider that keeps running while it waits, an MCP ask). */
export function inboxClass(key: string | null | undefined, unread: boolean, pending = 0): InboxClass {
  if (key === 'needs-you' || pending > 0) return 'needs';
  if (key === 'failed') return 'failed';
  // `running`: alive but idle — a thread whose agent is done while its
  // background work runs (taskStop.ts `withBackgroundState`) stays calm, never "Working".
  if (key === 'working' || key === 'stale') return 'working';
  return unread ? 'done' : 'rest';
}

/** The pending requests' kinds → the word a waiting row shows: Question when
 *  every one is a question (or an MCP form), else Approval. */
export function waitingWord(kinds: readonly string[]): 'Approval' | 'Question' {
  return kinds.length > 0 && kinds.every((k) => k === 'question' || k === 'mcp_elicitation') ? 'Question' : 'Approval';
}

/** The status word of a subagent row (a thread working under another one):
 *  exactly Working · Needs you · Done · Failed · Limited · Stopped (Limited:
 *  a failure on the usage limit — `limited`, the state label lib/status.ts gives it). */
export function subagentWord(key: string | null | undefined, opts?: { limited?: boolean }): 'Working' | 'Needs you' | 'Done' | 'Failed' | 'Limited' | 'Stopped' {
  if (key === 'needs-you') return 'Needs you';
  if (key === 'working' || key === 'running' || key === 'stale') return 'Working';
  if (key === 'failed') return opts?.limited ? 'Limited' : 'Failed';
  if (key === 'stopped') return 'Stopped';
  return 'Done';
}

/** "18s", "4m", "1h 5m" for a positive span. */
export function elapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  const m = Math.floor((s % 3600) / 60);
  return `${Math.floor(s / 3600)}h${m ? ` ${m}m` : ''}`;
}

/** The one status word a row shows, or null (it shows the relative time).
 *  The vocabulary is fixed: Approval · Question · Failed · Limited · Done ·
 *  Working. Limited: a failed turn that hit the provider's usage limit
 *  (`limited`, from the row's state label — lib/status.ts). */
export function statusWord(
  cls: InboxClass,
  opts: { kinds?: readonly string[]; workingSince?: number | null; now?: number; limited?: boolean } = {},
): string | null {
  switch (cls) {
    case 'needs':
      return waitingWord(opts.kinds ?? []);
    case 'failed':
      return opts.limited ? 'Limited' : 'Failed';
    case 'done':
      return 'Done';
    case 'working':
      return opts.workingSince && opts.now ? `Working ${elapsed(opts.now - opts.workingSince)}` : 'Working';
    default:
      return null;
  }
}

const WARNING_WORDS = new Set(['Approval', 'Question', 'Needs you', 'Limited', 'Needs sign-in']);
const DANGER_WORDS = new Set(['Failed', 'Can’t connect']);
const SUCCESS_WORDS = new Set(['Done', 'Connected']);

/** The one tone of a word of this vocabulary ("Working 18s" is calm). */
export function wordTone(word: string | null | undefined): WordTone {
  if (!word) return '';
  if (WARNING_WORDS.has(word)) return 'warning';
  if (DANGER_WORDS.has(word)) return 'danger';
  if (SUCCESS_WORDS.has(word)) return 'success';
  return '';
}

/** A session / thread state as lib/status.ts derives it (structural). */
export interface StateLike {
  key: string;
  label?: string;
}

/**
 * The word a session row shows on a page that is not the Threads sidebar
 * (Home, Agents): like {@link statusWord}, but a calm row still says what it
 * is — Idle, or Stopped once its process has ended — instead of a time.
 * `unread`: it finished while you looked elsewhere (Done). `kinds`: the
 * pending requests' kinds (Approval vs Question); none known → Needs you.
 */
export function sessionWord(
  state: StateLike,
  opts: { unread?: boolean; kinds?: readonly string[]; workingSince?: number | null; now?: number } = {},
): StatusWord {
  const limited = state.label === 'Limited';
  const cls = inboxClass(state.key, opts.unread === true, opts.kinds?.length ?? 0);
  let word: string;
  if (cls === 'needs') word = opts.kinds?.length ? waitingWord(opts.kinds) : 'Needs you';
  else if (cls === 'rest') word = state.key === 'stopped' || state.key === 'ended' ? 'Stopped' : 'Idle';
  else word = statusWord(cls, { workingSince: opts.workingSince, now: opts.now, limited }) ?? 'Idle';
  return { word, tone: wordTone(word) };
}

/** A resource's health as its page knows it. */
export type ResourceState = 'connected' | 'error' | 'auth' | 'unknown';

const RESOURCE_ALIASES: Record<string, ResourceState> = {
  connected: 'connected',
  ok: 'connected',
  up: 'connected',
  healthy: 'connected',
  error: 'error',
  failed: 'error',
  down: 'error',
  unreachable: 'error',
  auth: 'auth',
  unauthorized: 'auth',
  needs_auth: 'auth',
  expired: 'auth',
};

const RESOURCE_WORD: Record<ResourceState, string> = {
  connected: 'Connected',
  error: 'Can’t connect',
  auth: 'Needs sign-in',
  unknown: 'Not checked',
};

/** Connected · Can’t connect · Needs sign-in · Not checked (anything this
 *  vocabulary does not know — never "Unknown"). */
export function resourceWord(raw: string | null | undefined): StatusWord {
  const state = RESOURCE_ALIASES[(raw ?? '').trim().toLowerCase()] ?? 'unknown';
  const word = RESOURCE_WORD[state];
  return { word, tone: wordTone(word) };
}
