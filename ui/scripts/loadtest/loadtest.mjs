#!/usr/bin/env node
// Parallel-load test: CPU and RAM of Otto vs the number of parallel agents
// (perf round 3, r3-11). See README.md next to this file.
//
// Stands up an ISOLATED ottod (temp data dir, temp HOME, its own port — never
// the real daemon on :7700) the way ui/e2e/global-setup.ts does, plus two
// isolation gaps that harness leaves open:
//   * HOME -> a temp dir, so the daemon's usage tailer / monitor / transcript
//     code never reads the user's real ~/.claude or ~/.codex;
//   * a `claude` on PATH that is OUR agent emulator (fake-claude.mjs), not the
//     harness's echo+cat stand-in.
// The UI is the daemon's EMBEDDED production bundle (what the app runs),
// driven by Playwright Chromium (CDP metrics available; WebKit has none).
//
// Usage (from ui/, one heavy run at a time, foreground):
//   node scripts/loadtest/loadtest.mjs --mode scale --steps 1,3 --hold 180
//   node scripts/loadtest/loadtest.mjs --mode scale --steps 5,10 --hold 180
//   node scripts/loadtest/loadtest.mjs --mode leak --n 5 --leak-min 15
//   node scripts/loadtest/analyze.mjs <run dir>
// Env: OTTO_E2E_BIN (default <repo>/target/debug/ottod), OTTO_E2E_PORT (7831),
//      OTTO_LOADTEST_RUNS (default $TMPDIR/otto-loadtest), OTTO_LOADTEST_UI_DIR
//      (the ui/ whose node_modules provide Playwright; default this repo's).
import { spawn, execFileSync, spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const UI_DIR = process.env.OTTO_LOADTEST_UI_DIR ?? path.resolve(HERE, '..', '..');
const REPO = path.dirname(UI_DIR);
const require = createRequire(path.join(UI_DIR, 'package.json'));
const { chromium } = require('@playwright/test');

// ── args ────────────────────────────────────────────────────────────────────
const A = Object.fromEntries(
  process.argv.slice(2).reduce((acc, a, i, arr) => {
    if (a.startsWith('--')) acc.push([a.slice(2), arr[i + 1] && !arr[i + 1].startsWith('--') ? arr[i + 1] : 'true']);
    return acc;
  }, []),
);
const MODE = A.mode ?? 'scale';
const STEPS = (A.steps ?? '1,3,5,10').split(',').map(Number);
const HOLD = Number(A.hold ?? 180); // seconds per N
const LEAK_N = Number(A.n ?? 5);
const LEAK_MIN = Number(A['leak-min'] ?? 15);
const BASELINE = Number(A.baseline ?? 30);
const RECOVER = Number(A.recover ?? 90);
const SAMPLE_S = Number(A.sample ?? 5);
const DEADLINE_S = Number(A.deadline ?? 1e9); // hard self-teardown budget
const PORT = process.env.OTTO_E2E_PORT ?? A.port ?? '7831';
// The daemon binary must serve the UI (built with `--features embed-ui`),
// unless --ui-url points elsewhere. The run is isolated whichever binary it
// is: temp data dir, temp HOME, its own port.
const OTTOD =
  process.env.OTTO_E2E_BIN ??
  [path.join(REPO, 'target', 'release', 'ottod'), '/Applications/Otto.app/Contents/MacOS/ottod'].find((p) => fs.existsSync(p)) ??
  path.join(REPO, 'target', 'release', 'ottod');
const RUNS = process.env.OTTO_LOADTEST_RUNS ?? path.join(os.tmpdir(), 'otto-loadtest');
const OUT = path.resolve(A.out ?? path.join(RUNS, `${MODE}-${Date.now()}`));
const STACKS = A.stacks !== 'false';
const MAX_LOAD = Number(A['max-load'] ?? 12);
const MIN_FREE_GB = Number(A['min-free-gb'] ?? 2);
const API = `http://127.0.0.1:${PORT}/api/v1`;
const UI = A['ui-url'] ?? `http://127.0.0.1:${PORT}`;
const FIXTURE = path.join(REPO, 'crates/otto-transcript/fixtures/claude/01-basic-tools.jsonl');
if (PORT === '7700') throw new Error('refusing to use the real daemon port 7700');
if (!fs.existsSync(OTTOD)) {
  throw new Error(`ottod not found at ${OTTOD} — (cd ui && npm run build) && cargo build --release -p ottod --features embed-ui, or set OTTO_E2E_BIN`);
}
// Something already answers on the port (another run, or a daemon someone
// started there): never drive it — this test only drives the daemon it spawns.
try {
  await fetch(`http://127.0.0.1:${PORT}/api/v1/health`, { signal: AbortSignal.timeout(1500) });
  throw new Error(`port ${PORT} is already serving — pick another with OTTO_E2E_PORT`);
} catch (e) {
  if (String(e).includes('already serving')) throw e;
}
fs.mkdirSync(OUT, { recursive: true });
const T0 = Date.now();
const now = () => ((Date.now() - T0) / 1000).toFixed(1);
const log = (...a) => {
  const s = `[loadtest +${now()}s] ${a.join(' ')}`;
  console.log(s);
  fs.appendFileSync(path.join(OUT, 'driver.log'), s + '\n');
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ── safety ──────────────────────────────────────────────────────────────────
// macOS keeps "Pages free" near zero by design (it sat at ~1.9 GB on this Mac
// with NO test running), so the floor uses the kernel's own availability
// figure — kern.memorystatus_level (= memory_pressure's "System-wide memory
// free percentage") x RAM — and additionally aborts if swap use grows by more
// than SWAP_GROWTH_MB during the run (the machine is already ~5 GB into swap).
const MEMSIZE = Number(execFileSync('sysctl', ['-n', 'hw.memsize'], { encoding: 'utf8' }));
const SWAP_GROWTH_MB = Number(A['max-swap-growth-mb'] ?? 500);
const swapUsedMB = () => Number((execFileSync('sysctl', ['-n', 'vm.swapusage'], { encoding: 'utf8' }).match(/used = ([\d.]+)M/) ?? [0, 0])[1]);
const SWAP0 = swapUsedMB();
function machine() {
  const la = execFileSync('sysctl', ['-n', 'vm.loadavg'], { encoding: 'utf8' }).match(/[\d.]+/g).map(Number);
  const vm = execFileSync('vm_stat', { encoding: 'utf8' });
  const page = Number(vm.match(/page size of (\d+)/)[1]);
  const get = (k) => Number((vm.match(new RegExp(`${k}:\\s+(\\d+)`)) ?? [0, 0])[1]);
  const pagesFreeGB = ((get('Pages free') + get('Pages speculative')) * page) / 2 ** 30;
  const level = Number(execFileSync('sysctl', ['-n', 'kern.memorystatus_level'], { encoding: 'utf8' }));
  const availGB = (level / 100) * (MEMSIZE / 2 ** 30);
  return { load1: la[0], load5: la[1], freeGB: +availGB.toFixed(2), memFreePct: level, pagesFreeGB: +pagesFreeGB.toFixed(2), swapMB: swapUsedMB() };
}
class Abort extends Error {}
function safetyCheck(where) {
  const m = machine();
  if (m.load1 > MAX_LOAD || m.freeGB < MIN_FREE_GB || m.swapMB - SWAP0 > SWAP_GROWTH_MB) {
    throw new Abort(`SAFETY ABORT at ${where}: load1=${m.load1} (cap ${MAX_LOAD}) avail=${m.freeGB}GB (floor ${MIN_FREE_GB}GB) swap +${(m.swapMB - SWAP0).toFixed(0)}MB (cap ${SWAP_GROWTH_MB})`);
  }
  if (Date.now() - T0 > DEADLINE_S * 1000) throw new Abort(`deadline ${DEADLINE_S}s reached at ${where}`);
  return m;
}

// ── process accounting ──────────────────────────────────────────────────────
function parseTime(t) {
  // macOS ps `time`: [[dd-]hh:]mm:ss.cc
  let days = 0;
  if (t.includes('-')) [days, t] = [Number(t.split('-')[0]), t.split('-')[1]];
  const p = t.split(':').map(Number).reverse();
  return days * 86400 + (p[0] ?? 0) + (p[1] ?? 0) * 60 + (p[2] ?? 0) * 3600;
}
function psTable() {
  const out = execFileSync('ps', ['-axo', 'pid=,ppid=,rss=,time=,command='], { encoding: 'utf8', maxBuffer: 64 << 20 });
  const rows = new Map();
  for (const line of out.split('\n')) {
    const m = line.match(/^\s*(\d+)\s+(\d+)\s+(\d+)\s+(\S+)\s+(.*)$/);
    if (!m) continue;
    rows.set(+m[1], { pid: +m[1], ppid: +m[2], rssKB: +m[3], cpuS: parseTime(m[4]), cmd: m[5] });
  }
  return rows;
}
function descendants(rows, root) {
  const kids = new Map();
  for (const r of rows.values()) {
    if (!kids.has(r.ppid)) kids.set(r.ppid, []);
    kids.get(r.ppid).push(r.pid);
  }
  const out = [];
  const q = [root];
  while (q.length) {
    const p = q.shift();
    for (const k of kids.get(p) ?? []) {
      out.push(k);
      q.push(k);
    }
  }
  return out;
}
function threads(pid) {
  try {
    return execFileSync('ps', ['-M', '-p', String(pid)], { encoding: 'utf8' }).trim().split('\n').length - 1;
  } catch {
    return null;
  }
}
const ctx = { daemonPid: null, browserPid: null, root: null, dataDir: null };
function classify(rows) {
  const groups = {};
  const add = (g, r) => {
    groups[g] ??= { pids: [], rssKB: 0, cpuS: 0, n: 0, maxRssKB: 0 };
    const G = groups[g];
    G.pids.push(r.pid);
    G.rssKB += r.rssKB;
    G.cpuS += r.cpuS;
    G.n++;
    G.maxRssKB = Math.max(G.maxRssKB, r.rssKB);
  };
  const seen = new Set();
  if (ctx.daemonPid && rows.has(ctx.daemonPid)) {
    add('ottod', rows.get(ctx.daemonPid));
    for (const p of descendants(rows, ctx.daemonPid)) {
      const r = rows.get(p);
      seen.add(p);
      if (/clickhouse/.test(r.cmd)) add('clickhouse', r);
      else if (/fake-claude\.mjs/.test(r.cmd)) add('agents', r);
      else add('daemon-children', r);
    }
  }
  // ClickHouse may be re-parented (watchdog) — also catch it by its config path.
  if (ctx.dataDir) {
    for (const r of rows.values()) {
      if (!seen.has(r.pid) && r.cmd.includes(ctx.dataDir) && /clickhouse/.test(r.cmd)) add('clickhouse', r);
    }
  }
  if (ctx.browserPid && rows.has(ctx.browserPid)) {
    add('browser-main', rows.get(ctx.browserPid));
    for (const p of descendants(rows, ctx.browserPid)) {
      const r = rows.get(p);
      const t = (r.cmd.match(/--type=([\w-]+)/) ?? [0, 'other'])[1];
      add(t === 'renderer' ? 'renderer' : t === 'gpu-process' ? 'gpu' : `browser-${t}`, r);
    }
  }
  return groups;
}

// ── daemon lifecycle (mirrors ui/e2e/global-setup.ts) ───────────────────────
async function waitHealthy(ms) {
  const dl = Date.now() + ms;
  while (Date.now() < dl) {
    try {
      const r = await fetch(`${API}/health`, { signal: AbortSignal.timeout(2000) });
      if (r.ok) return;
    } catch {}
    await sleep(500);
  }
  throw new Error('daemon never healthy');
}
let TOKEN = '';
async function api(method, p, body) {
  const r = await fetch(`${API}${p}`, {
    method,
    headers: { Authorization: `Bearer ${TOKEN}`, 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (!r.ok) throw new Error(`${method} ${p} -> ${r.status} ${await r.text()}`);
  return r.status === 204 ? null : r.json().catch(() => null);
}
function sh(cmd, args, opts = {}) {
  const r = spawnSync(cmd, args, { encoding: 'utf8', maxBuffer: 256 << 20, ...opts });
  if (r.status !== 0) throw new Error(`${cmd} ${args.join(' ')} failed: ${r.stderr}`);
  return r.stdout;
}
function makeRepo(dir) {
  // ~400 commits over 60 files via fast-import, then a ~3k-line dirty worktree.
  fs.mkdirSync(dir, { recursive: true });
  sh('git', ['init', '-q', '-b', 'main', dir]);
  let s = '';
  const files = 60;
  for (let c = 1; c <= 400; c++) {
    s += `commit refs/heads/main\ncommitter Load Test <load@test> ${1_700_000_000 + c * 600} +0000\n`;
    const msg = `load commit ${c}: touch ${3} files`;
    s += `data ${Buffer.byteLength(msg)}\n${msg}\n`;
    for (let k = 0; k < (c === 1 ? files : 3); k++) {
      const f = c === 1 ? k : (c * 7 + k * 13) % files;
      let body = '';
      for (let l = 0; l < 120; l++) body += `line ${l} of file ${f} rev ${c} ${'x'.repeat((l * c) % 40)}\n`;
      s += `M 100644 inline src/mod_${f}.rs\ndata ${Buffer.byteLength(body)}\n${body}`;
    }
    s += '\n';
  }
  sh('git', ['-C', dir, 'fast-import', '--quiet'], { input: s });
  sh('git', ['-C', dir, 'checkout', '-q', '-f', 'main']);
  for (let f = 0; f < 25; f++) {
    let body = '';
    for (let l = 0; l < 120; l++) body += `edited line ${l} of file ${f} ${'y'.repeat(l % 30)}\n`;
    fs.writeFileSync(path.join(dir, 'src', `mod_${f}.rs`), body);
  }
}
async function startDaemon() {
  ctx.root = fs.mkdtempSync(path.join(os.tmpdir(), 'otto-loadtest-'));
  ctx.dataDir = path.join(ctx.root, 'data');
  const home = path.join(ctx.root, 'home');
  const fakeBin = path.join(ctx.root, 'fake-agent-bin');
  for (const d of [ctx.dataDir, home, fakeBin, path.join(home, '.claude/projects'), path.join(home, '.codex/sessions')])
    fs.mkdirSync(d, { recursive: true });
  for (const cli of ['codex', 'agy', 'gemini', 'grok']) {
    fs.writeFileSync(path.join(fakeBin, cli), `#!/bin/sh\necho "otto e2e: the real ${cli} CLI is disabled in tests"\nexec cat\n`, { mode: 0o755 });
  }
  fs.writeFileSync(path.join(fakeBin, 'claude'), `#!/bin/sh\nexec "${process.execPath}" "${path.join(HERE, 'fake-claude.mjs')}" "$@"\n`, { mode: 0o755 });
  fs.writeFileSync(path.join(OUT, 'run.json'), JSON.stringify({ root: ctx.root, port: PORT, ottod: OTTOD }, null, 2));
  log(`root=${ctx.root}`);
  const env = {
    ...process.env,
    HOME: home, // isolation gap in the stock harness: keep the daemon off the real ~/.claude, ~/.codex
    CODEX_HOME: path.join(home, '.codex'),
    OTTO_TRANSCRIPT_ROOTS: `${path.join(home, '.claude/projects')}:${path.join(home, '.codex/sessions')}`,
    OTTO_DATA_DIR: ctx.dataDir,
    OTTO_PORT: PORT,
    PATH: `${fakeBin}:${process.env.PATH ?? ''}`,
    OTTO_SELF_IMPROVE: '0',
    OTTO_CLI_UPDATE: '0',
    OTTO_E2E: '1',
    CLAUDE_BIN: '/nonexistent/otto-e2e-no-claude',
    OTTO_PLUGINS_HOME: path.join(ctx.dataDir, 'plugins-home'),
    OTTO_SECRETS: 'file',
    FAKE_FIXTURE: FIXTURE,
    ...(A['quiet-agents'] === 'true' ? { FAKE_QUIET: '1' } : {}),
    RUST_LOG: process.env.RUST_LOG ?? 'info',
  };
  const logFd = fs.openSync(path.join(OUT, 'ottod.log'), 'a');
  const child = spawn(OTTOD, [], { env, stdio: ['ignore', logFd, logFd], cwd: ctx.root });
  ctx.daemonPid = child.pid;
  fs.writeFileSync(path.join(OUT, 'pids.json'), JSON.stringify({ daemon: child.pid, root: ctx.root }));
  await waitHealthy(90_000);
  if (!A['ui-url']) {
    const r = await fetch(`${UI}/`).catch(() => null);
    const html = r?.ok ? await r.text() : '';
    if (!html.includes('<html')) {
      throw new Error(`${OTTOD} serves no UI — use a build with --features embed-ui (or pass --ui-url)`);
    }
  }
  const r = await fetch(`${API}/onboarding/root`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ password: 'otto-e2e-password', display_name: 'Load Root' }),
  });
  if (!r.ok) throw new Error(`onboarding ${r.status} ${await r.text()}`);
  TOKEN = (await r.json()).token;
  const wsRoot = path.join(ctx.root, 'ws');
  const repoDir = path.join(wsRoot, 'repo');
  makeRepo(repoDir);
  const ws = await api('POST', '/workspaces', { name: 'Load WS', root_path: wsRoot });
  const repo = await api('POST', `/workspaces/${ws.id}/repos`, { path: repoDir, name: 'load-repo' });
  let connId = null;
  try {
    const xml = fs.readFileSync(path.join(ctx.dataDir, 'clickhouse/server/config.xml'), 'utf8');
    const chPort = Number((xml.match(/<http_port>(\d+)<\/http_port>/) ?? [0, 0])[1]);
    if (chPort) {
      const c = await api('POST', `/workspaces/${ws.id}/connections`, { name: 'load-ch', kind: 'clickhouse', params: { host: '127.0.0.1', port: chPort, user: 'default', db: 'default' } });
      connId = c.id;
    }
  } catch (e) {
    log(`db connection: ${e}`);
  }
  log(`daemon pid=${child.pid} ws=${ws.id} repo=${repo.id} conn=${connId}`);
  return { wsId: ws.id, repoId: repo.id, repoDir, connId };
}

// ── browser ─────────────────────────────────────────────────────────────────
const PAGE_PROBE = () => {
  // Long tasks, frame jank, WS bytes by endpoint, request counts by endpoint.
  const w = window;
  w.__otto_lt = { lt: { n: 0, total: 0, max: 0 }, fr: { n: 0, jank: 0, max: 0 }, ws: {}, };
  try {
    new PerformanceObserver((l) => {
      for (const e of l.getEntries()) {
        w.__otto_lt.lt.n++;
        w.__otto_lt.lt.total += e.duration;
        w.__otto_lt.lt.max = Math.max(w.__otto_lt.lt.max, e.duration);
      }
    }).observe({ type: 'longtask', buffered: false });
  } catch {}
  let last = performance.now();
  const loop = (t) => {
    const d = t - last;
    last = t;
    w.__otto_lt.fr.n++;
    if (d > 50) w.__otto_lt.fr.jank++;
    if (d > w.__otto_lt.fr.max) w.__otto_lt.fr.max = d;
    requestAnimationFrame(loop);
  };
  requestAnimationFrame(loop);
  const WS = w.WebSocket;
  w.WebSocket = function (url, proto) {
    const s = proto === undefined ? new WS(url) : new WS(url, proto);
    let key = 'other';
    try {
      key = new URL(url, location.href).pathname.replace(/\/[0-9a-z_-]{16,}/gi, '/:id');
    } catch {}
    s.addEventListener('message', (ev) => {
      const b = typeof ev.data === 'string' ? ev.data.length : ev.data.byteLength ?? ev.data.size ?? 0;
      const k = (w.__otto_lt.ws[key] ??= { msgs: 0, bytes: 0, open: 0 });
      k.msgs++;
      k.bytes += b;
    });
    const k = (w.__otto_lt.ws[key] ??= { msgs: 0, bytes: 0, open: 0 });
    k.open++;
    s.addEventListener('close', () => (w.__otto_lt.ws[key].open--));
    return s;
  };
  w.WebSocket.prototype = WS.prototype;
  Object.assign(w.WebSocket, { CONNECTING: 0, OPEN: 1, CLOSING: 2, CLOSED: 3 });
};
async function startBrowser(wsId) {
  const browser = await chromium.launch({ headless: true, args: ['--enable-precise-memory-info', '--js-flags=--expose-gc'] });
  // Playwright exposes the browser process only for launchServer; find it by our unique user-data-dir parent.
  const rows = psTable();
  const cands = [...rows.values()].filter((r) => r.ppid === process.pid && /chrom/i.test(r.cmd));
  ctx.browserPid = cands.length ? cands[cands.length - 1].pid : null;
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 }, serviceWorkers: 'block' });
  await context.addInitScript(
    ([token, base, ws]) => {
      localStorage.setItem('otto_token', token);
      localStorage.setItem('otto_base', base);
      localStorage.setItem('otto_workspace', ws);
    },
    [TOKEN, UI, wsId],
  );
  await context.addInitScript(PAGE_PROBE);
  const page = await context.newPage();
  const errors = [];
  page.on('pageerror', (e) => errors.push(String(e).slice(0, 300)));
  const cdp = await context.newCDPSession(page);
  await cdp.send('Performance.enable');
  await page.goto(`${UI}/#/agents`);
  await page.waitForSelector('.navigator', { timeout: 30_000 });
  log(`browser pid=${ctx.browserPid}`);
  fs.writeFileSync(path.join(OUT, 'pids.json'), JSON.stringify({ daemon: ctx.daemonPid, browser: ctx.browserPid, root: ctx.root }));
  return { browser, context, page, cdp, errors };
}

