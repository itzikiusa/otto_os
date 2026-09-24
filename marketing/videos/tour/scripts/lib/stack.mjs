// The capture stack: an ISOLATED throwaway ottod (temp data dir, its own port,
// fake agent CLIs first on PATH, file-backed secrets) plus a tiny static server
// for the production UI build. Mirrors ui/e2e/global-setup.ts — it never talks
// to the user's real daemon on :7700 and never reads ~/Library/Application
// Support/Otto. Everything it starts is torn down by `stop()`.

import { spawn, execSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { dirname } from 'node:path';
import { copyFileSync, chmodSync, createReadStream, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { tmpdir } from 'node:os';
import { extname, join } from 'node:path';

export const PORT = process.env.OTTO_E2E_PORT ?? '7811';
export const UI_PORT = process.env.OTTO_E2E_PW_PORT ?? '5211';
export const BASE = `http://127.0.0.1:${PORT}`;
export const API = `${BASE}/api/v1`;
export const UI = `http://localhost:${UI_PORT}`;
const PASSWORD = 'otto-tour-password';

const MIME = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.woff2': 'font/woff2',
  '.woff': 'font/woff',
  '.ttf': 'font/ttf',
  '.json': 'application/json',
  '.wasm': 'application/wasm',
  '.ico': 'image/x-icon',
  '.webmanifest': 'application/manifest+json',
};

/** Serve `dist` (the `vite build` output) with an index.html fallback. */
/** Every file under `dir`, keyed by its URL path (`/assets/x.js`). */
function listFiles(dir, prefix = '') {
  const out = new Map();
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const abs = join(dir, e.name);
    const key = `${prefix}/${e.name}`;
    if (e.isDirectory()) for (const [k, v] of listFiles(abs, key)) out.set(k, v);
    else if (e.isFile()) out.set(key, abs);
  }
  return out;
}

function serveStatic(dist) {
  // The build is listed once up front and requests only ever pick from that
  // list, so a request path never becomes a filesystem path. Unknown paths
  // fall back to index.html (SPA routing).
  const files = listFiles(dist);
  const index = join(dist, 'index.html');
  const server = createServer((req, res) => {
    const url = decodeURIComponent((req.url ?? '/').split('?')[0]);
    const file = files.get(url) ?? index;
    res.writeHead(200, {
      'Content-Type': MIME[extname(file)] ?? 'application/octet-stream',
      'Cache-Control': 'no-store',
    });
    createReadStream(file).pipe(res);
  });
  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(Number(UI_PORT), '127.0.0.1', () => resolve(server));
  });
}

async function waitHealthy(ms = 120_000) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    try {
      const r = await fetch(`${API}/health`);
      if (r.ok) return;
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  throw new Error(`[tour] daemon never became healthy at ${API}/health`);
}

/** Kill clickhouse servers the daemon's usage engine spawned for `dataDir`
 *  (watchdog first so it can't respawn its child). */
function killClickhouseFor(dataDir) {
  try {
    const out = execSync('ps -axo pid=,ppid=,command=', { encoding: 'utf8' });
    for (const line of out.split('\n')) {
      if (!line.includes(dataDir) || !line.includes('clickhouse')) continue;
      const m = line.trim().match(/^(\d+)\s+(\d+)/);
      if (!m) continue;
      for (const p of [Number(m[2]), Number(m[1])]) {
        if (p > 1) {
          try {
            process.kill(p, 'SIGKILL');
          } catch {
            /* gone */
          }
        }
      }
    }
  } catch {
    /* ps unavailable */
  }
}

/**
 * Start the daemon + UI server and onboard a root user.
 * @param {{ bin: string, dist: string, log?: string, prepare?: (dirs: { dataDir: string, home: string, fakeBin: string }) => Promise<void> | void }} opts
 */
