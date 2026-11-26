#!/usr/bin/env node
// Load L1 (test plan §6.1): two runs × twelve reviewers on fake processes with
// the admission rule live (ceiling 6, a calm machine sample so only the ceiling
// and the floor rule decide), sampled every 2 s. Isolated (daemon.mjs).
//
//   node scripts/workflows/load-l1.mjs --ottod target/release/ottod --out <dir>
//     [--port 7792] [--scale 0.2] [--ceiling 6] [--work-s 300] [--recovery-s 300]
//
// Writes <dir>/samples.jsonl, summary.json, summary.md, ottod.log, driver.log.
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { args as parse, boot, cleanup as remove, isolatedRoot, sh, sleep } from './daemon.mjs';

const args = parse();
const OTTOD = path.resolve(args.ottod ?? 'target/release/ottod');
const FAKES = path.resolve(args.fakes ?? path.dirname(OTTOD));
const OUT = path.resolve(args.out ?? fs.mkdtempSync(path.join(os.tmpdir(), 'otto-l1-')));
const PORT = Number(args.port ?? 7792);
const SCALE = Number(args.scale ?? 0.2);
const CEILING = Number(args.ceiling ?? 6);
const WORK_S = Number(args['work-s'] ?? 300); // a reviewer's scripted work before scaling
const RECOVERY_S = Number(args['recovery-s'] ?? 300);
const LENSES = 6; // × claude + codex = 12 reviewers per run
fs.mkdirSync(OUT, { recursive: true });
const log = (...a) => { const l = new Date().toISOString() + ' ' + a.join(' '); console.log(l); fs.appendFileSync(path.join(OUT, 'driver.log'), l + '\n'); };

const iso = isolatedRoot(FAKES);
const dirs = iso.d;
// A reviewer works WORK_S (scaled), streaming ≈ 2 KB/s and holding 150 MB.
fs.writeFileSync(path.join(dirs.review, 'reviewer.pre'), 'rss 150\nstream ' + Math.round(2 * WORK_S * SCALE) * 1000 + ' ' + WORK_S * 1000 + '\n');
const lenses = Array.from({ length: LENSES }, (_, i) => 'lens-' + (i + 1) + '-review');
const plan = { summary: 'load', related_modules: [], risks: [], lenses: lenses.map((name) => ({ name, reason: 'load', instructions: 'look' })) };
fs.writeFileSync(path.join(dirs.review, 'scope.json'), JSON.stringify([{ severity: 'info', body: JSON.stringify(plan) }]));
let daemon, api;
const cleanup = () => { try { process.kill(-daemon.pid, 'SIGKILL'); } catch {} remove(iso.root); };
process.on('SIGINT', () => { cleanup(); process.exit(130); });

function sample(t0, runIds) {
  // -E appends each process's environment: everything the daemon started carries
  // the isolated root (HOME, OTTO_DATA_DIR), so a holder reparented to launchd counts too.
  const rows = sh('ps', ['-axEo', 'pid=,ppid=,rss=,pcpu=,command='], { maxBuffer: 64 << 20 }).trim().split('\n').map((l) => l.trim().split(/\s+/)).map(([pid, ppid, rss, cpu, ...c]) => ({ pid: +pid, ppid: +ppid, rss: +rss, cpu: +cpu, comm: path.basename(c[0] ?? ''), under: c.join(' ').includes(iso.root) }));
  const mine = new Set([daemon.pid, ...rows.filter((r) => r.under).map((r) => r.pid)]);
  for (let grew = true; grew; ) { grew = false; for (const r of rows) if (!mine.has(r.pid) && mine.has(r.ppid)) { mine.add(r.pid); grew = true; } }
  const by = {};
  for (const r of rows.filter((r) => mine.has(r.pid))) {
    const k = r.pid === daemon.pid ? 'ottod' : /otto-fake|claude|codex/.test(r.comm) ? 'fake' : /holder/.test(r.comm) ? 'holder' : /clickhouse/.test(r.comm) ? 'clickhouse' : r.comm;
    by[k] ??= { n: 0, rss_mb: 0, cpu: 0 }; by[k].n++; by[k].rss_mb += r.rss / 1024; by[k].cpu += r.cpu;
  }
  let fds = 0; try { fds = sh('/usr/sbin/lsof', ['-p', String(daemon.pid)]).split('\n').length - 2; } catch {}
  const swap = Number((sh('sysctl', ['-n', 'vm.swapusage']).match(/used = ([\d.]+)M/) ?? [0, 0])[1]);
  const q0 = Date.now();
  let threads = '', revs = 0;
  try {
    const db = path.join(dirs.data, 'otto.db');
    threads = sh('sqlite3', ['-readonly', '-cmd', '.timeout 2000', db, "SELECT status || ':' || COUNT(*) FROM agent_runtime_sessions GROUP BY status"]).trim().replace(/\n/g, ' ');
    revs = Number(sh('sqlite3', ['-readonly', '-cmd', '.timeout 2000', db, 'SELECT COALESCE(SUM(rev),0) FROM workflow_runs']).trim());
  } catch {}
  return { t: (Date.now() - t0) / 1000, by, fds, swap_mb: swap, threads, revs, db_read_ms: Date.now() - q0, load1: os.loadavg()[0] };
}

