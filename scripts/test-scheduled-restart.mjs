#!/usr/bin/env node
// Actual daemon-crash regression: API admission -> persisted approval handoff ->
// definition edit -> SIGKILL -> boot recovery -> human approval -> report.
// Run only with an explicitly chosen rebuilt binary; never targets installed data.
// OTTO_E2E_BIN=/absolute/path/to/ottod node scripts/test-scheduled-restart.mjs
import assert from 'node:assert/strict';
import { spawn, execFileSync } from 'node:child_process';
import { once } from 'node:events';
import { mkdtempSync, mkdirSync, writeFileSync, chmodSync, openSync, closeSync, rmSync, existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, isAbsolute } from 'node:path';
import { createServer } from 'node:net';

const binary = process.env.OTTO_E2E_BIN;
assert.ok(binary && isAbsolute(binary) && existsSync(binary), 'Set OTTO_E2E_BIN to the rebuilt absolute ottod path');
assert.equal(typeof WebSocket, 'function', 'Use Node 22+ with its built-in WebSocket');
const fixture = mkdtempSync(join(tmpdir(), 'otto-scheduled-restart-'));
const dataDir = join(fixture, 'data');
const fakeBin = join(fixture, 'fake-bin');
const workspace = join(fixture, 'workspace');
for (const dir of [dataDir, fakeBin, workspace]) mkdirSync(dir);
for (const cli of ['claude', 'codex', 'agy', 'gemini', 'grok']) {
  const file = join(fakeBin, cli);
  writeFileSync(file, '#!/bin/sh\necho "unexpected provider invocation in restart fixture" >&2\nexit 97\n');
  chmodSync(file, 0o755);
}
const listener = createServer();
listener.listen(0, '127.0.0.1');
await once(listener, 'listening');
const port = listener.address().port;
await new Promise((resolve) => listener.close(resolve));
const base = `http://127.0.0.1:${port}/api/v1`;
let owned = null;
let token = '';
let boot = 0;
let passed = false;
let socket;
const startIdentity = (pid) => execFileSync('/bin/ps', ['-p', String(pid), '-o', 'lstart='], { encoding: 'utf8' }).trim();
const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function poll(label, read, accept, timeout = 30000) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    assert.ok(owned && owned.child.exitCode === null && owned.child.signalCode === null,
      `${label}: owned daemon exited; inspect ${fixture}`);
    last = await read();
    if (accept(last)) return last;
    await delay(100);
  }
  throw new Error(`${label}: timed out; last=${JSON.stringify(last)}`);
}

async function api(method, path, body, raw = false) {
  const response = await fetch(`${base}${path}`, {
    method, headers: { 'Content-Type': 'application/json', ...(token ? { Authorization: `Bearer ${token}` } : {}) },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }), signal: AbortSignal.timeout(5000),
  });
  const text = await response.text();
  assert.ok(response.ok, `${method} ${path}: ${response.status} ${text}`);
  return raw ? text : JSON.parse(text);
}

async function start() {
  assert.equal(owned, null);
  const log = join(fixture, `daemon-${++boot}.log`);
  const fd = openSync(log, 'a', 0o600);
  const child = spawn(binary, [], {
    // Whitelist the ambient environment: no inherited OTTO listeners, tokens,
    // plugin paths or installed-daemon configuration may enter this fixture.
    env: {
      HOME: process.env.HOME, TMPDIR: process.env.TMPDIR || tmpdir(), LANG: process.env.LANG || 'en_US.UTF-8',
      PATH: `${fakeBin}:${process.env.PATH || '/usr/bin:/bin'}`,
      OTTO_DATA_DIR: dataDir, OTTO_LOG_DIR: join(fixture, 'logs'), OTTO_PORT: String(port),
      OTTO_ALT_LOOPBACK: '0', OTTO_E2E: '1', OTTO_SELF_IMPROVE: '0', OTTO_CLI_UPDATE: '0',
      OTTO_PLUGINS_HOME: join(fixture, 'plugins'), OTTO_SECRETS: 'file', OTTO_SECRETS_ALLOW_PLAINTEXT: '1',
      CLAUDE_BIN: join(fakeBin, 'claude'),
    },
    cwd: workspace, detached: true, stdio: ['ignore', fd, fd],
  });
  closeSync(fd);
  await once(child, 'spawn');
  owned = { child, started: startIdentity(child.pid), log };
  console.log(JSON.stringify({ stage: 'boot', boot, pid: child.pid, started: owned.started, fixture }));
  await poll('health', async () => {
    try { return await api('GET', '/health'); } catch { return null; }
  }, (value) => value !== null, 90000);
}

async function crash() {
  if (!owned) return;
  const current = owned;
  if (current.child.exitCode === null && current.child.signalCode === null) {
    assert.equal(startIdentity(current.child.pid), current.started, 'refuse to signal a PID with changed start identity');
    const exited = once(current.child, 'exit');
    // This group was created by our detached spawn. Includes only this daemon
    // and its children (e.g. embedded ClickHouse); no global name-based kills.
    process.kill(-current.child.pid, 'SIGKILL');
    await exited;
  }
  owned = null;
}

for (const signal of ['SIGINT', 'SIGTERM']) {
  process.once(signal, () => {
    crash().finally(() => {
      console.error(`Interrupted; owned fixture retained: ${fixture}`);
      process.exit(1);
    });
  });
}

