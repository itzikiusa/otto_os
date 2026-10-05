import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import { compile } from 'svelte/compiler';
import { deferred, loadSource } from './sourceHarness.ts';
import { componentFunctions } from './componentFunctions.ts';

const tick = async () => { for (let i = 0; i < 8; i++) await Promise.resolve(); };
const plain = (value: unknown) => JSON.parse(JSON.stringify(value));
const errorText = (e: unknown) => String(e);

function personal(api: Record<string, unknown>) {
  return loadSource(new URL('../src/lib/stores/personalAgents.svelte.ts', import.meta.url), {
    '../api/personalAgents': { personalAgentsApi: api },
    '../loadError': { loadErrorText: errorText }, '../lazyModule': { announceModule() {} },
  }).personalAgents;
}
for (const [method, endpoint, map] of [
  ['loadSchedules', 'schedules', 'schedulesByAgent'], ['loadRuns', 'runs', 'runsByAgent'],
] as const) {
  for (const order of [['A', 'B'], ['B', 'A']]) {
    test(`${method}: ${order.join(' then ')} preserves independent agent results`, async () => {
      const waits = { A: deferred<any[]>(), B: deferred<any[]>() };
      const store = personal({ [endpoint]: (id: 'A' | 'B') => waits[id].promise });
      store[map] = { unrelated: [{ id: 'held' }] };
      const calls = [store[method]('A'), store[method]('B')];
      waits[order[0] as 'A' | 'B'].resolve([{ id: order[0] }]); await tick();
      waits[order[1] as 'A' | 'B'].resolve([{ id: order[1] }]); await Promise.all(calls);
      assert.deepEqual(plain(store[map]), { unrelated: [{ id: 'held' }], A: [{ id: 'A' }], B: [{ id: 'B' }] });
    });
  }
  for (const staleFails of [false, true]) {
    test(`${method}: newer same-agent result survives stale ${staleFails ? 'failure' : 'success'}`, async () => {
      const old = deferred<any[]>(), fresh = deferred<any[]>(); let calls = 0;
      const store = personal({ [endpoint]: () => (++calls === 1 ? old.promise : fresh.promise) });
      const first = store[method]('A'), second = store[method]('A');
      fresh.resolve([{ id: 'fresh' }]); await second;
      if (staleFails) old.reject(new Error('obsolete failure')); else old.resolve([{ id: 'obsolete' }]);
      await first;
      assert.deepEqual(plain(store[map].A), [{ id: 'fresh' }]);
      if (method === 'loadRuns') assert.equal(store.runsError.A, undefined);
    });
  }
}

