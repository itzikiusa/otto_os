import { request, type FullConfig } from '@playwright/test';
import { spawn } from 'node:child_process';
import { mkdtempSync, mkdirSync, writeFileSync } from 'node:fs';
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

export default async function globalSetup(_config: FullConfig): Promise<void> {
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
      // Point the agent runner at a binary that does not exist so any agent /
      // planner invocation (e.g. the discovery-swarm planner) fails FAST and
      // falls back to its fixed task set instead of waiting on a real `claude`
      // CLI startup/retry budget. The throwaway daemon never runs agents
      // meaningfully, so this only makes that already-doomed path quick + and
      // deterministic; no other spec exercises agent execution.
      CLAUDE_BIN: '/nonexistent/otto-e2e-no-claude',
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
