import { request, type FullConfig } from '@playwright/test';
import { execSync, spawn } from 'node:child_process';
import { existsSync, mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
import { tmpdir, homedir } from 'node:os';
import { join } from 'node:path';

// Stand up an ISOLATED ottod for the whole E2E run:
//   - fresh temp data dir  -> fresh SQLite, no real sessions/DBs touched
//   - dedicated port 7799  -> never collides with the real daemon on 7700
// Then onboard a known root user and write a Playwright storageState that
// injects the resulting token (+ otto_base pointing at the test daemon) into
// localStorage for the Vite origin. global-teardown kills it and removes the dir.

const PORT = process.env.OTTO_E2E_PORT ?? '7799';
const API = `http://127.0.0.1:${PORT}/api/v1`;
const OTTOD =
  process.env.OTTO_E2E_BIN ??
  join(homedir(), 'Library', 'Application Support', 'Otto', 'bin', 'ottod');
const SLOT = process.env.OTTO_E2E_SLOT ?? '0';
const PW_PORT = process.env.OTTO_E2E_PW_PORT ?? '5173';
const UI_ORIGIN = process.env.OTTO_E2E_UI ?? `http://localhost:${PW_PORT}`;
const STATE_DIR = join(process.cwd(), 'e2e', `.auth-${SLOT}`);
const STATE_FILE = join(STATE_DIR, 'state.json');
const META_FILE = join(STATE_DIR, 'daemon.json');
const PASSWORD = 'otto-e2e-password';

// A route that only exists on a daemon built from this tree — used to detect a
// STALE installed binary (new UI against an old daemon → every new-feature spec
// fails with 404s that look like UI bugs). Axum's bare fallback answers a
// missing route with an EMPTY 404; the app's own not-found is a JSON problem.
const NEW_ROUTE_PROBE = '/sessions/e2e-probe/transcript';

export default async function globalSetup(_config: FullConfig): Promise<void> {
  sweepOrphanedClickhouse();
  if (!process.env.OTTO_E2E_BIN) {
    // eslint-disable-next-line no-console
    console.warn(
      `[e2e] WARNING: OTTO_E2E_BIN is unset — running the INSTALLED daemon (${OTTOD}). ` +
        'Specs for routes added in this tree will 404. Set OTTO_E2E_BIN=<repo>/target/debug/ottod.',
    );
  }
  if (!existsSync(OTTOD)) {
    throw new Error(`[e2e] ottod binary not found at ${OTTOD} (set OTTO_E2E_BIN or install Otto)`);
  }
  const dataDir = mkdtempSync(join(tmpdir(), 'otto-e2e-'));
  // eslint-disable-next-line no-console
  console.log(`[e2e] launching test daemon: ${OTTOD}\n[e2e]   OTTO_DATA_DIR=${dataDir} OTTO_PORT=${PORT}`);

  const child = spawn(OTTOD, [], {
    env: {
      ...process.env,
      OTTO_DATA_DIR: dataDir,
      OTTO_PORT: PORT,
      // Keep the throwaway daemon lean / non-networked.
      OTTO_SELF_IMPROVE: '0',
      OTTO_CLI_UPDATE: '0', // isolated tests never update the host's agent CLIs
      // Route agent turns (Discovery Chat, Canvas assist) through the orchestrator's
      // deterministic offline E2E stub instead of spawning a real `claude` PTY, so
      // those feature flows are reproducible + network-free. The stub branches on an
      // `OTTO_TASK:` sentinel embedded in each feature's prompt. See
      // crates/otto-orchestrator/src/e2e_stub.rs.
      OTTO_E2E: '1',
      // Point the agent runner at a binary that does not exist so any agent /
      // planner invocation (e.g. the discovery-swarm planner) fails FAST and
      // falls back to its fixed task set instead of waiting on a real `claude`
      // CLI startup/retry budget. The throwaway daemon never runs agents
      // meaningfully, so this only makes that already-doomed path quick + and
      // deterministic; no other spec exercises agent execution.
      CLAUDE_BIN: '/nonexistent/otto-e2e-no-claude',
      // Isolate runtime plugins: without this, a spec that installs a plugin
      // would write into the user's REAL ~/otto-plugins.
      OTTO_PLUGINS_HOME: join(dataDir, 'plugins-home'),
      // Never write test secrets into the user's macOS Keychain — file-backed
      // secrets live in the temp data dir and vanish with it.
      OTTO_SECRETS: process.env.OTTO_SECRETS ?? 'file',
    },
    stdio: ['ignore', 'inherit', 'inherit'],
    detached: false,
  });
  child.on('error', (e: Error) => {
    throw new Error(`[e2e] failed to spawn test daemon (${OTTOD}): ${e.message}`);
  });

  const ctx = await request.newContext();

  // Wait for health.
  const deadline = Date.now() + 90_000;
  let healthy = false;
  while (Date.now() < deadline) {
    try {
      const r = await ctx.get(`${API}/health`, { timeout: 2_000 });
      if (r.ok()) {
        healthy = true;
        break;
      }
    } catch {
      /* not up yet */
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  if (!healthy) {
    try {
      child.kill('SIGKILL');
    } catch {
      /* ignore */
    }
    throw new Error(`[e2e] test daemon never became healthy at ${API}/health`);
  }

  // Onboard root (valid only while 0 users — always true on a fresh data dir).
  const onb = await ctx.post(`${API}/onboarding/root`, {
    data: { password: PASSWORD, display_name: 'E2E Root' },
  });
  if (!onb.ok()) {
    child.kill('SIGKILL');
    throw new Error(`[e2e] onboarding failed: ${onb.status()} ${await onb.text()}`);
  }
  const { token } = (await onb.json()) as { token: string };

  // Stale-daemon probe: a route from this tree must be ROUTED (any status but a
  // bare 404). 404 with a JSON problem body = routed, session just doesn't exist.
  const probe = await ctx.get(`${API}${NEW_ROUTE_PROBE}`, {
    headers: { Authorization: `Bearer ${token}` },
  });
  if (probe.status() === 404) {
    let routed = false;
    try {
      routed = typeof ((await probe.json()) as { code?: unknown }).code === 'string';
    } catch {
      routed = false;
    }
    if (!routed) {
      child.kill('SIGKILL');
      throw new Error(
        `[e2e] stale daemon: ${OTTOD} does not serve ${NEW_ROUTE_PROBE} (bare 404). ` +
          'Rebuild with `cargo build -p ottod` and set OTTO_E2E_BIN=<repo>/target/debug/ottod.',
      );
    }
  }

  // Persist storageState (token + base) for the UI origin, and daemon meta for
  // teardown.
  mkdirSync(STATE_DIR, { recursive: true });
  writeFileSync(
    STATE_FILE,
    JSON.stringify(
      {
        cookies: [],
        origins: [
          {
            origin: UI_ORIGIN,
            localStorage: [
              { name: 'otto_token', value: token },
              { name: 'otto_base', value: `http://127.0.0.1:${PORT}` },
            ],
          },
        ],
      },
      null,
      2,
    ),
  );
  writeFileSync(META_FILE, JSON.stringify({ pid: child.pid, dataDir, port: PORT }));
  await ctx.dispose();
  // eslint-disable-next-line no-console
  console.log(`[e2e] test daemon ready (pid ${child.pid}); root onboarded.`);
}

/** Self-heal: kill clickhouse servers leaked by INTERRUPTED past runs (teardown
 *  never fired — Ctrl+C, crash). A leaked server's config path points into a
 *  temp `otto-*` data dir; if that dir is GONE the server is a zombie. Live
 *  parallel slots keep their dirs, so they are never touched. Watchdog (parent)
 *  dies first so it can't respawn the child. */
function sweepOrphanedClickhouse(): void {
  try {
    const out = execSync('ps -axo pid=,ppid=,command=', { encoding: 'utf8' });
    for (const line of out.split('\n')) {
      const cfg = / (--config-file=)(.*\/otto-[^/]+)\/clickhouse\/server\/config\.xml/.exec(line);
      if (!cfg || !line.includes('clickhouse')) continue;
      if (existsSync(cfg[2])) continue; // data dir alive → possibly a live run
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
      // eslint-disable-next-line no-console
      console.log(`[e2e] reaped orphaned clickhouse (data dir gone): ${cfg[2]}`);
    }
  } catch {
    /* ps unavailable — skip */
  }
}