// ── UI states ───────────────────────────────────────────────────────────────
async function uiState(B, state, env) {
  const { page } = B;
  const hash = (h) => page.evaluate((x) => (location.hash = x), h);
  try {
    if (state === 'tiled') {
      await hash('#/agents');
      await page.getByRole('button', { name: 'Tiled view', exact: true }).click({ timeout: 10_000 });
    } else if (state === 'focus') {
      await hash('#/agents');
      await page.getByRole('button', { name: 'Tabbed view', exact: true }).click({ timeout: 10_000 });
      const row = page.locator('.navigator .nested-item:not(.archived)', { hasText: 'load-1' }).first();
      if (await row.count()) await row.click({ timeout: 5_000 });
    } else if (state === 'home') {
      await hash('#/home');
    } else if (state === 'git') {
      await hash(`#/git/${env.repoId}/graph`);
    } else if (state === 'db') {
      await hash(env.connId ? `#/database/${env.connId}` : '#/database');
    } else if (state === 'idle-agents') {
      await hash('#/agents');
    }
    await sleep(1500);
    await page.screenshot({ path: path.join(OUT, `shot-${env.label}-${state}.png`) }).catch(() => {});
  } catch (e) {
    log(`uiState ${state} failed: ${String(e).split('\n')[0]}`);
  }
}

