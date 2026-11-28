import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join } from 'node:path';
import { apiCtx } from './seed';
import { bootPolyglotIde, daemonCall, type PolyglotWorkspaceSeed } from './ide-polyglot-fixtures';

// Unity Editor attach (rc.6 A1) on the isolated daemon: a Unity-shaped folder
// gets the debug-only "Attach to Unity Editor" launch target. Its label says
// whether an Editor has the project open; here the Playwright runner stands in
// for the Editor (Library/EditorInstance.json names its PID, and node's folder
// as app_path). Run is disabled with its title, and Debug without the Unity
// debugger installed offers its install. The attach itself is covered against a
// real Mono soft debugger by crates/otto-ide/tests/unity_attach_live.rs.

let seed: PolyglotWorkspaceSeed;

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'unity-attach'), { recursive: true });
  await page.screenshot({ path: join(dir, 'unity-attach', name) });
}

test.describe('Desktop IDE — Attach to Unity Editor', () => {
  test.describe.configure({ mode: 'serial' });
  test.use({ viewport: { width: 1280, height: 800 } });

  test.beforeAll(async () => {
    const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-unity-')));
    mkdirSync(join(root, 'Assets', 'Scripts'), { recursive: true });
    mkdirSync(join(root, 'ProjectSettings'), { recursive: true });
    mkdirSync(join(root, 'Library'), { recursive: true });
    writeFileSync(join(root, 'Assets', 'Scripts', 'Player.cs'), 'public class Player { public void Update() { } }\n');
    writeFileSync(join(root, 'ProjectSettings', 'ProjectVersion.txt'), 'm_EditorVersion: 6000.0.23f1\n');
    writeFileSync(
      join(root, 'Library', 'EditorInstance.json'),
      JSON.stringify({ process_id: process.pid, version: '6000.0.23f1', app_path: dirname(realpathSync(process.execPath)) }),
    );
    execFileSync('git', ['init', '-q'], { cwd: root });
    const c = await apiCtx();
    try {
      const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: 'IDE Unity E2E', root_path: root });
      seed = { base: c.base, token: c.token, workspaceId: ws.id, workspaceRoot: root, target: { kind: 'local', workspace_id: ws.id, root, connection_id: null } };
    } finally {
      await c.ctx.dispose();
    }
  });

  test.afterAll(() => {
    if (seed?.workspaceRoot) rmSync(seed.workspaceRoot, { recursive: true, force: true });
  });

  test.beforeEach(async ({}, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop-only launch flow');
  });

  for (const scheme of ['dark', 'light'] as const) {
    test(`the Unity target is debug-only, shows the running Editor and offers the debugger install (${scheme})`, async ({ page }) => {
      test.setTimeout(90_000);
      await bootPolyglotIde(page, seed, scheme);
      const selector = page.getByTestId('ide-run-config-select');
      await selector.click();
      const pop = page.getByTestId('ide-run-config-popover');
      const item = pop.locator('.run-item', { hasText: 'Attach to Unity Editor' });
      await expect(item).toContainText('Attach to Unity Editor · 6000.0.23f1 · running', { timeout: 20_000 });
      await shot(page, `target-menu-${scheme}.png`);
      await item.click();
      await expect(selector).toContainText('Attach to Unity Editor · 6000.0.23f1 · running');

      const run = page.getByTestId('ide-run-btn');
      await expect(run).toBeDisabled();
      await expect(run).toHaveAttribute('title', 'Attach targets can only be debugged');

      await page.getByTestId('ide-debug-btn').click();
      const card = page.getByTestId('ide-debug-error-card');
      await expect(card).toContainText('Unity debugger isn’t installed', { timeout: 30_000 });
      await expect(page.getByTestId('ide-debug-error-primary')).toHaveText('Install Unity debugger');
      await shot(page, `debug-card-${scheme}.png`);
    });
  }
});
