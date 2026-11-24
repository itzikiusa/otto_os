// Mock-mode structured-runtime threads (ui/src/lib/api/mock.ts routes it):
// just enough of `/workspaces/{id}/runtime-sessions` and `/runtime-sessions/*`
// for a page to find, start, prompt and embed a thread — the PR workspace's
// agent thread above all. No agent runs: a prompt is answered at once with a
// canned reply, and the conversation is served as a replay snapshot. The
// last create / prompt bodies are kept on `window.__mockRuntime` for e2e,
// with test hooks that seed rows and play live events (`seed`, `say`,
// `finish`, `queue`) for the Threads specs (e2e/*threads-groups*.spec.ts).
/* eslint-disable @typescript-eslint/no-explicit-any */
import { appLive } from '../live';
import type { QueuedPrompt, RuntimeRequest, RuntimeSession, RuntimeSnapshot, TurnItem } from './types';

type Out = { status?: number; json?: unknown } | undefined;
type MockRoute = { method: string; re: RegExp; handle: (m: RegExpMatchArray, body: any, q: URLSearchParams) => Out };

const threads: RuntimeSession[] = [];
const items = new Map<string, TurnItem[]>();
const queued = new Map<string, QueuedPrompt[]>();
const requests = new Map<string, RuntimeRequest[]>();
/** Per thread: the last event sequence (items, queue changes). */
const seqs = new Map<string, number>();
const bump = (sid: string): number => {
  const next = (seqs.get(sid) ?? 0) + 1;
  seqs.set(sid, next);
  return next;
};
let n = 0;
const now = (): string => new Date().toISOString();

function log(): { creates: any[]; prompts: any[] } {
  const w = window as any;
  return (w.__mockRuntime ??= { creates: [], prompts: [] });
}

function item(sid: string, kind: Record<string, unknown>): TurnItem {
  const list = items.get(sid) ?? [];
  const it = { ...kind, id: `itm_${sid}_${list.length}`, turn_id: `trn_${sid}`, ordinal: list.length, status: 'completed', title: null, started_at: now(), completed_at: now(), updated_at: now() } as unknown as TurnItem;
  items.set(sid, [...list, it]);
  bump(sid);
  return it;
}

function snapshot(sid: string): RuntimeSnapshot {
  const t = threads.find((x) => x.id === sid);
  return {
    provider_session: t ? { status: 'ready', cwd: t.cwd, model: null, updated_at: t.last_active_at, last_error: null } : null,
    provider_thread: null,
    turns: [],
    turn_items: items.get(sid) ?? [],
    runtime_requests: requests.get(sid) ?? [],
    plans: [],
    queued_prompts: queued.get(sid) ?? [],
  };
}

/** Deliver a runtime event the way the daemon's socket does. */
function emit(sid: string, body: Record<string, unknown>): void {
  appLive.dispatch({ type: 'agent_runtime_event', event: { session_id: sid, sequence: bump(sid), ts: now(), ...body } });
}

/** A request opened on `sid`: what the daemon publishes then — the request
 *  and the provider session `waiting` — so every client (rows, inbox) hears
 *  it as it would from a real agent. */
function announceWaiting(sid: string): void {
  const t = threads.find((x) => x.id === sid);
  for (const r of requests.get(sid) ?? []) emit(sid, { type: 'runtime_request.updated', runtime_request: r });
  emit(sid, { type: 'provider_session.updated', provider_session: { status: 'waiting', cwd: t?.cwd ?? '/', model: null, updated_at: now(), last_error: null } });
}

// e2e hooks (mock mode only — this module never loads otherwise).
Object.assign(log(), {
  /** Add a thread row as the daemon would list it (meta and all). */
  seed: (row: Partial<RuntimeSession> & { id: string; title: string }): string => {
    threads.push({
      workspace_id: 'wsp_otto', provider: 'claude', cwd: '/Users/dev/otto', status: 'idle', access_mode: 'approval-required',
      created_by: 'usr_root', created_at: now(), last_active_at: now(), last_sequence: 0, meta: null, ...row,
    } as RuntimeSession);
    return row.id;
  },
  /** The agent says `texts` live (one streamed-in message each). */
  say: (sid: string, texts: string[]): void => {
    for (const text of texts) {
      const it = item(sid, { type: 'assistant_message', message_id: `msg_${sid}_${Math.random()}`, text, streaming: false });
      appLive.dispatch({ type: 'agent_runtime_event', event: { session_id: sid, sequence: seqs.get(sid) ?? 0, ts: now(), type: 'turn_item.updated', turn_item: it } });
    }
  },
  /** `sid`'s agent starts (`running`) or stops (`ready`) working. */
  status: (sid: string, status: 'running' | 'ready'): void => {
    const t = threads.find((x) => x.id === sid);
    if (t) t.status = status === 'running' ? 'working' : 'idle';
    emit(sid, { type: 'provider_session.updated', provider_session: { status, cwd: t?.cwd ?? '/', model: null, updated_at: now(), last_error: null } });
  },
  /** A turn of `sid` ended (done). */
  finish: (sid: string): void => emit(sid, { type: 'turn.terminal', turn_id: `trn_${sid}`, status: 'completed' }),
  /** A message waits in `sid`'s queue. */
  queue: (sid: string, text: string): void => {
    queued.set(sid, [...(queued.get(sid) ?? []), { id: `qp_${sid}_${Date.now()}`, text, attachments: 0, created_at: now() }]);
    emit(sid, { type: 'queue.updated', queued_prompts: queued.get(sid) });
  },
});

