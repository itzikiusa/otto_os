#!/usr/bin/env node
// Upgrade with runs in flight and rollback (test plan §5.2, §5.3) on two ottod
// binaries and one data dir, isolated (scripts/workflows/daemon.mjs): never the
// owner's daemon, data dir, launchctl or app bundle — an installed app's
// Contents/MacOS/ottod is only executed, read-only, as the "old" build.
//
//   node scripts/workflows/upgrade-check.mjs --old <ottod> --new <ottod> [--fakes <dir>] [--out <dir>] [--port 7793]
//     [--only upgrade|rollback|restart]
//
// Upgrade: the old build runs (a) an agent step held mid-step, (b) a run parked
// at an approval, (f) a finished success and a finished failure; SIGTERM; the new
// build boots on the same dir: (a) re-runs on a thread with the same step file
// or fails saying why, (b) is still parked and approving it finishes the run,
// (f) stay as they were and retry-node of the failure runs on the new engine;
// definitions are unchanged; the new boot log has no ERROR.
// Rollback: the new build leaves a parked approval, two steps running side by
// side, and finished thread-step runs; SIGTERM; the old build boots on that
// schema, lists the workflows and runs and opens every run's page routes.
// Exit 0 when every check passes; results in <out>/upgrade.md and upgrade.json.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { args, boot, cleanup, isolatedRoot, sleep, sqlite } from './daemon.mjs';

const a = args();
if (!a.old || !a.new) throw new Error('usage: --old <ottod> --new <ottod>');
const OLD = path.resolve(a.old), NEW = path.resolve(a.new);
const FAKES = path.resolve(a.fakes ?? path.dirname(NEW));
const OUT = path.resolve(a.out ?? fs.mkdtempSync(path.join(os.tmpdir(), 'otto-upgrade-')));
const PORT = Number(a.port ?? 7793);
fs.mkdirSync(OUT, { recursive: true });
const checks = [];
const check = (name, ok, detail = '') => { checks.push({ name, ok: !!ok, detail: String(detail).slice(0, 400) }); console.log((ok ? 'pass ' : 'FAIL ') + name + (detail ? ' — ' + String(detail).slice(0, 200) : '')); };
const NEW_ENV = { OTTO_WF_STEP_RUNNER: 'threads', OTTO_FAKE_TIME_SCALE: '1' };
const OLD_ENV = { OTTO_E2E_STEP_HOLD_MS: '600000' };
const edge = (id, s, t) => ({ id, source: s, target: t });
const g = (nodes) => ({ nodes: nodes.map((n, i) => ({ x: 0, y: i * 80, name: n.id, params: {}, ...n })), edges: nodes.slice(1).map((n, i) => edge('e' + i, nodes[i].id, n.id)) });
const settled = (r) => ['success', 'error', 'canceled'].includes(r.status);
async function until(api, id, pred, ms = 60000) {
  const end = Date.now() + ms;
  for (;;) {
    const r = await api('GET', '/workflow-runs/' + id);
    if (pred(r)) return r;
    if (Date.now() > end) throw new Error('timed out on ' + id + ': ' + r.status + ' ' + JSON.stringify(r.nodes?.map((n) => [n.node_id, n.status])));
    await sleep(500);
  }
}
/** Like `until`, but a timeout returns the last view, so the check itself can fail. */
const poll = (api, id, pred, ms) => until(api, id, pred, ms).catch(() => api('GET', '/workflow-runs/' + id));
async function setup(api, d) {
  const ws = await api('POST', '/workspaces', { name: 'Upgrade', root_path: path.dirname(d['ws/repo']) });
  await api('POST', '/workspaces/' + ws.id + '/repos', { path: d['ws/repo'], name: 'repo' });
  return ws;
}
const mk = (api, ws, name, graph) => api('POST', '/workspaces/' + ws.id + '/workflows', { name, graph });
const input = (d) => ({ working_directory: d['ws/repo'], base: 'base' });