export async function startStack({ bin, dist, log, prepare }) {
  if (!existsSync(bin)) throw new Error(`[tour] ottod binary not found: ${bin} (set OTTO_E2E_BIN)`);
  if (!existsSync(join(dist, 'index.html'))) throw new Error(`[tour] UI build missing: ${dist}`);
  for (const p of [PORT, UI_PORT]) {
    try {
      const busy = execSync(`lsof -nP -iTCP:${p} -sTCP:LISTEN -t`, { encoding: 'utf8' }).trim();
      if (busy) throw new Error(`[tour] port ${p} is already in use (pid ${busy}) — refusing to start`);
    } catch (e) {
      if (String(e.message).includes('refusing')) throw e;
    }
  }
  const dataDir = mkdtempSync(join(tmpdir(), 'otto-tour-'));
  // Harmless stand-ins for the agent CLIs: sessions launch providers by bare
  // name, so without these a claude/codex session would start the user's REAL
  // CLI on their account.
  const fakeBin = join(dataDir, 'fake-agent-bin');
  mkdirSync(fakeBin, { recursive: true });
  for (const cli of ['claude', 'codex', 'agy', 'gemini', 'grok']) {
    const shim = join(fakeBin, cli);
    writeFileSync(shim, `#!/bin/sh\nexec "${join(fakeBin, 'agent-sim')}" ${cli}\n`);
    chmodSync(shim, 0o755);
  }
  // A scripted "agent": prints a believable transcript so terminal tiles look
  // alive, then echoes input. Pure shell — nothing leaves the machine.
  copyFileSync(join(dirname(fileURLToPath(import.meta.url)), 'agent-sim.sh'), join(fakeBin, 'agent-sim'));
  chmodSync(join(fakeBin, 'agent-sim'), 0o755);

  // A fake HOME: the daemon scans ~/.claude / ~/.codex for past transcripts
  // (History) and ~/.gitconfig etc. — point all of it at the temp dir so the
  // user's real agent history is never read. Seeded transcripts go here.
  const home = join(dataDir, 'home');
  mkdirSync(home, { recursive: true });
  writeFileSync(join(home, '.gitconfig'), '[user]\n\tname = Maya Chen\n\temail = maya@example.com\n[init]\n\tdefaultBranch = main\n');
  // Shell sessions: a neutral prompt (never the host's user@machine).
  writeFileSync(join(home, '.zshrc'), "PROMPT='%F{green}maya@acme%f %F{blue}%1~%f %# '\nunsetopt PROMPT_SP\n");
  writeFileSync(join(home, '.bashrc'), "PS1='maya@acme \\W $ '\n");
  // The usage engine needs a clickhouse binary; expose ONLY that one.
  for (const c of [process.env.OTTO_TOUR_CLICKHOUSE, `${process.env.HOME}/.local/bin/clickhouse`, '/opt/homebrew/bin/clickhouse']) {
    if (c && existsSync(c)) {
      try {
        (await import('node:fs')).symlinkSync(c, join(fakeBin, 'clickhouse'));
      } catch {
        /* ignore */
      }
      break;
    }
  }
  // Codex's presence check reads ~/.codex/auth.json — a placeholder keeps the
  // "re-login needed" notice off camera (the fake codex never uses it).
  mkdirSync(join(home, '.codex'), { recursive: true });
  writeFileSync(join(home, '.codex', 'auth.json'), JSON.stringify({ OPENAI_API_KEY: 'demo-placeholder' }));
  if (prepare) await prepare({ dataDir, home, fakeBin });
  const logFd = log ? (await import('node:fs')).openSync(log, 'a') : 'ignore';
  const child = spawn('nice', ['-n', '10', bin], {
    env: {
      ...process.env,
      HOME: home,
      OTTO_DATA_DIR: dataDir,
      OTTO_PORT: PORT,
      PATH: `${fakeBin}:${process.env.PATH ?? ''}`,
      OTTO_SELF_IMPROVE: '0',
      OTTO_CLI_UPDATE: '0',
      OTTO_E2E: '1',
      CLAUDE_BIN: '/nonexistent/otto-tour-no-claude',
      OTTO_PLUGINS_HOME: join(dataDir, 'plugins-home'),
      OTTO_SECRETS: 'file',
      // History reads the same fake-HOME transcript trees the usage tailer does.
      OTTO_TRANSCRIPT_ROOTS: `${join(home, '.claude', 'projects')}:${join(home, '.codex', 'sessions')}`,
    },
    stdio: ['ignore', logFd, logFd],
  });
  const server = await serveStatic(dist);
  await waitHealthy();
  const onb = await fetch(`${API}/onboarding/root`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ password: PASSWORD, display_name: 'Maya Chen' }),
  });
  if (!onb.ok) throw new Error(`[tour] onboarding failed: ${onb.status} ${await onb.text()}`);
  const { token } = await onb.json();

  const stop = async () => {
    try {
      process.kill(child.pid, 'SIGKILL');
    } catch {
      /* gone */
    }
    killClickhouseFor(dataDir);
    await new Promise((r) => server.close(() => r()));
    try {
      rmSync(dataDir, { recursive: true, force: true });
    } catch {
      /* ignore */
    }
  };
  return { token, dataDir, home, fakeBin, pid: child.pid, stop };
}

/** Browser storage state that signs the UI into the throwaway daemon. */
export function storageState(token, extra = []) {
  return {
    cookies: [],
    origins: [
      {
        origin: UI,
        localStorage: [
          { name: 'otto_token', value: token },
          { name: 'otto_base', value: BASE },
          ...extra,
        ],
      },
    ],
  };
}

/** Minimal authed JSON client for seeding. */
export function client(token) {
  const call = async (method, path, body) => {
    const r = await fetch(`${API}${path}`, {
      method,
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const text = await r.text();
    if (!r.ok) throw new Error(`${method} ${path} → ${r.status} ${text.slice(0, 400)}`);
    try {
      return text ? JSON.parse(text) : null;
    } catch {
      return text;
    }
  };
  return {
    get: (p) => call('GET', p),
    post: (p, b) => call('POST', p, b ?? {}),
    put: (p, b) => call('PUT', p, b ?? {}),
    patch: (p, b) => call('PATCH', p, b ?? {}),
    del: (p) => call('DELETE', p),
    /** Like post, but logs and returns null instead of throwing. */
    try: async (method, p, b) => {
      try {
        return await call(method, p, b);
      } catch (e) {
        console.warn(`[seed] ${e.message}`);
        return null;
      }
    },
  };
}

