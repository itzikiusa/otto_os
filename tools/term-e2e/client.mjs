// Shared library for the terminal-robustness E2E harness.
//
// TermClient is a FAITHFUL model of ui/src/lib/components/Terminal.svelte's
// "base model" (v2) semantics — if the component changes its lifecycle, change
// this in lockstep:
//   - attach:  WS open → resize (forced, immediate) → scrollback request
//   - binary frames  → term.write            (client buffer = source of truth)
//   - scrollback frm → term.reset() + write  (rebuild ONLY on attach / server
//                                             lagged-recovery push — never in
//                                             response to our own resize)
//   - resize → local term.resize NOW (native xterm reflow), ONE trailing
//              debounced PTY notification after RESIZE_SETTLE_MS
import pkg from '@xterm/headless';
import WebSocket from 'ws';
import { spawn } from 'node:child_process';
import { mkdtempSync, createWriteStream } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const { Terminal } = pkg;

export const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
export const b64 = (s) => Buffer.from(s, 'utf8').toString('base64');

/** Matches Terminal.svelte's RESIZE_SETTLE_MS. */
export const RESIZE_SETTLE_MS = 200;

// ── daemon bootstrap ─────────────────────────────────────────────────────────

/** Use OTTO_BASE+OTTO_API_TOKEN when provided (test a running daemon), else
 *  spawn an ISOLATED throwaway ottod on a temp data dir — never touches the
 *  user's real sessions or DBs. */
export async function bootstrap() {
  if (process.env.OTTO_BASE && process.env.OTTO_API_TOKEN) {
    return {
      base: process.env.OTTO_BASE,
      token: process.env.OTTO_API_TOKEN,
      stop: async () => {},
      isolated: false,
    };
  }
  const port = Number(process.env.OTTO_E2E_PORT ?? 7911);
  const base = `http://127.0.0.1:${port}`;
  const dataDir = mkdtempSync(join(tmpdir(), 'otto-term-e2e-'));
  const bin = process.env.OTTO_E2E_BIN ?? new URL('../../target/debug/ottod', import.meta.url).pathname;
  const child = spawn(bin, [], {
    env: {
      ...process.env,
      OTTO_DATA_DIR: dataDir,
      OTTO_PORT: String(port),
      // File-backed secrets: a DEBUG ottod otherwise hangs on a macOS Keychain
      // prompt the harness can never answer.
      OTTO_SECRETS: 'file',
      OTTO_SELF_IMPROVE: '0',
      // A fresh data dir has no cli-update last-run cursor: the catch-up run
      // fires at startup and its session reload SIGHUPs agent sessions
      // seconds after spawn. Kill the scheduler for throwaway daemons.
      OTTO_CLI_UPDATE: '0',
    },
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  const logChunks = [];
  const logPath = join(dataDir, 'ottod.log');
  const logStream = createWriteStream(logPath);
  child.stdout.on('data', (d) => { logChunks.push(d); logStream.write(d); });
  child.stderr.on('data', (d) => { logChunks.push(d); logStream.write(d); });
  console.log(`ottod log: ${logPath}`);
  const dead = new Promise((r) => child.once('exit', (code) => r(code)));

  // Wait for health.
  const deadline = Date.now() + 30_000;
  for (;;) {
    if (Date.now() > deadline) {
      child.kill('SIGKILL');
      throw new Error(`ottod never became healthy:\n${Buffer.concat(logChunks)}`);
    }
    try {
      const r = await fetch(`${base}/api/v1/health`);
      if (r.ok) break;
    } catch { /* not up yet */ }
    await sleep(200);
  }

  // Onboard root, then log in for a token.
  const pw = 'term-e2e-password-1';
  await fetch(`${base}/api/v1/onboarding/root`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ password: pw, display_name: 'root' }),
  });
  const login = await fetch(`${base}/api/v1/auth/login`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ username: 'root', password: pw }),
  });
  if (!login.ok) {
    child.kill('SIGKILL');
    throw new Error(`login failed: ${login.status} ${await login.text()}`);
  }
  const token = (await login.json()).token;
  return {
    base,
    token,
    isolated: true,
    stop: async () => {
      child.kill('SIGTERM');
      await Promise.race([dead, sleep(3000)]);
      child.kill('SIGKILL');
    },
  };
}