function proofFixture() {
  const lists: { args: any[]; reply: ReturnType<typeof deferred<any>> }[] = [];
  const details: ReturnType<typeof deferred<any>>[] = [];
  const store = loadSource(new URL('../src/lib/stores/proof.svelte.ts', import.meta.url), {
    '../api/proof': {
      listProofPacksPage: (...args: any[]) => { const reply = deferred<any>(); lists.push({ args, reply }); return reply.promise; },
      getProofPack: () => { const reply = deferred<any>(); details.push(reply); return reply.promise; },
      proofSummary: async () => ({ rows: [] }), PROOF_SUMMARY_CHUNK: 100,
    }, '../loadError': { loadErrorText: errorText },
  }).proof;
  return { store, lists, details };
}
const page = (id: string, next: string | null = null) => ({ packs: [{ id }], next });
for (const staleFails of [false, true]) {
  test(`Proof filter replacement ignores old ${staleFails ? 'failure' : 'success'} and its loading completion`, async () => {
    const { store, lists } = proofFixture();
    const old = store.loadPacks('W');
    const current = store.loadPacks('W', { status: 'failed' });
    if (staleFails) lists[0].reply.reject(new Error('obsolete list')); else lists[0].reply.resolve(page('wrong', 'wrong-cursor'));
    await old;
    const pending = { loading: store.loading, error: store.error, cursor: store.nextCursor };
    lists[1].reply.resolve(page('failed', 'failed-cursor')); await current;
    assert.equal(pending.loading, true, 'the current query still owns loading');
    assert.equal(pending.error, null);
    assert.equal(pending.cursor, null);
    assert.deepEqual(plain(store.packs), [{ id: 'failed' }]);
    assert.equal(store.nextCursor, 'failed-cursor');
  });
}
test('Proof workspace A-B-A rejects the first A response and retains final cursor', async () => {
  const { store, lists } = proofFixture();
  const a1 = store.loadPacks('A'), b = store.loadPacks('B'), a2 = store.loadPacks('A');
  lists[2].reply.resolve(page('new-A', 'new-cursor')); await a2;
  lists[0].reply.resolve(page('old-A', 'old-cursor')); lists[1].reply.resolve(page('B')); await Promise.all([a1, b]);
  assert.deepEqual(plain(store.packs), [{ id: 'new-A' }]); assert.equal(store.nextCursor, 'new-cursor');
});
for (const staleFails of [false, true]) {
  test(`Proof old load-more ${staleFails ? 'failure' : 'success'} cannot publish across filter change`, async () => {
    const { store, lists } = proofFixture();
    const seed = store.loadPacks('W'); lists[0].reply.resolve(page('all', 'shared-cursor')); await seed;
    const more = store.loadMore();
    const current = store.loadPacks('W', { status: 'failed' });
    const replacementCursor = store.nextCursor;
    lists[2].reply.resolve(page('failed', 'shared-cursor')); await current;
    if (staleFails) lists[1].reply.reject(new Error('obsolete page')); else lists[1].reply.resolve(page('wrong-page', 'wrong-next'));
    await more;
    assert.equal(replacementCursor, null, 'replacement immediately revokes the old cursor');
    assert.deepEqual(plain(store.packs), [{ id: 'failed' }]); assert.equal(store.error, null);
    assert.equal(store.nextCursor, 'shared-cursor');
  });
}
for (const staleFails of [false, true]) {
  test(`Proof close invalidates pending detail ${staleFails ? 'failure' : 'success'}`, async () => {
    const { store, details } = proofFixture();
    const initial = store.open('P'); details[0].resolve({ pack: { id: 'P' } }); await initial;
    const refresh = store.refreshDetail(); store.closeDetail();
    if (staleFails) details[1].reject(new Error('obsolete detail')); else details[1].resolve({ pack: { id: 'P' } });
    await refresh;
    assert.equal(store.detail, null); assert.equal(store.detailError, null); assert.equal(store.loading, false);
  });
}
test('Proof detail completion cannot clear an active list spinner', async () => {
  const { store, lists, details } = proofFixture();
  const list = store.loadPacks('W'), detail = store.open('P');
  details[0].resolve({ pack: { id: 'P' } }); await detail;
  const listStillLoading = store.loading;
  lists[0].reply.resolve(page('P')); await list;
  assert.equal(listStillLoading, true); assert.equal(store.loading, false);
});

function settingsFixture(first: ReturnType<typeof deferred<any>>, failLater = false) {
  const bodies: any[] = [], errors: unknown[][] = [], timers = new Map<number, { fn: () => void; ms: number }>(); let timerId = 0;
  const cfg = { daily: false, weekly: false, monthly: false, provider: 'claude', model: 'A' };
  const context = componentFunctions(new URL('../src/modules/settings/InsightsSettings.svelte', import.meta.url),
    ['flashSaved', 'onModelChange', 'saveAgent', 'toggle', 'drainQueuedAgent', 'retryAgent'], {
      cfg, modelDraft: 'A', saving: false, queuedAgent: null, failedAgent: null, agentSaveError: '', savedIn: null, savedTimer: null, modelSaveTimer: null,
      insightsApi: { putConfig: async (body: any) => { bodies.push(plain(body)); if (bodies.length === 1) return first.promise; if (failLater) throw new Error('model save failed'); return plain(body); } },
      toastError: (...args: unknown[]) => errors.push(args), loadErrorText: errorText,
      setTimeout: (fn: () => void, ms: number) => { const id = ++timerId; timers.set(id, { fn, ms }); return id; },
      clearTimeout: (id: number) => timers.delete(id),
    });
  return { context, bodies, errors, allowWrites: () => { failLater = false; }, debounce: async () => { for (const [id, timer] of [...timers]) if (timer.ms === 500) { timers.delete(id); timer.fn(); } await tick(); } };
}
for (const scheduleFails of [false, true]) {
  test(`Insights queued latest model drains after schedule ${scheduleFails ? 'failure' : 'success'}`, async () => {
    const first = deferred<any>(); const { context: c, bodies, errors, debounce } = settingsFixture(first);
    const toggle = c.toggle('daily'); c.onModelChange('B'); await debounce(); c.onModelChange('C'); await debounce();
    assert.equal(bodies.length, 1, 'settings writes serialize');
    if (scheduleFails) first.reject(new Error('schedule save failed')); else first.resolve(bodies[0]);
    await toggle; await tick();
    assert.equal(bodies.length, 2, 'queued model must reach persistence without another interaction');
    assert.equal(bodies[1].model, 'C'); assert.equal(bodies[1].daily, !scheduleFails);
    assert.equal(c.cfg.model, 'C'); assert.equal(c.modelDraft, 'C'); assert.equal(c.saving, false);
    assert.equal(errors.length, scheduleFails ? 1 : 0);
  });
}
test('Insights failed queued model stays retryable after successful schedule save', async () => {
  const first = deferred<any>(); const { context: c, bodies, errors, debounce, allowWrites } = settingsFixture(first, true);
  const toggle = c.toggle('daily'); c.onModelChange('B'); await debounce(); first.resolve(bodies[0]); await toggle; await tick();
  assert.equal(bodies.length, 2); assert.equal(errors.length, 1); assert.equal(c.cfg.daily, true);
  assert.equal(c.cfg.model, 'A'); assert.equal(c.modelDraft, 'B'); assert.equal(c.saving, false);
  assert.match(c.agentSaveError, /model save failed/);
  allowWrites(); await c.retryAgent();
  assert.equal(bodies.length, 3); assert.equal(bodies[2].model, 'B'); assert.equal(bodies[2].daily, true);
  assert.equal(c.cfg.model, 'B'); assert.equal(c.agentSaveError, '');
});
test('Insights provider reset cancels an older debounced model from the prior provider', async () => {
  const first = deferred<any>(); const { context: c, bodies, debounce } = settingsFixture(first);
  c.onModelChange('claude-old-draft'); c.modelDraft = '';
  const changed = c.saveAgent({ provider: 'codex', model: '' }); first.resolve(bodies[0]); await changed;
  await debounce();
  assert.equal(c.cfg.provider, 'codex'); assert.equal(c.cfg.model, '');
  assert.ok(bodies.every(body => body.provider !== 'codex' || body.model !== 'claude-old-draft'));
});

