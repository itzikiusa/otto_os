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
import type { QueuedPrompt, RuntimeSession, RuntimeSnapshot, TurnItem } from './types';

type Out = { status?: number; json?: unknown } | undefined;
type MockRoute = { method: string; re: RegExp; handle: (m: RegExpMatchArray, body: any, q: URLSearchParams) => Out };

const threads: RuntimeSession[] = [];
const items = new Map<string, TurnItem[]>();
const queued = new Map<string, QueuedPrompt[]>();
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
    runtime_requests: [],
    plans: [],
    queued_prompts: queued.get(sid) ?? [],
  };
}

/** Deliver a runtime event the way the daemon's socket does. */
function emit(sid: string, body: Record<string, unknown>): void {
  appLive.dispatch({ type: 'agent_runtime_event', event: { session_id: sid, sequence: bump(sid), ts: now(), ...body } });
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

// Council (docs/features/threads-structured-runtime.md "Council"). e2e:
// `__mockRuntime.councilFail = 'codex'` makes that provider's seat fail to start.
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
        const label = x.model ? `${x.provider} ${x.model}` : x.provider;
        out.push(addThread(m[1], {
          provider: x.provider, model: x.model, title: `${b?.title ?? 'Council'} · ${label}`, cwd: b?.cwd ?? '/Users/dev/otto',
          parent_id: out[0]?.id ?? null,
          meta: { council: { id, seat: i + 1, role: 'seat', ...(x.effort ? { effort: x.effort } : {}) }, read_only: true },
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
      const base = (find(root)?.title ?? 'Council').replace(/ · [^·]*$/, '');
      const judge = addThread(me.workspace_id, {
        provider: b?.provider ?? 'claude', title: `${base} · synthesis`, parent_id: root,
        meta: { council: { id: c.id, seat: 0, role: 'synthesis' }, read_only: true },
      }, 'Compare the answers.', 'Mock synthesis: they agree on the approach.');
      return { status: 201, json: judge };
    },
  },
];

export const runtimeMockRoutes: MockRoute[] = [
  ...councilRoutes,
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
