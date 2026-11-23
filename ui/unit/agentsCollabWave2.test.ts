import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

test('ShareModal.revoke ignores concurrent duplicate clicks for the same shareId', async () => {
  const del = deferred<void>();
  let delCalls = 0;
  let askCalls = 0;
  const state = componentFunctions(new URL('../src/modules/agents/ShareModal.svelte', import.meta.url), ['revoke'], {
    revoking: {} as Record<string, boolean>,
    shares: [{ id: 'sh-1' }],
    mintedShareId: null,
    mintedUrl: null,
    confirmer: { ask: () => { askCalls++; return Promise.resolve(true); } },
    api: { del: () => { delCalls++; return del.promise; } },
    toasts: { success() {} },
    toastError() {},
  });
  const first = state.revoke('sh-1');
  await new Promise((r) => setTimeout(r, 0));
  await state.revoke('sh-1');
  assert.equal(askCalls, 1);
  assert.equal(delCalls, 1);
  del.resolve();
  await first;
  assert.equal(state.shares.length, 0);
  assert.equal(state.revoking['sh-1'], undefined);
});

test('InfoPanel.onDefaultAgentChange rolls back <select> value on save failure', async () => {
  let errCount = 0;
  const state = componentFunctions(new URL('../src/modules/panels/InfoPanel.svelte', import.meta.url), ['onDefaultAgentChange'], {
    wsDefaultAgent: 'claude',
    ws: { saveDefaultAgent: async () => { throw new Error('forbidden'); } },
    toasts: { info() {} },
    toastError: () => { errCount++; },
  });
  const sel = { value: 'codex' };
  await state.onDefaultAgentChange({ currentTarget: sel });
  assert.equal(sel.value, 'claude', 'failed save must revert select to prior default');
  assert.equal(errCount, 1);
});

test('BrowserPanel.sendToAgent guards against concurrent double-click sends', async () => {
  const post = deferred<void>();
  let postCalls = 0;
  const state = componentFunctions(new URL('../src/modules/panels/BrowserPanel.svelte', import.meta.url), ['sendToAgent'], {
    sending: false,
    ws: { activeSessionId: 's1', activeSession: { kind: 'agent' } },
    annotations: [{ desc: 'button', comment: 'align left', url: 'http://localhost:5173' }],
    current: 'http://localhost:5173',
    api: { post: () => { postCalls++; return post.promise; } },
    plural: (n: number, w: string) => `${n} ${w}`,
    toasts: { error() {}, success() {} },
    toastError() {},
  });
  const first = state.sendToAgent();
  const second = state.sendToAgent();
  assert.equal(postCalls, 1);
  assert.equal(state.sending, true);
  post.resolve();
  await Promise.all([first, second]);
  assert.equal(state.sending, false);
  assert.equal(state.annotations.length, 0);
});

test('RoomRecap.withdraw sends refusal directly when pre-consent & idle, confirms when consented/capturing', async () => {
  const sent: Array<{ type: string; allow?: boolean }> = [];
  let askCalls = 0;
  const state = componentFunctions(new URL('../src/modules/rooms/RoomRecap.svelte', import.meta.url), ['withdraw'], {
    room: { recap: { epoch: 2 } },
    consented: false,
    capturing: false,
    capture: { cancelCapture() {} },
    confirmer: { ask: async () => { askCalls++; return true; } },
    send: (a: { type: string; allow?: boolean }) => { sent.push(a); return true; },
  });
  await state.withdraw();
  assert.equal(askCalls, 0, 'pre-consent refusal must not show scary Pause capture confirm');
  assert.equal(sent.length, 1);
  assert.equal(sent[0].type, 'recap_consent');
  assert.equal(sent[0].allow, false);

  state.consented = true;
  await state.withdraw();
  assert.equal(askCalls, 1, 'withdrawing active consent must confirm');
  assert.equal(sent.length, 2);
});

test('RoomRecap.openPrepare ignores late capability response after closePrepare', async () => {
  const req = deferred<{ speech_ready: boolean }>();
  let announcements = 0;
  const state = componentFunctions(new URL('../src/modules/rooms/RoomRecap.svelte', import.meta.url), ['openPrepare', 'closePrepare'], {
    prepareSeq: 0,
    partial: false,
    prepare: false,
    busy: false,
    capability: null,
    error: null,
    recapRequest: () => req.promise,
    announce: () => { announcements++; },
    fail() {},
  });
  const pending = state.openPrepare();
  state.closePrepare();
  req.resolve({ speech_ready: false });
  await pending;
  assert.equal(state.prepare, false);
  assert.equal(state.capability, null);
  assert.equal(announcements, 0);
});

