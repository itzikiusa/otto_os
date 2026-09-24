import type { Page, Route } from '@playwright/test';
import type {
  AssistantLimitState,
  AssistantMemory,
  AssistantMemoryView,
  AssistantRoutingSettings,
  AssistantTask,
  AssistantThread,
  AssistantTurn,
} from '../src/lib/api/types';

// A stateful stand-in for the Otto Assistant API (`/api/v1/assistant/*`, plus
// the usage summary and subscription accounts the routing page reads), served
// through page.route on top of the isolated e2e daemon — everything else
// (auth, workspaces, WS) is the real daemon. Mutations update the in-memory
// state so the UI can be driven end to end; `calls` records every write.

export interface AssistantMock {
  threads: AssistantThread[];
  turns: Record<string, AssistantTurn[]>;
  tasks: AssistantTask[];
  memory: AssistantMemoryView;
  routing: AssistantRoutingSettings;
  limits: AssistantLimitState[];
  calls: { method: string; path: string; body: unknown }[];
  /** Force a status for a path prefix (error-state checks). */
  fail: Record<string, number>;
}

const DAY = new Date();
// Fixture clock: "10:40 today" is 5 minutes ago, so everything the scenario
// did already happened (the live-row guards drop rows older than what they
// hold — a fixture in the future would make every server response "stale").
const ANCHOR = Date.now() - 5 * 60_000;
function at(h: number, m: number, dayOffset = 0): string {
  return new Date(ANCHOR + ((h * 60 + m) - (10 * 60 + 40)) * 60_000 + dayOffset * 86_400_000).toISOString();
}

function thread(id: string, title: string, slot: 1 | 2 | 3 | 4 | null, provider: string, model: string | null, updated: string): AssistantThread {
  return {
    id,
    space_slot: slot,
    title,
    provider,
    model,
    account_id: null,
    route_pinned: false,
    session_id: null,
    incognito: false,
    failover_choice: 'ask',
    status: 'idle',
    last_turn_at: updated,
    created_at: at(8, 0, -3),
    updated_at: updated,
  };
}

function turn(id: string, thread_id: string, role: AssistantTurn['role'], kind: AssistantTurn['kind'], text: string, created_at: string, extra: Partial<AssistantTurn> = {}): AssistantTurn {
  return { id, thread_id, role, kind, text, provider: null, model: null, route_reason: null, session_id: null, attachments: [], data: null, created_at, ...extra };
}

function task(id: string, extra: Partial<AssistantTask>): AssistantTask {
  return {
    id,
    thread_id: null,
    kind: 'task',
    state: 'running',
    title: id,
    detail: '',
    origin: 'app',
    run_at: null,
    timezone: 'Europe/Lisbon',
    schedule_id: null,
    agent_id: null,
    agent_run_id: null,
    needs_you: null,
    result: null,
    created_at: at(10, 30),
    updated_at: at(10, 30),
    finished_at: null,
    ...extra,
  };
}

function memoryRow(id: string, text: string, created_at: string, extra: Partial<AssistantMemory> = {}): AssistantMemory {
  return { id, text, kind: 'fact', tags: [], state: 'accepted', source: { kind: 'agent', thread_id: 'th-personal', file: null }, created_at, updated_at: created_at, ...extra };
}