const find = (id: string): RuntimeSession | undefined => threads.find((t) => t.id === id);

/** A mock thread row: idle, the prompt asked and answered at once. */
function addThread(ws: string, row: Partial<RuntimeSession> & { title: string; provider: RuntimeSession['provider'] }, prompt: string, answer: string): RuntimeSession {
  n += 1;
  const t = {
    id: `rts_mock${n}`, workspace_id: ws, cwd: '/Users/dev/otto', status: 'idle', access_mode: 'approval-required',
    created_by: 'usr_root', created_at: now(), last_active_at: now(), last_sequence: 0, meta: null, ...row,
  } as RuntimeSession;
  threads.push(t);
  item(t.id, { type: 'user_message', text: prompt });
  item(t.id, { type: 'assistant_message', message_id: `msg_${t.id}`, text: answer, streaming: false });
  return t;
}

const councilOf = (t: RuntimeSession | undefined): { id: string; seat: number; role: string } | undefined =>
  t?.meta?.council as { id: string; seat: number; role: string } | undefined;

/** The council `id` sits on: its seats in order. */
function councilSeats(id: string): RuntimeSession[] {
  const me = find(id);
  const c = councilOf(me);
  if (!me || !c) return [];
  const rootId = me.parent_id ?? me.id;
  return threads
    .filter((t) => (t.id === rootId || t.parent_id === rootId) && councilOf(t)?.id === c.id && councilOf(t)?.role === 'seat')
    .sort((a, b) => (councilOf(a)?.seat ?? 0) - (councilOf(b)?.seat ?? 0));
}

/** The judge's first message as the daemon builds it (council.rs
 *  `synthesis_prompt`): every seat's latest question and answer. */
function judgePrompt(seats: RuntimeSession[]): string {
  const last = (sid: string, type: string): string => {
    const hit = (items.get(sid) ?? []).filter((i) => i.type === type).pop() as { text?: string } | undefined;
    return hit?.text ?? '';
  };
  const parts = seats.map((t) => `## Answer ${councilOf(t)?.seat} — ${t.provider}\n\nAsked: ${last(t.id, 'user_message')}\n\n${last(t.id, 'assistant_message') || '(no answer)'}`);
  return `You are the judge of a council: ${seats.length} agents were asked the same question, and their latest answers are below.\n\n${parts.join('\n\n')}\n`;
}

