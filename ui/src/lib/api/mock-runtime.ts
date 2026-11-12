// Mock-mode structured-runtime threads (ui/src/lib/api/mock.ts routes it):
// just enough of `/workspaces/{id}/runtime-sessions` and `/runtime-sessions/*`
// for a page to find, start, prompt and embed a thread — the PR workspace's
// agent thread above all. No agent runs: a prompt is answered at once with a
// canned reply, and the conversation is served as a replay snapshot. The
// last create / prompt bodies are kept on `window.__mockRuntime` for e2e.
/* eslint-disable @typescript-eslint/no-explicit-any */
import type { RuntimeSession, RuntimeSnapshot, TurnItem } from './types';

type Out = { status?: number; json?: unknown } | undefined;
type MockRoute = { method: string; re: RegExp; handle: (m: RegExpMatchArray, body: any, q: URLSearchParams) => Out };

const threads: RuntimeSession[] = [];
const items = new Map<string, TurnItem[]>();
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
  };
}

const find = (id: string): RuntimeSession | undefined => threads.find((t) => t.id === id);

export const runtimeMockRoutes: MockRoute[] = [
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
    handle: (m) => ({ json: { kind: 'snapshot', snapshot_sequence: (items.get(m[1]) ?? []).length, snapshot: snapshot(m[1]) } }),
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
      return { status: 202, json: { turn_id: `trn_${t.id}`, sequence: (items.get(t.id) ?? []).length } };
    },
  },
];
