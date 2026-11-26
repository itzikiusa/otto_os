// Capture the Workflows chapter → public/capture/wf/: the Runs inbox, a
// 12-reviewer fan-out on its run page (stills + a clip), a reviewer's
// conversation, a failed run, the same run on a phone, a goal run with its
// criteria, the editor's model and effort, Optimize and the Goal form.
// Runs scripts/workflows/film.spec.ts with the UI's Playwright against the
// ISOLATED e2e daemon (ui/e2e/global-setup.ts: temp data dir, its own port,
// the build's fake agent CLIs on its PATH: otto-fake-claude / -codex / -agent
// next to OTTO_E2E_BIN). Nothing here can reach a real agent CLI: the process
// env is built from scratch, PATH is the system dirs plus node and npm.
//
//   OTTO_E2E_BIN=/path/to/target/debug/ottod node scripts/capture-workflows.mjs
//
// Env: OTTO_E2E_PORT (7897), OTTO_E2E_PW_PORT (5297).
import { execFileSync } from 'node:child_process';
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const tour = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const ui = resolve(tour, '../../../ui');
const work = join(tour, '.cache/workflows');
const shots = join(work, 'shots');
const out = join(tour, 'public/capture/wf');
if (!process.env.OTTO_E2E_BIN) throw new Error('set OTTO_E2E_BIN to a daemon built from this tree (its fakes sit next to it)');
rmSync(work, { recursive: true, force: true });
mkdirSync(shots, { recursive: true });

const SLOT = 'tourwf';
const PORT = process.env.OTTO_E2E_PORT ?? '7897';
const PW_PORT = process.env.OTTO_E2E_PW_PORT ?? '5297';
const auth = join(ui, 'e2e/.auth-' + SLOT);
writeFileSync(join(work, 'film.spec.ts'), readFileSync(join(tour, 'scripts/workflows/film.spec.ts'), 'utf8')
  .replaceAll("'UI_E2E/", "'" + join(ui, 'e2e') + '/')
  .replaceAll("'TOUR_LIB/", "'" + join(tour, 'scripts/lib') + '/'));
symlinkSync(join(ui, 'node_modules'), join(work, 'node_modules'));
writeFileSync(join(work, 'playwright.config.ts'), `import { defineConfig, devices } from '@playwright/test';
const use = { ...devices['Desktop Chrome'], viewport: { width: 1600, height: 900 }, deviceScaleFactor: 2, colorScheme: 'dark' as const, storageState: ${JSON.stringify(join(auth, 'state.json'))} };
export default defineConfig({
  testDir: ${JSON.stringify(work)},
  globalSetup: ${JSON.stringify(join(ui, 'e2e/global-setup.ts'))},
  globalTeardown: ${JSON.stringify(join(ui, 'e2e/global-teardown.ts'))},
  outputDir: ${JSON.stringify(join(work, 'test-results'))},
  timeout: 900_000, expect: { timeout: 15_000 }, workers: 1, retries: 0, reporter: [['line']],
  use: { baseURL: 'http://localhost:${PW_PORT}', ...use },
  webServer: { command: 'npm run dev -- --port ${PW_PORT} --strictPort', cwd: ${JSON.stringify(ui)}, url: 'http://localhost:${PW_PORT}', reuseExistingServer: false, timeout: 90_000 },
  projects: [{ name: 'desktop-browser', use }],
});
`);

const bin = join(work, 'bin');
mkdirSync(bin);
const which = (cmd) => execFileSync('/usr/bin/which', [cmd], { encoding: 'utf8' }).trim();
for (const cmd of ['node', 'npm', 'npx']) symlinkSync(cmd === 'node' ? process.execPath : which(cmd), join(bin, cmd));
// The system temp dir: the agent sandbox lets reviewers write their findings there.
const tmp = process.env.TMPDIR ?? '/tmp/';
rmSync(join(tmp, 'acme-film'), { recursive: true, force: true });
// Not under the temp dir: the sandbox refuses a writable root that holds HOME.
const home = mkdtempSync('/private/var/tmp/acme-home-');
try {
  execFileSync(process.execPath, [join(ui, 'node_modules/@playwright/test/cli.js'), 'test', '--config', join(work, 'playwright.config.ts')], {
    cwd: ui,
    stdio: 'inherit',
    env: {
      HOME: home,
      PATH: bin + ':/usr/bin:/bin:/usr/sbin:/sbin',
      TMPDIR: tmp,
      LANG: 'en_US.UTF-8',
      CI: '1',
      PLAYWRIGHT_BROWSERS_PATH: process.env.PLAYWRIGHT_BROWSERS_PATH ?? join(process.env.HOME ?? '', 'Library/Caches/ms-playwright'),
      OTTO_E2E_BIN: process.env.OTTO_E2E_BIN,
      OTTO_E2E_SLOT: SLOT,
      OTTO_E2E_PORT: PORT,
      OTTO_E2E_PW_PORT: PW_PORT,
      OTTO_E2E_SWEEP_ORPHANS: '0',
      OTTO_E2E_REQUIRE_FAKES: '1',
      // The inbox shows four live runs at once (default: two run slots).
      OTTO_WF_MAX_PARALLEL_RUNS: '4',
      // A daemon built before 5ad2d387c overflows tokio's 2 MiB workers on a run's future tree.
      ...(process.env.RUST_MIN_STACK ? { RUST_MIN_STACK: process.env.RUST_MIN_STACK } : {}),
      OTTO_TOUR_FILM_DIR: shots,
      OTTO_TOUR_STATE: join(auth, 'state.json'),
      OTTO_TOUR_DAEMON_META: join(auth, 'daemon.json'),
      OTTO_TOUR_BASE_URL: 'http://localhost:' + PW_PORT,
    },
  });
  mkdirSync(out, { recursive: true });
  cpSync(shots, out, { recursive: true });
  console.log('[tour] workflows stills → ' + out);
} finally {
  rmSync(home, { recursive: true, force: true });
  rmSync(join(tmp, 'acme-film'), { recursive: true, force: true });
  rmSync(auth, { recursive: true, force: true });
}
