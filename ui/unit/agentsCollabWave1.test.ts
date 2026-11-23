import { test } from 'node:test';
import assert from 'node:assert/strict';
import { componentFunctions } from './componentFunctions.ts';
import { deferred } from './sourceHarness.ts';

test('OutputsPanel.safeHttpUrl allows http/https and rejects javascript/data/file schemes', () => {
  const state = componentFunctions(new URL('../src/modules/panels/OutputsPanel.svelte', import.meta.url), ['safeHttpUrl'], { URL });
  assert.equal(state.safeHttpUrl('https://github.com/org/repo/pull/1'), 'https://github.com/org/repo/pull/1');
  assert.equal(state.safeHttpUrl('http://127.0.0.1:7700/health'), 'http://127.0.0.1:7700/health');
  assert.equal(state.safeHttpUrl('javascript:alert(1)'), undefined);
  assert.equal(state.safeHttpUrl('data:text/html,<script>alert(1)</script>'), undefined);
  assert.equal(state.safeHttpUrl('file:///etc/passwd'), undefined);
  assert.equal(state.safeHttpUrl('   '), undefined);
});

test('BrowserPanel.normalize blocks non-http(s) schemes and prefixes bare hosts with https://', () => {
  const state = componentFunctions(new URL('../src/modules/panels/BrowserPanel.svelte', import.meta.url), ['normalize'], {});
  assert.equal(state.normalize('example.com/docs'), 'https://example.com/docs');
  assert.equal(state.normalize('http://localhost:5173'), 'http://localhost:5173');
  assert.equal(state.normalize('javascript:alert(1)'), '');
  assert.equal(state.normalize('file:///etc/hosts'), '');
  assert.equal(state.normalize('data:text/html,hi'), '');
});

test('CanvasPanel.togglePreview ignores stale scene fetch after session switch', async () => {
  const resp = deferred<{ doc_json: string }>();
  let rendered = 0;
  const state = componentFunctions(new URL('../src/modules/panels/CanvasPanel.svelte', import.meta.url), ['togglePreview'], {
    sessionId: 'sess-a',
    expanded: {},
    previews: {},
    ui: { resolvedScheme: 'dark' },
    api: { get: () => resp.promise },
    renderFromDoc: async () => { rendered++; },
    loadErrorText: String,
  });
  const pending = state.togglePreview({ id: 'scene-1', format: 'mermaid', updated_at: '1' });
  state.sessionId = 'sess-b';
  state.previews = {};
  resp.resolve({ doc_json: JSON.stringify({ source: 'graph TD; A-->B;' }) });
  await pending;
  assert.equal(rendered, 0);
  assert.equal(state.previews['scene-1'], undefined);
});

test('RunDetail.openPromote sequence-guards promoteGate across iterations', async () => {
  const gateA = deferred<{ allowed: boolean; score: number; threshold: number; proof_status: string; reasons: string[] }>();
  const gateB = deferred<{ allowed: boolean; score: number; threshold: number; proof_status: string; reasons: string[] }>();
  let calls = 0;
  const state = componentFunctions(new URL('../src/modules/skills-eval/RunDetail.svelte', import.meta.url), ['openPromote'], {
    run: { id: 'run-1', source_skill: 'demo' },
    promoteIterId: '',
    promoteSource: 'tested',
    promoteName: '',
    promoteForce: false,
    promoteGate: null,
    gateLoading: false,
    gateError: false,
    promoteOpen: false,
    gateSeq: 0,
    disposed: false,
    skillsEvalApi: { promoteGate: () => (++calls === 1 ? gateA.promise : gateB.promise) },
  });

  const pA = state.openPromote({ id: 'iter-a', skill_name: 'demo' }, 'tested');
  const pB = state.openPromote({ id: 'iter-b', skill_name: 'demo' }, 'tested');
  // Stale iter-a resolves with allowed=true AFTER iter-b was opened
  gateA.resolve({ allowed: true, score: 95, threshold: 80, proof_status: 'verified', reasons: [] });
  await pA;
  assert.equal(state.promoteGate, null, 'stale gateA must not land on iter-b');
  assert.equal(state.gateLoading, true);

  gateB.resolve({ allowed: false, score: 40, threshold: 80, proof_status: 'failed', reasons: ['score too low'] });
  await pB;
  assert.equal((state.promoteGate as { allowed: boolean } | null)?.allowed, false);
  assert.equal(state.gateLoading, false);
});