async function upgrade() {
  const iso = isolatedRoot(FAKES);
  const { d } = iso;
  let life;
  try {
    life = await boot(OLD, iso, PORT, OLD_ENV, path.join(OUT, 'upgrade-old.log'));
    const ws = await setup(life.api, d);
    const wfs = {
      hold: await mk(life.api, ws, 'Held step', g([{ id: 'n1', kind: 'manual_trigger' }, { id: 'n2', kind: 'agent_prompt', name: 'Step two', params: { provider: 'claude', prompt: 'Write the step file.\nOTTO_E2E_SUBAGENTS: 1' } }, { id: 'n3', kind: 'log' }])),
      gate: await mk(life.api, ws, 'Gate', g([{ id: 'n1', kind: 'manual_trigger' }, { id: 'n2', kind: 'human_approval', params: { message: 'ok?' } }, { id: 'n3', kind: 'log' }])),
      ok: await mk(life.api, ws, 'Fine', g([{ id: 'n1', kind: 'manual_trigger' }, { id: 'n2', kind: 'log' }])),
      bad: await mk(life.api, ws, 'Broken', g([{ id: 'n1', kind: 'manual_trigger' }, { id: 'n2', kind: 'http_request', params: { url: 'http://127.0.0.1:1/refused-by-the-ssrf-guard' } }, { id: 'n3', kind: 'log' }])),
    };
    const run = (k) => life.api('POST', '/workflows/' + wfs[k].id + '/run', { input: input(d) });
    const ok = await run('ok'), bad = await run('bad');
    await until(life.api, ok.id, settled), await until(life.api, bad.id, settled);
    const held = await run('hold');
    await until(life.api, held.id, (r) => r.nodes?.some((n) => n.node_id === 'n2' && n.status === 'running'));
    await sleep(2000);
    const gate = await run('gate');
    await until(life.api, gate.id, (r) => r.waiting_approval);
    const before = Object.fromEntries(await Promise.all(Object.entries(wfs).map(async ([k, w]) => [k, JSON.stringify((await life.api('GET', '/workflows/' + w.id)).graph)])));
    const db = path.join(d.data, 'otto.db');
    check('old build: the held step is running in SQLite before the stop', sqlite(db, "SELECT status FROM workflow_runs WHERE id = '" + held.id + "'") === 'running');
    const beforeRuns = { ok: await life.api('GET', '/workflow-runs/' + ok.id), bad: await life.api('GET', '/workflow-runs/' + bad.id) };
    await life.stop();

    life = await boot(NEW, iso, PORT, NEW_ENV, path.join(OUT, 'upgrade-new.log'));
    const h = await until(life.api, held.id, settled, 120000);
    const n2 = h.nodes.find((n) => n.node_id === 'n2');
    const files = fs.readdirSync(path.join(d.data, 'workflow-context', held.id));
    check('(a) the interrupted agent step re-ran on the new engine or failed saying why', h.status === 'success' || /restart|upgrade|interrupt/i.test(h.error ?? n2.error ?? ''), h.status + ' ' + (h.error ?? '') + ' ' + (n2.error ?? ''));
    check('(a) same step file name', files.some((f) => /^step2-step-two\.md$/.test(f)), files.join(','));
    check('(a) resume_attempts ≤ 2', (h.resume_attempts ?? 0) <= 2, h.resume_attempts);
    check('(a) re-ran as a thread', !!n2.thread || h.status !== 'success', JSON.stringify(n2.thread ?? null));
    const gp = await life.api('GET', '/workflow-runs/' + gate.id);
    check('(b) the approval is still parked after the upgrade', gp.waiting_approval && !settled(gp), gp.status);
    await life.api('POST', '/workflow-runs/' + gate.id + '/approve', { node_id: 'n2', approved: true });
    const ga = await until(life.api, gate.id, settled);
    check('(b) approving after the upgrade finishes the run', ga.status === 'success', ga.status + ' ' + (ga.error ?? ''));
    const afterOk = await life.api('GET', '/workflow-runs/' + ok.id), afterBad = await life.api('GET', '/workflow-runs/' + bad.id);
    check('(f) finished runs unchanged', afterOk.status === beforeRuns.ok.status && afterBad.status === beforeRuns.bad.status, afterOk.status + ' / ' + afterBad.status);
    // The failure was settled before the retry: only a later revision that is settled again is the retry's outcome.
    await life.api('POST', '/workflow-runs/' + bad.id + '/retry-node', { node_id: 'n2' });
    const rr = await poll(life.api, bad.id, (r) => r.rev > afterBad.rev && settled(r), 60000);
    check('(f) retry-node of the old failure runs on the new engine', rr.rev > afterBad.rev && settled(rr), 'rev ' + afterBad.rev + ' → ' + rr.rev + ', ' + rr.status + ' ' + (rr.error ?? ''));
    const after = Object.fromEntries(await Promise.all(Object.entries(wfs).map(async ([k, w]) => [k, JSON.stringify((await life.api('GET', '/workflows/' + w.id)).graph)])));
    check('definitions unchanged by the upgrade', Object.keys(before).every((k) => before[k] === after[k]), Object.keys(before).filter((k) => before[k] !== after[k]).join(','));
    await life.stop();
    const errs = fs.readFileSync(path.join(OUT, 'upgrade-new.log'), 'utf8').split('\n').filter((l) => /\bERROR\b/.test(l));
    check('new build boot log has no ERROR', errs.length === 0, errs.slice(0, 3).join(' | '));
  } finally {
    cleanup(iso.root, life?.child.pid);
  }
}