// ── sampling ────────────────────────────────────────────────────────────────
const samplesFile = () => path.join(OUT, 'samples.jsonl');
let prevCpu = null; // Map pid -> cpuS
let prevT = null;
let prevCdp = null;
async function sample(B, labels) {
  const t = Date.now();
  const rows = psTable();
  const groups = classify(rows);
  const dt = prevT ? (t - prevT) / 1000 : null;
  const cpuNow = new Map([...rows.values()].map((r) => [r.pid, r.cpuS]));
  const out = { t: +((t - T0) / 1000).toFixed(1), ...labels, machine: machine(), groups: {} };
  for (const [g, G] of Object.entries(groups)) {
    let d = 0;
    if (prevCpu && dt) for (const p of G.pids) d += cpuNow.get(p) - (prevCpu.get(p) ?? cpuNow.get(p));
    out.groups[g] = {
      n: G.n,
      rssMB: +(G.rssKB / 1024).toFixed(1),
      maxRssMB: +(G.maxRssKB / 1024).toFixed(1),
      cpuPct: dt ? +((d / dt) * 100).toFixed(1) : null,
    };
  }
  // threads for the big single processes
  if (groups.ottod) {
    out.groups.ottod.threads = threads(groups.ottod.pids[0]);
    try {
      out.groups.ottod.fds = execFileSync('lsof', ['-n', '-P', '-p', String(groups.ottod.pids[0])], { encoding: 'utf8', maxBuffer: 64 << 20 }).split('\n').length - 2;
    } catch {}
  }
  if (groups.clickhouse) out.groups.clickhouse.threads = groups.clickhouse.pids.map(threads).reduce((a, b) => a + (b ?? 0), 0);
  if (groups.renderer) {
    const big = groups.renderer.pids.map((p) => rows.get(p)).sort((a, b) => b.rssKB - a.rssKB)[0];
    out.groups.renderer.threads = threads(big.pid);
  }
  prevCpu = cpuNow;
  prevT = t;
  // CDP + in-page probes
  if (B) {
    try {
      const m = Object.fromEntries((await B.cdp.send('Performance.getMetrics')).metrics.map((x) => [x.name, x.value]));
      const probe = await B.page.evaluate(() => {
        const w = window;
        const r = JSON.parse(JSON.stringify(w.__otto_lt));
        w.__otto_lt.lt = { n: 0, total: 0, max: 0 };
        w.__otto_lt.fr = { n: 0, jank: 0, max: 0 };
        for (const k of Object.keys(w.__otto_lt.ws)) Object.assign(w.__otto_lt.ws[k], { msgs: 0, bytes: 0 });
        const res = performance.getEntriesByType('resource');
        performance.clearResourceTimings();
        const req = {};
        for (const e of res) {
          try {
            const u = new URL(e.name);
            if (!u.pathname.startsWith('/api/')) continue;
            const k = u.pathname.replace(/\/(ses|ws|rep|wsp|usr|tok|[a-z]{2,4})_[0-9A-Za-z]+/g, '/$1_:id').replace(/\/[0-9a-f-]{20,}/g, '/:id');
            req[k] = (req[k] ?? 0) + 1;
          } catch {}
        }
        return { ...r, req, dom: document.getElementsByTagName('*').length, mem: performance.memory ? { used: performance.memory.usedJSHeapSize, total: performance.memory.totalJSHeapSize } : null };
      });
      const d = (k) => (prevCdp && dt ? +((m[k] - prevCdp[k]) / dt).toFixed(3) : null);
      out.page = {
        jsHeapMB: +(m.JSHeapUsedSize / 2 ** 20).toFixed(1),
        jsHeapTotalMB: +(m.JSHeapTotalSize / 2 ** 20).toFixed(1),
        nodes: m.Nodes,
        domNow: probe.dom,
        listeners: m.JSEventListeners,
        layoutsPerS: d('LayoutCount'),
        styleRecalcsPerS: d('RecalcStyleCount'),
        layoutMsPerS: prevCdp && dt ? +(((m.LayoutDuration - prevCdp.LayoutDuration) / dt) * 1000).toFixed(1) : null,
        styleMsPerS: prevCdp && dt ? +(((m.RecalcStyleDuration - prevCdp.RecalcStyleDuration) / dt) * 1000).toFixed(1) : null,
        scriptMsPerS: prevCdp && dt ? +(((m.ScriptDuration - prevCdp.ScriptDuration) / dt) * 1000).toFixed(1) : null,
        taskMsPerS: prevCdp && dt ? +(((m.TaskDuration - prevCdp.TaskDuration) / dt) * 1000).toFixed(1) : null,
        longTasks: probe.lt,
        frames: probe.fr,
        ws: probe.ws,
        req: probe.req,
      };
      prevCdp = m;
    } catch (e) {
      out.pageErr = String(e).slice(0, 200);
    }
  }
  fs.appendFileSync(samplesFile(), JSON.stringify(out) + '\n');
  return out;
}
async function holdPhase(B, seconds, labels) {
  const end = Date.now() + seconds * 1000;
  let last;
  while (Date.now() < end) {
    safetyCheck(`${labels.phase}/${labels.state}`);
    last = await sample(B, labels);
    const g = last.groups;
    log(
      `${labels.phase} N=${labels.n} ${labels.state}: ottod ${g.ottod?.cpuPct}%/${g.ottod?.rssMB}MB ch ${g.clickhouse?.cpuPct}%/${g.clickhouse?.rssMB}MB ` +
        `agents ${g.agents?.n ?? 0}x ${g.agents?.cpuPct ?? 0}% rend ${g.renderer?.cpuPct}%/${g.renderer?.rssMB}MB heap ${last.page?.jsHeapMB}MB nodes ${last.page?.nodes} dom ${last.page?.domNow} lay/s ${last.page?.layoutsPerS} load ${last.machine.load1} avail ${last.machine.freeGB}G pgfree ${last.machine.pagesFreeGB}G swap ${last.machine.swapMB}M`,
    );
    const left = end - Date.now();
    await sleep(Math.min(SAMPLE_S * 1000, Math.max(0, left)));
  }
}
async function forceGc(B) {
  try {
    await B.cdp.send('HeapProfiler.collectGarbage');
    await B.cdp.send('HeapProfiler.collectGarbage');
  } catch {}
}
async function stackSample(pid, name) {
  // Only OUR test processes. 5 s wall, 1 ms interval.
  if (!STACKS || !pid) return;
  try {
    spawnSync('sample', [String(pid), '5', '-file', path.join(OUT, `sample-${name}.txt`)], { timeout: 30_000 });
  } catch {}
}
async function jsProfile(B, seconds, name) {
  try {
    await B.cdp.send('Profiler.enable');
    await B.cdp.send('Profiler.setSamplingInterval', { interval: 500 });
    await B.cdp.send('Profiler.start');
    await sleep(seconds * 1000);
    const { profile } = await B.cdp.send('Profiler.stop');
    const self = new Map();
    const byId = new Map(profile.nodes.map((n) => [n.id, n]));
    const counts = new Map();
    for (const s of profile.samples) counts.set(s, (counts.get(s) ?? 0) + 1);
    const total = profile.samples.length;
    for (const [id, c] of counts) {
      const n = byId.get(id);
      const f = n.callFrame;
      const k = `${f.functionName || '(anon)'} ${f.url.split('/').pop()}:${f.lineNumber + 1}:${f.columnNumber + 1}`;
      self.set(k, (self.get(k) ?? 0) + c);
    }
    const top = [...self.entries()].sort((a, b) => b[1] - a[1]).slice(0, 40).map(([k, c]) => `${((c / total) * 100).toFixed(1)}%  ${k}`);
    fs.writeFileSync(path.join(OUT, `jsprofile-${name}.txt`), `samples=${total} interval=0.5ms\n` + top.join('\n') + '\n');
  } catch (e) {
    log(`jsProfile failed: ${e}`);
  }
}

