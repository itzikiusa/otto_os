import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { deferred, loadSource } from './sourceHarness.ts';

function functions(file: string, names: string[], context: Record<string, any>) {
  const text = readFileSync(new URL(`../src/${file}`, import.meta.url), 'utf8');
  const script = text.slice(text.indexOf('>') + 1, text.indexOf('</script>'));
  const ast = ts.createSourceFile('component.ts', script, ts.ScriptTarget.Latest, true);
  const source = ast.statements.filter((s) => ts.isFunctionDeclaration(s) && names.includes(s.name?.text ?? '')).map((s) => s.getText(ast)).join('\n');
  runInNewContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, context);
  return context;
}

test('template retry preserves the first created agent and schedule request key', async () => {
  let creates = 0;
  const attempts: any[] = [];
  const c = functions('modules/personal-agents/AgentEditSheet.svelte', ['save', 'buildDelivery'], {
    busy: false, error: '', agent: null, createdAgent: null, pendingSchedule: null,
    fName: 'Recap', fAvatar: '', fSoul: '', fProvider: 'claude', fModel: '', fCwd: '', fBrowser: false,
    fEnabled: true, fDestType: 'none', fTemplate: 'recap', ws: { currentId: 'workspace' },
    crypto: { randomUUID: () => 'stable-request' }, Intl,
    templateById: () => ({ schedule: { cadence: 'interval', every_min: 60 }, directive: 'Recap' }),
    personalAgents: { create: async () => { creates++; return { id: 'first-agent', name: 'Recap' }; },
      update: async () => ({ id: 'first-agent', name: 'Recap' }),
      createSchedule: async (id: string, body: any) => { attempts.push({ id, body }); if (attempts.length === 1) throw new Error('offline'); } },
    toasts: { success() {} }, loadErrorText: (e: Error) => e.message, onclose() {},
  });
  await c.save();
  assert.match(c.error, /Agent created; its schedule could not be added/);
  await c.save();
  assert.equal(creates, 1);
  assert.equal(attempts.length, 2);
  assert.equal(attempts[0].id, attempts[1].id);
  assert.equal(attempts[0].body.idempotency_key, attempts[1].body.idempotency_key);
});

test('Athena history keeps its response region and rejects a late foreign history', async () => {
  const historyCalls: ReturnType<typeof deferred<any>>[] = [];
  const statusCalls: any[][] = [], cancelCalls: any[][] = [];
  const c = functions('modules/aws/AthenaView.svelte', ['loadHistory', 'openExecution', 'poll', 'cancel'], {
    historyGeneration: 0, executionGeneration: 0, history: [], historyRegion: '', historyLoading: false, historyError: '',
    account: { id: 'account' }, workgroup: 'primary', rq: 'eu-west-1', qRegion: '', qid: null,
    qstate: null, sql: '', submitting: false, result: null, resultError: null, scanned: 0, execMs: 0, ranSql: '', tab: 'history', pollN: 0,
    pollFails: 0, statusUnknown: '', qreason: '', MAX_POLL_FAILS: 5,
    stopPoll() {}, schedulePoll() {}, nextPollMs: () => 1000,
    awsApi: { athenaHistory: () => { const r = deferred<any>(); historyCalls.push(r); return r.promise; },
      athenaStatus: async (...args: any[]) => { statusCalls.push(args); return { state: 'RUNNING' }; },
      athenaCancel: async (...args: any[]) => { cancelCalls.push(args); } },
    toasts: { info() {}, error() {} },
  });
  const eu = { id: 'eu-query', query: 'select 1', state: 'RUNNING' };
  const first = c.loadHistory(); historyCalls[0].resolve({ executions: [eu] }); await first;
  c.openExecution(eu); await c.poll(); await c.cancel();
  assert.equal(statusCalls[0][4], 'eu-west-1'); assert.equal(cancelCalls[0][2], 'eu-west-1');
  const stale = c.loadHistory(); c.rq = 'ap-south-1'; const current = c.loadHistory();
  const ap = { ...eu, id: 'ap-query' };
  historyCalls[2].resolve({ executions: [ap] }); await current;
  historyCalls[1].resolve({ executions: [eu] }); await stale;
  assert.equal(c.history[0].id, 'ap-query');
  c.openExecution(eu); assert.equal(c.qid, 'eu-query');
  c.openExecution(ap); await c.poll(); assert.equal(statusCalls[1][4], 'ap-south-1');
});