async function rollback() {
  const iso = isolatedRoot(FAKES);
  const { d } = iso;
  let life;
  try {
    // A step that works for a minute: two of them run side by side at the stop.
    fs.writeFileSync(path.join(d.scripts, 'long.txt'), 'sleep 60000\nwrite $HANDOFF @filler:400\nreply done\n');
    life = await boot(NEW, iso, PORT, NEW_ENV, path.join(OUT, 'rollback-new.log'));
    const ws = await setup(life.api, d);
    const agent = (id, name) => ({ id, kind: 'agent_prompt', name, params: { provider: 'claude', prompt: 'Do it.\nOTTO_FAKE: @script' } });
    const side = { nodes: [{ id: 'n1', kind: 'manual_trigger' }, agent('n2', 'long'), agent('n3', 'long'), { id: 'n4', kind: 'log' }].map((n) => ({ x: 0, y: 0, name: n.id, params: {}, ...n })), edges: [edge('e1', 'n1', 'n2'), edge('e2', 'n1', 'n3'), edge('e3', 'n2', 'n4'), edge('e4', 'n3', 'n4')] };
    const wfs = {
      done: await mk(life.api, ws, 'Thread step', g([{ id: 'n1', kind: 'manual_trigger' }, agent('n2', 'quick'), { id: 'n3', kind: 'log' }])),
      gate: await mk(life.api, ws, 'Gate', g([{ id: 'n1', kind: 'manual_trigger' }, { id: 'n2', kind: 'human_approval', params: { message: 'ok?' } }, { id: 'n3', kind: 'log' }])),
      side: await mk(life.api, ws, 'Side by side', side),
    };
    const run = (k) => life.api('POST', '/workflows/' + wfs[k].id + '/run', { input: input(d) });
    const done = await run('done');
    const dr = await until(life.api, done.id, settled);
    check('new build: a thread-step run finished', dr.status === 'success', dr.status + ' ' + (dr.error ?? ''));
    const gate = await run('gate');
    await until(life.api, gate.id, (r) => r.waiting_approval);
    const sideRun = await run('side');
    const both = await poll(life.api, sideRun.id, (r) => r.nodes?.filter((n) => n.status === 'running').length === 2);
    check('new build: two steps running side by side at the stop', both.nodes?.filter((n) => n.status === 'running').length === 2, JSON.stringify(both.nodes?.map((n) => [n.node_id, n.status])));
    await life.stop();

    life = await boot(OLD, iso, PORT, OLD_ENV, path.join(OUT, 'rollback-old.log'));
    const list = await life.api('GET', '/workspaces/' + ws.id + '/workflows');
    check('old build boots on the new schema and lists the workflows', list.length === 3, list.length);
    const runs = await life.api('GET', '/workspaces/' + ws.id + '/workflow-runs');
    check('old build lists the runs', (runs.items ?? runs).length >= 3, (runs.items ?? runs).length);
    for (const id of [done.id, gate.id, sideRun.id]) {
      const routes = ['/workflow-runs/' + id, '/workflow-runs/' + id + '/progress', '/workspaces/' + ws.id + '/workflow-runs/active', '/workspaces/' + ws.id + '/workflow-runs/queue'];
      const res = await Promise.all(routes.map((p) => life.api('GET', p).then(() => 'ok', (e) => p + ' ' + e.status)));
      check('old build opens run ' + id + "'s page routes", res.every((r) => r === 'ok'), res.filter((r) => r !== 'ok').join(', '));
    }
    // Its steps' scripted work is 60 s: resumed or failed, the run settles well inside 180 s.
    const sv = await poll(life.api, sideRun.id, settled, 180000);
    check('old build settles the side-by-side run it found running', settled(sv), sv.status + ' ' + (sv.error ?? '') + ' ' + JSON.stringify(sv.nodes?.map((n) => [n.node_id, n.status])));
    const gv = await life.api('GET', '/workflow-runs/' + gate.id);
    check('old build still shows the parked approval', gv.waiting_approval, gv.status);
    await life.stop();
    const errs = fs.readFileSync(path.join(OUT, 'rollback-old.log'), 'utf8').split('\n').filter((l) => /\bERROR\b/.test(l));
    check('old build boot log on the new schema has no ERROR', errs.length === 0, errs.slice(0, 3).join(' | '));
  } finally {
    cleanup(iso.root, life?.child.pid);
  }
}

