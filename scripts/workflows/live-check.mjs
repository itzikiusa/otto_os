#!/usr/bin/env node
// The rationed live check of workflows on the real agent CLIs (test plan §8):
// one scenario on a throwaway repository, a turn budget the harness enforces,
// and the PTY-versus-Threads review comparison.
//
//   node scripts/workflows/live-check.mjs --fakes --ottod <ottod> --out <dir> [--fakes-dir <dir>] [--real-home]
//   node scripts/workflows/live-check.mjs --live --i-am-the-coordinator --ottod <ottod> --out <dir>
//     [--only smoke|pty|compare] [--port 7951]
//
// --fakes runs the same steps on the fake agents (otto-fake-* through shims, a
// temporary HOME): it proves the harness, not the CLIs. --live runs the real
// claude, codex and agy found on the login shell's PATH, on their own logins,
// and is refused without --i-am-the-coordinator: it spends real turns.
//
// Turn budget: live-smoke.json gives every agent step its expected and
// worst-case turns. Turns are counted from the daemon's thread rows (and PTY
// agent sessions) twice a second; the check is aborted — runs canceled, daemon
// stopped, everything cleaned up, report written — as soon as the steps that
// start next could pass the cap (30), or a step spends more than its worst
// case. A failed step is never retried. A provider that is not installed or
// not signed in is left out and reported.
//
// The daemon: a temporary root /private/var/tmp/otto-live-<ts>/ (data dir,
// context bundles, plugins, logs, TMPDIR, the repository and its bare origin),
// port 7951, never 7700, never the installed app's data dir, no launchctl
// (a stub is first on PATH), self-improvement and CLI updates off. --live keeps
// the real HOME (the CLIs' logins live there); what the daemon and the CLIs
// write under it is listed in the report, and the harness removes what names
// the throwaway root: folder-trust grants, the CLIs' transcripts.
//
// Writes <out>/report.md, checks.json, ledger.json, runs/*.json, reviews/*.json
// and the daemon logs. Exit 0: every pass line holds; 1: a pass line failed;
// 2: refused; 3: aborted (budget).
import { execFileSync } from 'node:child_process';
import crypto from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { args as parseArgs, boot, sh, sleep } from './daemon.mjs';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const SPEC = JSON.parse(fs.readFileSync(path.join(HERE, 'live-smoke.json'), 'utf8'));
const a = parseArgs();
const refuse = (msg) => { console.error('live-check: refused — ' + msg); process.exit(2); };
const LIVE = a.live === 'true', FAKES = a.fakes === 'true';
if (LIVE === FAKES) refuse('give exactly one of --fakes or --live');
const PREFLIGHT = LIVE && a.preflight === 'true'; // what --live would use and do; starts nothing
if (LIVE && !PREFLIGHT && a['i-am-the-coordinator'] !== 'true') refuse('--live runs the real agent CLIs on the owner\'s accounts (up to ' + SPEC.cap + ' turns). Only the coordinator starts it, once, with --i-am-the-coordinator. Use --fakes to try the harness.');
if (LIVE && ['test-cap', 'test-fail', 'real-home', 'fakes-dir'].some((k) => a[k])) refuse('--test-cap, --test-fail, --real-home and --fakes-dir are for --fakes only');
const PARTS = ['smoke', 'pty', 'compare'];
const ONLY = a.only ?? 'all';
if (ONLY !== 'all' && !PARTS.includes(ONLY)) refuse('--only takes smoke, pty or compare');
const PORT = Number(a.port ?? 7951);
if (!Number.isInteger(PORT) || PORT === 7700) refuse('never the owner daemon\'s port 7700');
if (!a.out) refuse('--out <dir> is required');
const OUT = path.resolve(a.out);
const OTTOD = path.resolve(a.ottod ?? 'target/debug/ottod');
if (!fs.existsSync(OTTOD)) refuse('no ottod at ' + OTTOD + ' (--ottod <path>)');
const FAKES_DIR = path.resolve(a['fakes-dir'] ?? path.dirname(OTTOD));
const CAP = Number(a['test-cap'] ?? SPEC.cap);
const TEST_FAIL = a['test-fail'];
const REAL_HOME = os.userInfo().homedir;
const OWNER_HOME = LIVE || a['real-home'] === 'true'; // the daemon runs on the real HOME
const SETTLE_S = Number(a['settle-s'] ?? (LIVE ? 60 : 10));
const KEEP = FAKES && a['test-keep'] === 'true'; // leave the root for a look
const portTaken = (p) => { try { return sh('/usr/sbin/lsof', ['-nP', '-iTCP:' + p, '-sTCP:LISTEN']).trim() !== ''; } catch { return false; } };
if (portTaken(PORT)) refuse('port ' + PORT + ' is in use');

const stamp = new Date().toISOString().replace(/[-:]/g, '').replace('T', '-').slice(0, 15);
const ROOT = '/private/var/tmp/otto-live-' + stamp;
if (fs.existsSync(ROOT)) refuse(ROOT + ' already exists');
const d = Object.fromEntries(['home', 'data', 'bin', 'scripts', 'review', 'state', 'tmp', 'logs', 'ws/repo', 'ws/compare-threads', 'ws/compare-pty'].map((n) => [n, path.join(ROOT, n)]));
const DATA_HOME = path.join(REAL_HOME, 'Library/Application Support/Otto');
if (!d.data.startsWith(ROOT + '/') || d.data.startsWith(DATA_HOME)) refuse('the data dir must be under ' + ROOT);
if (!PREFLIGHT) for (const sub of ['runs', 'reviews']) fs.mkdirSync(path.join(OUT, sub), { recursive: true });
const log = (...m) => { const l = new Date().toISOString().slice(11, 19) + ' ' + m.join(' '); console.log(l); fs.appendFileSync(path.join(OUT, 'driver.log'), l + '\n'); };
const save = (name, value) => fs.writeFileSync(path.join(OUT, name), typeof value === 'string' ? value : JSON.stringify(value, null, 1));

class Abort extends Error {}
const checks = [];
const check = (part, name, ok, detail = '') => { checks.push({ part, name, ok: !!ok, detail: String(detail).slice(0, 600) }); log((ok ? 'pass ' : 'FAIL ') + '[' + part + '] ' + name + (detail ? ' — ' + String(detail).slice(0, 240) : '')); return !!ok; };
const notes = []; // reported, never a pass line
const note = (text) => { notes.push(text); log('note ' + text); };