function assistantFixture() {
  const requests: { id: string; request: ReturnType<typeof deferred<any[]>>; signal: AbortSignal }[] = [];
  const exports = loadSource(new URL('../src/lib/stores/assistant.svelte.ts', import.meta.url), {
    svelte: { untrack: (fn: () => any) => fn() },
    '../api/client': { api: {}, ApiError: class extends Error {} },
    '../poll': { mapLimit: async (items: any[], _n: number, f: (v: any) => Promise<any>) => { for (const item of items) await f(item); } },
    '../api/assistant': { assistantApi: { turns: (id: string, _before: any, _limit: any, signal: AbortSignal) => {
      const request = deferred<any[]>(); requests.push({ id, request, signal }); return request.promise;
    }, needsYou: async () => [] } },
    '../../modules/assistant/model': { needsYouByThread: () => ({}), reduceNeedsYou: (state: any) => state,
      upsertNewer: (list: any[], row: any) => [...list.filter((t) => t.id !== row.id), row] },
  });
  return { store: exports.assistant, requests };
}

test('100 visited Assistant threads retain only acquired data and reconnect refreshes one view', async () => {
  const { store, requests } = assistantFixture();
  for (let i = 0; i < 100; i++) {
    const release = store.acquireTurns(`thread-${i}`);
    requests.at(-1)!.request.resolve([{ id: `${i}`, role: 'assistant', text: 'saved', created_at: '1' }]);
    await Promise.resolve(); release();
  }
  assert.equal(Object.keys(store.turns).length, 0);
  const release = store.acquireTurns('visible'); requests.at(-1)!.request.resolve([]); await Promise.resolve();
  const before = requests.length; store.resync();
  assert.equal(requests.length - before, 1); assert.equal(requests.at(-1)!.id, 'visible');
  requests.at(-1)!.request.resolve([]); await Promise.resolve(); release();
});

test('late Assistant fetch and live events cannot revive released history or remove pending sends', async () => {
  const { store, requests } = assistantFixture();
  const release = store.acquireTurns('thread');
  store.pending.thread = [{ id: 'local', text: 'pending', attachments: [], created_at: '1' }];
  release(); assert.equal(requests[0].signal.aborted, true);
  requests[0].request.resolve([{ id: 'stale', role: 'assistant', text: 'late', created_at: '2' }]); await Promise.resolve();
  store.applyEvent({ type: 'assistant_turn', thread_id: 'thread', turn: { id: 'event', role: 'assistant', text: 'event' } });
  assert.equal(store.turns.thread, undefined); assert.equal(store.pending.thread.length, 1);
  const again = store.acquireTurns('thread'); assert.equal(requests.length, 2);
  requests[1].request.resolve([]); await Promise.resolve();
  for (let i = 0; i < 250; i++) store.applyEvent({ type: 'assistant_turn', thread_id: 'thread', turn: { id: `${i}`, role: 'assistant', text: 'x'.repeat(65536), created_at: `${i}` } });
  assert.ok(JSON.stringify(store.turns.thread.data).length * 2 < 2.1 * 1024 * 1024);
  assert.ok(store.turns.thread.data.length <= 200); again();
});

for (const [file, globals] of [
  ['AgentDocuments', { editing: true, draft: 'unsaved', document: { content: 'saved' } }],
  ['AgentAutonomy', { dirty: true }],
] as const) {
  test(`${file} navigation keeps dirty drafts and blocks pending-save departure`, async () => {
    const text = readFileSync(new URL(`../src/modules/personal-agents/${file}.svelte`, import.meta.url), 'utf8');
    const script = text.slice(text.indexOf('>') + 1, text.indexOf('</script>'));
    const ast = ts.createSourceFile('component.ts', script, ts.ScriptTarget.Latest, true);
    const source = ast.statements.filter((s) => ts.isExpressionStatement(s) && /\$effect\(.*(?:router.guard|guardUnsaved)/.test(s.getText(ast))).map((s) => s.getText(ast)).join('\n');
    const guards: (() => boolean)[] = [];
    const c: any = { ...globals, saving: false, $effect: (f: () => void) => f(), router: { guard: (f: () => boolean) => guards.push(f) },
      guardUnsaved: (dirty: () => boolean) => guards.push(() => !dirty()) };
    runInNewContext(source, c);
    assert.equal(guards.length, 2, 'both pending-save and dirty-draft guards are registered');
    assert.equal(guards.every((guard) => guard()), false);
    if (file === 'AgentDocuments') c.draft = 'saved'; else c.dirty = false;
    assert.equal(guards.every((guard) => guard()), true);
    c.saving = true; assert.equal(guards.every((guard) => guard()), false);
  });
}
