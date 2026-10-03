#!/usr/bin/env node
// Daemon perf budget (perf/03 F8): boots an ISOLATED ottod (temp data dir,
// throwaway port, plaintext secrets in that dir, no agents, no CLI updates),
// then asserts budgets on
//   - boot_ms        spawn → first 200 from /api/v1/health
//   - idle_cpu_pct   CPU time / wall time over an idle window
//   - idle_wakeups   voluntary context switches per second, all threads
//                    (Linux /proc; a proxy for timer wakeups — skipped elsewhere)
//   - threads        OS threads after the idle window
//   - wal_bytes      otto.db-wal size after the run (journal_size_limit cap)
//   - free_pct       freelist share of otto.db (read from the file header)
//   - boot phases    the daemon's `boot: ready in N ms (db_compact=… …)` log
//                    line (perf2/03 N4): per-phase wall time
//   - ch_threads / ch_cpu_pct   the embedded ClickHouse child, when a
//                    `clickhouse` binary is on PATH (macOS job)
// and exits non-zero when a BLOCKING budget is over. Blocking = deterministic
// (threads, wal_bytes, free_pct, the compaction outcome); time/CPU budgets on
// shared runners are reported only unless OTTO_PERF_STRICT_TIMING=1.
//
// Seeded DB (perf2/03 N4, default on; OTTO_PERF_SEED=0 skips): a first boot
// creates the schema, the script then fragments otto.db (≈130 MB inserted,
// 70 % deleted → ≈90 MB on the freelist, auto_vacuum=0) and boots again, so
// the measured boot runs the offline compaction and free_pct / wal_bytes are
// measured on a real-sized file instead of an empty one.
// Never touches port 7700 or the real data dir. Usage:
//   node scripts/perf/daemon-budget.mjs <path/to/ottod>
// Env: OTTO_PERF_BUDGET_SCALE (multiplies time/CPU budgets; CI uses a debug
// build on shared runners), OTTO_PERF_IDLE_SECS (default 20),
// OTTO_PERF_SETTLE_SECS (default 30), OTTO_PERF_PORT (default 7893),
// OTTO_PERF_SEED (default 1), OTTO_PERF_STRICT_TIMING (default 0),
// OTTO_PERF_BLOCK_CH (default 0; 1 = ch_threads blocks).
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, readFileSync, existsSync, statSync, rmSync, readdirSync, openSync, readSync, closeSync, appendFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const bin = process.argv[2];
if (!bin || !existsSync(bin)) {
  console.error(`usage: daemon-budget.mjs <ottod>  (not found: ${bin ?? '—'})`);
  process.exit(2);
}
const SCALE = Number(process.env.OTTO_PERF_BUDGET_SCALE ?? '1') || 1;
const IDLE_SECS = Number(process.env.OTTO_PERF_IDLE_SECS ?? '20') || 20;
// Boot-time background work (first retention/catalog passes) burns CPU for
// ~20 s on a debug build; measured locally: ~60 % at +5 s, 0.4 % from +20 s.
const SETTLE_SECS = Number(process.env.OTTO_PERF_SETTLE_SECS ?? '30') || 30;
const PORT = process.env.OTTO_PERF_PORT ?? '7893';
if (PORT === '7700') {
  console.error('refusing to use the live daemon port 7700');
  process.exit(2);
}
const API = `http://127.0.0.1:${PORT}/api/v1`;

const SEED = (process.env.OTTO_PERF_SEED ?? '1') !== '0';
const STRICT_TIMING = process.env.OTTO_PERF_STRICT_TIMING === '1';

// Budgets: time/CPU scale with OTTO_PERF_BUDGET_SCALE, sizes don't.
const BUDGET = {
  boot_ms: 2_000 * SCALE,
  // Offline compaction of the seeded file (167 MB, 69 % free → 52 MB): 363 ms
  // locally on 2026-10-03, debug build; runners are slower.
  boot_db_compact_ms: 3_000 * SCALE,
  boot_db_open_ms: 1_000 * SCALE,
  idle_cpu_pct: 1.0 * SCALE,
  idle_wakeups_per_s: 50 * SCALE,
  threads: 96,
  wal_bytes: 64 * 1024 * 1024,
  free_pct: 20,
  // Embedded ClickHouse (only measured when a binary is on PATH). Measured
  // 69 threads / 0.5 % CPU on 2026-10-03 (M-series, fresh dir, debug ottod);
  // the trim target is < 60 — ratchet this down as the pools shrink.
  ch_threads: 80,
  ch_cpu_pct: 0.5 * SCALE,
};
/** Deterministic metrics: over budget fails the run. */
const BLOCKING = new Set(['threads', 'wal_bytes', 'free_pct']);
if (STRICT_TIMING) for (const k of Object.keys(BUDGET)) BLOCKING.add(k);
// The macOS nightly (ClickHouse on PATH) also blocks on the CH thread count.
if (process.env.OTTO_PERF_BLOCK_CH === '1') BLOCKING.add('ch_threads');