// ── REST helpers ─────────────────────────────────────────────────────────────

export function makeApi(base, token) {
  return async (method, path, body) => {
    const r = await fetch(`${base}${path}`, {
      method,
      headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
      body: body ? JSON.stringify(body) : undefined,
    });
    if (!r.ok) throw new Error(`${method} ${path}: ${r.status} ${await r.text()}`);
    if (r.status === 204) return null;
    return r.json();
  };
}

export async function ensureWorkspace(api) {
  let wss = await api('GET', '/api/v1/workspaces');
  if (!wss.length) {
    await api('POST', '/api/v1/workspaces', { name: 'term-e2e', root_path: process.env.HOME });
    wss = await api('GET', '/api/v1/workspaces');
  }
  const id = wss[0]?.id ?? wss[0]?.workspace?.id;
  if (!id) throw new Error('no workspace');
  return id;
}

// ── the faithful client ──────────────────────────────────────────────────────

export class TermClient {
  constructor(base, token, sessionId, { cols = 120, rows = 30, claimOnConnect = false } = {}) {
    this.wsBase = base.replace('http', 'ws');
    this.token = token;
    this.sessionId = sessionId;
    this.claimOnConnect = claimOnConnect;
    this.term = new Terminal({ cols, rows, scrollback: 10_000, allowProposedApi: true });
    // Faithful to Terminal.svelte's term.onData → input frame: xterm answers
    // terminal queries (DSR cursor-position, DA, mode reports) on onData, and
    // agent TUIs BLOCK their input loop waiting for those answers. Dropping
    // them leaves claude's composer deaf to typed text.
    this.term.onData((d) => this.type(d));
    this.sock = null;
    this.snapshotCount = 0;
    this.lastSentCols = 0;
    this.lastSentRows = 0;
    this.resizeTimer = null;
    this.exitCode = null;
  }

  /** Attach; resolves once the first snapshot has been applied. */
  connect({ waitSnapshot = true } = {}) {
    return new Promise((resolve, reject) => {
      const sock = new WebSocket(`${this.wsBase}/ws/term/${this.sessionId}`, ['otto-bearer', this.token]);
      sock.binaryType = 'arraybuffer';
      this.sock = sock;
      const before = this.snapshotCount;
      sock.on('open', () => {
        // Mirrors sock.onopen: primary panes claim authority first, then the
        // forced resize, then the ONE attach snapshot.
        if (this.claimOnConnect) this.sendJson({ type: 'claim' });
        this.lastSentCols = this.term.cols;
        this.lastSentRows = this.term.rows;
        this.sendJson({ type: 'resize', cols: this.term.cols, rows: this.term.rows });
        this.sendJson({ type: 'scrollback', lines: 10_000 });
        if (!waitSnapshot) resolve();
      });
      sock.on('error', (e) => reject(e));
      sock.on('message', (data, isBinary) => {
        if (isBinary) {
          this.term.write(new Uint8Array(data));
          return;
        }
        try {
          const msg = JSON.parse(data.toString());
          if (msg.type === 'scrollback' && msg.data) {
            // Full rebuild — exactly what the component does. Resolve only
            // once the (async) write has been fully APPLIED to the buffer, so
            // callers can read it immediately and latency numbers are honest.
            this.term.reset();
            this.term.write(Buffer.from(msg.data, 'base64'), () => {
              this.snapshotCount++;
              if (waitSnapshot && this.snapshotCount === before + 1) resolve();
            });
          } else if (msg.type === 'exit') {
            this.exitCode = msg.code ?? 0;
          }
        } catch { /* malformed control frame */ }
      });
    });
  }

