// Plain-English "Ask Otto" engine, shared by the ⌘I palette sheet and the
// floating bar (through the `ask()` adapter in lib/ask.ts). Extracted from
// shell/Palette.svelte so both surfaces resolve a request the same way:
//
//   1. close / delete   — "close sessions 1,2", "close all claude sessions",
//                          "close ronaldo", "delete …" (permanent)
//   2. addressed send   — "send to messi hi", "session 1: hi", "all: hi"
//   3. literal plan     — "open 2 claude sessions" (deterministic, no LLM)
//   4. AI planner       — POST /orchestrate → a plan the caller must CONFIRM
//
// It never touches toasts or navigation: every path returns an `EnglishOutcome`
// and the caller renders it (a toast in the palette, a thread row in the bar).
// Session state comes in through `OrchestrateCtx`, so a window without the
// workspace store (the ⌥Space bar panel) can build one from the API.

import { api } from './api/client';
import type { Action, BroadcastResp, ExecuteResult, Id, OrchestrateResp, Session } from './api/types';
import { parseCommand, parseClose, startsWithCloseVerb, type CloseRequest } from './commandParser';
import { allProviders } from './providers';

export interface OrchestrateCtx {
  workspaceId: string;
  /** Session the request is "about" (the focused pane), for the AI planner. */
  focusedSessionId: string | null;
  /** Every session the workspace knows (provider-scoped closes). */
  sessions: Session[];
  /** Visible foreground agent sessions — the only ones that answer to a name. */
  nameable: Session[];
  /** On-screen order, i.e. how the user counts "session 2". */
  order: string[];
  /** Recoverable close (archive) and permanent delete of one session. */
  archive: (id: Id) => Promise<void>;
  kill: (id: Id) => Promise<void>;
  /** Rewrite the prompt before planning / fall through to the AI planner. */
  optimize: boolean;
  aiFallback: boolean;
  /** Return permanent deletes as `confirm-close` instead of running them. */
  confirmDestructive?: boolean;
}

export type EnglishOutcome =
  | { kind: 'empty' }
  | { kind: 'closed'; count: number; permanent: boolean }
  | { kind: 'confirm-close'; ids: Id[]; titles: string[]; permanent: boolean }
  | { kind: 'nothing-to-close' }
  | { kind: 'sent'; count: number; broadcast: boolean; message: string }
  | { kind: 'not-delivered' }
  | { kind: 'no-session' }
  | { kind: 'executed'; ok: number; fail: number; results: ExecuteResult[] }
  | { kind: 'plan'; plan: Action[]; optimizedText: string | null }
  | { kind: 'unparsed' };

// Words that must never be treated as a session NAME when resolving a
// "close <name>" command (verbs, fillers, nouns, providers, numbers).
const CLOSE_SKIP = new Set([
  'please', 'pls', 'kindly', 'close', 'kill', 'end', 'stop', 'terminate',
  'quit', 'remove', 'exit', 'shut', 'down', 'and', 'the', 'all', 'every',
  'everything', 'everyone', 'them', 'session', 'sessions', 'pane', 'panes',
  'tab', 'tabs', 'terminal', 'terminals', 'window', 'windows', 'agent', 'agents',
  'claude', 'codex', 'agy', 'shell', 'gemini', 'antigravity', 'gpt', 'bash', 'zsh',
]);

/** The folded tokens an open agent session answers to: its handle, title, and
 *  the individual words of both (so "diego" matches "Diego Ferreira"). */
function sessionAnswerTokens(s: { title: string; meta?: unknown }): Set<string> {
  const meta = s.meta as Record<string, unknown> | undefined;
  const handle = String(meta?.name_handle ?? s.title).toLowerCase();
  const full = String(meta?.name_full ?? '').toLowerCase();
  return new Set(
    [handle, s.title.toLowerCase(), ...full.split(/\s+/), ...s.title.toLowerCase().split(/\s+/)]
      .filter(Boolean),
  );
}

/** Open agent session ids whose answer-tokens include `tok` (lowercased).
 *  Only VISIBLE foreground sessions answer to a name — hidden background
 *  ones (workflow steps, review agents, …) share common title words like
 *  "open"/"tests" and would hijack ordinary commands if they counted. */
function matchSessionsByToken(ctx: OrchestrateCtx, tok: string): string[] {
  const ids: string[] = [];
  for (const s of ctx.nameable) {
    if (sessionAnswerTokens(s).has(tok)) ids.push(s.id);
  }
  return ids;
}

/** Remove the first `n` whitespace-delimited words from `text`. */
function stripLeadingWords(text: string, n: number): string {
  let rest = text;
  for (let i = 0; i < n; i++) {
    rest = rest.trimStart();
    const m = rest.search(/\s/);
    if (m === -1) return '';
    rest = rest.slice(m);
  }
  return rest.trimStart();
}