// Council (docs/features/threads-structured-runtime.md "Council"). e2e:
// `__mockRuntime.councilFail = 'codex'` makes that provider's seat fail to
// start; each seat answers at once ("Mock <provider> (seat N): my answer.").
const councilRoutes: MockRoute[] = [
  {
    method: 'POST', re: /^\/workspaces\/([^/]+)\/runtime-sessions\/council$/,
    handle: (m, b) => {
      const l = log() as any;
      (l.councils ??= []).push(b);
      const seats: any[] = b?.seats ?? [];
      if (seats.length < 2 || seats.length > 4) return { status: 400, json: { code: 'invalid', message: `invalid: a council has 2 to 4 seats, not ${seats.length}` } };
      const bad = seats.findIndex((x) => x.provider === l.councilFail);
      if (bad >= 0) return { status: 400, json: { code: 'invalid', message: `invalid: seat ${bad + 1} (${seats[bad].provider}): could not start ${seats[bad].provider}` } };
      const id = `cnc_mock${n + 1}`;
      const out: RuntimeSession[] = [];
      seats.forEach((x, i) => {
        // council.rs `SeatPick::title`: "<title> · N · Claude [model] [Effort]".
        const who = [String(x.provider).charAt(0).toUpperCase() + String(x.provider).slice(1), x.model,
          x.effort ? String(x.effort).charAt(0).toUpperCase() + String(x.effort).slice(1) : null].filter(Boolean).join(' ');
        out.push(addThread(m[1], {
          provider: x.provider, model: x.model, title: `${b?.title ?? 'Council'} · ${i + 1} · ${who}`, cwd: b?.cwd ?? '/Users/dev/otto',
          parent_id: out[0]?.id ?? null,
          meta: { council: { id, seat: i + 1, role: 'seat', provider: x.provider, ...(x.model ? { model: x.model } : {}), ...(x.effort ? { effort: x.effort } : {}) }, read_only: true },
        }, String(b?.prompt ?? ''), `Mock ${x.provider} (seat ${i + 1}): my answer.`));
      });
      return { status: 201, json: { seats: out } };
    },
  },
  {
    method: 'POST', re: /^\/runtime-sessions\/([^/]+)\/council\/synthesize$/,
    handle: (m, b) => {
      const me = find(m[1]);
      const c = councilOf(me);
      if (!me || !c) return { status: 400, json: { code: 'invalid', message: 'invalid: this thread is not part of a council' } };
      const root = me.parent_id ?? me.id;
      const members = threads.filter((t) => (t.id === root || t.parent_id === root) && councilOf(t)?.id === c.id);
      const busy = members.find((t) => t.status === 'working');
      if (busy) return { status: 409, json: { code: 'conflict', message: `conflict: seat ${councilOf(busy)?.seat} is still working` } };
      const t1 = find(root)?.title ?? 'Council';
      const base = / · (Seat )?\d+ · /.test(t1) ? t1.replace(/ · (Seat )?\d+ · .*$/, '') : t1.replace(/ · [^·]*$/, '');
      const judge = addThread(me.workspace_id, {
        provider: b?.provider ?? 'claude', title: `${base} · compared`, parent_id: root,
        meta: { council: { id: c.id, seat: 0, role: 'synthesis', provider: b?.provider ?? 'claude' }, read_only: true },
      }, judgePrompt(councilSeats(me.id)), 'Mock synthesis: they agree on the approach.');
      return { status: 201, json: judge };
    },
  },
];

/** A notice in `sid`'s timeline, delivered live like the daemon's. */
function notice(sid: string, message: string): void {
  const it = item(sid, { type: 'system_notice', message });
  appLive.dispatch({ type: 'agent_runtime_event', event: { session_id: sid, sequence: seqs.get(sid) ?? 0, ts: now(), type: 'turn_item.updated', turn_item: it } });
}

// Threads inside a thread (docs/features/threads-structured-runtime.md
// "Threads inside a thread"). e2e: `__mockRuntime.childDone(childId, line)`
// writes the "finished" notice into the thread the child was asked from.
const childRoutes: MockRoute[] = [
  {
    method: 'POST', re: /^\/runtime-sessions\/([^/]+)\/children$/,
    handle: (m, b) => {
      const l = log() as any;
      (l.asks ??= []).push({ id: m[1], ...b });
      const me = find(m[1]);
      if (!me) return { status: 404, json: { code: 'not_found', message: 'thread' } };
      const targets: any[] = b?.targets ?? [];
      if (targets.length < 1 || targets.length > 4) return { status: 400, json: { code: 'invalid', message: `invalid: ask 1 to 4 agents at once, not ${targets.length}` } };
      const root = me.parent_id ?? me.id;
      const open = threads.filter((t) => t.status !== 'closed' && (t.id === root || t.parent_id === root)).length;
      if (open + targets.length > 9) return { status: 409, json: { code: 'conflict', message: `conflict: this thread already has ${open - 1} open threads beside it (at most 8); there is room for ${Math.max(0, 9 - open)} more — close one before asking another agent` } };
      const kids = targets.map((x) => addThread(me.workspace_id, {
        provider: x.provider, model: x.model ?? null, title: String(b?.prompt ?? 'Question').split('\n')[0].slice(0, 60), cwd: me.cwd,
        parent_id: root, access_mode: b?.access_mode ?? me.access_mode, meta: { child_of: { id: me.id, by: 'person' } },
      }, String(b?.prompt ?? ''), `Mock ${x.provider}: here is my answer.`));
      notice(me.id, `You asked ${targets.map((x) => x.provider).join(', ')}: ${String(b?.prompt ?? '').split('\n')[0]} [otto-thread:${kids.map((k) => k.id).join(',')}:started]`);
      return { status: 201, json: { children: kids } };
    },
  },
];

