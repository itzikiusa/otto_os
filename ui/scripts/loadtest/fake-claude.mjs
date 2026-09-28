#!/usr/bin/env node
// Fake `claude` CLI for Otto parallel-load tests (scripts/loadtest, r3-11).
//
// Emulates what a busy Claude Code agent costs the DAEMON and the UI, without
// any network or model:
//   - PTY output: ANSI-coloured prose / tool lines at a per-agent steady rate
//     (FAKE_RATE_MIN..FAKE_RATE_MAX KB/s, default 2..20), a spinner/status line
//     redrawn every tick ("esc to interrupt" so screen heuristics see "working"),
//     and an occasional ~200 KB burst (a file dump) every FAKE_BURST_MIN..MAX s.
//   - Transcript: appends real-shaped JSONL turns (records cloned from the
//     otto-transcript fixture, ids/timestamps rewritten) to
//     $HOME/.claude/projects/<enc(cwd)>/<session-id>.jsonl — the exact path the
//     daemon resolves for `claude --session-id <sid>`.
//   - Hooks: POSTs UserPromptSubmit / PostToolUse / Stop to the per-session
//     ingest endpoint exactly like Otto's injected hook command would.
// It only ever writes under $HOME (the test's temp home) and talks only to
// $OTTO_INGEST_BASE (the isolated daemon).
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';

const argv = process.argv.slice(2);
if (argv.includes('--version') || argv.includes('-v')) {
  process.stdout.write('2.1.999 (Claude Code, otto loadtest fake)\n');
  process.exit(0);
}
const sidIdx = Math.max(argv.indexOf('--session-id'), argv.indexOf('--resume'));
const SID = sidIdx >= 0 ? argv[sidIdx + 1] : crypto.randomUUID();

const env = process.env;
const num = (k, d) => (env[k] !== undefined && env[k] !== '' ? Number(env[k]) : d);
// Deterministic per-agent randomness (seeded by the session id) so reruns match.
let seed = parseInt(crypto.createHash('sha1').update(SID).digest('hex').slice(0, 8), 16) || 1;
const rnd = () => {
  seed ^= seed << 13; seed >>>= 0;
  seed ^= seed >>> 17;
  seed ^= seed << 5; seed >>>= 0;
  return seed / 4294967296;
};
const RATE_MIN = num('FAKE_RATE_MIN', 2) * 1024;
const RATE_MAX = num('FAKE_RATE_MAX', 20) * 1024;
const RATE = RATE_MIN + rnd() * (RATE_MAX - RATE_MIN); // bytes/s for this agent
const BURST_BYTES = num('FAKE_BURST_KB', 200) * 1024;
const BURST_MIN = num('FAKE_BURST_MIN', 45);
const BURST_MAX = num('FAKE_BURST_MAX', 120);
const TURN_MIN = num('FAKE_TURN_MIN', 3);
const TURN_MAX = num('FAKE_TURN_MAX', 8);
const TICK_MS = 100;
const FIXTURE = env.FAKE_FIXTURE;

// ── PTY output ───────────────────────────────────────────────────────────────
const C = ['\x1b[31m', '\x1b[32m', '\x1b[33m', '\x1b[34m', '\x1b[35m', '\x1b[36m', '\x1b[90m', '\x1b[1;37m'];
const R = '\x1b[0m';
const WORDS = ('the daemon session transcript tail fold render terminal agent diff commit branch cargo test ' +
  'function returns value error handle async await tokio mutex channel broadcast event payload json ' +
  'workspace repository config sqlite query index layout style effect store derived svelte').split(' ');
const word = () => WORDS[(rnd() * WORDS.length) | 0];
function proseLine() {
  const n = 6 + ((rnd() * 14) | 0);
  let s = '';
  for (let i = 0; i < n; i++) s += (rnd() < 0.15 ? C[(rnd() * C.length) | 0] + word() + R : word()) + ' ';
  return s.trimEnd();
}
function toolLine() {
  const tools = ['Bash', 'Read', 'Edit', 'Grep', 'Write'];
  const t = tools[(rnd() * tools.length) | 0];
  return `\x1b[32m⏺\x1b[0m \x1b[1m${t}\x1b[0m(${word()}/${word()}.rs)\r\n  \x1b[90m⎿  ${(rnd() * 400) | 0} lines\x1b[0m`;
}
const SPIN = ['✻', '✶', '✳', '✢', '·', '✽'];
let spinI = 0;
let out = 0;
let dead = false;
function write(s) {
  if (dead) return;
  try {
    process.stdout.write(s);
    out += s.length;
  } catch {
    dead = true;
  }
}
process.stdout.on('error', () => {
  dead = true;
  process.exit(0);
});
for (const sig of ['SIGHUP', 'SIGTERM', 'SIGINT']) process.on(sig, () => process.exit(0));
process.stdin.on('data', () => {});
process.stdin.on('error', () => {});
process.stdin.resume();