/** Resolve the open agent sessions a "close/delete <name>" command names. */
function resolveCloseNames(ctx: OrchestrateCtx, text: string): string[] {
  const toks = text
    .toLowerCase()
    .replace(/[:,]/g, ' ')
    .split(/\s+/)
    .filter((t) => t.length >= 2 && !CLOSE_SKIP.has(t) && !/^\d+$/.test(t));
  if (toks.length === 0) return [];
  const ids = new Set<string>();
  for (const t of toks) for (const id of matchSessionsByToken(ctx, t)) ids.add(id);
  return [...ids];
}

/** Close ids now (archive, or delete when `permanent`). */
export async function applyClose(ctx: OrchestrateCtx, ids: Id[], permanent: boolean): Promise<number> {
  for (const id of ids) {
    if (permanent) await ctx.kill(id);
    else await ctx.archive(id);
  }
  return ids.length;
}

/** Handle a close/delete command: by name, by provider (+ optional index), by
 *  on-screen position, or all. "close/end" archives (recoverable);
 *  "delete/kill/destroy" removes permanently. Returns null when nothing
 *  resolved (so the caller can fall through to AI for a free-form request like
 *  "delete the file foo"). */
async function handleClose(
  ctx: OrchestrateCtx,
  text: string,
  req: CloseRequest | null,
): Promise<EnglishOutcome | null> {
  const permanent = req?.permanent ?? /\b(kill|delete|destroy)\b/.test(text.toLowerCase());
  const order = ctx.order;

  let ids: string[] = [];
  // A structured target (provider / all / position) is a DELIBERATE close — we
  // warn rather than fall through when it matches nothing.
  const deliberate = !!req && (!!req.provider || req.all || req.positions.length > 0);
  if (req?.provider) {
    const prov = ctx.sessions.filter(
      (s) => !s.archived && s.kind === 'agent' && s.provider === req.provider,
    );
    ids =
      req.positions.length > 0
        ? req.positions.map((p) => prov[p - 1]?.id).filter((x): x is string => !!x)
        : prov.map((s) => s.id);
  } else if (req?.all) {
    ids = order.slice();
  } else if (req && req.positions.length > 0) {
    ids = req.positions.map((p) => order[p - 1]).filter((x): x is string => !!x);
  } else {
    // Name-based ("close ronaldo"). If nothing matches, fall through — the text
    // may be a free-form request ("delete the temp files") for the AI.
    ids = resolveCloseNames(ctx, text);
    if (ids.length === 0) return null;
  }

  if (ids.length === 0) {
    if (!deliberate) return null;
    return { kind: 'nothing-to-close' };
  }
  if (permanent && ctx.confirmDestructive) {
    const titles = ids.map((id) => ctx.sessions.find((s) => s.id === id)?.title ?? id);
    return { kind: 'confirm-close', ids, titles, permanent };
  }
  const count = await applyClose(ctx, ids, permanent);
  return { kind: 'closed', count, permanent };
}

/** Parse an addressed send and resolve its target session(s). Returns null
 *  when the text isn't an addressed send (caller falls through). */
function resolveAddress(
  ctx: OrchestrateCtx,
  text: string,
): { ids: string[]; broadcast: boolean; message: string } | null {
  let s = text.trim();
  let forceBroadcast = false;
  let hadVerb = false;
  // Optional leading send verb ("send to", "tell", "ask", "broadcast", …).
  const verb = s.match(/^(?:please\s+)?(send|tell|message|msg|ask|say|broadcast|relay|whisper)\b[:,]?\s*/i);
  if (verb) {
    hadVerb = true;
    forceBroadcast = /^broadcast$/i.test(verb[1]);
    s = s.slice(verb[0].length).replace(/^(?:a\s+message\s+to\s+|to\s+|the\s+)/i, '');
  }
  s = s.trim();
  const tokens = s.split(/\s+/).filter(Boolean);
  const order = ctx.order;
  const first = (tokens[0] ?? '').toLowerCase().replace(/[:,]+$/, '');

  // Broadcast: "all/everyone …" or the "broadcast" verb.
  if (forceBroadcast || /^(all|everyone|everybody)$/.test(first)) {
    const consumed = /^(all|everyone|everybody)$/.test(first) ? 1 : 0;
    const message = stripLeadingWords(s, consumed).replace(/^to\s+/i, '').trim();
    return { ids: [], broadcast: true, message };
  }

  // Position: "session 1", "pane 2", "#3".
  let pos: number | null = null;
  let consumed = 0;
  if (/^(session|sessions|pane|panes|tab|tabs|window|windows|number|no)$/.test(first) && /^\d+$/.test(tokens[1] ?? '')) {
    pos = parseInt(tokens[1], 10);
    consumed = 2;
  } else if (/^#\d+$/.test(tokens[0] ?? '')) {
    pos = parseInt(tokens[0].slice(1), 10);
    consumed = 1;
  }
  if (pos !== null) {
    const id = order[pos - 1];
    const message = stripLeadingWords(s, consumed).replace(/^to\s+/i, '').trim();
    return { ids: id ? [id] : [], broadcast: false, message };
  }

  // Bare (unverbed, un-colon'd) addressing must never shadow a command the
  // deterministic parser understands: "open claude session" is a spawn even
  // when some session's title happens to contain the word "open". Explicit
  // forms — a send verb ("tell messi …") or a colon ("messi: …") — still win.
  if (!hadVerb && !(tokens[0] ?? '').endsWith(':') && parseCommand(s, allProviders()) !== null) {
    return null;
  }

  // By name, greedily from the start ("ronaldo …", "ronaldo, messi: …").
  const matched: string[] = [];
  let consumedN = 0;
  for (let i = 0; i < tokens.length; i++) {
    const bare = tokens[i].toLowerCase().replace(/^@/, '').replace(/[:,]+$/, '');
    if (bare === '') break;
    if ((bare === 'and' || bare === '&') && matched.length > 0) {
      consumedN = i + 1;
      continue;
    }
    const hit = matchSessionsByToken(ctx, bare);
    if (hit.length === 0) break;
    for (const id of hit) if (!matched.includes(id)) matched.push(id);
    consumedN = i + 1;
    if (tokens[i].endsWith(':')) break;
  }
  if (matched.length === 0) return null; // not addressed → fall through
  const message = stripLeadingWords(s, consumedN).replace(/^[:,]\s*/, '').replace(/^to\s+/i, '').trim();
  return { ids: matched, broadcast: false, message };
}