export function assistantState(): AssistantMock {
  const P = 'th-personal';
  const SONNET = { provider: 'claude', model: 'claude-sonnet-4-5' };
  return {
    threads: [
      thread(P, 'Lisbon weekend', 1, 'claude', 'claude-sonnet-4-5', at(10, 38)),
      thread('th-work', 'Work', 2, 'claude', 'claude-sonnet-4-5', at(9, 50)),
      thread('th-research', 'Research', 3, 'codex', null, at(8, 20, -1)),
      thread('th-receipts', 'Receipts cleanup', null, 'codex', null, at(10, 5)),
      thread('th-bill', 'Electricity bill', null, 'claude', 'claude-sonnet-4-5', at(17, 10, -1)),
      thread('th-kitchen', 'Kitchen quotes', null, 'claude', null, at(12, 0, -2)),
    ],
    turns: {
      [P]: [
        turn('u1', P, 'user', 'message', 'Find me a hotel in Lisbon for Oct 10–12, under €180 a night near Alfama, and remind me Thursday to book the Sintra train.', at(10, 31)),
        turn('m1', P, 'system', 'memory', 'Remembered: city-break budget €180/night', at(10, 31, 0), {
          data: { action: 'remembered', memory_ids: ['mem-budget'], undo: { kind: 'delete', memory_id: 'mem-budget' } },
        }),
        turn('a1', P, 'assistant', 'message', 'On it. I’ll search three booking sites in the browser, filter for Alfama and quiet rooms, and set your reminder.', at(10, 32), SONNET),
        turn('r1', P, 'system', 'reminder', 'Reminder: Book the Sintra train', at(10, 32, 0), { data: { task_id: 'task-rem' } }),
        turn('t1', P, 'system', 'task', 'Comparing Alfama hotels', at(10, 32, 0), { data: { task_id: 'task-hotels' } }),
        turn(
          'a2',
          P,
          'assistant',
          'message',
          'Three good fits. All are quiet-rated, and two have rooms on high floors:\n\n| Hotel | € / night | Walk to Alfama | Noise in reviews |\n|---|---|---|---|\n| Casa do Largo | 164 | 3 min | “very quiet” ×12 |\n| Memória Suites | 178 | in Alfama | street noise on low floors |\n| Rio Terrace | 149 | 8 min | quiet, small rooms |',
          at(10, 38),
          SONNET,
        ),
        turn('d1', P, 'system', 'delegation', 'Asked Daily Recap to include the trip in Friday’s recap', at(10, 38, 0), { data: { task_id: 'task-del' } }),
        turn('ap1', P, 'system', 'approval', 'Approval: send the shortlist to Dana', at(10, 38, 0), { data: { task_id: 'task-appr' } }),
      ],
      'th-work': [
        turn('w1', 'th-work', 'user', 'message', 'Summarise the open PRs that are waiting on me.', at(9, 40)),
        turn('w2', 'th-work', 'assistant', 'message', 'Two PRs are waiting on your review: **#412** (payments retry) and **#415** (docs). Both have green CI.', at(9, 45), SONNET),
        turn('w3', 'th-work', 'system', 'limit', 'Claude usage limit reached — resets 14:00', at(9, 50), { data: { task_id: 'task-limit' } }),
      ],
      'th-research': [],
      'th-receipts': [
        turn('x1', 'th-receipts', 'user', 'message', '@codex put every receipt in ~/Documents/Receipts into a sheet by month', at(10, 4)),
        turn('x2', 'th-receipts', 'assistant', 'message', 'Reading 38 receipts now. I’ll write `receipts-2026.csv` next to them.', at(10, 5), { provider: 'codex', model: 'gpt-5-codex' }),
      ],
      'th-bill': [],
      'th-kitchen': [],
    },
    tasks: [
      task('task-rem', { thread_id: P, kind: 'reminder', state: 'queued', title: 'Book the Sintra train', run_at: new Date(DAY.getFullYear(), DAY.getMonth(), DAY.getDate() + 1, 9, 0).toISOString(), created_at: at(10, 32), updated_at: at(10, 32) }),
      task('task-hotels', {
        thread_id: P,
        title: 'Comparing Alfama hotels',
        detail: 'Filtering: Alfama, ≤ €180, quiet room notes',
        created_at: at(10, 32),
        updated_at: at(10, 36),
        result: {
          browser: {
            url: 'booking-site.example/lisbon?checkin=2026-10-10',
            tab_id: null,
            thumbnail_url: null,
            note: 'No logins needed. Booking would be an outward action and will ask first.',
            steps: [
              { label: 'Opened 3 sites in the assistant’s browser profile', state: 'done' },
              { label: 'Set dates Oct 10–12, 2 adults', state: 'done' },
              { label: 'Filtering: Alfama, ≤ €180, quiet room notes', state: 'current' },
              { label: 'Read reviews for noise', state: 'todo' },
              { label: 'Build shortlist', state: 'todo' },
            ],
          },
        },
      }),
      task('task-del', { thread_id: P, kind: 'delegation', title: 'Include the Lisbon trip in Friday’s recap', agent_id: 'pa-recap', result: { agent_name: 'Daily Recap' }, created_at: at(10, 38), updated_at: at(10, 38) }),
      task('task-appr', {
        thread_id: P,
        kind: 'approval',
        state: 'needs_you',
        title: 'Send the shortlist to Dana',
        created_at: at(10, 39),
        updated_at: at(10, 39),
        needs_you: {
          kind: 'approval',
          prompt: 'Send a message',
          approval: {
            where: 'Telegram → Dana (via your Otto bot)',
            what: 'Lisbon Oct 10–12, top 3 near Alfama:\n1) Casa do Largo €164, very quiet\n2) Memória Suites €178, ask for a high floor\n3) Rio Terrace €149, small but quiet\nWhich one?',
            who_sees: 'Dana only',
            reason: 'You said you travel with Dana; she usually picks the hotel.',
            tool: 'telegram_send',
            destination: 'Dana on Telegram',
            category: 'send',
            always_allow_allowed: true,
          },
        },
      }),
      task('task-limit', {
        thread_id: 'th-work',
        kind: 'limit',
        state: 'needs_you',
        title: 'Claude limit reached',
        created_at: at(9, 50),
        updated_at: at(9, 50),
        needs_you: {
          kind: 'limit',
          prompt: 'Claude limit reached',
          limit: { provider: 'claude', account_id: null, limited: true, until: at(14, 0), message: 'Usage limit reached · resets 2pm', source: 'pty', detected_at: at(9, 50) },
          suggestion: { provider: 'codex', model: null, account_id: null },
        },
      }),
      task('task-receipts', { thread_id: 'th-receipts', title: 'Receipts → sheet', detail: 'Reading 38 receipts', created_at: at(10, 5), updated_at: at(10, 20) }),
      task('task-plans', { thread_id: 'th-bill', title: 'Compare electricity plans', state: 'done', result: { summary: 'Switching saves about €14 a month.' }, created_at: at(17, 0, -1), updated_at: at(17, 10, -1), finished_at: at(17, 10, -1) }),
    ],
    memory: {
      profile: {
        content: '- Travels with Dana; she usually picks the hotel\n- Prefers quiet rooms on a high floor\n- Aisle seat on flights under 3 hours',
        version: 'v1',
        exists: true,
      },
      memories: [
        memoryRow('mem-budget', 'City-break budget €180/night', at(10, 31)),
        memoryRow('mem-quiet', 'Prefers quiet rooms, high floor', at(9, 0, -12), { tags: ['travel'] }),
        memoryRow('mem-dana', 'Travels with Dana', at(9, 0, -30), { source: { kind: 'user', thread_id: null, file: null } }),
      ],
      pending: [memoryRow('mem-h1', 'Zendesk macros live in the Support space', at(8, 0), { state: 'pending', source: { kind: 'hermes', thread_id: null, file: 'MEMORY.md' } })],
      memory_approval: false,
    },
    routing: {
      targets: {
        chat: { provider: 'claude', model: 'claude-sonnet-4-5', account_id: null },
        code: { provider: 'codex', model: null, account_id: null },
        hard: { provider: 'claude', model: 'claude-opus-4-1', account_id: null },
        voice: { provider: 'claude', model: 'claude-haiku-4-5', account_id: null },
      },
      extra_keywords: { code: [], hard: [] },
      auto_failover: false,
      memory_approval: false,
      updated_at: null,
    },
    limits: [{ provider: 'claude', account_id: null, limited: true, until: at(14, 0), message: 'Usage limit reached · resets 2pm', source: 'pty', detected_at: at(9, 50) }],
    calls: [],
    fail: {},
  };
}