const dataDir = mkdtempSync(join(tmpdir(), 'otto-perf-'));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function cpuSeconds(pid) {
  if (existsSync(`/proc/${pid}/stat`)) {
    // Fields after the ")" : utime is #14, stime #15 (1-based) in clock ticks.
    const s = readFileSync(`/proc/${pid}/stat`, 'utf8');
    const f = s.slice(s.lastIndexOf(')') + 2).split(' ');
    const ticks = Number(execFileSync('getconf', ['CLK_TCK']).toString().trim()) || 100;
    return (Number(f[11]) + Number(f[12])) / ticks;
  }
  // macOS / BSD: cumulative CPU time as [[dd-]hh:]mm:ss(.cc).
  const t = execFileSync('ps', ['-o', 'time=', '-p', String(pid)]).toString().trim();
  const [rest, frac = '0'] = t.split('.');
  const parts = rest.split(/[-:]/).map(Number).reverse();
  const mult = [1, 60, 3600, 86400];
  return parts.reduce((a, v, i) => a + v * mult[i], 0) + Number(`0.${frac}`);
}

function voluntarySwitches(pid) {
  const dir = `/proc/${pid}/task`;
  if (!existsSync(dir)) return null;
  let n = 0;
  for (const t of readdirSync(dir)) {
    try {
      const m = readFileSync(`${dir}/${t}/status`, 'utf8').match(/^voluntary_ctxt_switches:\s+(\d+)/m);
      if (m) n += Number(m[1]);
    } catch {
      /* thread exited */
    }
  }
  return n;
}

function threadCount(pid) {
  if (existsSync(`/proc/${pid}/status`)) {
    const m = readFileSync(`/proc/${pid}/status`, 'utf8').match(/^Threads:\s+(\d+)/m);
    return m ? Number(m[1]) : null;
  }
  try {
    // `ps -M` prints one line per thread plus a header.
    return execFileSync('ps', ['-M', '-p', String(pid)]).toString().trim().split('\n').length - 1;
  } catch {
    return null;
  }
}

/** page_size / page_count / freelist_count from the SQLite file header. */
function dbHeader(path) {
  const fd = openSync(path, 'r');
  const b = Buffer.alloc(100);
  readSync(fd, b, 0, 100, 0);
  closeSync(fd);
  let pageSize = b.readUInt16BE(16);
  if (pageSize === 1) pageSize = 65536;
  return { pageSize, pageCount: b.readUInt32BE(28), freelist: b.readUInt32BE(36) };
}

const results = {};
const fail = [];
const warn = [];
const daemonEnv = {
    ...process.env,
    OTTO_DATA_DIR: dataDir,
    OTTO_PORT: PORT,
    OTTO_SELF_IMPROVE: '0',
    OTTO_CLI_UPDATE: '0',
    OTTO_E2E: '1',
    CLAUDE_BIN: '/nonexistent/otto-perf-no-claude',
    OTTO_PLUGINS_HOME: join(dataDir, 'plugins-home'),
    OTTO_SECRETS: 'file',
    OTTO_SECRETS_ALLOW_PLAINTEXT: '1',
    // `ottod=info` for the `boot: ready …` / `db maintenance: …` lines.
    RUST_LOG: process.env.RUST_LOG ?? 'warn,ottod=info',
};

/** Spawn ottod with stderr captured (and echoed for warn+ lines). */
function startDaemon() {
  const c = spawn(bin, [], { env: daemonEnv, stdio: ['ignore', 'ignore', 'pipe'] });
  c.log = '';
  c.stderr.setEncoding('utf8');
  c.stderr.on('data', (d) => {
    c.log += d;
    for (const line of d.split('\n')) if (/\b(WARN|ERROR)\b/.test(line)) process.stderr.write(`${line}\n`);
  });
  return c;
}

async function waitHealthy(c) {
  const t0 = performance.now();
  while (performance.now() - t0 < 60_000) {
    if (c.exitCode !== null) throw new Error(`ottod exited early (code ${c.exitCode})`);
    try {
      const r = await fetch(`${API}/health`, { signal: AbortSignal.timeout(1_000) });
      if (r.ok) return Math.round(performance.now() - t0);
    } catch {
      /* not listening yet */
    }
    await sleep(25);
  }
  throw new Error('ottod never became healthy within 60 s');
}

async function stopDaemon(c) {
  c.kill('SIGTERM');
  for (let i = 0; i < 100 && c.exitCode === null && c.signalCode === null; i++) await sleep(100);
  if (c.exitCode === null && c.signalCode === null) c.kill('SIGKILL');
}