for (const debounceFirst of [false, true]) {
  test(`Insights rejected provider preserves its model when rejection is ${debounceFirst ? 'after' : 'before'} debounce`, async () => {
    const first = deferred<any>();
    const { context: c, bodies, debounce, allowWrites } = settingsFixture(first, true);
    const change = c.saveAgent({ provider: 'codex', model: '' });
    c.onModelChange('codex-only-model');
    if (debounceFirst) await debounce();
    first.reject(new Error('provider save failed'));
    await change;
    if (!debounceFirst) await debounce();
    assert.ok(bodies.every(body => body.provider !== 'claude' || body.model !== 'codex-only-model'),
      'a model draft must retain the provider whose picker produced it');
    assert.equal(bodies.length, 1, 'failed provider/model pair waits for explicit retry');
    assert.equal(c.cfg.provider, 'claude'); assert.equal(c.cfg.model, 'A');
    assert.equal(c.modelDraft, 'codex-only-model'); assert.match(c.agentSaveError, /provider save failed/);
    assert.equal(c.failedAgent.provider, 'codex'); assert.equal(c.failedAgent.model, 'codex-only-model');
    allowWrites(); await c.retryAgent();
    assert.equal(bodies.length, 2); assert.equal(bodies[1].provider, 'codex'); assert.equal(bodies[1].model, 'codex-only-model');
    assert.equal(c.cfg.provider, 'codex'); assert.equal(c.cfg.model, 'codex-only-model'); assert.equal(c.agentSaveError, '');
  });
}
test('Insights Retry debounce state compiles without a nonreactive update warning', () => {
  const path = new URL('../src/modules/settings/InsightsSettings.svelte', import.meta.url);
  const result = compile(readFileSync(path, 'utf8'), { filename: path.pathname, generate: 'client' });
  const warnings = result.warnings.filter(warning => warning.code === 'non_reactive_update' && warning.message.includes('modelSaveTimer'));
  assert.deepEqual(warnings.map(warning => warning.message), []);
});