  sendJson(obj) {
    if (this.sock && this.sock.readyState === WebSocket.OPEN) this.sock.send(JSON.stringify(obj));
  }

  type(text) {
    this.sendJson({ type: 'input', data: b64(text) });
  }

  claim() {
    this.sendJson({ type: 'claim' });
  }

  /** v2 resize: local grid changes NOW (xterm reflows its own buffer), the PTY
   *  hears about it once, RESIZE_SETTLE_MS after the last step of a burst. */
  resize(cols, rows) {
    this.term.resize(cols, rows);
    if (this.resizeTimer) clearTimeout(this.resizeTimer);
    this.resizeTimer = setTimeout(() => {
      this.resizeTimer = null;
      if (this.term.cols === this.lastSentCols && this.term.rows === this.lastSentRows) return;
      this.lastSentCols = this.term.cols;
      this.lastSentRows = this.term.rows;
      this.sendJson({ type: 'resize', cols: this.term.cols, rows: this.term.rows });
    }, RESIZE_SETTLE_MS);
  }

  /** Resize and wait for the debounced send + a TUI repaint window. */
  async resizeSettled(cols, rows, extraMs = 600) {
    this.resize(cols, rows);
    await sleep(RESIZE_SETTLE_MS + extraMs);
  }

  close() {
    if (this.resizeTimer) clearTimeout(this.resizeTimer);
    try { this.sock?.close(); } catch { /* closed */ }
    this.sock = null;
  }

  // ── buffer inspection ──────────────────────────────────────────────────────

  /** Every buffer line (scrollback + viewport), trailing blanks trimmed. */
  bufferLines() {
    const buf = this.term.buffer.active;
    const lines = [];
    for (let i = 0; i < buf.length; i++) {
      const line = buf.getLine(i);
      lines.push(line ? line.translateToString(true) : '');
    }
    while (lines.length && lines[lines.length - 1] === '') lines.pop();
    return lines;
  }

  bufferText() {
    return this.bufferLines().join('\n');
  }

  /** Only what is on screen right now (the last `rows` of the buffer). */
  viewportText() {
    const buf = this.term.buffer.active;
    const lines = [];
    for (let i = buf.baseY; i < buf.baseY + this.term.rows; i++) {
      const line = buf.getLine(i);
      lines.push(line ? line.translateToString(true) : '');
    }
    return lines.join('\n');
  }

  dump(label) {
    const lines = this.bufferLines();
    console.log(`\n===== ${label} (cols=${this.term.cols} rows=${this.term.rows} buflen=${lines.length}) =====`);
    console.log(lines.map((l, i) => String(i).padStart(4) + '|' + l).join('\n'));
    return lines;
  }
}

export function count(text, needle) {
  let n = 0;
  for (let i = text.indexOf(needle); i !== -1; i = text.indexOf(needle, i + needle.length)) n++;
  return n;
}

/** Poll until `fn()` is truthy or `timeoutMs` elapses; returns the last value. */
export async function waitFor(fn, timeoutMs = 30_000, intervalMs = 250) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const v = fn();
    if (v) return v;
    if (Date.now() > deadline) return v;
    await sleep(intervalMs);
  }
}

/** Wait until the client's buffer stops changing for `quietMs` (TUI settled). */
export async function waitBufferStable(client, { quietMs = 3000, timeoutMs = 120_000 } = {}) {
  const deadline = Date.now() + timeoutMs;
  let last = client.bufferText();
  let lastChange = Date.now();
  for (;;) {
    await sleep(500);
    const cur = client.bufferText();
    if (cur !== last) {
      last = cur;
      lastChange = Date.now();
    }
    if (Date.now() - lastChange >= quietMs) return true;
    if (Date.now() > deadline) return false;
  }
}