// ── the throwaway root: shims, repository, origin ───────────────────────────
const HOME = OWNER_HOME ? REAL_HOME : d.home;
const exe = (file, body) => fs.writeFileSync(file, '#!/bin/sh\n' + body, { mode: 0o755 });
function loginPath() {
  const shell = process.env.SHELL || '/bin/zsh';
  const out = execFileSync(shell, ['-lic', 'printf "\\n__OTTO_PATH__%s\\n" "$PATH"'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'], env: { HOME: REAL_HOME, USER: os.userInfo().username, SHELL: shell, TERM: 'dumb' } });
  return (out.split('\n').find((l) => l.startsWith('__OTTO_PATH__')) ?? '').slice('__OTTO_PATH__'.length);
}
const onPath = (name, PATH) => PATH.split(':').filter(Boolean).map((dir) => path.join(dir, name)).find((p) => { try { fs.accessSync(p, fs.constants.X_OK); return fs.statSync(p).isFile(); } catch { return false; } });
const SYSTEM_PATH = '/usr/bin:/bin:/usr/sbin:/sbin';
/** The dirs ottod's augment_path prepends when they are missing from PATH (crates/ottod/src/main.rs):
 *  listed after the shims, so none is prepended ahead of them. */
const AUGMENT = ['.local/bin', '.bun/bin', '.claude/local', '.cargo/bin', 'bin', '.opencode/bin', 'go/bin', 'Library/Application Support/Otto/bin'].map((p) => path.join(REAL_HOME, p)).concat('/opt/homebrew/bin', '/usr/local/bin');
let daemonPath = '';
const providers = {}; // name → {available, why}
/** --live: the login shell's PATH and which of the three CLIs it (or the dirs the daemon adds) holds. Runs none of them. */
function findClis() {
  const login = loginPath();
  for (const cli of ['claude', 'codex', 'agy']) {
    const found = onPath(cli, login + ':' + AUGMENT.join(':'));
    providers[cli] = found ? { available: true, why: found } : { available: false, why: 'not installed (not on the login PATH)' };
  }
  return login;
}
if (PREFLIGHT) {
  const login = findClis();
  const plan = Object.entries(SPEC.workflows).map(([k, w]) => k + ': ' + Object.entries(w.turns).map(([n, [e, m]]) => n + ' ' + e + '–' + m).join(', '));
  console.log(['live-check preflight — nothing is started.', 'CLIs: ' + Object.entries(providers).map(([p, v]) => p + ' → ' + v.why).join('; '), 'login PATH: ' + login,
    'turns per step (expected–worst case; a review is its reviewers + 2–3): ' + plan.join(' | ') + '; the comparison runs its review twice; cap ' + SPEC.cap,
    'daemon: ' + OTTOD + ' on port ' + PORT + ', root /private/var/tmp/otto-live-<stamp>/, HOME ' + REAL_HOME].join('\n'));
  process.exit(0);
}
function makeRoot() {
  Object.values(d).forEach((p) => fs.mkdirSync(p, { recursive: true }));
  fs.chmodSync(ROOT, 0o700);
  // The daemon's boot sweep runs launchctl list / remove (ottod post_listen_housekeeping): never the real one.
  exe(path.join(d.bin, 'launchctl'), 'echo "$@" >> ' + JSON.stringify(path.join(d.logs, 'launchctl-calls.log')) + '\nexit 1\n');
  if (LIVE) { daemonPath = d.bin + ':' + findClis() + ':' + SYSTEM_PATH; return; }
  const fake = (name) => { const p = path.join(FAKES_DIR, name); if (!fs.existsSync(p)) refuse('missing ' + p + ' (cargo build -p otto-agent-protocol --bins)'); return p; };
  const shim = (cli, word, bin) => exe(path.join(d.bin, cli), 'for a in "$@"; do case "$a" in ' + word + ") exec '" + bin + "' \"$@\";; esac; done\necho 'the real " + cli + " CLI is disabled'\nexit 1\n");
  shim('claude', 'stream-json', fake('otto-fake-claude'));
  shim('codex', 'app-server', fake('otto-fake-codex'));
  shim('agy', '--acp|acp|stdio', fake('otto-fake-agent'));
  // Every other name the daemon could resolve under the real HOME's tool dirs.
  for (const cli of ['gemini', 'grok', 'opencode', 'muse', 'pi', 'npm', 'npx', 'go']) exe(path.join(d.bin, cli), "echo 'disabled in the live-check rehearsal'\nexit 1\n");
  // The daemon wraps some spawns in $SHELL -lic: a login shell rebuilds PATH (path_helper) with /usr/local/bin
  // and /opt/homebrew/bin — real CLIs — ahead of the shims. This one runs the same script on the daemon's PATH.
  exe(path.join(d.bin, 'login-shell'), 'case "$1" in -*c*) shift;; esac\nexec /bin/sh -c "$@"\n');
  daemonPath = d.bin + ':' + (OWNER_HOME ? AUGMENT : AUGMENT.slice(-2)).join(':') + ':' + SYSTEM_PATH;
  for (const cli of ['claude', 'codex', 'agy']) providers[cli] = { available: true, why: 'fake' };
}
function makeRepo() {
  const repo = d['ws/repo'];
  const git = (...c) => sh('git', ['-C', repo, '-c', 'user.name=Live Check', '-c', 'user.email=live-check@example.invalid', '-c', 'commit.gpgsign=false', ...c]);
  const commit = ({ message, files }) => {
    for (const [name, text] of Object.entries(files)) { fs.mkdirSync(path.dirname(path.join(repo, name)), { recursive: true }); fs.writeFileSync(path.join(repo, name), text); }
    git('add', '.'), git('commit', '-qm', message);
  };
  git('init', '-q', '-b', 'main');
  SPEC.repo.main.forEach(commit);
  sh('git', ['clone', '-q', '--bare', repo, path.join(ROOT, 'origin.git')]);
  git('remote', 'add', 'origin', 'file://' + path.join(ROOT, 'origin.git'));
  for (const b of [SPEC.repo.feature, SPEC.repo.compare]) { git('checkout', '-q', '-b', b.branch, 'main'), commit(b), git('push', '-q', 'origin', b.branch); }
  git('checkout', '-q', 'main'), git('fetch', '-q', 'origin');
  // Each side of the comparison reviews its own clone: a second review of one repository is handed the first one's findings.
  for (const side of ['ws/compare-threads', 'ws/compare-pty']) { fs.rmdirSync(d[side]); sh('git', ['clone', '-q', 'file://' + path.join(ROOT, 'origin.git'), d[side]]); }
  return { tips: Object.fromEntries(['main', 'feature', 'compare'].map((b) => [b, git('rev-parse', b).trim()])) };
}

// ── the owner's HOME: what must not change ──────────────────────────────────
const sha = (buf) => crypto.createHash('sha256').update(buf).digest('hex').slice(0, 16);
const WATCH_FILES = ['.claude/settings.json', '.claude/settings.local.json', '.claude/CLAUDE.md', '.codex/config.toml', '.codex/AGENTS.md', '.gemini/settings.json', '.gemini/trustedFolders.json', '.gemini/GEMINI.md', '.grok/trusted_folders.toml', '.gitconfig'];
const WATCH_TREES = ['.claude/skills', '.codex/skills', '.gemini/skills', '.claude/hooks', '.claude/commands', '.claude/agents', 'Library/LaunchAgents', 'otto-plugins'];
/** Paths the daemon and the CLIs know the throwaway root by: /private/var/tmp/… and /var/tmp/…. */
const ROOT_NAMES = [ROOT, ROOT.replace(/^\/private/, '')];
const namesRoot = (s) => ROOT_NAMES.some((r) => s === r || s.startsWith(r + '/'));
const encodeCwd = (p) => p.replace(/[^A-Za-z0-9]/g, '-');
function treeStamp(dir) {
  const out = [];
  const walk = (p, depth) => {
    let st; try { st = fs.lstatSync(p); } catch { return; }
    out.push(path.relative(dir, p) + ' ' + (st.isDirectory() ? 'dir' : st.size + ' ' + Math.floor(st.mtimeMs)));
    if (st.isDirectory() && !st.isSymbolicLink() && depth < 6) for (const n of fs.readdirSync(p).sort()) walk(path.join(p, n), depth + 1);
  };
  walk(dir, 0);
  return { entries: out.length, hash: sha(out.join('\n')) };
}
function homeSnapshot() {
  const at = (rel) => path.join(HOME, rel);
  const read = (rel) => { try { return fs.readFileSync(at(rel)); } catch { return null; } };
  const snap = { files: {}, trees: {}, claude: null, contexts: [] };
  for (const f of WATCH_FILES) { const b = read(f); snap.files[f] = b ? sha(b) : 'absent'; }
  for (const t of WATCH_TREES) snap.trees[t] = fs.existsSync(at(t)) ? treeStamp(at(t)) : 'absent';
  const cj = read('.claude.json');
  if (cj) { try { const j = JSON.parse(cj); snap.claude = { mcpServers: sha(JSON.stringify(j.mcpServers ?? null)), topKeys: Object.keys(j).length, ours: Object.keys(j.projects ?? {}).filter(namesRoot) }; } catch { snap.claude = { unreadable: true }; } }
  const ctx = at('.otto/context');
  if (fs.existsSync(ctx)) for (const p of fs.readdirSync(ctx)) { try { for (const b of fs.readdirSync(path.join(ctx, p))) if (b.startsWith(encodeCwd(ROOT))) snap.contexts.push(p + '/' + b); } catch {} }
  return snap;
}
/** Read-change-write of a config file a CLI may save at any moment: write beside it, rename only if it did not change meanwhile. */
function editConfig(file, change) {
  for (let i = 0; i < 3; i++) {
    let st; try { st = fs.statSync(file); } catch { return 0; }
    const before = fs.readFileSync(file, 'utf8');
    const [after, n] = change(before);
    if (!n) return 0;
    const tmp = path.join(path.dirname(file), '.' + path.basename(file) + '.live-check.' + process.pid);
    fs.writeFileSync(tmp, after, { mode: st.mode & 0o777 });
    const now = fs.statSync(file);
    if (now.size !== st.size || now.mtimeMs !== st.mtimeMs) { fs.rmSync(tmp); continue; }
    fs.renameSync(tmp, file);
    return n;
  }
  throw new Error(file + ' kept changing; the entries naming ' + ROOT + ' are still in it');
}
/** Remove the folder-trust grants and per-project entries that name the throwaway root (otto-sessions trust.rs writes them at every spawn). */
function removeTrustGrants() {
  const removed = [];
  const json = (rel, pick) => {
    const n = editConfig(path.join(HOME, rel), (text) => {
      const j = JSON.parse(text), map = pick(j) ?? {}, keys = Object.keys(map).filter(namesRoot);
      for (const k of keys) delete map[k];
      return [JSON.stringify(j), keys.length];
    });
    if (n) removed.push(rel + ': ' + n + ' entr' + (n === 1 ? 'y' : 'ies'));
  };
  json('.claude.json', (j) => j.projects);
  json('.gemini/trustedFolders.json', (j) => j);
  const n = editConfig(path.join(HOME, '.codex/config.toml'), (text) => {
    const lines = text.split('\n'), keep = [];
    let cut = 0;
    for (let i = 0; i < lines.length; i++) {
      const m = /^\[projects\."(.*)"\]$/.exec(lines[i].trim());
      if (m && namesRoot(m[1]) && (lines[i + 1] ?? '').trim() === 'trust_level = "trusted"') {
        if (keep.length && keep[keep.length - 1] === '') keep.pop(); // the blank line trust.rs put before the table
        i++, cut++;
      } else keep.push(lines[i]);
    }
    return [keep.join('\n'), cut];
  });
  if (n) removed.push('.codex/config.toml: ' + n + ' table' + (n === 1 ? '' : 's'));
  return removed;
}

// ── the daemon ──────────────────────────────────────────────────────────────
let life = null, api = null, lifeKind = '', lives = 0;
const daemonPids = [];
function envFor(kind) {
  const base = { HOME, PATH: daemonPath, SHELL: path.join(d.bin, 'login-shell'), OTTO_E2E: kind === 'canned' ? '1' : '0', OTTO_E2E_FAST_TIMERS: '1', OTTO_FAKE_LOG: path.join(d.logs, 'fake-turns.jsonl'), OTTO_FAKE_ARGV_LOG: path.join(d.logs, 'fake-argv.jsonl') };
  if (!LIVE) return base;
  const me = os.userInfo();
  // What launchd gives the installed daemon, and nothing of this shell's: the CLIs find their logins through HOME and USER.
  return { ...base, USER: me.username, LOGNAME: me.username, SHELL: process.env.SHELL || '/bin/zsh', OTTO_FAKE_DEFAULT: '', OTTO_FAKE_SCRIPTS: '', OTTO_FAKE_REVIEW_DIR: '' };
}
/** One daemon life of 'kind': 'real' (no e2e short-circuits: threads and PTY sessions run their CLI) or
 *  'canned' (OTTO_E2E=1: a PTY step answers from the offline stub — the only PTY the fakes can stand in for). */
async function ensureLife(kind) {
  if (!LIVE && OWNER_HOME) kind = 'canned'; // the rehearsal on the real HOME never writes a trust grant there
  if (life && lifeKind === kind) return;
  await stopDaemon();
  const logFile = path.join(d.logs, 'ottod-' + ++lives + '-' + kind + '.log');
  life = await boot(OTTOD, { root: ROOT, d }, PORT, envFor(kind), logFile);
  lifeKind = kind, api = life.api;
  daemonPids.push(life.child.pid);
  const said = /starting \(data dir ([^)]+)\)/.exec(fs.readFileSync(logFile, 'utf8'))?.[1];
  if (said !== d.data) { try { process.kill(-life.child.pid, 'SIGKILL'); } catch {} throw new Abort('the daemon resolved its data dir to ' + said + ', not ' + d.data); }
  log('daemon up (' + kind + ', pid ' + life.child.pid + ', port ' + PORT + ')');
}
async function stopDaemon() {
  if (!life) return;
  await life.stop();
  life = null, api = null;
}
/** Every process this check started that is still alive: the daemons' process groups, their descendants,
 *  and anything whose command line or environment names the root (a holder or CLI reparented to launchd). */
function census() {
  const rows = sh('ps', ['-axEo', 'pid=,ppid=,pgid=,command='], { maxBuffer: 256 << 20 }).split('\n').map((l) => /^\s*(\d+)\s+(\d+)\s+(\d+)\s+(.*)$/.exec(l)).filter(Boolean)
    .map((m) => ({ pid: +m[1], ppid: +m[2], pgid: +m[3], cmd: m[4] }));
  const mine = new Set(rows.filter((r) => r.pid !== process.pid && (daemonPids.includes(r.pgid) || r.cmd.includes(ROOT))).map((r) => r.pid));
  for (let grew = true; grew; ) { grew = false; for (const r of rows) if (!mine.has(r.pid) && mine.has(r.ppid) && r.pid !== process.pid) { mine.add(r.pid); grew = true; } }
  return rows.filter((r) => mine.has(r.pid)).map((r) => ({ pid: r.pid, ppid: r.ppid, name: path.basename(r.cmd.split(' ')[0]), daemon: daemonPids.includes(r.pid) }));
}
const agentsAlive = () => census().filter((p) => !p.daemon && !/clickhouse/.test(p.name));

// ── the turn ledger ─────────────────────────────────────────────────────────
// Thread rows: one 'turn' entity per turn. PTY agent sessions: one prompt each. A removed thread (a finished
// sub-thread) leaves the table, so the ledger keeps the most it ever saw of each row.
const ledger = new Map();
const TURNS_SQL = "SELECT 'threads', s.id, s.provider, COALESCE(json_extract(s.meta,'$.source'),''), COALESCE(json_extract(s.meta,'$.run_id'),''), COALESCE(json_extract(s.meta,'$.node_id'),''), (SELECT COUNT(*) FROM agent_runtime_entities e WHERE e.session_id = s.id AND e.kind = 'turn') FROM agent_runtime_sessions s"
  + " UNION ALL SELECT 'pty', id, provider, COALESCE(json_extract(meta_json,'$.source'),''), COALESCE(json_extract(meta_json,'$.run_id'),''), COALESCE(json_extract(meta_json,'$.node_id'),''), 1 FROM sessions WHERE kind = 'agent'";
function tally() {
  let out = '';
  try { out = sh('sqlite3', ['-readonly', '-separator', '\t', '-cmd', '.timeout 3000', path.join(d.data, 'otto.db'), TURNS_SQL]); } catch (e) { tallyError = String(e.stderr ?? e.message).slice(0, 200); return; }
  tallyError = '';
  for (const l of out.split('\n').filter(Boolean)) {
    const [runner, id, provider, source, run, node, turns] = l.split('\t');
    const had = ledger.get(id);
    ledger.set(id, { runner, id, provider, source, run, node, turns: Math.max(Number(turns), had?.turns ?? 0), step: had?.step ?? currentStep });
  }
}
let currentStep = '', tallyError = '';
/** A sub-thread a step's agent opened and finished between two looks never showed its turn: the run record still lists it. */
function countChildren(run, wf) {
  const agentStep = (id) => wf.graph.nodes.some((n) => n.id === id && n.kind === 'agent_prompt');
  for (const n of (run.nodes ?? []).filter((x) => agentStep(x.node_id))) for (const c of n.children ?? []) {
    const id = c.thread_id, had = ledger.get(id);
    if (id && !(had?.turns > 0)) ledger.set(id, { runner: 'threads', id, provider: c.provider, source: had?.source || 'workflow', run: run.id, node: n.node_id, turns: 1, step: had?.step ?? currentStep });
  }
}
const used = () => [...ledger.values()].reduce((n, r) => n + r.turns, 0);
const usedBy = (pred) => [...ledger.values()].filter(pred).reduce((n, r) => n + r.turns, 0);
const perProvider = (rows) => Object.entries(rows.reduce((m, r) => ({ ...m, [r.provider]: (m[r.provider] ?? 0) + r.turns }), {})).map(([p, n]) => p + ' ' + n).join(', ');

// ── workflows ───────────────────────────────────────────────────────────────
const settled = (r) => ['success', 'error', 'canceled'].includes(r.status);
const started = [];
let ws = null;
const have = (p) => providers[p]?.available;
let catalog = {};
const pickNotes = new Set();
/** {model, effort} as the daemon will accept them for 'provider': the catalog's id for "haiku", an effort only when the model lists it. */
function pick(provider, want) {
  const models = catalog[provider] ?? [];
  const out = Object.fromEntries(Object.entries(want).filter(([, v]) => v));
  if (want.model && models.length) {
    const m = models.find((x) => x.id === want.model) ?? models.find((x) => new RegExp(want.model, 'i').test(x.id) && (!want.effort || !x.supported_efforts?.length || x.supported_efforts.includes(want.effort)));
    if (m) out.model = m.id; else { delete out.model; pickNotes.add(provider + ': the catalog lists no model like "' + want.model + '" — the provider default is used'); }
  }
  const listed = models.find((x) => x.id === out.model)?.supported_efforts ?? [];
  if (out.effort && listed.length && !listed.includes(out.effort)) { delete out.effort; pickNotes.add(provider + ' ' + out.model + ' lists no effort "' + want.effort + '" — its default is used'); }
  return out;
}
/** The stored graph of a spec workflow: fake lines appended with --fakes, and every step of a provider that is
 *  not available taken out (its neighbours joined). */
function graphFor(key) {
  const wf = SPEC.workflows[key];
  let nodes = structuredClone(wf.graph.nodes), edges = structuredClone(wf.graph.edges);
  const drop = (id, why) => {
    const ins = edges.filter((e) => e.target === id), outs = edges.filter((e) => e.source === id);
    edges = edges.filter((e) => e.source !== id && e.target !== id).concat(ins.flatMap((i) => outs.map((o) => ({ id: i.source + '-' + o.target, source: i.source, target: o.target }))));
    nodes = nodes.filter((n) => n.id !== id);
    note('step "' + id + '" of ' + wf.name + ' left out: ' + why);
  };
  for (const n of [...nodes]) {
    const p = n.params ?? {};
    if (n.kind === 'agent_prompt') {
      const needs = [p.provider, ...(n.id === 'f' ? ['codex'] : [])].filter((x) => !have(x));
      if (needs.length) { drop(n.id, needs.join(', ') + ' is not available'); continue; }
      if (p.continue_from && !nodes.some((x) => x.id === p.continue_from)) delete p.continue_from;
      const chosen = pick(p.provider, { model: p.model, effort: p.effort });
      for (const k of ['model', 'effort']) if (chosen[k]) p[k] = chosen[k]; else delete p[k];
      const lines = FAKES ? (TEST_FAIL === n.id ? ['fail provider'] : wf.fake[n.id] ?? []) : [];
      if (lines.length) p.prompt += '\n' + lines.map((l) => 'OTTO_FAKE: ' + l).join('\n');
    }
    if (n.kind === 'review_run') {
      for (const r of p.reviewers) r.providers = r.providers.filter(have);
      p.reviewers = p.reviewers.filter((r) => r.providers.length);
      for (const [prov, want] of Object.entries(p.picks ?? {})) p.picks[prov] = pick(prov, want);
      p.summarizer = { provider: p.summarizer.provider, ...pick(p.summarizer.provider, { model: p.summarizer.model, effort: p.summarizer.effort }) };
      if (!p.reviewers.length || !have(p.summarizer.provider)) drop(n.id, 'no reviewer provider is available');
    }
  }
  const budget = Object.fromEntries(nodes.filter((n) => wf.turns[n.id]).map((n) => {
    let [expected, max] = wf.turns[n.id];
    if (n.kind === 'review_run') { const agents = n.params.reviewers.reduce((s, r) => s + r.providers.length, 0); expected = agents + 2, max = agents + 3; }
    return [n.id, { expected, max }];
  }));
  return { name: wf.name, graph: { nodes, edges }, budget, input: wf.input };
}
const planned = (key) => Object.values(graphFor(key).budget);

/** What the steps that start next could still spend: every running step's rest, plus the worst case of each
 *  budgeted step whose budgeted predecessors have all started. */
function exposure(run, wf) {
  const state = Object.fromEntries((run?.nodes ?? []).map((n) => [n.node_id, n.status]));
  const begun = (id) => ['running', 'success', 'error', 'skipped'].includes(state[id]);
  // A review's PTY sessions carry no run id: they are the review rows first seen in this step.
  const spent = (id) => usedBy((r) => (r.run === run?.id && r.node === id) || (id === 'review' && r.source.startsWith('review') && r.step === currentStep));
  const preds = (id, seen = new Set()) => wf.graph.edges.filter((e) => e.target === id && !seen.has(e.source)).flatMap((e) => (seen.add(e.source), wf.budget[e.source] ? [e.source] : preds(e.source, seen)));
  let rest = 0;
  const next = [];
  for (const [id, b] of Object.entries(wf.budget)) {
    // Two past its worst case is a runaway (a retry loop), not a slow step.
    if (spent(id) > b.max + 2) throw new Abort('step "' + id + '" has used ' + spent(id) + ' turns; its worst case is ' + b.max);
    if (state[id] === 'running') rest += Math.max(1, b.max - spent(id));
    else if (!begun(id) && preds(id).every(begun)) { rest += b.max; next.push(id); }
  }
  return { rest, next };
}
function guard(run, wf) {
  const { rest, next } = exposure(run, wf);
  if (used() > CAP) throw new Abort(used() + ' turns used: over the cap of ' + CAP);
  if ((next.length || run) && used() + rest > CAP) throw new Abort(used() + ' turns used; the next step' + (next.length > 1 ? 's' : '') + ' (' + next.join(', ') + ') could bring it to ' + (used() + rest) + ', past the cap of ' + CAP);
}

const stepRows = []; // the scenario table
/** Run one spec workflow to its end under the budget guard. onTick sees every poll (approvals). */
async function runStep(part, id, key, { settings = {}, onTick = null, limitMs = 30 * 60_000, repo = 'ws/repo' } = {}) {
  currentStep = id;
  const wf = graphFor(key);
  const row = { part, id, workflow: wf.name, expected: Object.values(wf.budget).reduce((s, b) => s + b.expected, 0), max: Object.values(wf.budget).reduce((s, b) => s + b.max, 0), turns: 0, status: 'not run', seconds: 0 };
  stepRows.push(row);
  if (!Object.keys(wf.budget).length) { row.status = 'skipped'; note(id + ': nothing to run (its providers are not available)'); return null; }
  tally();
  const before = used();
  guard(null, wf);
  log('── ' + id + ': ' + before + ' turns used so far; this step expects ' + row.expected + ' (worst case ' + row.max + '); cap ' + CAP);
  await api('PUT', '/settings', settings);
  const stored = await api('POST', '/workspaces/' + ws.id + '/workflows', { name: wf.name + ' (' + id + ')', graph: wf.graph });
  const t0 = Date.now();
  let run = await api('POST', '/workflows/' + stored.id + '/run', { input: { working_directory: d[repo], ...wf.input } });
  started.push(run.id);
  try {
    for (;;) {
      await sleep(400);
      tally();
      run = await api('GET', '/workflow-runs/' + run.id);
      row.turns = used() - before;
      countChildren(run, wf);
      if (settled(run)) break;
      guard(run, wf);
      if (onTick) await onTick(run);
      if (Date.now() - t0 > limitMs) { await api('POST', '/workflow-runs/' + run.id + '/cancel', {}).catch(() => {}); row.status = 'timed out'; check(part, id + ': finished inside ' + limitMs / 60000 + ' min', false, 'canceled by the harness'); break; }
    }
  } finally {
    // The settle writes of the last turns (a child removed, a nudge) land a moment after the run.
    await sleep(1500), tally();
    row.turns = used() - before, row.seconds = Math.round((Date.now() - t0) / 1000);
    run = await api('GET', '/workflow-runs/' + run.id).catch(() => run);
    countChildren(run, wf), row.turns = used() - before;
    save('runs/' + id + '.json', run);
  }
  if (row.status === 'not run') row.status = run.status;
  // A provider that turns out not to be signed in is left out of everything after; nothing is tried again.
  for (const n of run.nodes ?? []) {
    const provider = wf.graph.nodes.find((x) => x.id === n.node_id)?.params?.provider;
    if (provider && n.status === 'error' && (n.error_class === 'auth' || /not logged in|please run \/login|unauthorized|401/i.test(n.error ?? ''))) { providers[provider] = { available: false, why: 'not signed in: ' + String(n.error).slice(0, 120) }; note(provider + ' is not signed in — left out of the steps after "' + id + '"'); }
  }
  const mine = [...ledger.values()].filter((r) => r.step === id);
  row.by = perProvider(mine);
  log('── ' + id + ': ' + run.status + ' in ' + row.seconds + ' s, ' + row.turns + ' turns (' + (row.by || 'none') + '); total ' + used() + ' of ' + CAP);
  return { run, wf, row };
}
const nodeOf = (run, id) => run.nodes?.find((n) => n.node_id === id);
const contextFile = (run, name) => { try { return fs.readFileSync(path.join(d.data, 'workflow-context', run.id, name), 'utf8'); } catch { return null; } };
const handoff = (run, id) => { const n = nodeOf(run, id); return n?.step_file ? contextFile(run, n.step_file + '.md') : null; };
const tokens = (u) => u ? Object.entries(u).filter(([k, v]) => k.endsWith('_tokens') && typeof v === 'number').reduce((s, [, v]) => s + v, 0) : 0;

// ── the scenario (test plan §8, the table) ──────────────────────────────────
const timing = []; // per agent step
function recordTiming(step, run, ids) {
  for (const id of ids) {
    const n = nodeOf(run, id);
    if (!n) continue;
    const at = (phase) => { const m = (n.phases ?? []).find((p) => p.phase === phase); return m ? Date.parse(m.at) : null; };
    const ready = at('starting') ?? (n.started_at ? Date.parse(n.started_at) : null), working = at('working');
    timing.push({ step, node: id, provider: n.thread?.provider ?? '', runner: n.thread?.runner ?? (n.sessions?.length ? 'pty' : ''), model: n.thread?.model ?? '', effort: n.thread?.effort ?? '',
      ready_to_turn_ms: ready !== null && working !== null ? working - ready : null, duration_ms: n.duration_ms ?? null, turns: n.thread?.turns ?? null, tokens: tokens(n.thread?.usage) });
  }
}
const median = (xs) => { const s = [...xs].sort((x, y) => x - y); return s.length ? s[Math.floor((s.length - 1) / 2)] : null; };
const planted = (key, findings) => SPEC.planted[key].map((p) => {
  const hits = findings.filter((f) => String(f.path ?? f.file ?? '').endsWith(p.path) && new RegExp(p.words, 'i').test([f.title, f.body, f.summary, f.message].filter(Boolean).join(' ')));
  return { id: p.id, found: hits.length > 0, as_bug: hits.some((f) => f.severity === 'bug') };
});
async function reviewOf(run, name) {
  const out = nodeOf(run, 'review')?.output ?? {};
  if (!out.review_id) return { out, review: null, findings: [] };
  const review = await api('GET', '/reviews/' + out.review_id).catch((e) => ({ error: e.message }));
  const got = await api('GET', '/reviews/' + out.review_id + '/findings').catch((e) => ({ error: e.message }));
  save('reviews/' + name + '.json', { output: out, review, findings: got });
  // The review's final comments (path, line, severity bug|warn|info, body) are what is compared; the findings
  // route must answer with a list for the same review.
  return { out, review, findings: review.comments ?? [], valid: Array.isArray(got) && Array.isArray(review.comments) };
}

async function smoke() {
  const P = 'smoke';
  if (FAKES) for (const [f, body] of Object.entries(SPEC.fakeReview.smoke)) fs.writeFileSync(path.join(d.review, f), JSON.stringify(body));
  await ensureLife('real');
  const done = await runStep(P, 'smoke', 'smoke', { settings: { wf_step_runner: 'threads', agent_skip_permissions: true } });
  if (done) {
    const { run, wf } = done, has = (id) => wf.graph.nodes.some((n) => n.id === id), ok = (id) => nodeOf(run, id)?.status === 'success';
    check(P, 'the run succeeds', run.status === 'success', run.status + ' ' + (run.error ?? '') + ' ' + (run.nodes ?? []).filter((n) => n.status === 'error').map((n) => n.node_id + ': ' + n.error).join('; '));
    const co = nodeOf(run, 'checkout')?.output ?? {};
    check(P, '1 checkout: real git in a real worktree, at the feature branch', ok('checkout') && co.rows?.[0]?.head === repoInfo.tips.feature && co.rows?.[0]?.merge_base === repoInfo.tips.main && co.rows?.[0]?.files_changed === 1 && namesRoot(co.repos?.[0]?.worktree ?? ''), 'worktree ' + co.repos?.[0]?.worktree + ', head ' + String(co.rows?.[0]?.head).slice(0, 9) + ' (feature ' + repoInfo.tips.feature.slice(0, 9) + '), ' + co.rows?.[0]?.files_changed + ' file changed');
    if (has('a')) {
      const h = handoff(run, 'a'), t = nodeOf(run, 'a')?.thread;
      check(P, '2 A: the handoff is at the name the engine gave and holds the changed-file count', ok('a') && /changed files: 1\b/.test(h ?? ''), nodeOf(run, 'a')?.step_file + '.md: ' + JSON.stringify((h ?? 'missing').slice(0, 80)));
      check(P, '2 A: the model and effort reached the CLI', /haiku/i.test(t?.model ?? '') && (t?.effort ?? '') === 'low', 'model ' + t?.model + ', effort ' + t?.effort);
      check(P, '2 A: the background command brought the CLI\'s own follow-up turn', (t?.turns ?? 0) >= 2, (t?.turns ?? 0) + ' turns on the thread');
    }
    if (has('b') && has('c')) {
      const [b, c] = ['b', 'c'].map((id) => nodeOf(run, id)), end = (n) => Date.parse(n.started_at) + (n.duration_ms ?? 0);
      check(P, '3 B and C: two drivers worked at the same time', ok('b') && ok('c') && Date.parse(b.started_at) < end(c) && Date.parse(c.started_at) < end(b), 'B ' + b?.started_at + ' +' + b?.duration_ms + ' ms, C ' + c?.started_at + ' +' + c?.duration_ms + ' ms');
      check(P, '3 B and C: both list the changed file', ['b', 'c'].every((id) => /window\.go/.test(handoff(run, id) ?? '')), ['b', 'c'].map((id) => JSON.stringify((handoff(run, id) ?? 'missing').slice(0, 60))).join(' / '));
    } else for (const id of ['b', 'c']) if (has(id)) check(P, '3 ' + id.toUpperCase() + ' lists the changed file', ok(id) && /window\.go/.test(handoff(run, id) ?? ''), JSON.stringify((handoff(run, id) ?? 'missing').slice(0, 60)));
    if (has('d') && has('a')) {
      const ta = nodeOf(run, 'a')?.thread, td = nodeOf(run, 'd')?.thread;
      check(P, '4 D: continued A\'s thread and repeated its number', ok('d') && td?.thread_id === ta?.thread_id && td?.continued_from === 'a' && /changed files: 1\b/.test(handoff(run, 'd') ?? ''), 'thread ' + td?.thread_id + ' (A ' + ta?.thread_id + '), ' + JSON.stringify((handoff(run, 'd') ?? 'missing').slice(0, 60)));
    }
    if (has('f')) {
      const f = nodeOf(run, 'f'), kids = (f?.children ?? []).filter((c) => c.provider === 'codex');
      const gone = await Promise.all(kids.map((k) => api('GET', '/runtime-sessions/' + (k.thread_id ?? k.id)).then(() => false, (e) => e.status === 404)));
      let kept = ''; try { kept = fs.readdirSync(path.join(d.data, 'workflow-context', run.id, '.threads')).filter((x) => x.startsWith('f-')).map((x) => contextFile(run, '.threads/' + x)).join('\n'); } catch {}
      check(P, '6 F: the agent opened a codex sub-thread and finished it (removed)', ok('f') && kids.length === 1 && gone.every(Boolean), kids.length + ' codex sub-thread(s), removed: ' + gone.join(','));
      check(P, '6 F: the sub-thread\'s answer stays with the parent', /pong/i.test(kept) && /pong/i.test(handoff(run, 'f') ?? ''), 'kept timeline says pong: ' + /pong/i.test(kept) + ', handoff ' + JSON.stringify((handoff(run, 'f') ?? 'missing').slice(0, 40)));
    }
    if (has('review')) {
      const r = await reviewOf(run, 'smoke');
      check(P, '7 review: findings JSON valid and a verdict computed', ok('review') && r.valid && typeof r.out.score === 'number' && typeof r.out.passed === 'boolean', 'status ' + r.out.status + ', score ' + r.out.score + ', passed ' + r.out.passed + ', ' + r.findings.length + ' finding(s)');
      note('smoke review: planted off-by-one ' + (planted('feature', r.findings)[0].found ? 'found' : 'NOT found') + ' (reported, not required)');
    }
    check(P, '8 condition → log ran', ok('scored') && ok('done'), nodeOf(run, 'scored')?.status + ' / ' + nodeOf(run, 'done')?.status);
    const agents = ['a', 'b', 'c', 'd', 'f'].filter(has);
    recordTiming('smoke', run, agents);
    const noUse = agents.filter((id) => !tokens(nodeOf(run, id)?.thread?.usage));
    check(P, 'usage of every step is on the run record', !noUse.length && tokens(run.overview?.usage) > 0, 'without usage: ' + (noUse.join(', ') || 'none') + '; run total ' + tokens(run.overview?.usage) + ' tokens');
  }

  // 5: E in approval-required mode — the engine has no per-step access mode (step_runner.rs creates a thread in
  // the provider default), so the step runs alone with Settings › Providers "skip permission prompts" off.
  let seen = null;
  const answered = new Set();
  const e = await runStep(P, 'approval', 'approval', {
    settings: { wf_step_runner: 'threads', agent_skip_permissions: false }, limitMs: 15 * 60_000,
    onTick: async (run) => {
      for (const at of (run.overview?.attention ?? []).filter((x) => x.kind === 'approval' && x.request_id && !answered.has(x.request_id))) {
        seen ??= { asked: Date.parse(at.since), shown: Date.now() };
        answered.add(at.request_id);
        if (answered.size > 8) throw new Error('more than 8 approvals asked');
        if (at.via === 'mcp') await api('POST', '/mcp/approvals/' + at.request_id + '/decide', { decision: 'approve' });
        else await api('POST', '/runtime-sessions/' + at.thread_id + '/requests/' + at.request_id, { decision: 'accept' });
      }
    },
  });
  await api('PUT', '/settings', { agent_skip_permissions: true });
  if (e) {
    check(P, '5 E: the CLI asked and the run showed "Needs you" within 2 s', seen && seen.shown - seen.asked <= 2000 + 400, seen ? 'asked → seen by the harness in ' + (seen.shown - seen.asked) + ' ms (poll 400 ms)' : 'no approval was asked');
    check(P, '5 E: approval through the API continued it', e.run.status === 'success' && /commits: \d+/.test(handoff(e.run, 'e') ?? ''), e.run.status + ', ' + answered.size + ' approval(s), handoff ' + JSON.stringify((handoff(e.run, 'e') ?? 'missing').slice(0, 40)));
    recordTiming('approval', e.run, ['e']);
  }
}

async function ptyFallback() {
  const P = 'pty';
  await ensureLife(FAKES ? 'canned' : 'real');
  const done = await runStep(P, 'pty', 'pty', { settings: { wf_step_runner: 'pty', agent_skip_permissions: true }, limitMs: 15 * 60_000 });
  await api('PUT', '/settings', { wf_step_runner: 'threads' });
  if (!done) return;
  const n = nodeOf(done.run, 'p');
  check(P, '9 the one-step run on the PTY fallback succeeds', done.run.status === 'success' && n?.status === 'success', done.run.status + ' ' + (n?.error ?? ''));
  check(P, '9 the step ran on a PTY session, not a thread', n?.thread?.runner !== 'threads' && ((n?.sessions ?? []).length > 0 || n?.thread?.runner === 'pty'), 'sessions ' + JSON.stringify(n?.sessions ?? []) + ', thread ' + JSON.stringify(n?.thread?.runner ?? null));
  if (FAKES) note('pty: with --fakes the offline daemon answers the PTY step from its stub (no CLI, no session row): 0 turns are counted; --live counts one per PTY agent session');
  recordTiming('pty', done.run, ['p']);
}

// ── PTY versus Threads: one diff, the same review, both runners ─────────────
const comparison = { lines: [] };
const shape = (v, at = '', out = new Set()) => {
  if (Array.isArray(v)) v.forEach((x) => shape(x, at + '[]', out));
  else if (v && typeof v === 'object') for (const [k, x] of Object.entries(v)) { if (!/(^|_)(id|ids|at|thread|thread_id|session_id|sessions)$/.test(k)) shape(x, at + '.' + k, out); }
  else out.add(at);
  return out;
};
async function compare() {
  const P = 'compare';
  if (FAKES) for (const [f, body] of Object.entries(SPEC.fakeReview.compare)) fs.writeFileSync(path.join(d.review, f), JSON.stringify(body));
  const sides = {};
  for (const runner of ['threads', 'pty']) {
    await ensureLife('real');
    // A PTY reviewer cannot be faked (a fake's PTY session runs no CLI), so with --fakes the second side runs on
    // threads too, with one more finding from one reviewer: the comparison and its listing still have work.
    const second = FAKES && runner === 'pty';
    if (second) { for (const [f, body] of Object.entries(SPEC.fakeReview.compareSecond)) fs.writeFileSync(path.join(d.review, f), JSON.stringify(body)); note('compare: with --fakes the "pty" side ran on threads (a PTY reviewer cannot be faked) with one extra warn finding planted for the listing'); }
    const done = await runStep(P, 'compare-' + runner, 'compare', { settings: { wf_step_runner: second ? 'threads' : runner, agent_skip_permissions: true }, limitMs: 60 * 60_000, repo: 'ws/compare-' + runner });
    if (!done) return;
    const r = await reviewOf(done.run, 'compare-' + runner);
    sides[runner] = { ...r, run: done.run, planted: planted('compare', r.findings) };
    check(P, runner + ': the run succeeds and every reviewer left a valid findings JSON', done.run.status === 'success' && r.valid && (r.review?.agents ?? []).length > 0 && r.review.agents.every((x) => x.status === 'done'), done.run.status + '; agents ' + (r.review?.agents ?? []).map((x) => (x.name ?? x.lens ?? '?') + '=' + (x.status ?? '?')).join(', '));
    const bugs = sides[runner].planted.filter((p) => p.as_bug).map((p) => p.id);
    check(P, runner + ': at least 2 of the 3 planted defects found as bugs', bugs.length >= 2, bugs.join(', ') || 'none');
  }
  await api('PUT', '/settings', { wf_step_runner: 'threads' });
  const [t, p] = [sides.threads, sides.pty];
  const lens = (s) => [...new Set((s.review?.agents ?? []).map((x) => x.lens ?? x.skill ?? x.name))].sort().join(', ');
  if (lens(t) !== lens(p)) comparison.lines.push('**The scope agents kept different lenses** — threads: ' + lens(t) + '; pty: ' + lens(p));
  const only = (x, y) => [...shape(x.review)].filter((k) => !shape(y.review).has(k));
  check(P, 'the stored review rows have the same shape on both runners', !only(t, p).length && !only(p, t).length, 'only on threads: ' + (only(t, p).slice(0, 6).join(' ') || '—') + '; only on pty: ' + (only(p, t).slice(0, 6).join(' ') || '—'));
  const verdict = (s) => [s.review?.verdict, s.out.outcome, 'passed ' + s.out.passed].join(' / ');
  check(P, 'the verdicts are equal', verdict(t) === verdict(p), 'threads: ' + verdict(t) + ' (score ' + t.out.score + '); pty: ' + verdict(p) + ' (score ' + p.out.score + ')');
  const miss = (x, y) => x.planted.filter((q, i) => q.found && !y.planted[i].found).map((q) => q.id);
  check(P, 'no planted defect is found on one runner only', !miss(t, p).length && !miss(p, t).length, 'threads only: ' + (miss(t, p).join(', ') || '—') + '; pty only: ' + (miss(p, t).join(', ') || '—'));
  // Every finding one runner has and the other does not, matched by file, severity and the planted defect it names.
  const key = (f) => [f.path ?? f.file ?? '', f.severity ?? '', SPEC.planted.compare.find((q) => new RegExp(q.words, 'i').test([f.title, f.body].filter(Boolean).join(' ')))?.id ?? String(f.title ?? f.body ?? '').toLowerCase().replace(/[^a-z ]/g, '').split(' ').slice(0, 6).join(' ')].join(' | ');
  for (const [name, x, y] of [['threads', t, p], ['pty', p, t]]) {
    const other = new Set(y.findings.map(key));
    for (const f of x.findings.filter((q) => !other.has(key(q)))) comparison.lines.push('only on ' + name + ': ' + (f.severity ?? '?') + ' ' + (f.path ?? f.file ?? '?') + ':' + (f.line ?? '?') + ' — ' + String(f.title ?? f.body ?? '').replace(/\s+/g, ' ').slice(0, 160));
  }
  comparison.table = ['threads', 'pty'].map((r) => ({ runner: r, findings: sides[r].findings.length, bugs: sides[r].findings.filter((f) => f.severity === 'bug').length, score: sides[r].out.score, passed: sides[r].out.passed, planted: sides[r].planted.map((q) => q.id + (q.as_bug ? ' ✓' : q.found ? ' (not as a bug)' : ' ✗')).join(', ') }));
}

// ── afterwards: processes, the owner's HOME, transcripts, the root ──────────
const cleanupLog = [];
/** The CLIs' own records of the throwaway root under the real HOME: each path is listed before it is deleted,
 *  and each is found through the root's own name — never a pattern above it. */
function removeTranscripts(home) {
  const doomed = [];
  const rm = (p) => { if (!p.startsWith(home + '/.') || !fs.existsSync(p)) throw new Error('refusing to delete ' + p); doomed.push(p); };
  // claude: ~/.claude/projects/<the cwd, every non-alphanumeric as '-'>/ — the root and everything under it.
  const projects = path.join(home, '.claude/projects');
  if (fs.existsSync(projects)) for (const n of fs.readdirSync(projects)) if (ROOT_NAMES.some((r) => n === encodeCwd(r) || n.startsWith(encodeCwd(r) + '-'))) rm(path.join(projects, n));
  // codex: ~/.codex/sessions/<y>/<m>/<d>/rollout-*.jsonl whose first line (session_meta) has a cwd under the root.
  const days = new Set([startedAt, new Date()].flatMap((t) => [t, new Date(t.getTime() - 86400000), new Date(t.getTime() + 86400000)]).flatMap((t) => [t.toISOString().slice(0, 10), new Date(t.getTime() - t.getTimezoneOffset() * 60000).toISOString().slice(0, 10)]));
  for (const day of days) {
    const dir = path.join(home, '.codex/sessions', ...day.split('-'));
    if (!fs.existsSync(dir)) continue;
    for (const n of fs.readdirSync(dir).filter((x) => x.endsWith('.jsonl'))) {
      const file = path.join(dir, n);
      if (fs.statSync(file).mtimeMs < startedAt.getTime()) continue;
      const fd = fs.openSync(file, 'r'), buf = Buffer.alloc(65536), len = fs.readSync(fd, buf, 0, buf.length, 0);
      fs.closeSync(fd);
      let cwd = ''; try { const j = JSON.parse(buf.subarray(0, len).toString('utf8').split('\n')[0]); cwd = j.payload?.cwd ?? j.cwd ?? ''; } catch {}
      if (namesRoot(cwd)) rm(file);
    }
  }
  // agy: ~/.gemini/antigravity-cli/cache/last_conversations.json maps a cwd to its conversation.
  const agy = path.join(home, '.gemini/antigravity-cli');
  const ids = [];
  try {
    editConfig(path.join(agy, 'cache/last_conversations.json'), (text) => {
      const j = JSON.parse(text), keys = Object.keys(j).filter(namesRoot);
      for (const k of keys) { if (typeof j[k] === 'string' && /^[\w-]+$/.test(j[k])) ids.push(j[k]); delete j[k]; }
      return [JSON.stringify(j), keys.length];
    });
  } catch (e) { cleanupLog.push('agy conversation map: ' + e.message); }
  for (const id of ids) for (const p of [path.join(agy, 'conversations', id + '.db'), path.join(agy, 'conversations', id + '.pb'), path.join(agy, 'brain', id)]) if (fs.existsSync(p)) rm(p);
  // The whole list first, then the deletes.
  log('transcripts of the root to delete (' + doomed.length + '):' + doomed.map((p) => '\n  ' + p).join(''));
  for (const p of doomed) { fs.rmSync(p, { recursive: true, force: true }); cleanupLog.push('deleted ' + p); }
  return doomed;
}
let homeBefore = null, finished = false;
/** --fakes on a temporary HOME: provider files as an owner has them, and transcripts of the root ("ours") beside a
 *  neighbour's, so the cleanup that --live runs on the real HOME is proven on files that can be compared. */
const seeded = {};
function seedHome() {
  const day = new Date().toISOString().slice(0, 10).split('-').join('/');
  const rollout = (cwd) => JSON.stringify({ type: 'session_meta', payload: { id: 'x', cwd } }) + '\n{"type":"event"}\n';
  Object.assign(seeded, {
    '.claude.json': JSON.stringify({ numStartups: 3, mcpServers: { kept: { command: 'true' } }, projects: { '/somewhere/else': { hasTrustDialogAccepted: true, lastCost: 1 } } }),
    '.codex/config.toml': 'model = "kept"\n\n[projects."/somewhere/else"]\ntrust_level = "trusted"\n',
    '.claude/settings.json': '{"env":{"KEPT":"1"}}\n',
    '.claude/skills/kept/SKILL.md': '---\nname: kept\n---\n',
    ['.claude/projects/' + encodeCwd(ROOT) + '-ws-repo/ours.jsonl']: '{"ours":1}\n',
    ['.claude/projects/' + encodeCwd(ROOT) + '/ours.jsonl']: '{"ours":1}\n',
    ['.claude/projects/' + encodeCwd(ROOT) + '0-neighbour/kept.jsonl']: '{}\n',
    '.claude/projects/-somewhere-else/kept.jsonl': '{}\n',
    ['.codex/sessions/' + day + '/rollout-ours.jsonl']: rollout(d['ws/repo']),
    ['.codex/sessions/' + day + '/rollout-kept.jsonl']: rollout('/somewhere/else'),
    '.gemini/antigravity-cli/cache/last_conversations.json': JSON.stringify({ '/somewhere/else': 'kept-1', [d['ws/repo']]: 'ours-1' }),
    '.gemini/antigravity-cli/conversations/ours-1.db': 'x', '.gemini/antigravity-cli/conversations/kept-1.db': 'x',
    '.gemini/antigravity-cli/brain/ours-1/ours.md': 'x', '.gemini/antigravity-cli/brain/kept-1/kept.md': 'x',
  });
  for (const [f, body] of Object.entries(seeded)) { fs.mkdirSync(path.dirname(path.join(d.home, f)), { recursive: true }); fs.writeFileSync(path.join(d.home, f), body); }
}
async function finish(aborted) {
  if (finished) return;
  finished = true;
  const P = 'cleanup';
  try {
    for (const id of started) if (api) await api('POST', '/workflow-runs/' + id + '/cancel', {}).catch(() => {});
    if (api) for (let i = 0; i < 40; i++) { const open = await Promise.all(started.map((id) => api('GET', '/workflow-runs/' + id).then((r) => !settled(r), () => false))); if (!open.some(Boolean)) break; await sleep(500); }
    // Pass line: no agent process of a run is alive SETTLE_S after the runs ended.
    let alive = [];
    const t0 = Date.now();
    if (life) for (;;) { alive = agentsAlive(); if (!alive.length || Date.now() - t0 > SETTLE_S * 1000) break; await sleep(1000); }
    if (life) check(P, 'no agent process of a run alive ' + SETTLE_S + ' s after the runs ended', !alive.length, alive.length ? alive.map((p) => p.name + ' ' + p.pid).join(', ') : 'none after ' + Math.round((Date.now() - t0) / 1000) + ' s');
    tally();
    await stopDaemon();
  } catch (e) { check(P, 'the daemon stopped cleanly', false, e.message); }
  await sleep(500);
  const left = census();
  for (const p of left) try { process.kill(p.pid, 'SIGKILL'); } catch {}
  check(P, 'no process started by the check outlives the daemon', !left.length, left.length ? 'killed: ' + left.map((p) => p.name + ' ' + p.pid).join(', ') : 'none');
  if (OWNER_HOME) {
    try { const removed = removeTrustGrants(); cleanupLog.push(...removed.map((r) => 'removed folder-trust entries naming the root — ' + r)); } catch (e) { check(P, 'the folder-trust entries naming the root were removed', false, e.message); }
    if (LIVE) try { removeTranscripts(REAL_HOME); } catch (e) { check(P, 'the CLIs\' transcripts of the root were removed', false, e.message); }
    const after = homeSnapshot();
    const changed = [...WATCH_FILES.filter((f) => homeBefore.files[f] !== after.files[f]), ...WATCH_TREES.filter((t) => JSON.stringify(homeBefore.trees[t]) !== JSON.stringify(after.trees[t]))];
    save('home-guard.json', { home: HOME, before: homeBefore, after });
    check(P, 'the owner\'s provider config, skills, hooks and launch agents are byte-for-byte as before', !changed.length, changed.length ? 'CHANGED: ' + changed.join(', ') + ' — nothing was restored: the daemon has no code that writes these, so look before overwriting' : WATCH_FILES.length + ' files, ' + WATCH_TREES.length + ' trees');
    check(P, '~/.claude.json: MCP servers unchanged, no entry names the root', after.claude && !after.claude.unreadable && after.claude.mcpServers === homeBefore.claude?.mcpServers && !after.claude.ours.length, JSON.stringify(after.claude));
    check(P, 'no context bundle of the root under ~/.otto/context', !after.contexts.length, after.contexts.join(', ') || 'none');
    // Whatever is still named after the root, or still names it, under the CLIs' own folders.
    const dirs = ['.claude', '.codex', '.gemini', '.grok', '.otto'].map((x) => path.join(HOME, x)).filter((x) => fs.existsSync(x));
    let named = []; try { named = sh('find', [...dirs, '-maxdepth', '6', '-name', '*' + stamp + '*'], { maxBuffer: 64 << 20 }).trim().split('\n').filter(Boolean); } catch (e) { named = ['find failed: ' + e.message]; }
    const mentions = (f) => { try { return fs.readFileSync(path.join(HOME, f), 'utf8').includes('otto-live-' + stamp); } catch { return false; } };
    check(P, 'nothing under ~/.claude, ~/.codex, ~/.gemini, ~/.grok or ~/.otto is named after the root, and no config names it', !named.length && !['.claude.json', '.codex/config.toml', '.gemini/trustedFolders.json'].some(mentions), named.slice(0, 8).join(', ') || 'none');
    const history = ['.claude/history.jsonl', '.codex/history.jsonl'].filter(mentions);
    if (history.length) note('left as they are: lines naming the root in ' + history.map((h) => '~/' + h).join(', ') + ' (the CLIs\' own prompt history)');
    const calls = fs.existsSync(path.join(d.logs, 'launchctl-calls.log')) ? fs.readFileSync(path.join(d.logs, 'launchctl-calls.log'), 'utf8').trim().split('\n') : [];
    note('launchctl: the daemon called the stub ' + calls.length + ' time(s) (' + [...new Set(calls)].join(' / ') + '); the real launchctl was never run');
  } else {
    // A temporary HOME holds only what seedHome planted: anything else in it now is what a daemon on the real
    // HOME would write there, and the planted files show what the cleanup removes and what it leaves.
    const all = () => sh('find', [d.home, '-mindepth', '1', '-not', '-type', 'd']).trim().split('\n').filter(Boolean).map((p) => path.relative(d.home, p));
    const text = (f) => { try { return fs.readFileSync(path.join(d.home, f), 'utf8'); } catch { return ''; } };
    const wrote = all().filter((f) => !(f in seeded) || text(f) !== seeded[f]);
    save('home-writes.txt', wrote.map((f) => '~/' + f + (f in seeded ? ' (changed)' : ' (new)')).join('\n') + '\n');
    note('written under the temporary HOME by the daemon and the fakes (home-writes.txt): ' + (wrote.map((f) => '~/' + f).join(', ') || 'nothing'));
    const removed = removeTrustGrants();
    cleanupLog.push(...removed.map((r) => 'removed folder-trust entries naming the root — ~/' + r));
    try { removeTranscripts(d.home); } catch (e) { check(P, 'the planted transcripts of the root were removed', false, e.message); }
    const canon = (v) => (Array.isArray(v) ? v.map(canon) : v && typeof v === 'object' ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, canon(v[k])])) : v);
    const MAP = '.gemini/antigravity-cli/cache/last_conversations.json';
    // JSON the daemon rewrote is compared by value (it writes keys in its own order); everything else byte for byte.
    const same = (f) => (f === MAP ? text(f) === JSON.stringify({ '/somewhere/else': 'kept-1' }) : f.endsWith('.json') ? JSON.stringify(canon(JSON.parse(text(f) || 'null'))) === JSON.stringify(canon(JSON.parse(seeded[f]))) : text(f) === seeded[f]);
    const ours = Object.keys(seeded).filter((f) => f.includes('ours'));
    const others = Object.keys(seeded).filter((f) => !ours.includes(f));
    check(P, 'the folder-trust entries the daemon wrote are removed; the config files are as planted', wrote.some((f) => f === '.claude.json') && removed.length > 0 && others.every(same), (removed.join('; ') || 'nothing removed') + '; differing from the planted text: ' + (others.filter((f) => !same(f)).join(', ') || 'none'));
    check(P, 'only the transcripts of the root are deleted; a neighbour\'s stay', ours.every((f) => !fs.existsSync(path.join(d.home, f))) && others.every((f) => fs.existsSync(path.join(d.home, f))), 'left of ours: ' + (ours.filter((f) => fs.existsSync(path.join(d.home, f))).join(', ') || 'none'));
  }
  for (const f of fs.readdirSync(d.logs)) fs.copyFileSync(path.join(d.logs, f), path.join(OUT, f));
  cleanupLog.push('delete ' + ROOT);
  if (!/^\/private\/var\/tmp\/otto-live-\d{8}-\d{6}$/.test(ROOT)) throw new Error('refusing to delete ' + ROOT);
  if (!KEEP) fs.rmSync(ROOT, { recursive: true, force: true });
  check(P, 'the temporary root is gone and no process names it', (KEEP || !fs.existsSync(ROOT)) && !census().length, ROOT + (KEEP ? ' (kept: --test-keep)' : ''));
  report(aborted);
}