// ── sessions ────────────────────────────────────────────────────────────────
const sessions = [];
async function scaleTo(env, n) {
  while (sessions.length < n) {
    const i = sessions.length + 1;
    const s = await api('POST', `/workspaces/${env.wsId}/sessions`, { kind: 'agent', provider: 'claude', title: `load-${i}`, cwd: env.repoDir });
    sessions.push(s.id);
  }
  await sleep(3000);
}
async function closeAll() {
  for (const id of sessions.splice(0)) {
    try {
      await api('DELETE', `/sessions/${id}`);
    } catch (e) {
      log(`delete ${id}: ${e}`);
    }
  }
}

// Proves the transcript half of the load is live: the daemon resolves the
// emulator's JSONL for a session and folds turns from it.
async function verifyTranscripts(n) {
  try {
    const t = await api('GET', `/sessions/${sessions[0]}/transcript?limit=5`);
    let bytes = 0;
    const root = path.join(ctx.root, 'home/.claude/projects');
    for (const d of fs.readdirSync(root)) for (const f of fs.readdirSync(path.join(root, d))) bytes += fs.statSync(path.join(root, d, f)).size;
    log(`transcripts N=${n}: first session turns=${t?.turns?.length} total_turns=${t?.stats?.turns} unavailable=${t?.unavailable_reason ?? '-'}; on-disk JSONL total ${(bytes / 2 ** 20).toFixed(1)} MB`);
  } catch (e) {
    log(`verifyTranscripts: ${e}`);
  }
}