/** Fragment otto.db: ≈130 MB in, 70 % out → ≈90 MB free, auto_vacuum=0. */
async function seedDb(path) {
  const { DatabaseSync } = await import('node:sqlite');
  const db = new DatabaseSync(path);
  db.exec('PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS perf_seed (id INTEGER PRIMARY KEY, k TEXT, b BLOB)');
  const ins = db.prepare('INSERT INTO perf_seed (k, b) VALUES (?, randomblob(1024))');
  db.exec('BEGIN');
  for (let i = 0; i < 120_000; i++) ins.run(`k${i % 97}`);
  db.exec('COMMIT');
  // A contiguous range, so whole pages land on the freelist (a scattered
  // delete would leave every page partly full and nothing free).
  db.exec('DELETE FROM perf_seed WHERE id <= 84000; PRAGMA wal_checkpoint(TRUNCATE);');
  db.close();
  const h = dbHeader(path);
  results.seed_free_pct = Number(((h.freelist / h.pageCount) * 100).toFixed(1));
  results.seed_db_bytes = h.pageSize * h.pageCount;
}

/** The embedded ClickHouse child of `pid`, if one is running. */
function clickhousePid(pid) {
  try {
    const out = execFileSync('pgrep', ['-P', String(pid), '-f', 'clickhouse']).toString().trim();
    return out ? Number(out.split('\n')[0]) : null;
  } catch {
    return null;
  }
}

let child = null;
try {
  if (SEED) {
    const first = startDaemon();
    try {
      await waitHealthy(first);
    } finally {
      await stopDaemon(first);
    }
    await seedDb(join(dataDir, 'otto.db'));
  }
  child = startDaemon();
  results.boot_ms = await waitHealthy(child);
  const m = child.log.match(/boot: ready in (\d+) ms \(([^)]*)\)/);
  if (m) {
    results.boot_logged_ms = Number(m[1]);
    for (const kv of m[2].split(' ')) {
      const [k, v] = kv.split('=');
      if (k && v !== undefined) results[`boot_${k}_ms`] = Number(v);
    }
  } else {
    warn.push('no `boot: ready` line in the daemon log');
  }
  if (SEED) {
    results.offline_compacted = /compacted offline/.test(child.log);
    if (!results.offline_compacted) fail.push('seeded fragmented DB was not compacted offline at boot');
  }

  // Let boot-time background work (retention first passes, sweeps) settle.
  await sleep(SETTLE_SECS * 1_000);
  const c0 = cpuSeconds(child.pid);
  const v0 = voluntarySwitches(child.pid);
  const w0 = performance.now();
  await sleep(IDLE_SECS * 1_000);
  const wall = (performance.now() - w0) / 1_000;
  results.idle_cpu_pct = Number((((cpuSeconds(child.pid) - c0) / wall) * 100).toFixed(2));
  const v1 = voluntarySwitches(child.pid);
  if (v0 !== null && v1 !== null) results.idle_wakeups_per_s = Number(((v1 - v0) / wall).toFixed(1));
  results.threads = threadCount(child.pid);
  const ch = clickhousePid(child.pid);
  if (ch) {
    const c0 = cpuSeconds(ch);
    const w = performance.now();
    await sleep(IDLE_SECS * 1_000);
    results.ch_cpu_pct = Number((((cpuSeconds(ch) - c0) / ((performance.now() - w) / 1_000)) * 100).toFixed(2));
    results.ch_threads = threadCount(ch);
  }
} catch (e) {
  fail.push(String(e instanceof Error ? e.message : e));
} finally {
  if (child) await stopDaemon(child);
}

const db = join(dataDir, 'otto.db');
if (existsSync(db)) {
  const wal = `${db}-wal`;
  results.wal_bytes = existsSync(wal) ? statSync(wal).size : 0;
  const h = dbHeader(db);
  results.free_pct = h.pageCount ? Number(((h.freelist / h.pageCount) * 100).toFixed(1)) : 0;
  results.db_bytes = h.pageSize * h.pageCount;
}

for (const [k, budget] of Object.entries(BUDGET)) {
  const v = results[k];
  if (v === undefined || v === null) continue; // not measurable on this OS
  if (v > budget) (BLOCKING.has(k) ? fail : warn).push(`${k} = ${v} exceeds budget ${budget}`);
}

const report = {
  scale: SCALE,
  idle_secs: IDLE_SECS,
  seeded: SEED,
  strict_timing: STRICT_TIMING,
  results,
  budget: BUDGET,
  blocking: [...BLOCKING],
  ok: fail.length === 0,
  failures: fail,
  warnings: warn,
};
console.log(JSON.stringify(report, null, 2));
if (process.env.GITHUB_STEP_SUMMARY) {
  const rows = Object.entries(BUDGET)
    .map(([k, b]) => `| ${k} | ${results[k] ?? 'n/a'} | ${b} | ${BLOCKING.has(k) ? 'yes' : 'report'} |`)
    .join('\n');
  appendFileSync(
    process.env.GITHUB_STEP_SUMMARY,
    `### Daemon perf budget ${fail.length ? '❌' : '✅'}\n\n| metric | value | budget | blocking |\n|---|---|---|---|\n${rows}\n\n` +
      `${fail.map((f) => `- ❌ ${f}`).join('\n')}\n${warn.map((f) => `- ⚠️ ${f}`).join('\n')}\n`,
  );
}
try {
  rmSync(dataDir, { recursive: true, force: true });
} catch {
  /* best effort */
}
process.exit(fail.length ? 1 : 0);