function report(aborted) {
  const rows = [...ledger.values()];
  const table = (head, body) => ['| ' + head.join(' | ') + ' |', '|' + head.map(() => '---').join('|') + '|', ...body.map((r) => '| ' + r.map((c) => String(c ?? '—').replace(/\|/g, '/').replace(/\n/g, ' ')).join(' | ') + ' |')].join('\n');
  const failed = checks.filter((c) => !c.ok);
  const fast = (p) => median(timing.filter((t) => t.provider === p && t.runner === 'threads' && t.ready_to_turn_ms !== null).map((t) => t.ready_to_turn_ms));
  for (const p of ['claude', 'codex']) if (fast(p) !== null) checks.push({ part: 'timing', name: p + ': ready → turn started ≤ 5 s (median)', ok: fast(p) <= 5000, detail: fast(p) + ' ms' });
  if (fast('agy') !== null) notes.push('agy: ready → turn started ' + fast('agy') + ' ms (reported, no line)');
  const md = [
    '# Live check — ' + (LIVE ? 'REAL CLIs' : 'fake agents' + (OWNER_HOME ? ' on the real HOME (rehearsal)' : '')) + ', ' + startedAt.toISOString(),
    '',
    aborted ? '**ABORTED: ' + aborted + '**' : checks.every((c) => c.ok) ? '**Every pass line holds.**' : '**' + checks.filter((c) => !c.ok).length + ' pass line(s) FAILED.**',
    '',
    'Turns: **' + used() + ' of ' + CAP + '** (' + (perProvider(rows) || 'none') + '). Parts run: ' + (ONLY === 'all' ? PARTS.join(', ') : ONLY) + '. Daemon ' + OTTOD + ', port ' + PORT + ', root ' + ROOT + ' (removed).',
    '',
    '## Providers', table(['provider', 'used', 'why'], Object.entries(providers).map(([p, v]) => [p, v.available ? 'yes' : '**left out**', v.why])),
    '',
    '## Steps', table(['part', 'step', 'result', 'turns', 'expected', 'worst case', 'by provider', 'seconds'], stepRows.map((r) => [r.part, r.id, r.status, r.turns, r.expected, r.max, r.by, r.seconds])),
    '',
    '## Pass lines', table(['part', 'line', 'result', 'measured'], checks.map((c) => [c.part, c.name, c.ok ? 'pass' : '**FAIL**', c.detail])),
    '',
    '## Timing', table(['step', 'node', 'provider', 'runner', 'model', 'effort', 'ready → turn started (ms)', 'duration (ms)', 'turns', 'tokens'], timing.map((t) => [t.step, t.node, t.provider, t.runner, t.model, t.effort, t.ready_to_turn_ms, t.duration_ms, t.turns, t.tokens])),
    '',
    '## Review comparison: PTY versus Threads',
    comparison.table ? table(['runner', 'findings', 'bugs', 'score', 'passed', 'planted defects'], comparison.table.map((r) => [r.runner, r.findings, r.bugs, r.score, r.passed, r.planted])) : 'not run',
    '', ...(comparison.lines.length ? ['Findings on one runner and not the other:', '', ...comparison.lines.map((l) => '- ' + l)] : comparison.table ? ['No finding is on one runner only.'] : []),
    '',
    '## Turn ledger', table(['step', 'runner', 'provider', 'source', 'node', 'turns', 'thread or session'], rows.map((r) => [r.step, r.runner, r.provider, r.source, r.node, r.turns, r.id])),
    '',
    '## Notes', ...[...notes, ...pickNotes].map((n) => '- ' + n),
    '- screenshots: not taken (the run pages need a UI build and a browser; the raw run records are in runs/).',
    '',
    '## Cleanup', ...cleanupLog.map((l) => '- ' + l),
    '',
  ].join('\n');
  save('report.md', md), save('checks.json', checks), save('ledger.json', { cap: CAP, used: used(), rows, steps: stepRows, timing, aborted: aborted ?? null });
  console.log('\n' + md.split('## Pass lines')[0] + (failed.length ? 'FAILED:\n' + failed.map((c) => '- [' + c.part + '] ' + c.name + ' — ' + c.detail).join('\n') + '\n' : '') + '\nreport: ' + path.join(OUT, 'report.md'));
}