// ── teardown ────────────────────────────────────────────────────────────────
async function teardown(B) {
  log('teardown');
  try {
    await B?.browser?.close();
  } catch {}
  const rows = psTable();
  const mine = new Set();
  if (ctx.daemonPid) {
    mine.add(ctx.daemonPid);
    for (const p of descendants(rows, ctx.daemonPid)) mine.add(p);
  }
  for (const r of rows.values()) {
    if (ctx.root && r.cmd.includes(ctx.root)) mine.add(r.pid);
    if (r.cmd.includes(path.join(HERE, 'fake-claude.mjs'))) mine.add(r.pid);
  }
  if (ctx.browserPid) {
    mine.add(ctx.browserPid);
    for (const p of descendants(rows, ctx.browserPid)) mine.add(p);
  }
  mine.delete(process.pid);
  try {
    if (ctx.daemonPid) process.kill(ctx.daemonPid, 'SIGTERM');
  } catch {}
  await sleep(4000);
  for (const p of mine) {
    try {
      process.kill(p, 'SIGKILL');
    } catch {}
  }
  await sleep(500);
  const left = [...psTable().values()].filter((r) => mine.has(r.pid) || (ctx.root && r.cmd.includes(ctx.root)));
  log(`teardown: killed ${mine.size} pids; leftovers=${left.length}`);
  if (ctx.root && !A['keep-root']) fs.rmSync(ctx.root, { recursive: true, force: true });
}