// The new build restarted under a step mid-turn (a deploy of the same build):
// the step resumes on its thread and the daemon survives the resume (a debug
// build overflowed a 2 MiB worker stack there — wf_restart's ignored repro).
async function restart() {
  const iso = isolatedRoot(FAKES);
  const { d } = iso;
  let life;
  try {
    fs.writeFileSync(path.join(d.scripts, 'long.txt'), 'sleep 8000\nwrite $HANDOFF @filler:400\nreply done\n');
    life = await boot(NEW, iso, PORT, NEW_ENV, path.join(OUT, 'restart-new-1.log'));
    const ws = await setup(life.api, d);
    const wf = await mk(life.api, ws, 'Long step', g([{ id: 'n1', kind: 'manual_trigger' }, { id: 'n2', kind: 'agent_prompt', name: 'long', params: { provider: 'claude', prompt: 'Do it.\nOTTO_FAKE: @script' } }, { id: 'n3', kind: 'log' }]));
    const run = await life.api('POST', '/workflows/' + wf.id + '/run', { input: input(d) });
    await until(life.api, run.id, (r) => r.nodes?.some((n) => n.node_id === 'n2' && n.status === 'running' && n.thread));
    await sleep(1500);
    await life.stop();
    life = await boot(NEW, iso, PORT, NEW_ENV, path.join(OUT, 'restart-new-2.log'));
    const r = await until(life.api, run.id, settled, 120000);
    check('restart mid agent step: the run finishes after the boot', r.status === 'success', r.status + ' ' + (r.error ?? '') + ' resume_attempts ' + r.resume_attempts);
    await sleep(2000);
    check('restart mid agent step: the daemon is still alive after the resume', life.child.exitCode === null && life.child.signalCode === null, 'exit ' + life.child.exitCode + ' ' + (life.child.signalCode ?? ''));
    await life.stop();
  } finally {
    cleanup(iso.root, life?.child.pid);
  }
}

const which = a.only ?? 'all';
try {
  if (['all', 'upgrade'].includes(which)) await upgrade();
  if (['all', 'rollback'].includes(which)) await rollback();
  if (['all', 'restart'].includes(which)) await restart();
} catch (e) {
  check('harness ran to the end', false, e.stack ?? e);
}
const md = ['| check | result | detail |', '|---|---|---|', ...checks.map((c) => '| ' + c.name + ' | ' + (c.ok ? 'pass' : '**FAIL**') + ' | ' + c.detail.replace(/\|/g, '/').replace(/\n/g, ' ') + ' |')].join('\n');
fs.writeFileSync(path.join(OUT, 'upgrade.md'), md + '\n');
fs.writeFileSync(path.join(OUT, 'upgrade.json'), JSON.stringify(checks, null, 1));
process.exit(checks.every((c) => c.ok) ? 0 : 1);