async function main() {
  const life = await boot(OTTOD, iso, PORT, {
    OTTO_WF_STEP_RUNNER: 'threads', OTTO_TEST_MACHINE_SAMPLE: 'calm', OTTO_FAKE_TIME_SCALE: String(SCALE),
    OTTO_FAKE_LOG: path.join(OUT, 'fake-turns.jsonl'),
  }, path.join(OUT, 'ottod.log'));
  daemon = life.child;
  api = life.api;
  const ws = await api('POST', '/workspaces', { name: 'L1', root_path: path.dirname(dirs['ws/repo']) });
  await api('POST', '/workspaces/' + ws.id + '/repos', { path: dirs['ws/repo'], name: 'repo' });
  await api('PUT', '/settings', { agent_background_max_live: CEILING });
  const reviewers = lenses.map((lens) => ({ lens, providers: ['claude', 'codex'] }));
  const edge = (id, a, b) => ({ id, source: a, target: b });
  const graph = { nodes: [
    { id: 'n1', kind: 'manual_trigger', name: 'Start', x: 0, y: 0, params: {} },
    { id: 'nc', kind: 'git_checkout', name: 'Checkout', x: 0, y: 0, params: { mode: 'branch' } },
    { id: 'n2', kind: 'review_run', name: 'Review', x: 0, y: 0, params: { reviewers, cross_provider_check: true, threshold: 80, scope_assessment: 'configured', summarizer: { provider: 'claude' }, timeout_s: 7200 } },
    { id: 'n3', kind: 'log', name: 'Done', x: 0, y: 0, params: {} },
  ], edges: [edge('e1', 'n1', 'nc'), edge('e0', 'nc', 'n2'), edge('e2', 'n2', 'n3')] };
  const wf = await api('POST', '/workspaces/' + ws.id + '/workflows', { name: 'L1 review', graph });
  const t0 = Date.now();
  const samples = [];
  const take = (phase, runIds) => { const s = { phase, ...sample(t0, runIds) }; samples.push(s); fs.appendFileSync(path.join(OUT, 'samples.jsonl'), JSON.stringify(s) + '\n'); return s; };
  log('baseline 120 s'); for (let i = 0; i < 60; i++) { take('baseline'); await sleep(2000); }
  const input = { working_directory: dirs['ws/repo'], base: 'base', pr_branch: 'feature' };
  const runs = await Promise.all([1, 2].map(() => api('POST', '/workflows/' + wf.id + '/run', { input })));
  const ids = runs.map((r) => r.id);
  log('runs', ids.join(' '));
  let waits = [], done = [];
  for (;;) {
    const s = take('load', ids);
    const views = await Promise.all(ids.map((id) => api('GET', '/workflow-runs/' + id)));
    for (const v of views) for (const n of v.nodes ?? []) if (n.phase === 'waiting') waits.push({ t: s.t, node: n.node_id, wait: n.wait ?? null });
    if (views.every((v) => ['success', 'error', 'canceled'].includes(v.status))) { done = views; break; }
    if (s.t > 3600) throw new Error('deadline');
    await sleep(2000);
  }
  const ended = (Date.now() - t0) / 1000;
  log('runs settled', done.map((v) => v.status).join(' '), 'recovery ' + RECOVERY_S + ' s');
  for (let i = 0; i < RECOVERY_S / 2; i++) { take('recovery'); await sleep(2000); }
  // Reviews' own rows say how many reviewers ran per run and in what overlap.
  const reviewRows = sh('sqlite3', ['-readonly', path.join(dirs.data, 'otto.db'), "SELECT COUNT(*) FROM agent_runtime_sessions"]).trim();
  return { samples, done, waits, ended, reviewRows };
}