Object.assign(log(), {
  /** `parent`'s agent delegated to subagents (`otto.thread_open`), one turn:
   *  each kid a child thread (ULID id) with its state, plus the call in the
   *  parent's timeline. `working` kids count on the waiting line. */
  delegate: (parent: string, kids: { id: string; provider: string; title: string; working?: boolean; answer?: string }[]): void => {
    for (const k of kids) {
      threads.push({
        id: k.id, workspace_id: 'wsp_otto', provider: k.provider, title: k.title, cwd: '/Users/dev/otto', status: k.working ? 'working' : 'idle',
        access_mode: 'approval-required', created_by: 'usr_root', created_at: now(), last_active_at: now(), last_sequence: 0,
        parent_id: parent, meta: { child_of: { id: parent, by: 'agent' } }, last_turn_status: k.working ? null : 'completed',
      } as unknown as RuntimeSession);
      item(k.id, { type: 'user_message', text: `[Subagent task from thread "${find(parent)?.title ?? 'Thread'}" (claude, id ${parent}). Do the work yourself.]\n\n${k.title}` });
      if (k.answer) item(k.id, { type: 'assistant_message', message_id: `msg_${k.id}`, text: k.answer, streaming: false });
      const it = item(parent, {
        type: 'dynamic_tool', tool_name: 'mcp__otto__otto_thread_open', input: { provider: k.provider, title: k.title, prompt: k.title },
        output: JSON.stringify({ id: k.id, access_mode: 'approval-required', next: 'Completion wakes this thread.' }), error: null,
      });
      appLive.dispatch({ type: 'agent_runtime_event', event: { session_id: parent, sequence: seqs.get(parent) ?? 0, ts: now(), type: 'turn_item.updated', turn_item: it } });
    }
  },
  /** Subagent `sid` asks a question its parent's agent did not answer. */
  question: (sid: string, prompt: string): void => {
    const t = find(sid);
    if (t) t.status = 'awaiting_permission';
    requests.set(sid, [{ id: `req_q_${sid}`, turn_id: `trn_${sid}`, kind: 'question', status: 'pending', response_capability: { type: 'live' }, created_at: now(), resolved_at: null,
      questions: [{ id: 'db', header: 'Database', prompt, kind: 'single', required: true, allow_free_text: false, options: [{ value: 'pg', label: 'Postgres' }, { value: 'my', label: 'MySQL' }] }] } as unknown as RuntimeRequest]);
    announceWaiting(sid);
  },
  /** Subagent `sid` asks the person to approve running `command`. */
  approval: (sid: string, command: string): void => {
    const t = find(sid);
    if (t) t.status = 'awaiting_permission';
    const id = `req_${sid}`;
    requests.set(sid, [{ id, turn_id: `trn_${sid}`, kind: 'command', status: 'pending', response_capability: { type: 'live' }, created_at: now(), resolved_at: null } as RuntimeRequest]);
    item(sid, { type: 'approval_request', request_id: id, request_kind: 'command', prompt: command, options: [] });
    announceWaiting(sid);
  },
  /** Child `sid`'s turn ended: the thread it was asked from hears it. */
  childDone: (sid: string, line: string): void => {
    const kid = find(sid);
    const of = (kid?.meta?.child_of as { id?: string } | undefined)?.id;
    if (kid && of) notice(of, `${kid.provider} finished: ${line} [otto-thread:${sid}:finished]`);
  },
});