try {
  await start();
  const onboarding = await api('POST', '/onboarding/root', { password: 'isolated-restart-password', display_name: 'Restart fixture' });
  token = onboarding.token;
  assert.ok(token);
  const ws = await api('POST', '/workspaces', { name: 'Restart fixture', root_path: workspace });
  const wf = await api('POST', `/workspaces/${ws.id}/workflows`, {
    name: 'Persisted approval restart marker', graph: {
      nodes: [{ id: 'gate', kind: 'human_approval', name: 'Offline restart checkpoint', params: { timeout_s: 600 } }], edges: [],
    },
  });
  const admittedDestination = { type: 'restart-fixture-admitted-A' };
  const task = await api('POST', `/workspaces/${ws.id}/scheduled-tasks`, {
    name: 'Original admission', kind: 'workflow', workflow_id: wf.id, enabled: true,
    schedule: { cadence: 'once', run_at: '2000-01-01T00:00:00Z' }, destination: admittedDestination,
  });
  // Restart starts the real scheduler's first tick immediately, avoiding a
  // 60-second timing assumption. If already admitted, recovery preserves it.
  await crash();
  await start();
  const runsPath = `/scheduled-tasks/${task.id}/runs`;
  const rows = await poll('scheduled admission', () => api('GET', runsPath), (value) => value.length === 1 && value[0].workflow_run_id);
  const run = rows[0];
  assert.equal(run.trigger, 'schedule');
  assert.equal(run.status, 'running');
  const wfPath = `/workflow-runs/${run.workflow_run_id}`;
  await poll('approval checkpoint', () => api('GET', wfPath), (value) => value.waiting_approval && value.approval_node_id === 'gate');
  const futureSchedule = { cadence: 'once', run_at: new Date(Date.now() + 86400000).toISOString() };
  const edited = await api('PATCH', `/scheduled-tasks/${task.id}`, {
    name: 'Edited after admission', schedule: futureSchedule, destination: { type: 'restart-fixture-edited-B' },
  });
  assert.equal(edited.last_run_at, null);
  assert.ok(edited.next_run_at);
  // Public JSON intentionally omits internal generations; observable fencing
  // is asserted through last_run_at and next_run_at below. No DB writes/seams.
  await crash();
  await start();
  const resumed = await poll('same recovered approval', () => api('GET', wfPath), (value) => value.waiting_approval && value.approval_node_id === 'gate');
  assert.equal(resumed.id, run.workflow_run_id);
  const beforeApproval = await api('GET', runsPath);
  assert.equal(beforeApproval.length, 1);
  assert.equal(beforeApproval[0].id, run.id);
  assert.equal(beforeApproval[0].status, 'running');
  // The terminal event follows schedule settlement, unlike the run-row write.
  // Waiting for it prevents a false pass by racing the old generation update.
  let subscribed = false;
  let terminalEvent = null;
  socket = new WebSocket(`ws://127.0.0.1:${port}/ws/events`, ['otto-bearer', token]);
  socket.addEventListener('message', ({ data }) => {
    const event = JSON.parse(String(data));
    if (event.type === 'subscribe_ack') subscribed = true;
    if (event.type === 'scheduled_task_run_updated' && event.run_id === run.id && event.status !== 'running') terminalEvent = event;
  });
  await once(socket, 'open', { signal: AbortSignal.timeout(10000) });
  socket.send(JSON.stringify({ type: 'subscribe', topics: ['scheduled_task_run_updated'] }));
  await poll('event subscription acknowledgment', async () => subscribed, Boolean);
  await api('POST', `${wfPath}/approve`, { node_id: 'gate', approved: true, note: 'isolated restart accepted' });
  await poll('post-settlement event', async () => terminalEvent, Boolean);
  assert.equal(terminalEvent.status, 'ok');
  socket.close();
  socket = null;
  const settled = await poll('recovered scheduled completion', () => api('GET', runsPath), (value) => value[0]?.status !== 'running');
  assert.equal(settled.length, 1);
  const done = settled[0];
  assert.equal(done.id, run.id);
  assert.equal(done.status, 'ok', JSON.stringify(done));
  assert.equal(done.workflow_run_id, run.workflow_run_id);
  assert.equal(done.delivered, false);
  assert.equal(done.delivery_error, "unknown destination type 'restart-fixture-admitted-A'");
  assert.ok(done.report_rel.includes(run.id), 'report belongs to admitted run');
  const report = await api('GET', `/scheduled-tasks/runs/${run.id}/report`, undefined, true);
  assert.ok(report.includes(wf.name), 'persisted report contains the recovered workflow');
  const after = await api('GET', `/scheduled-tasks/${task.id}`);
  assert.deepEqual(after.schedule, futureSchedule);
  assert.equal(after.last_run_at, null, 'old completion must not consume the retimed occurrence');
  assert.equal(after.next_run_at, edited.next_run_at, 'old completion must preserve the retimed cursor');
  assert.deepEqual(after.destination, { type: 'restart-fixture-edited-B' });
  // A further real boot checks that completion/report persisted and is not
  // reattached or delivered a second time after another process boundary.
  await crash();
  await start();
  const persisted = await api('GET', runsPath);
  assert.equal(persisted.length, 1);
  assert.deepEqual(persisted[0], done);
  assert.equal(await api('GET', `/scheduled-tasks/runs/${run.id}/report`, undefined, true), report);
  assert.equal((await api('GET', `/scheduled-tasks/${task.id}`)).last_run_at, null);
  passed = true;
  console.log(JSON.stringify({ result: 'PASS', boots: boot, assertions: ['scheduled API admission', 'crash recovery at approval', 'original destination', 'retimed occurrence preserved', 'one run', 'run-owned durable report', 'second restart idempotence'] }));
} finally {
  socket?.close();
  await crash();
  if (passed && process.env.OTTO_RESTART_KEEP_FIXTURE !== '1') rmSync(fixture, { recursive: true });
  else console.error(`Owned fixture retained for inspection: ${fixture}`);
}