function summarize({ samples, done, waits, ended, reviewRows }) {
  const of = (ph) => samples.filter((s) => s.phase === ph);
  const g = (s, k, f) => s.by[k]?.[f] ?? 0;
  const mean = (a) => a.reduce((x, y) => x + y, 0) / Math.max(1, a.length);
  const base = of('baseline').slice(-15), load = of('load'), rec = of('recovery');
  const baseRss = mean(base.map((s) => g(s, 'ottod', 'rss_mb'))), baseFds = mean(base.map((s) => s.fds));
  const liveMax = Math.max(...load.map((s) => g(s, 'fake', 'n')));
  const busy = load.filter((s) => g(s, 'fake', 'n') >= CEILING);
  const last60 = rec.filter((s) => s.t >= ended + 60);
  const revRate = load.length > 1 ? (load.at(-1).revs - load[0].revs) / (load.at(-1).t - load[0].t) : 0;
  const lines = [
    ['twelve reviewers per run (≥ ' + 4 * LENSES + ' agent threads over both runs)', Number(reviewRows) >= 2 * LENSES * 2, reviewRows + ' thread rows'],
    ['live fake agents reach but never pass the ceiling (' + CEILING + ')', liveMax > 0 && liveMax <= CEILING && busy.length > 0, 'max ' + liveMax + ', ' + busy.length + ' samples at the ceiling'],
    ['both runs succeed', done.every((v) => v.status === 'success'), done.map((v) => v.status + (v.error ? ' (' + v.error.slice(0, 120) + ')' : '')).join(' / ')],
    ['ottod CPU mean ≤ 30 % with the ceiling live', busy.length > 0 && mean(busy.map((s) => g(s, 'ottod', 'cpu'))) <= 30, mean(busy.map((s) => g(s, 'ottod', 'cpu'))).toFixed(1) + ' % over ' + busy.length + ' samples'],
    ['ottod RSS peak ≤ baseline + 250 MB', Math.max(...load.map((s) => g(s, 'ottod', 'rss_mb'))) <= baseRss + 250, 'baseline ' + baseRss.toFixed(0) + ' MB, peak ' + Math.max(...load.map((s) => g(s, 'ottod', 'rss_mb'))).toFixed(0)],
    ['ottod RSS ≤ baseline + 50 MB at the end of recovery', g(rec.at(-1), 'ottod', 'rss_mb') <= baseRss + 50, g(rec.at(-1), 'ottod', 'rss_mb').toFixed(0) + ' MB'],
    ['ottod fds back to baseline ± 10', Math.abs(rec.at(-1).fds - baseFds) <= 10, 'baseline ' + baseFds.toFixed(0) + ', end ' + rec.at(-1).fds + ', peak ' + Math.max(...load.map((s) => s.fds))],
    ['fd peak ≤ baseline + 40 per live agent', Math.max(...load.map((s) => s.fds)) <= baseFds + 40 * Math.max(1, liveMax), ''],
    ['0 fake or holder processes 60 s after the runs', last60.length > 0 && last60.every((s) => !g(s, 'fake', 'n') && !g(s, 'holder', 'n')), last60.length + ' samples'],
    ['swap growth ≤ 100 MB', Math.max(...samples.map((s) => s.swap_mb)) - samples[0].swap_mb <= 100, (Math.max(...samples.map((s) => s.swap_mb)) - samples[0].swap_mb).toFixed(0) + ' MB'],
  ];
  const md = ['| pass line | result | measured |', '|---|---|---|', ...lines.map(([l, ok, m]) => '| ' + l + ' | ' + (ok ? 'pass' : '**FAIL**') + ' | ' + m + ' |')].join('\n');
  const extra = '\nruns took ' + ended.toFixed(0) + ' s (time scale ' + SCALE + ', reviewer work ' + WORK_S + ' s scripted); run-row writes ' + revRate.toFixed(2) + '/s while loaded; db read probe p95 ' +
    load.map((s) => s.db_read_ms).sort((a, b) => a - b)[Math.floor(load.length * 0.95)] + ' ms; thread rows at the end: ' + reviewRows + '; fake RSS max ' + Math.max(...load.map((s) => g(s, 'fake', 'rss_mb'))).toFixed(0) + ' MB; ottod CPU max ' + Math.max(...load.map((s) => g(s, 'ottod', 'cpu'))).toFixed(0) + ' %.\n';
  fs.writeFileSync(path.join(OUT, 'summary.md'), md + '\n' + extra);
  fs.writeFileSync(path.join(OUT, 'summary.json'), JSON.stringify({ lines, ended, waits: waits.slice(0, 50), done: done.map((v) => ({ id: v.id, status: v.status, error: v.error })) }, null, 1));
  console.log(md + '\n' + extra);
  return lines.every(([, ok]) => ok);
}

main().then((r) => { const ok = summarize(r); cleanup(); process.exit(ok ? 0 : 1); }, (e) => { log('error', e.stack ?? e); cleanup(); process.exit(2); });