function json(route: Route, body: unknown, status = 200): Promise<void> {
  return route.fulfill({ status, contentType: 'application/json', body: JSON.stringify(body) });
}

let seq = 0;
const now = (): string => new Date(Date.now() + ++seq).toISOString();

/** Serve the assistant API from `s` (mutating it on writes). Call before page.goto. */
export async function mockAssistant(page: Page, s: AssistantMock = assistantState()): Promise<AssistantMock> {
  // context.route: the daemon is cross-origin (page.route would miss it); specs
  // must also block the UI's fetch-proxying service worker (serviceWorkers: 'block').
  await page.context().route(/\/api\/v1\/(assistant|usage\/summary|auth\/provider-accounts)/, async (route) => {
    const req = route.request();
    const url = new URL(req.url());
    const path = url.pathname.replace(/^\/api\/v1/, '');
    const method = req.method();
    let body: unknown = null;
    try {
      body = req.postDataJSON();
    } catch {
      body = null;
    }
    if (method !== 'GET') s.calls.push({ method, path, body });
    for (const [prefix, status] of Object.entries(s.fail)) {
      if (path.startsWith(prefix)) return json(route, { code: 'fixture', message: 'Fixture failure' }, status);
    }
    const b = (body ?? {}) as Record<string, unknown>;
    const findTask = (id: string) => s.tasks.find((t) => t.id === id);
    const touch = (t: AssistantTask, patch: Partial<AssistantTask>) => Object.assign(t, patch, { updated_at: now() });

    // ── usage + accounts (routing page) ──
    if (path === '/usage/summary') {
      return json(route, { days: 7, providers: [
        { provider: 'claude', events: 320, input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, total_tokens: 4_120_000, cost_usd: 0 },
        { provider: 'codex', events: 140, input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, cache_write_tokens: 0, total_tokens: 1_870_000, cost_usd: 0 },
      ], daily: [], sessions: [], by_kind: [], total_events: 0, total_input_tokens: 0, total_output_tokens: 0, total_cache_read_tokens: 0, total_cache_write_tokens: 0, total_tokens: 0, total_cost_usd: 0 });
    }
    if (path === '/auth/provider-accounts') return json(route, [{ id: 'acc-personal', provider: 'claude', label: 'Max (personal)', created_at: at(8, 0, -40) }]);
    if (/^\/auth\/provider-accounts\/[^/]+\/status$/.test(path)) return json(route, { signed_in: true });

    // ── threads ──
    if (path === '/assistant/threads' && method === 'GET') return json(route, s.threads);
    if (path === '/assistant/threads' && method === 'POST') {
      const t = thread(`th-new-${s.threads.length}`, (b.title as string) || 'New thread', (b.space_slot as 1 | 2 | 3 | 4 | null) ?? null, 'claude', null, now());
      s.threads.push(t);
      s.turns[t.id] = [];
      return json(route, t);
    }
    let m = /^\/assistant\/threads\/([^/]+)(\/[a-z]+)?$/.exec(path);
    if (m) {
      const t = s.threads.find((x) => x.id === m![1]);
      if (!t) return json(route, { code: 'not_found', message: 'No such thread' }, 404);
      const sub = m[2] ?? '';
      if (sub === '' && method === 'GET') return json(route, t);
      if (sub === '/turns' && method === 'GET') return json(route, s.turns[t.id] ?? []);
      if (sub === '/turns' && method === 'POST') {
        const text = String(b.text ?? '');
        const mention = /^\s*@(claude|codex)\b[\s,:;]*/i.exec(text);
        const u = turn(`u-${now()}`, t.id, 'user', 'message', mention ? text.slice(mention[0].length) : text, now(), { provider: mention ? mention[1].toLowerCase() : t.provider, route_reason: mention ? 'mention' : 'default' });
        (s.turns[t.id] ??= []).push(u);
        t.status = 'working';
        t.updated_at = now();
        return json(route, { turn: u, route: { provider: u.provider, model: null, account_id: null, kind: 'chat', reason: u.route_reason, matched: [], text: u.text }, thread: t });
      }
      if (sub === '/route' && method === 'POST') {
        if (b.provider == null) Object.assign(t, { route_pinned: false });
        else Object.assign(t, { route_pinned: true, provider: b.provider as string, model: (b.model as string | null) ?? null });
        t.updated_at = now();
        return json(route, t);
      }
      if (sub === '/attachments' && method === 'POST') {
        return json(route, { id: `att-${now()}`, name: String(b.name), path: `/inbox/${String(b.name)}`, mime: String(b.mime ?? 'application/octet-stream'), size: 1234 });
      }
    }

    // ── tasks / needs you ──
    if (path === '/assistant/needs-you') return json(route, s.tasks.filter((t) => t.state === 'needs_you'));
    if (path === '/assistant/tasks' && method === 'GET') return json(route, [...s.tasks].sort((a, c) => (a.updated_at < c.updated_at ? 1 : -1)));
    m = /^\/assistant\/tasks\/([^/]+)(?:\/([a-z]+))?$/.exec(path);
    if (m) {
      const t = findTask(m[1]);
      if (!t) return json(route, { code: 'not_found', message: 'No such task' }, 404);
      const action = m[2];
      if (!action) return json(route, t);
      if (action === 'approve') {
        if (t.kind === 'limit') {
          const th = s.threads.find((x) => x.id === t.thread_id);
          if (th) Object.assign(th, { provider: b.provider, model: null, updated_at: now() });
        }
        touch(t, { state: 'done', needs_you: null, result: { ...(t.result ?? {}), decision: 'approved' }, finished_at: now() });
      } else if (action === 'deny') touch(t, { state: 'cancelled', needs_you: null, result: { ...(t.result ?? {}), decision: 'denied', reason: b.reason ?? null } });
      else if (action === 'takeover') touch(t, { state: 'needs_you', needs_you: { kind: 'takeover', prompt: 'You have the browser' } });
      else if (action === 'handback') touch(t, { state: 'running', needs_you: null });
      else if (action === 'cancel') touch(t, { state: 'cancelled' });
      return json(route, t);
    }

    // ── memory ──
    if (path === '/assistant/memory' && method === 'GET') return json(route, s.memory);
    if (path === '/assistant/memory' && method === 'PUT') {
      const p = (b.profile ?? {}) as { content?: string; version?: string };
      if (p.version !== s.memory.profile.version) return json(route, { code: 'conflict', message: 'Stale profile version' }, 409);
      s.memory.profile = { content: p.content ?? '', version: `v${Number(s.memory.profile.version.slice(1)) + 1}`, exists: true };
      return json(route, s.memory.profile);
    }
    if (path === '/assistant/memory/undo') {
      const id = String(b.undo_token ?? '').replace(/^undo-/, '');
      const row = memoryRow(id, `Restored ${id}`, now());
      s.memory.memories.unshift(row);
      return json(route, row);
    }
    m = /^\/assistant\/memory\/([^/]+)(\/accept)?$/.exec(path);
    if (m && m[1] !== 'import') {
      const id = m[1];
      if (m[2] && method === 'POST') {
        const i = s.memory.pending.findIndex((x) => x.id === id);
        const row = { ...s.memory.pending[i], state: 'accepted' as const };
        s.memory.pending.splice(i, 1);
        s.memory.memories.unshift(row);
        return json(route, row);
      }
      if (method === 'DELETE') {
        s.memory.memories = s.memory.memories.filter((x) => x.id !== id);
        s.memory.pending = s.memory.pending.filter((x) => x.id !== id);
        return json(route, { ok: true, undo_token: `undo-${id}` });
      }
    }
    if (path === '/assistant/forget') {
      const q = String(b.query ?? '').toLowerCase();
      const gone = s.memory.memories.filter((x) => x.text.toLowerCase().includes(q));
      s.memory.memories = s.memory.memories.filter((x) => !gone.includes(x));
      return json(route, { forgotten: gone, undo_tokens: gone.map((g) => `undo-${g.id}`) });
    }
    if (path === '/assistant/memory/import/hermes' && method === 'GET') {
      return json(route, { available: true, files: [{ name: 'MEMORY.md', entries: 3 }, { name: 'USER.md', entries: 1 }], entries: [
        { file: 'MEMORY.md', text: 'Zendesk macros live in the Support space', duplicate: true },
        { file: 'MEMORY.md', text: 'Prefers terse replies in Slack', duplicate: false },
        { file: 'MEMORY.md', text: 'On call every second week', duplicate: false },
        { file: 'USER.md', text: 'Based in Lisbon (UTC+1)', duplicate: false },
      ] });
    }
    if (path === '/assistant/memory/import/hermes' && method === 'POST') {
      for (const [i, text] of ['Prefers terse replies in Slack', 'On call every second week', 'Based in Lisbon (UTC+1)'].entries()) {
        s.memory.pending.push(memoryRow(`mem-hq${i}`, text, now(), { state: 'pending', source: { kind: 'hermes', thread_id: null, file: 'MEMORY.md' } }));
      }
      return json(route, { queued: 3, duplicates: 1, files: ['MEMORY.md', 'USER.md'] });
    }

    // ── routing / limits ──
    if (path === '/assistant/routing' && method === 'GET') return json(route, s.routing);
    if (path === '/assistant/routing' && method === 'PUT') {
      s.routing = { ...s.routing, ...(b as Partial<AssistantRoutingSettings>), updated_at: now() };
      return json(route, s.routing);
    }
    if (path === '/assistant/limits') return json(route, s.limits);

    return json(route, { code: 'not_found', message: `unmocked ${method} ${path}` }, 404);
  });
  return s;
}