export const runtimeMockRoutes: MockRoute[] = [
  {
    method: 'POST', re: /^\/runtime-sessions\/([^/]+)\/stop-family$/,
    handle: (m, b) => {
      const l = log() as any;
      (l.familyStops ??= []).push({ id: m[1], ...b });
      const kids = threads.filter((t) => (t.meta?.child_of as { id?: string } | undefined)?.id === m[1] && t.status !== 'closed');
      const turns = kids.filter((t) => t.status === 'working').length;
      for (const k of kids) if (k.status === 'working') k.status = 'idle';
      return { json: { subagents: kids.length, turns, tasks: 0 } };
    },
  },
  {
    method: 'POST', re: /^\/runtime-sessions\/([^/]+)\/requests\/([^/]+)$/,
    handle: (m, b) => {
      const l = log() as any;
      (l.answers ??= []).push({ id: m[1], request: m[2], ...b });
      requests.set(m[1], (requests.get(m[1]) ?? []).map((r) => (r.id === m[2] ? { ...r, status: 'resolved', decision: b?.decision } : r)));
      const t = find(m[1]);
      if (t) t.status = 'working';
      for (const r of requests.get(m[1]) ?? []) emit(m[1], { type: 'runtime_request.updated', runtime_request: r });
      emit(m[1], { type: 'provider_session.updated', provider_session: { status: 'running', cwd: t?.cwd ?? '/', model: null, updated_at: now(), last_error: null } });
      return { status: 204 };
    },
  },
  ...councilRoutes,
  ...childRoutes,
  // The new-thread stage checks its folder before it sends (lib/folderCheck).
  { method: 'GET', re: /^\/fs\/stat$/, handle: (_m, _b, q) => ({ json: { path: q.get('path') ?? '', is_dir: true } }) },
  {
    method: 'GET', re: /^\/workspaces\/([^/]+)\/runtime-sessions$/,
    handle: (m, _b, q) => {
      const source = q.get('source');
      const origin = q.get('origin_id');
      const rows = threads.filter((t) => t.workspace_id === m[1]
        && (!source || t.meta?.source === source)
        && (!origin || (t.meta?.origin as { id?: string } | undefined)?.id === origin));
      return { json: [...rows].sort((a, b) => b.last_active_at.localeCompare(a.last_active_at)) };
    },
  },
  {
    method: 'POST', re: /^\/workspaces\/([^/]+)\/runtime-sessions$/,
    handle: (m, b) => {
      log().creates.push(b);
      n += 1;
      const t: RuntimeSession = {
        id: `rts_mock${n}`, workspace_id: m[1], provider: b?.provider ?? 'claude', title: b?.title ?? 'Thread',
        cwd: b?.cwd ?? '/Users/dev', status: 'idle', access_mode: b?.access_mode ?? 'approval-required',
        created_by: 'usr_root', created_at: now(), last_active_at: now(), last_sequence: 0,
        meta: b?.source ? { source: b.source, ...(b.origin ? { origin: b.origin } : {}) } : null,
        capabilities: { steer: true, images: true, files: true },
      };
      threads.push(t);
      return { status: 201, json: t };
    },
  },
  { method: 'GET', re: /^\/runtime-sessions\/([^/]+)$/, handle: (m) => { const t = find(m[1]); return t ? { json: t } : { status: 404, json: { code: 'not_found', message: 'thread' } }; } },
  { method: 'GET', re: /^\/runtime-sessions\/([^/]+)\/peers$/, handle: (m) => { const t = find(m[1]); return { json: t ? [{ id: t.id, provider: t.provider, title: t.title, status: t.status, is_self: true }] : [] }; } },
  {
    method: 'GET', re: /^\/runtime-sessions\/([^/]+)\/events$/,
    handle: (m) => ({ json: { kind: 'snapshot', snapshot_sequence: seqs.get(m[1]) ?? 0, snapshot: snapshot(m[1]) } }),
  },
  {
    method: 'DELETE', re: /^\/runtime-sessions\/([^/]+)\/queue\/([^/]+)$/,
    handle: (m) => {
      queued.set(m[1], (queued.get(m[1]) ?? []).filter((q) => q.id !== m[2]));
      emit(m[1], { type: 'queue.updated', queued_prompts: queued.get(m[1]) });
      return { status: 204 };
    },
  },
  { method: 'GET', re: /^\/runtime-sessions\/([^/]+)\/draft$/, handle: () => ({ json: { revision: 0, draft: null } }) },
  { method: 'PUT', re: /^\/runtime-sessions\/([^/]+)\/draft$/, handle: (_m, b) => ({ json: { revision: (b?.revision ?? 0) + 1, draft: b?.draft ?? null } }) },
  {
    method: 'POST', re: /^\/runtime-sessions\/([^/]+)\/(reopen|close)$/,
    handle: (m) => {
      const t = find(m[1]);
      if (!t) return { status: 404, json: { code: 'not_found', message: 'thread' } };
      t.status = m[2] === 'close' ? 'closed' : 'idle';
      return { json: t };
    },
  },
  {
    method: 'POST', re: /^\/runtime-sessions\/([^/]+)\/prompt$/,
    handle: (m, b) => {
      const t = find(m[1]);
      if (!t) return { status: 404, json: { code: 'not_found', message: 'thread' } };
      if (t.status === 'closed') return { status: 409, json: { code: 'conflict', message: 'the thread is closed: reopen it first' } };
      log().prompts.push({ id: t.id, ...b });
      if (b?.access_mode) t.access_mode = b.access_mode;
      t.last_active_at = now();
      item(t.id, { type: 'user_message', text: String(b?.text ?? '') });
      item(t.id, { type: 'assistant_message', message_id: `msg_${t.id}`, text: 'Mock agent: here is what this pull request changes.', streaming: false });
      return { status: 202, json: { turn_id: `trn_${t.id}`, sequence: seqs.get(t.id) ?? 0 } };
    },
  },
];
