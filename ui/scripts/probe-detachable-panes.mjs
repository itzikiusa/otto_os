// Run the real bundled SPA in isolated native webviews against the standard
// throwaway E2E daemon. Build ui/ and the pane_spa_probe example first.
import { execFile } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';

process.chdir(fileURLToPath(new URL('..', import.meta.url)));
const binary = resolve('../apps/desktop/src-tauri/target/debug/examples/pane_spa_probe');
if (!existsSync(binary)) throw new Error('Build the pane_spa_probe desktop example before running this script.');
process.env.OTTO_E2E_SLOT = 'native-pane-probe';
process.env.OTTO_E2E_PORT = '7821';
process.env.OTTO_E2E_PW_PORT = '5201';
process.env.OTTO_E2E_SWEEP_ORPHANS = '0';
const { default: setup } = await import('../e2e/global-setup.ts');
const { default: teardown } = await import('../e2e/global-teardown.ts');
const scratch = mkdtempSync(join(tmpdir(), 'otto-native-pane-ui-'));
try {
  await setup({});
  const storage = JSON.parse(readFileSync('e2e/.auth-native-pane-probe/state.json', 'utf8'));
  const settings = Object.fromEntries(storage.origins[0].localStorage.map(({ name, value }) => [name, value]));
  const base = settings.otto_base;
  const token = settings.otto_token;
  if (new URL(base).port !== '7821' || !token) throw new Error('Invalid isolated fixture settings.');
  const response = await fetch(`${base}/api/v1/workspaces`, {
    method: 'POST', headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: 'Native pane probe', root_path: scratch }),
  });
  if (!response.ok) throw new Error(`Fixture workspace failed (${response.status})`);
  const { id: workspace } = await response.json();
  const config = join(scratch, 'config.json');
  writeFileSync(config, JSON.stringify({ base, token, workspace }), { mode: 0o600 });
  const { stdout, stderr } = await promisify(execFile)(binary, [], {
    env: { ...process.env, OTTO_PANE_UI_CONFIG: config }, timeout: 120_000, maxBuffer: 4 * 1024 * 1024,
  });
  process.stdout.write(stdout);
  process.stderr.write(stderr);
} finally {
  await teardown();
  rmSync(scratch, { recursive: true, force: true });
}