test('RoomPage.onKeydown ignores terminal shortcuts in chat view or open modal, and forceEndWithoutRecap ends room', () => {
  let finds = 0;
  const sent: unknown[] = [];
  const state = componentFunctions(new URL('../src/modules/rooms/RoomPage.svelte', import.meta.url), ['onKeydown', 'forceEndWithoutRecap'], {
    room: { session_id: 's1' },
    connected: true,
    view: 'chat',
    inviteOpen: false,
    ui: { overlayOpen: false, termZoomIn() {}, termZoomOut() {}, termZoomReset() {} },
    termRef: { openFind() { finds++; } },
    waitingForRecap: true,
    recapForceReady: true,
    finishing: true,
    send: (a: unknown) => { sent.push(a); return true; },
  });
  let prevented = false;
  state.onKeydown({ metaKey: true, ctrlKey: false, altKey: false, shiftKey: false, key: 'f', preventDefault() { prevented = true; } });
  assert.equal(finds, 0);
  assert.equal(prevented, false);

  state.view = 'terminal';
  state.inviteOpen = true;
  state.onKeydown({ metaKey: true, ctrlKey: false, altKey: false, shiftKey: false, key: 'f', preventDefault() { prevented = true; } });
  assert.equal(finds, 0);

  state.inviteOpen = false;
  state.onKeydown({ metaKey: true, ctrlKey: false, altKey: false, shiftKey: false, key: 'f', preventDefault() { prevented = true; } });
  assert.equal(finds, 1);
  assert.equal(prevented, true);

  state.forceEndWithoutRecap();
  assert.equal(state.waitingForRecap, false);
  assert.equal(state.recapForceReady, false);
  assert.equal(sent.length, 1);
  assert.equal((sent[0] as { type: string }).type, 'end');
});

test('RoomConversation submit and interrupt guard against concurrent double-fire', async () => {
  const firstPrompt = deferred<void>();
  const firstStop = deferred<void>();
  let promptCalls = 0;
  let stopCalls = 0;
  const state = componentFunctions(new URL('../src/modules/rooms/RoomConversation.svelte', import.meta.url), ['submit', 'interrupt'], {
    origin: 'http://127.0.0.1:7700',
    credential: { room_id: 'r1', token: 't1' },
    grantEpoch: 1,
    draft: 'hello',
    sending: false,
    stopping: false,
    sendError: '',
    cadence: { active() {} },
    wake() {},
    loadErrorText: String,
    roomPrompt: (_o: string, _c: unknown, _e: number, text: string) => {
      if (text === '\u001b') { stopCalls++; return firstStop.promise; }
      promptCalls++;
      return firstPrompt.promise;
    },
  });

  const s1 = state.submit();
  const s2 = state.submit();
  assert.equal(promptCalls, 1);
  firstPrompt.resolve();
  await Promise.all([s1, s2]);

  const i1 = state.interrupt();
  const i2 = state.interrupt();
  assert.equal(stopCalls, 1);
  firstStop.resolve();
  await Promise.all([i1, i2]);
});

test('ThreadInspector.onKey only activates bare single-letter shortcuts on the empty launcher', () => {
  const launched: string[] = [];
  const state = componentFunctions(new URL('../src/modules/threads/ThreadInspector.svelte', import.meta.url), ['onKey'], {
    active: 'files',
    launcher: [{ id: 'changes', key: 'C', disabled: null }],
    launch: (s: { id: string }) => launched.push(s.id),
  });
  const ev = { metaKey: false, ctrlKey: false, altKey: false, isComposing: false, key: 'c', target: { closest: () => null }, preventDefault() {} };
  state.onKey(ev);
  assert.deepEqual(launched, [], 'must not hijack C while Files tab is open');

  state.active = null;
  state.onKey(ev);
  assert.deepEqual(launched, ['changes']);
});

test('ThreadFileTabs.rawMediaSrc uses authedBlobUrl with no token in query string', async () => {
  const paths: string[] = [];
  const state = componentFunctions(new URL('../src/modules/threads/ThreadFileTabs.svelte', import.meta.url), ['rawMediaSrc'], {
    resolveFilePath: (p: string) => `/repo/${p}`,
    authedBlobUrl: async (route: string) => {
      paths.push(route);
      return 'blob:http://127.0.0.1:7700/abc';
    },
  });
  const url = await state.rawMediaSrc('img/demo.png');
  assert.equal(url, 'blob:http://127.0.0.1:7700/abc');
  assert.equal(paths.length, 1);
  assert.equal(paths[0].includes('token='), false);
  assert.equal(paths[0], '/fs/raw?path=%2Frepo%2Fimg%2Fdemo.png');
});
