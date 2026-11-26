// One isolated ottod for the workflow harness scripts (load-l1, upgrade-check):
// an env built from scratch (never the caller's), a temporary HOME and data
// dir under `root`, a port other than 7700, and a PATH whose claude / codex
// are the fake agents. Never the owner's daemon, data or app bundle.
import { spawn, execFileSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

export const sh = (cmd, a, opts = {}) => execFileSync(cmd, a, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'], ...opts });
export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
export const args = () =>
  Object.fromEntries(process.argv.slice(2).reduce((acc, a, i, all) => (a.startsWith('--') ? [...acc, [a.slice(2), all[i + 1]?.startsWith('--') || all[i + 1] === undefined ? 'true' : all[i + 1]]] : acc), []));

/** A fresh root with home, data, fake-agent bin, fake scripts/review/state dirs and a repo
 *  with an origin: `base` (= origin/main) and `feature`, one commit ahead (also local main). */
export function isolatedRoot(fakesDir) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'otto-wf-harness-'));
  const d = Object.fromEntries(['home', 'data', 'bin', 'scripts', 'review', 'state', 'tmp', 'ws/repo'].map((n) => [n, path.join(root, n)]));
  Object.values(d).forEach((p) => fs.mkdirSync(p, { recursive: true }));
  for (const [cli, fake, word] of [['claude', 'otto-fake-claude', 'stream-json'], ['codex', 'otto-fake-codex', 'app-server']]) {
    const bin = path.join(fakesDir, fake);
    if (!fs.existsSync(bin)) throw new Error('missing ' + bin + ' (cargo build --release -p otto-agent-protocol --bins)');
    fs.writeFileSync(path.join(d.bin, cli), '#!/bin/sh\nfor a in "$@"; do [ "$a" = ' + word + " ] && exec '" + bin + "' \"$@\"; done\necho 'the real CLI is disabled'\nexec cat\n", { mode: 0o755 });
  }
  const git = (...a) => sh('git', ['-C', d['ws/repo'], '-c', 'user.name=Harness', '-c', 'user.email=harness@example.invalid', ...a]);
  git('init', '-q', '-b', 'main');
  fs.writeFileSync(path.join(d['ws/repo'], 'README.md'), 'harness\n');
  // main is one commit ahead of `base`: a run's worktree (main) has a diff to review.
  // origin/main stays at the base commit; local main gets one more commit.
  git('add', '.'), git('commit', '-qm', 'base'), git('branch', 'base');
  sh('git', ['clone', '-q', '--bare', d['ws/repo'], path.join(root, 'origin.git')]);
  git('remote', 'add', 'origin', path.join(root, 'origin.git')), git('fetch', '-q', 'origin');
  fs.writeFileSync(path.join(d['ws/repo'], 'lib.rs'), 'pub fn add(a: i32, b: i32) -> i32 { a - b }\n');
  git('add', '.'), git('commit', '-qm', 'feature'), git('branch', 'feature'), git('push', '-q', 'origin', 'feature');
  return { root, d };
}

/** Boot `ottod` on `root`; resolves once /health answers. */
export async function boot(ottod, { root, d }, port, extra = {}, logFile) {
  if (Number(port) === 7700) throw new Error('never the owner daemon port 7700');
  try { sh('/usr/sbin/lsof', ['-iTCP:' + port, '-sTCP:LISTEN']); throw new Error('port ' + port + ' is in use'); } catch (e) { if (String(e.message).startsWith('port')) throw e; }
  const env = {
    HOME: d.home, TMPDIR: d.tmp, PATH: d.bin + ':/usr/bin:/bin:/usr/sbin:/sbin', LANG: 'en_US.UTF-8',
    OTTO_DATA_DIR: d.data, OTTO_PORT: String(port), OTTO_E2E: '1', OTTO_SELF_IMPROVE: '0', OTTO_CLI_UPDATE: '0',
    OTTO_TAILSCALE_CLI: path.join(d.data, 'no-tailscale'), OTTO_SECRETS: 'file', OTTO_SECRETS_ALLOW_PLAINTEXT: '1',
    OTTO_PLUGINS_HOME: path.join(root, 'plugins'), OTTO_SCRATCHES_HOME: path.join(root, 'scratches'),
    OTTO_CONTEXT_ROOT: path.join(root, 'context'), OTTO_IDE_BUILD_CACHE_DIR: path.join(root, 'ide'),
    CLAUDE_BIN: '/nonexistent/otto-harness-no-claude', RUST_LOG: 'info',
    OTTO_FAKE_SCRIPTS: d.scripts, OTTO_FAKE_REVIEW_DIR: d.review, OTTO_FAKE_DEFAULT: 'step', OTTO_FAKE_STATE_DIR: d.state,
    ...extra,
  };
  const fd = fs.openSync(logFile, 'a');
  const child = spawn(ottod, [], { env, cwd: root, stdio: ['ignore', fd, fd], detached: true });
  const base = 'http://127.0.0.1:' + port + '/api/v1';
  for (let i = 0; ; i++) {
    try { if ((await fetch(base + '/health')).ok) break; } catch {}
    if (child.exitCode !== null || i > 240) throw new Error('ottod did not come up (see ' + logFile + ')');
    await sleep(500);
  }
  let token = '';
  const api = async (method, p, body) => {
    const r = await fetch(base + p, { method, headers: { 'Content-Type': 'application/json', Authorization: 'Bearer ' + token }, body: body && JSON.stringify(body) });
    const t = await r.text();
    if (!r.ok) throw Object.assign(new Error(method + ' ' + p + ' ' + r.status + ' ' + t.slice(0, 300)), { status: r.status });
    return t ? JSON.parse(t) : null;
  };
  const login = async () => {
    const first = await fetch(base + '/onboarding/root', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ password: 'otto-harness-password', display_name: 'Harness' }) });
    const res = first.ok ? first : await fetch(base + '/auth/login', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ username: 'root', password: 'otto-harness-password' }) });
    if (!res.ok) throw new Error('login ' + res.status + ' ' + (await res.text()).slice(0, 200));
    token = (await res.json()).token;
  };
  await login();
  /** SIGTERM (a deploy's stop), SIGKILL after 20 s. */
  const stop = async () => {
    try { process.kill(-child.pid, 'SIGTERM'); } catch {}
    for (let i = 0; i < 40 && child.exitCode === null && child.signalCode === null; i++) await sleep(500);
    try { process.kill(-child.pid, 'SIGKILL'); } catch {}
  };
  return { child, api, stop, base };
}

/** Kill whatever still runs under `root` and remove it. */
export function cleanup(root, ...pids) {
  for (const p of pids) try { process.kill(-p, 'SIGKILL'); } catch {}
  for (const l of sh('ps', ['-axo', 'pid=,command=']).split('\n')) {
    if (l.includes(root)) try { process.kill(Number(l.trim().split(' ')[0]), 'SIGKILL'); } catch {}
  }
  fs.rmSync(root, { recursive: true, force: true });
}

export const sqlite = (db, q) => sh('sqlite3', ['-readonly', '-cmd', '.timeout 2000', db, q]).trim();
