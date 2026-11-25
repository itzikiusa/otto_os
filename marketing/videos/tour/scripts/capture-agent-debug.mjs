// Capture the "Debug with an agent" and "DB Watch" stills → public/capture/dbg/.
// Reuses the real-debugger e2e (ui/e2e/desktop-ide-agent-debug-real.spec.ts):
// its helpers and seed (everything before its first test) plus the film's own
// test (scripts/agent-debug/film.ts), run by the UI's Playwright against an
// ISOLATED throwaway daemon (ui/e2e/global-setup.ts: temp data dir, its own
// port). The agent is a stand-in `shell` session; delve, Go and the MariaDB +
// Redis containers are real. A temp HOME with a neutral prompt keeps the
// machine's user and host off camera.
//
//   OTTO_E2E_BIN=/path/to/ottod (test-util build) OTTO_E2E_GO_DIR=/dir/with/dlv \
//     node scripts/capture-agent-debug.mjs
//
// Env: OTTO_E2E_PORT (7731), OTTO_E2E_PW_PORT (5731), OTTO_E2E_APP_PORT (5733);
// GOMODCACHE / GOPROXY / GOFLAGS / GOCACHE pass through (an offline module cache works).
import { execFileSync } from 'node:child_process';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const ui = resolve(tour, '../../../ui');
const work = join(tour, '.cache/agent-debug');
const shots = join(work, 'shots');
const out = join(tour, 'public/capture/dbg');
rmSync(work, { recursive: true, force: true });
mkdirSync(shots, { recursive: true });

// The e2e's helpers + seed, then the film test. The workspace and its folder
// get film names (the e2e's are random).
const e2e = readFileSync(join(ui, 'e2e/desktop-ide-agent-debug-real.spec.ts'), 'utf8');
const cut = e2e.indexOf("\ntest('A+C+D");
if (cut < 0) throw new Error('the agent-debug e2e changed: no "A+C+D" test to cut before');
const swap = (s, a, b) => {
  if (!s.includes(a)) throw new Error(`the agent-debug e2e changed: "${a}" not found`);
  return s.replace(a, b);
};
let spec = e2e.slice(0, cut);
spec = swap(spec, "from './seed';", `from '${join(ui, 'e2e/seed')}';`);
spec = swap(spec, '{ name: `Agent debug ${RUN}`, root_path: root }', "{ name: 'Acme Orders', root_path: root }");
spec = swap(spec, "root = tempDir('otto-agentdbg-');", "root = join(tempDir('otto-film-'), 'acme-orders');");
writeFileSync(join(work, 'film.spec.ts'), `${spec}\n${readFileSync(join(tour, 'scripts/agent-debug/film.ts'), 'utf8')}`);
symlinkSync(join(ui, 'node_modules'), join(work, 'node_modules'));
const PW_PORT = process.env.OTTO_E2E_PW_PORT ?? '5731';
const SLOT = 'tourfilm';
writeFileSync(join(work, 'playwright.config.ts'), `import { defineConfig, devices } from '@playwright/test';
const STATE = ${JSON.stringify(join(ui, `e2e/.auth-${SLOT}/state.json`))};
const use = { ...devices['Desktop Chrome'], viewport: { width: 1600, height: 900 }, deviceScaleFactor: 2, storageState: STATE };
export default defineConfig({
  testDir: ${JSON.stringify(work)},
  globalSetup: ${JSON.stringify(join(ui, 'e2e/global-setup.ts'))},
  globalTeardown: ${JSON.stringify(join(ui, 'e2e/global-teardown.ts'))},
  outputDir: ${JSON.stringify(join(work, 'test-results'))},
  timeout: 600_000, expect: { timeout: 15_000 }, workers: 1, retries: 0, reporter: [['line']],
  use: { baseURL: 'http://localhost:${PW_PORT}', ...use },
  webServer: { command: 'npm run dev -- --port ${PW_PORT} --strictPort', cwd: ${JSON.stringify(ui)}, url: 'http://localhost:${PW_PORT}', reuseExistingServer: false, timeout: 90_000 },
  projects: [{ name: 'desktop-browser', use }],
});
`);

// No agent CLI may be reachable (the daemon probes providers by bare name):
// PATH is the system dirs plus a folder holding only node, npm, npx and docker.
const bin = join(work, 'bin');
mkdirSync(bin);
const which = (cmd) => execFileSync('/usr/bin/which', [cmd], { encoding: 'utf8' }).trim();
for (const cmd of ['node', 'npm', 'npx', 'docker']) symlinkSync(cmd === 'node' ? process.execPath : which(cmd), join(bin, cmd));
const home = mkdtempSync(join(tmpdir(), 'otto-film-home-'));
writeFileSync(join(home, '.zshrc'), "PROMPT='%F{green}maya@acme%f %F{blue}%1~%f %# '\n");
const demoTmp = '/private/tmp/acme-demo';
mkdirSync(demoTmp, { recursive: true });
try {
  execFileSync(process.execPath, [join(ui, 'node_modules/@playwright/test/cli.js'), 'test', '--config', join(work, 'playwright.config.ts')], {
    cwd: ui,
    stdio: 'inherit',
    env: {
      ...process.env,
      HOME: home,
      PATH: `${bin}:/usr/bin:/bin:/usr/sbin:/sbin`,
      TMPDIR: `${demoTmp}/`,
      CI: '1',
      PLAYWRIGHT_BROWSERS_PATH: process.env.PLAYWRIGHT_BROWSERS_PATH ?? join(process.env.HOME ?? '', 'Library/Caches/ms-playwright'),
      OTTO_E2E_SLOT: SLOT,
      OTTO_E2E_PORT: process.env.OTTO_E2E_PORT ?? '7731',
      OTTO_E2E_PW_PORT: PW_PORT,
      OTTO_E2E_APP_PORT: process.env.OTTO_E2E_APP_PORT ?? '5733',
      OTTO_E2E_SWEEP_ORPHANS: '0',
      OTTO_TOUR_FILM_DIR: shots,
      // The temp HOME must not move Go's module cache (offline runs read it) or its build cache.
      GOMODCACHE: process.env.GOMODCACHE ?? join(process.env.HOME ?? '', 'go/pkg/mod'),
      GOCACHE: process.env.GOCACHE ?? join(tour, '.cache/gocache'),
    },
  });
  mkdirSync(out, { recursive: true });
  cpSync(shots, out, { recursive: true });
  console.log(`[tour] agent-debug stills → ${out}`);
} finally {
  rmSync(home, { recursive: true, force: true });
}