test('SkillActivity.load ignores stale workspace results after workspace switch', async () => {
  const revA = deferred<Array<{ id: string }>>();
  const goldA = deferred<Array<{ id: string }>>();
  const revB = deferred<Array<{ id: string }>>();
  const goldB = deferred<Array<{ id: string }>>();
  const state = componentFunctions(new URL('../src/modules/skills-lab/SkillActivity.svelte', import.meta.url), ['load'], {
    loadSeq: 0,
    disposed: false,
    wsError: null,
    reviews: null,
    golden: null,
    skillReviewApi: { list: (w: string) => (w === 'ws-a' ? revA.promise : revB.promise) },
    skillsEvalApi: { listGolden: (w: string) => (w === 'ws-a' ? goldA.promise : goldB.promise) },
    loadErrorText: String,
  });
  const pA = state.load('ws-a');
  const pB = state.load('ws-b');
  revB.resolve([{ id: 'r-b' }]);
  goldB.resolve([{ id: 'g-b' }]);
  await pB;
  revA.resolve([{ id: 'r-a' }]);
  goldA.resolve([{ id: 'g-a' }]);
  await pA;
  assert.equal((state.reviews as Array<{ id: string }>)[0].id, 'r-b');
  assert.equal((state.golden as Array<{ id: string }>)[0].id, 'g-b');
});

test('SkillsEvalPage.loadCompareRun guards workspace switch and onRunDeleted evicts compare state', async () => {
  const getRun = deferred<{ id: string }>();
  const state = componentFunctions(new URL('../src/modules/skills-eval/SkillsEvalPage.svelte', import.meta.url), ['loadCompareRun', 'onRunDeleted'], {
    disposed: false,
    ws: { currentId: 'ws-a' },
    runs: [{ id: 'r1' }, { id: 'r2' }],
    selectedId: 'r1',
    mode: 'detail',
    compareSel: new Set(['r1', 'r2']),
    compareFull: { r1: { id: 'r1' } } as Record<string, unknown>,
    compareErrors: { r1: 'err' } as Record<string, string>,
    compareLoading: {} as Record<string, boolean>,
    skillsEvalApi: { get: () => getRun.promise },
    loadErrorText: String,
  });

  state.loadCompareRun('r2');
  state.ws.currentId = 'ws-b';
  getRun.resolve({ id: 'r2' });
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(state.compareFull.r2, undefined, 'stale compare fetch must not land after workspace switch');

  state.onRunDeleted('r1');
  assert.equal(state.compareSel.has('r1'), false);
  assert.equal(state.compareFull.r1, undefined);
  assert.equal(state.compareErrors.r1, undefined);
});

test('RunDetail.unifiedDiffLineClass distinguishes headers/hunks and toggleImplDiff guards run switch', async () => {
  const diffReq = deferred<{ diff: string }>();
  const state = componentFunctions(new URL('../src/modules/skills-eval/RunDetail.svelte', import.meta.url), ['unifiedDiffLineClass', 'toggleImplDiff'], {
    disposed: false,
    run: { id: 'run-a' },
    openImplDiffs: new Set<string>(),
    implDiffs: {} as Record<string, { diff: string }>,
    implDiffLoading: new Set<string>(),
    toggle: (s: Set<string>, id: string) => { const n = new Set(s); if (n.has(id)) n.delete(id); else n.add(id); return n; },
    skillsEvalApi: { implDiff: () => diffReq.promise },
    toastError() {},
  });
  assert.equal(state.unifiedDiffLineClass('+++ b/foo.ts'), 'hdr');
  assert.equal(state.unifiedDiffLineClass('--- a/foo.ts'), 'hdr');
  assert.equal(state.unifiedDiffLineClass('@@ -1,2 +1,3 @@'), 'hunk');
  assert.equal(state.unifiedDiffLineClass('+const x = 1;'), 'add');
  assert.equal(state.unifiedDiffLineClass('-const x = 0;'), 'del');
  assert.equal(state.unifiedDiffLineClass(' const y = 2;'), 'ctx');

  const pending = state.toggleImplDiff({ id: 'it-1' });
  state.run = { id: 'run-b' };
  diffReq.resolve({ diff: '+hello' });
  await pending;
  assert.equal(state.implDiffs['it-1'], undefined, 'stale implDiff must not land on replacement run');
});

test('StartEvalForm.load sequence-guards workspace switches', async () => {
  const srcA = deferred<{ sources: Array<{ name: string }> }>();
  const srcB = deferred<{ sources: Array<{ name: string }> }>();
  const cfg = { validations: [], iterations: 2, validator_passes: 1 };
  const state = componentFunctions(new URL('../src/modules/skills-eval/StartEvalForm.svelte', import.meta.url), ['load'], {
    loadSeq: 0,
    disposed: false,
    loaded: false,
    loadError: null,
    validations: [],
    iterations: 2,
    validatorPasses: 1,
    testDefault: '',
    lintDefault: '',
    sources: [],
    implCli: '',
    improverProvider: '',
    improverModel: '',
    initialSkill: null,
    sourceSel: -1,
    ws: { currentId: 'ws-a' },
    skillsEvalApi: {
      getConfig: async () => cfg,
      listSources: (w: string) => (w === 'ws-a' ? srcA.promise : srcB.promise),
    },
    defaultAgentProvider: () => 'claude',
    matchSource: () => -1,
    loadErrorText: String,
  });
  const pA = state.load('ws-a');
  const pB = state.load('ws-b');
  srcB.resolve({ sources: [{ name: 'skill-b' }] });
  await pB;
  srcA.resolve({ sources: [{ name: 'skill-a' }] });
  await pA;
  assert.equal((state.sources as Array<{ name: string }>)[0].name, 'skill-b');
});