test('PAT mint keeps the uncopied secret through repeated submit and clipboard failure until Done', async () => {
  const posts: any[] = [], revoked: string[] = [], copied: string[] = [];
  const c = componentFunctions(new URL('../src/modules/settings/PersonalAccessTokens.svelte', import.meta.url),
    ['mint', 'copySecret', 'dismissSecret', 'revoke'], {
      minting: false, freshSecret: null, freshInfo: null, newLabel: 'A', tokens: [], revoking: new Set(),
      api: { post: async (_path: string, body: any) => { posts.push(body); return { token: `secret-${posts.length}`, info: { id: `id-${posts.length}`, label: body.label } }; }, del: async (path: string) => revoked.push(path) },
      copyTextOrThrow: async (text: string) => { copied.push(text); throw new Error('clipboard denied'); },
      confirmer: { ask: async () => true }, toasts: { success() {}, error() {} }, loadErrorText: errorText,
    });
  await c.mint(); c.newLabel = 'B'; await c.mint(); await c.mint();
  assert.equal(posts.length, 1); assert.equal(c.freshSecret, 'secret-1');
  await c.copySecret(); assert.deepEqual(copied, ['secret-1']); assert.equal(c.freshSecret, 'secret-1');
  c.dismissSecret(); await c.mint(); assert.equal(posts.length, 2); assert.equal(c.freshSecret, 'secret-2');
  await c.revoke({ id: 'id-1', label: 'A' }); assert.deepEqual(revoked, ['/auth/tokens/id-1']); assert.equal(c.freshSecret, 'secret-2');
});

// Capture the actual account-loading effect in addition to named handlers.
// Transport and lifecycle are deterministic adapters; no reactivity/DOM claim.
function accountFixture() {
  const effects: (() => void | (() => void))[] = [], destroys: (() => void)[] = [];
  const requests: ReturnType<typeof deferred<any[]>>[] = [];
  const c = componentFunctions(new URL('../src/lib/components/AccountPicker.svelte', import.meta.url),
    ['loadAccounts', 'cancelAccountLoad', 'add', 'login', 'check'], {
      provider: 'claude', value: 'explicit-account', workspaceId: 'W', accounts: [], label: 'Keep this label', adding: true,
      busy: false, error: '', status: '', loginSession: null, listLoading: false, listError: '', accountLoadSeq: 0,
      api: { get: () => { const reply = deferred<any[]>(); requests.push(reply); return reply.promise; } },
      loadErrorText: errorText, onchange: () => assert.fail('loading must not change the selected account'),
      $effect: (fn: () => void | (() => void)) => effects.push(fn), onDestroy: (fn: () => void) => destroys.push(fn),
    });
  const script = readFileSync(new URL('../src/lib/components/AccountPicker.svelte', import.meta.url), 'utf8').split('<script lang="ts">')[1].split('</script>')[0];
  const file = ts.createSourceFile('account.ts', script, ts.ScriptTarget.Latest, true, ts.ScriptKind.TS);
  const lifecycle = file.statements.filter(n => ts.isExpressionStatement(n) && ts.isCallExpression(n.expression) && ['$effect', 'onDestroy'].includes(n.expression.expression.getText(file))).map(n => n.getText(file)).join('\n');
  runInNewContext(ts.transpileModule(lifecycle, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, c);
  return { c, effects, requests, destroys };
}
test('AccountPicker exposes pending lookup and an in-place retry without losing drafts or explicit account', async () => {
  const { c, effects, requests } = accountFixture(); effects[0]();
  const pending = c.listLoading; requests[0].reject(new Error('temporary lookup failure')); await tick();
  assert.equal(pending, true); assert.equal(c.listLoading, false); assert.match(c.listError, /temporary|account/i);
  assert.equal(typeof c.loadAccounts, 'function', 'Retry invokes a production loader');
  const retry = c.loadAccounts(); requests[1].resolve([{ id: 'explicit-account', provider: 'claude', label: 'Work' }]); await retry;
  assert.equal(c.listError, ''); assert.equal(c.label, 'Keep this label'); assert.equal(c.value, 'explicit-account');
  assert.deepEqual(plain(c.accounts).map((a: any) => a.id), ['explicit-account']);
});
test('AccountPicker provider change and unmount reject obsolete lookup publication', async () => {
  const { c, effects, requests, destroys } = accountFixture();
  const closeOld = effects[0](); c.provider = 'codex'; if (typeof closeOld === 'function') closeOld(); const closeCurrent = effects[0]();
  requests[1].resolve([{ id: 'codex', provider: 'codex' }]); await tick();
  requests[0].resolve([{ id: 'claude', provider: 'claude' }]); await tick();
  assert.deepEqual(plain(c.accounts).map((a: any) => a.id), ['codex']);
  if (typeof closeCurrent === 'function') closeCurrent(); const closePending = effects[0](); if (typeof closePending === 'function') closePending(); destroys.forEach(fn => fn());
  requests[2].reject(new Error('after unmount')); await tick();
  assert.equal(c.error, ''); assert.equal(c.listError, ''); assert.equal(c.label, 'Keep this label');
});