// ── main ────────────────────────────────────────────────────────────────────
async function main() {
  log(`mode=${MODE} steps=${STEPS} hold=${HOLD}s out=${OUT} ottod=${OTTOD}`);
  safetyCheck('start');
  let B = null;
  let aborted = null;
  try {
    const env = await startDaemon();
    B = await startBrowser(env.wsId);
    // warm-up: let boot-time scans settle, then baseline (0 agents)
    await sleep(10_000);
    await uiState(B, 'tiled', { ...env, label: 'n0' });
    await sample(B, { phase: 'warm', n: 0, state: 'tiled' });
    await holdPhase(B, BASELINE, { phase: 'baseline', n: 0, state: 'agents-empty' });
    await forceGc(B);
    await sample(B, { phase: 'baseline-gc', n: 0, state: 'agents-empty' });
    if (MODE === 'scale') {
      const states = ['tiled', 'focus', 'home', 'git'];
      for (const n of STEPS) {
        safetyCheck(`before N=${n}`);
        await scaleTo(env, n);
        for (const st of states) {
          await uiState(B, st, { ...env, label: `n${n}` });
          await holdPhase(B, HOLD / states.length, { phase: 'step', n, state: st });
          if (st === 'tiled') {
            await stackSample(ctx.daemonPid, `ottod-n${n}-tiled`);
            await jsProfile(B, 8, `n${n}-tiled`);
          }
        }
        await forceGc(B);
        await sample(B, { phase: 'step-gc', n, state: 'git' });
        await verifyTranscripts(n);
      }
    } else if (MODE === 'leak') {
      // (b) module switching under constant N: Agents(tiled) -> Home -> Git -> DB -> Agents(focus),
      // then a GC'd Home checkpoint every cycle: heap / DOM / listeners must not ratchet.
      await scaleTo(env, LEAK_N);
      const cycle = ['tiled', 'home', 'git', 'db', 'focus'];
      const per = Number(A['state-s'] ?? 20);
      const end = Date.now() + LEAK_MIN * 60_000;
      let c = 0;
      while (Date.now() < end) {
        c++;
        for (const st of cycle) {
          await uiState(B, st, { ...env, label: `leak` });
          await holdPhase(B, per, { phase: 'leak', n: LEAK_N, state: st, cycle: c });
        }
        await uiState(B, 'home', { ...env, label: 'leak-ckpt' });
        await sleep(3000);
        await forceGc(B);
        await sample(B, { phase: 'leak-ckpt', n: LEAK_N, state: 'home-gc', cycle: c });
      }
    } else if (MODE === 'churn') {
      await scaleTo(env, LEAK_N);
      await uiState(B, 'tiled', { ...env, label: 'churn-p1' });
      await holdPhase(B, 45, { phase: 'p1-main-tiled', n: LEAK_N, state: 'tiled' });
      // (c) a pop-out window (?popout=1 + __OTTO_WIN__, what the Tauri shell injects) on session 1
      const pop = await B.context.newPage();
      await pop.addInitScript(() => {
        window.__OTTO_WIN__ = 'popout-loadtest';
        window.__OTTO_POPOUT__ = { title: 'load-1' };
      });
      await pop.goto(`${UI}/?popout=1#/agents/${sessions[0]}`);
      await sleep(3000);
      await pop.screenshot({ path: path.join(OUT, 'shot-popout.png') }).catch(() => {});
      await holdPhase(B, 60, { phase: 'p2-main-tiled+popout', n: LEAK_N, state: 'tiled+popout' });
      try {
        const pc = await B.context.newCDPSession(pop);
        const m = Object.fromEntries((await pc.send('Performance.getMetrics')).metrics.map((x) => [x.name, x.value]));
        log(`popout page: heap ${(m.JSHeapUsedSize / 2 ** 20).toFixed(1)}MB nodes ${m.Nodes} listeners ${m.JSEventListeners}`);
      } catch {}
      await pop.close();
      await holdPhase(B, 15, { phase: 'p2b-popout-closed', n: LEAK_N, state: 'tiled' });
      // (a1) view churn: open each session's view, then leave; repeat
      await uiState(B, 'home', { ...env, label: 'churn-pre' });
      await sleep(3000);
      await forceGc(B);
      await holdPhase(B, 20, { phase: 'p3-pre-home', n: LEAK_N, state: 'home' });
      await forceGc(B);
      await sample(B, { phase: 'p3-pre-home-gc', n: LEAK_N, state: 'home-gc' });
      const churnEnd = Date.now() + 90_000;
      let opens = 0;
      await B.page.evaluate(() => (location.hash = '#/agents'));
      await B.page.getByRole('button', { name: 'Tabbed view', exact: true }).click({ timeout: 10_000 }).catch(() => {});
      while (Date.now() < churnEnd) {
        safetyCheck('view-churn');
        for (let i = 1; i <= LEAK_N; i++) {
          await B.page.evaluate(() => (location.hash = '#/agents'));
          const row = B.page.locator('.navigator .nested-item:not(.archived)', { hasText: `load-${i}` }).first();
          await row.click({ timeout: 5_000 }).catch(() => {});
          opens++;
          await sleep(1200);
          await B.page.evaluate(() => (location.hash = '#/home'));
          await sleep(800);
        }
        await sample(B, { phase: 'p3-view-churn', n: LEAK_N, state: 'churn', opens });
      }
      log(`view churn: ${opens} session-view opens`);
      await uiState(B, 'home', { ...env, label: 'churn-post' });
      await holdPhase(B, 20, { phase: 'p3-post-home', n: LEAK_N, state: 'home' });
      await forceGc(B);
      await sample(B, { phase: 'p3-post-home-gc', n: LEAK_N, state: 'home-gc' });
      // (a2) session churn: create -> 5 s -> DELETE, and create -> kill -> DELETE
      const agentCount = () => classify(psTable()).agents?.n ?? 0;
      log(`session churn start: emulators=${agentCount()}`);
      for (let k = 1; k <= 8; k++) {
        safetyCheck('session-churn');
        const s1 = await api('POST', `/workspaces/${env.wsId}/sessions`, { kind: 'agent', provider: 'claude', title: `churn-${k}`, cwd: env.repoDir });
        await sleep(4000);
        const during = agentCount();
        if (k % 2) await api('DELETE', `/sessions/${s1.id}`);
        else {
          await api('POST', `/sessions/${s1.id}/kill`);
          await sleep(1000);
          await api('DELETE', `/sessions/${s1.id}`);
        }
        await sleep(2000);
        const r = await sample(B, { phase: 'p4-session-churn', n: LEAK_N, state: 'home', cycle: k });
        log(`session churn ${k}: emulators during=${during} after=${agentCount()} ottod ${r.groups.ottod?.rssMB}MB thr ${r.groups.ottod?.threads} fds ${r.groups.ottod?.fds}`);
      }
      await holdPhase(B, 20, { phase: 'p4-post', n: LEAK_N, state: 'home' });
    } else if (MODE === 'suspend') {
      // r3-05-01 A/B on a live daemon: quiet agents (run with --quiet-agents), grace 60 s.
      await api('PUT', '/settings', { idle_suspend_grace_secs: 60 });
      const man = [];
      const del = [];
      for (let i = 1; i <= 2; i++) {
        const s = await api('POST', `/workspaces/${env.wsId}/sessions`, { kind: 'agent', provider: 'claude', title: `load-${i}`, cwd: env.repoDir });
        man.push(s.id);
        sessions.push(s.id);
      }
      for (let i = 1; i <= 2; i++) {
        const r = await api('POST', `/workspaces/${env.wsId}/sessions/open`, { provider: 'claude', title: `deleg-${i}`, cwd: env.repoDir });
        del.push(r.session.id);
        sessions.push(r.session.id);
      }
      await sleep(5000);
      const agentCount = () => classify(psTable()).agents?.n ?? 0;
      const status = async () => {
        const all = await api('GET', `/workspaces/${env.wsId}/sessions?archived=false`);
        return Object.fromEntries(all.filter((x) => sessions.includes(x.id)).map((x) => [x.title, `${x.status}${x.live ? '/live' : ''}`]));
      };
      log(`suspend A/B: created; emulators=${agentCount()} ${JSON.stringify(await status())}`);
      for (const id of man) await api('POST', `/sessions/${id}/kill`);
      await sleep(3000);
      log(`manual sessions killed (reconnectable); emulators=${agentCount()} ${JSON.stringify(await status())}`);
      // user opens each manual session in the UI -> terminal attach -> ensure_live resumes it
      await B.page.evaluate(() => (location.hash = '#/agents'));
      await B.page.getByRole('button', { name: 'Tabbed view', exact: true }).click({ timeout: 10_000 }).catch(() => {});
      for (let i = 1; i <= 2; i++) {
        await B.page.locator('.navigator .nested-item:not(.archived)', { hasText: `load-${i}` }).first().click({ timeout: 5_000 }).catch((e) => log(`open load-${i}: ${e}`));
        await sleep(6000);
      }
      await B.page.screenshot({ path: path.join(OUT, 'shot-suspend-opened.png') }).catch(() => {});
      log(`opened in UI; emulators=${agentCount()} ${JSON.stringify(await status())}`);
      await uiState(B, 'home', { ...env, label: 'suspend' });
      const endS = Date.now() + Number(A['watch-s'] ?? 240) * 1000;
      while (Date.now() < endS) {
        safetyCheck('suspend-watch');
        await sample(B, { phase: 'suspend-watch', n: agentCount(), state: 'home' });
        log(`watch: emulators=${agentCount()} ${JSON.stringify(await status())}`);
        await sleep(15_000);
      }
      const holds = fs.readFileSync(path.join(OUT, 'ottod.log'), 'utf8').split('\n').filter((l) => /suspend|hold|origin=manual/i.test(l)).slice(-15);
      fs.writeFileSync(path.join(OUT, 'suspend-log-lines.txt'), holds.join('\n'));
    }
    // recovery: close every session, go to Home, GC, watch memory come back
    await closeAll();
    await uiState(B, 'tiled', { ...env, label: 'closed' });
    await holdPhase(B, RECOVER / 2, { phase: 'recover', n: 0, state: 'tiled-closed' });
    await uiState(B, 'home', { ...env, label: 'closed' });
    await holdPhase(B, RECOVER / 2, { phase: 'recover', n: 0, state: 'home-closed' });
    await forceGc(B);
    await sample(B, { phase: 'recover-gc', n: 0, state: 'home-closed' });
    fs.writeFileSync(path.join(OUT, 'page-errors.json'), JSON.stringify(B.errors, null, 2));
  } catch (e) {
    aborted = String(e);
    log(`ABORT: ${e.stack ?? e}`);
  } finally {
    await teardown(B);
  }
  log(aborted ? `done (aborted: ${aborted})` : 'done');
  process.exitCode = aborted ? 1 : 0;
}
for (const sig of ['SIGINT', 'SIGTERM']) process.on(sig, async () => {
  log(`signal ${sig}`);
  await teardown(null);
  process.exit(130);
});
main();