/** Deliver a name/position/all-addressed message to the resolved session(s)
 *  via the broadcast endpoint. Returns null when the text isn't addressed. */
async function handleAddressedSend(ctx: OrchestrateCtx, text: string): Promise<EnglishOutcome | null> {
  const r = resolveAddress(ctx, text);
  if (!r) return null;
  const message = r.message.trim();
  if (message === '') return null; // nothing to send → let other handlers try
  if (!r.broadcast && r.ids.length === 0) return { kind: 'no-session' };
  const resp = await api.post<BroadcastResp>(`/workspaces/${ctx.workspaceId}/broadcast`, {
    text: message,
    session_ids: r.broadcast ? [] : r.ids,
  });
  const n = resp.session_ids.length;
  // The daemon only delivers to RUNNING sessions — targets can all be
  // suspended/exited, in which case "sent" would be a lie.
  if (n === 0) return { kind: 'not-delivered' };
  return { kind: 'sent', count: n, broadcast: r.broadcast, message };
}

/** Execute a (confirmed) plan. */
export async function executePlan(workspaceId: string, plan: Action[]): Promise<EnglishOutcome> {
  const resp = await api.post<{ results: ExecuteResult[] }>(
    `/workspaces/${workspaceId}/orchestrate/execute`,
    { plan },
  );
  const ok = resp.results.filter((r) => r.ok).length;
  return { kind: 'executed', ok, fail: resp.results.length - ok, results: resp.results };
}

/** Resolve one plain-English request. Throws on transport errors (the caller
 *  shows them); every other result is an outcome. */
export async function runEnglish(text: string, ctx: OrchestrateCtx): Promise<EnglishOutcome> {
  if (text.trim() === '') return { kind: 'empty' };

  // Close / delete commands, anchored on a leading close verb. A null result
  // (nothing resolved, e.g. "delete the file foo") falls through to the AI.
  const closeReq = parseClose(text, allProviders());
  if (closeReq || startsWithCloseVerb(text)) {
    const r = await handleClose(ctx, text, closeReq);
    if (r) return r;
  }

  // Addressed send: "send to messi hi", "send to session 1 hi", "tell ronaldo
  // to stand down", "messi: hi", "messi hi", "all: hi", "broadcast run tests".
  const sent = await handleAddressedSend(ctx, text);
  if (sent) return sent;

  // Deterministic first: common intents ("open 2 claude sessions") execute
  // instantly with no LLM and no confirmation step. Only fall through to the
  // AI planner when this can't parse the request AND AI fallback is on.
  const literal = parseCommand(text, allProviders());
  if (literal && literal.length > 0) return executePlan(ctx.workspaceId, literal);

  if (!ctx.aiFallback) return { kind: 'unparsed' };

  const resp = await api.post<OrchestrateResp>(`/workspaces/${ctx.workspaceId}/orchestrate`, {
    text,
    optimize: ctx.optimize,
    ai_fallback: ctx.aiFallback,
    focused_session_id: ctx.focusedSessionId,
  });
  return { kind: 'plan', plan: resp.plan, optimizedText: resp.optimized_text };
}

/** One-line description of a planned action. */
export function describeAction(a: Action): string {
  switch (a.action) {
    case 'spawn_sessions':
      return `Spawn ${a.count} ${a.provider} session${a.count === 1 ? '' : 's'}`;
    case 'broadcast':
      return `Broadcast to all sessions: "${a.text}"`;
    case 'open_connection':
      return `Open connection ${a.connection_id}`;
    case 'run_command':
      return `Send to session: "${a.text}"`;
  }
}

/** Plural helper for outcome copy. */
export function plural(n: number, word: string): string {
  return `${n} ${word}${n === 1 ? '' : 's'}`;
}