// ── main ────────────────────────────────────────────────────────────────────
const startedAt = new Date();
let repoInfo = null;
const want = (part) => ONLY === 'all' || ONLY === part;
let aborted = null;
process.on('SIGINT', () => finish('interrupted').finally(() => process.exit(130)));
process.on('SIGTERM', () => finish('terminated').finally(() => process.exit(143)));
try {
  makeRoot();
  repoInfo = makeRepo();
  if (OWNER_HOME) homeBefore = homeSnapshot(); else seedHome();
  if (OWNER_HOME && homeBefore.claude?.ours.length) throw new Abort('~/.claude.json already names ' + ROOT);
  log((LIVE ? 'LIVE' : 'fakes') + ' — root ' + ROOT + ', HOME ' + HOME + ', cap ' + CAP + ', providers: ' + Object.entries(providers).map(([p, v]) => p + (v.available ? '' : ' (left out: ' + v.why + ')')).join(', '));
  if (!have('claude')) throw new Abort('claude is not available: every step needs it');
  // First life offline (OTTO_E2E=1): only to turn the usage store off before a daemon that is not offline
  // boots — on the real HOME that one would import the owner's whole transcript history into a temporary ClickHouse.
  await ensureLife('canned');
  await api('PUT', '/settings', { usage: { enabled: false, retention_days: 180, metrics_interval_secs: 60 } });
  await ensureLife('real');
  ws = await api('POST', '/workspaces', { name: 'Live check', root_path: path.dirname(d['ws/repo']) });
  // The daemon accepts only a model id its catalog lists (provider_models_pick.rs): "haiku" in live-smoke.json
  // stands for the newest haiku the catalog has. An unknown catalog (the fakes' empty HOME) takes the word as is.
  catalog = (await api('GET', '/meta')).provider_models ?? {};
  save('provider-models.json', catalog);
  for (const r of ['ws/repo', 'ws/compare-threads', 'ws/compare-pty']) await api('POST', '/workspaces/' + ws.id + '/repos', { path: d[r], name: path.basename(r) });
  if (want('smoke')) await smoke();
  if (want('pty')) await ptyFallback();
  if (want('compare')) await compare();
} catch (e) {
  aborted = e instanceof Abort ? e.message : null;
  if (aborted) log('ABORT: ' + aborted); else check('harness', 'ran to the end', false, e.stack ?? e);
}
await finish(aborted);
process.exit(aborted ? 3 : checks.every((c) => c.ok) ? 0 : 1);