let budget = 0;
function tick() {
  budget += (RATE * TICK_MS) / 1000;
  // clear the status line, print content lines, redraw status line
  let chunk = '\r\x1b[2K';
  while (budget > 0) {
    const l = rnd() < 0.2 ? toolLine() : proseLine();
    chunk += l + '\r\n';
    budget -= l.length + 2;
  }
  spinI = (spinI + 1) % SPIN.length;
  chunk += `\x1b[33m${SPIN[spinI]}\x1b[0m Cooking… \x1b[90m(${Math.round(out / 1024)} KB · esc to interrupt)\x1b[0m`;
  write(chunk);
}
function burst() {
  // A file dump: numbered, syntax-coloured lines, ~BURST_BYTES in one write.
  let s = '\r\x1b[2K\x1b[32m⏺\x1b[0m \x1b[1mRead\x1b[0m(src/big_file.rs)\r\n';
  let i = 1;
  while (s.length < BURST_BYTES) {
    s += `\x1b[90m${String(i).padStart(5)}\x1b[0m  \x1b[35mfn\x1b[0m \x1b[36m${word()}_${i}\x1b[0m(${word()}: &\x1b[33m${word()}\x1b[0m) -> Result<()> { ${word()}(); }\r\n`;
    i++;
  }
  write(s);
  setTimeout(burst, (BURST_MIN + rnd() * (BURST_MAX - BURST_MIN)) * 1000);
}
write(`\x1b[2J\x1b[H\x1b[1;38;5;208m✻ Claude Code\x1b[0m (fake, loadtest) session ${SID}\r\n\r\n`);
// FAKE_QUIET=1: an agent sitting at its prompt (finished turn / freshly
// resumed) — banner + prompt, then silence. Used by the idle-suspend A/B.
const QUIET = env.FAKE_QUIET === '1' || (env.FAKE_QUIET_RESUME === '1' && argv.includes('--resume'));
if (QUIET) {
  write('\x1b[90m────────────────────────────\x1b[0m\r\n❯ ');
} else {
  setInterval(tick, TICK_MS);
  setTimeout(burst, (BURST_MIN * 0.5 + rnd() * BURST_MIN) * 1000);
}

// ── transcript + hooks ───────────────────────────────────────────────────────
const home = env.HOME || '/nonexistent';
let cwd = process.cwd();
try {
  cwd = fs.realpathSync(cwd);
} catch {}
const enc = cwd.replace(/[^A-Za-z0-9]/g, '-');
const tdir = path.join(home, '.claude', 'projects', enc);
const tpath = path.join(tdir, `${SID}.jsonl`);
let template = [];
try {
  template = fs
    .readFileSync(FIXTURE, 'utf8')
    .split('\n')
    .filter(Boolean)
    .map((l) => JSON.parse(l))
    .filter((r) => r.type === 'user' || r.type === 'assistant');
} catch {
  template = [];
}
let ti = 0;
let parent = null;
const bigResult = () => {
  let s = '';
  const n = 20_000 + ((rnd() * 40_000) | 0);
  while (s.length < n) s += `${word()} ${word()} ${word()}\n`;
  return s;
};
async function hook(payload) {
  if (!env.OTTO_INGEST_TOKEN || !env.OTTO_INGEST_BASE) return;
  try {
    await fetch(`${env.OTTO_INGEST_BASE}/api/v1/ingest/claude`, {
      method: 'POST',
      headers: {
        'X-Otto-Session': env.OTTO_SESSION_ID ?? '',
        'X-Otto-Token': env.OTTO_INGEST_TOKEN,
        'Content-Type': 'application/json',
      },
      body: JSON.stringify({ session_id: SID, transcript_path: tpath, cwd, ...payload }),
      signal: AbortSignal.timeout(3000),
    });
  } catch {
    /* hooks never block */
  }
}
function appendRecord() {
  if (template.length === 0) return;
  const src = template[ti % template.length];
  ti++;
  const r = structuredClone(src);
  const uuid = crypto.randomUUID();
  r.uuid = uuid;
  r.parentUuid = parent;
  parent = uuid;
  r.sessionId = SID;
  if ('session_id' in r) r.session_id = SID;
  r.cwd = cwd;
  r.timestamp = new Date().toISOString();
  if (r.type === 'assistant') {
    r.requestId = `req_${crypto.randomUUID()}`;
    if (r.message) r.message.id = `msg_${crypto.randomUUID()}`;
    const tu = Array.isArray(r.message?.content) && r.message.content.find((b) => b.type === 'tool_use');
    if (tu) {
      tu.id = `toolu_${crypto.randomUUID().replace(/-/g, '').slice(0, 24)}`;
      appendRecord.lastTool = tu;
    }
  } else if (Array.isArray(r.message?.content)) {
    const tr = r.message.content.find((b) => b.type === 'tool_result');
    if (tr) {
      if (appendRecord.lastTool) tr.tool_use_id = appendRecord.lastTool.id;
      if (rnd() < 0.15) tr.content = bigResult(); // occasional large tool output
    }
  }
  try {
    fs.appendFileSync(tpath, JSON.stringify(r) + '\n');
  } catch {}
  if (r.type === 'assistant' && appendRecord.lastTool && r.message?.content?.some?.((b) => b.type === 'tool_use')) {
    void hook({ hook_event_name: 'PostToolUse', tool_name: appendRecord.lastTool.name, tool_input: appendRecord.lastTool.input ?? {} });
  }
}
function turn() {
  // one "turn": prompt submit, 2-6 records, stop
  void hook({ hook_event_name: 'UserPromptSubmit', prompt: `${word()} the ${word()}` });
  const k = 2 + ((rnd() * 5) | 0);
  for (let i = 0; i < k; i++) appendRecord();
  void hook({ hook_event_name: 'Stop' });
  setTimeout(turn, (TURN_MIN + rnd() * (TURN_MAX - TURN_MIN)) * 1000);
}
try {
  fs.mkdirSync(tdir, { recursive: true });
  // Claude writes a couple of header records before the first turn.
  fs.appendFileSync(tpath, JSON.stringify({ type: 'permission-mode', permissionMode: 'bypassPermissions', sessionId: SID }) + '\n');
} catch {}
void hook({ hook_event_name: 'SessionStart', source: argv.includes('--resume') ? 'resume' : 'startup' });
if (!QUIET) setTimeout(turn, 1500);
