import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport } from './helpers';
import { bootPolyglotIde, daemonCall, openPolyglotFile, PY_PIPELINE_SOURCE, type PolyglotWorkspaceSeed } from './ide-polyglot-fixtures';

// IDE launch life on a REAL Python debug session (built-in bdb adapter):
//  I2-01 Run / Debug act on the file being looked at: with `review.py` open and a
//        saved configuration for `pipeline.py`, Debug launches review.py; the
//        launch popover pins "Current file" first; a README can't be the target.
//  I2-03 A second Debug while paused asks first; "Stop and debug again" ends the
//        old session on the daemon before the new one starts — no live orphan.

const REVIEW_SOURCE = [
  'import time',
  '',
  '',
  'def main():',
  '    total = 0',
  '    for i in range(3):',
  '        total += i',
  '    return total',
  '',
  '',
  'if __name__ == "__main__":',
  '    print("total", main())',
  '',
].join('\n');

interface DebugSnap { id: string; state: string; launch?: { program?: string } | null }

let seed: PolyglotWorkspaceSeed;

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-DBG-SESSION'), { recursive: true });
  await page.screenshot({ path: join(dir, 'fix-DBG-SESSION', name) });
}

async function clickLineNumber(page: Page, line: number): Promise<void> {
  const lineEl = page
    .locator('.ide-editor-area .cm-lineNumbers .cm-gutterElement')
    .filter({ hasText: new RegExp(`^\\s*${line}\\s*$`) })
    .first();
  await expect(lineEl).toBeVisible({ timeout: 10_000 });
  const box = await lineEl.boundingBox();
  expect(box).toBeTruthy();
  await page.mouse.click(box!.x + box!.width / 2, box!.y + box!.height / 2);
}

async function liveSessions(): Promise<DebugSnap[]> {
  const c = await apiCtx();
  try {
    const r = await daemonCall<{ sessions: DebugSnap[] }>(c.ctx, c.base, c.token, 'GET', '/ide/debug/sessions');
    return r.sessions.filter((s) => s.state === 'starting' || s.state === 'running' || s.state === 'paused');
  } finally {
    await c.ctx.dispose();
  }
}

async function stopAll(ids: string[]): Promise<void> {
  const c = await apiCtx();
  try {
    for (const id of ids) {
      await c.ctx
        .post(`${c.base}/api/v1/ide/debug/sessions/${encodeURIComponent(id)}/command`, { data: { action: 'stop' } })
        .catch(() => undefined);
    }
  } finally {
    await c.ctx.dispose();
  }
}

test.describe('Desktop IDE — launch target and one live debug session', () => {
  test.describe.configure({ mode: 'serial' });
  test.use({ viewport: { width: 1280, height: 800 } });

  test.beforeAll(async () => {
    test.setTimeout(60_000);
    const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-launch-')));
    mkdirSync(join(root, 'pyapp'), { recursive: true });
    writeFileSync(join(root, 'pyapp', 'pipeline.py'), PY_PIPELINE_SOURCE);
    writeFileSync(join(root, 'pyapp', 'review.py'), REVIEW_SOURCE);
    writeFileSync(join(root, 'README.md'), '# Launch target fixture\n');
    execFileSync('git', ['init', '-q'], { cwd: root });
    const c = await apiCtx();
    try {
      const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', {
        name: 'IDE launch target E2E', root_path: root,
      });
      const target = { kind: 'local' as const, workspace_id: ws.id, root, connection_id: null };
      await daemonCall(c.ctx, c.base, c.token, 'POST', '/ide/run-configs', {
        target, id: 'py-settlement-debug', name: 'Python Settlement Debug', runner: 'python', command: 'python3',
        args: [], cwd: root, target_path: join(root, 'pyapp', 'pipeline.py'),
      });
      seed = { base: c.base, token: c.token, workspaceId: ws.id, workspaceRoot: root, target };
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

  test('I2-01 + I2-03: Debug launches the open file; a second Debug asks and leaves no orphan', async ({ page }) => {
    test.setTimeout(150_000);
    const started: string[] = [];
    try {
      await bootPolyglotIde(page, seed, 'dark');
      await openPolyglotFile(page, 'pyapp', 'review.py');
      await expect(page.locator('.ide-editor-area .cm-content').first()).toContainText('total += i');

      // The selector names the file being looked at, not the saved pipeline.py config.
      const selector = page.getByTestId('ide-run-config-select');
      await expect(selector).toContainText('Current file (review.py)');
      await selector.click();
      const pop = page.getByTestId('ide-run-config-popover');
      await expectFullyInViewport(page, pop, 'launch popover');
      const first = pop.locator('.run-item').first();
      await expect(first).toHaveAttribute('data-testid', 'ide-run-target-current-file');
      await expect(first).toContainText('Current file — review.py');
      await expect(first).toHaveAttribute('aria-current', 'true');
      await expect(pop).toContainText('Python Settlement Debug');
      await shot(page, 'run-config-popover-dark.png');
      await page.keyboard.press('Escape');
      await expect(pop).toBeHidden();

      await clickLineNumber(page, 7);
      await expect(page.locator('.ide-editor-area [data-testid="ide-bp-marker"]')).toHaveCount(1);

      const firstStart = page.waitForResponse(
        (r) => r.request().method() === 'POST' && /\/api\/v1\/ide\/debug\/sessions$/.test(r.url()),
        { timeout: 20_000 },
      );
      await page.getByTestId('ide-debug-btn').click();
      const firstResp = await firstStart;
      const firstBody = firstResp.request().postDataJSON() as { launch: { program: string }; config_id?: string };
      expect(firstBody.launch.program.endsWith('/pyapp/review.py')).toBe(true);
      expect(firstBody.config_id ?? null).toBeNull();
      const firstId = ((await firstResp.json()) as { session: DebugSnap }).session.id;
      started.push(firstId);

      const dbgPanel = page.getByTestId('ide-debugger-panel');
      await expect(dbgPanel).toContainText(/Paused/i, { timeout: 30_000 });

      // A second Debug while paused asks first; Cancel keeps the session and launches nothing.
      let extraLaunches = 0;
      const countLaunch = (r: { method(): string; url(): string }): void => {
        if (r.method() === 'POST' && /\/api\/v1\/ide\/debug\/sessions$/.test(r.url())) extraLaunches++;
      };
      page.on('request', countLaunch);
      await page.getByTestId('ide-debug-btn').click();
      const ask = page.getByRole('dialog').filter({ hasText: /Stop it and start a new debug session/ });
      await expect(ask).toBeVisible({ timeout: 10_000 });
      await expect(ask).toContainText('review.py is paused');
      await shot(page, 'debug-again-ask-dark.png');
      await ask.getByRole('button', { name: 'Cancel' }).click();
      await expect(ask).toBeHidden();
      await expect(dbgPanel).toContainText(/Paused/i);
      expect(extraLaunches).toBe(0);
      expect((await liveSessions()).map((s) => s.id)).toContain(firstId);

      // "Stop and debug again": the old session ends on the daemon, then one new session starts.
      const secondStart = page.waitForResponse(
        (r) => r.request().method() === 'POST' && /\/api\/v1\/ide\/debug\/sessions$/.test(r.url()),
        { timeout: 20_000 },
      );
      await page.getByTestId('ide-debug-btn').click();
      await expect(ask).toBeVisible({ timeout: 10_000 });
      await ask.getByRole('button', { name: 'Stop and debug again' }).click();
      const secondId = ((await (await secondStart).json()) as { session: DebugSnap }).session.id;
      started.push(secondId);
      page.off('request', countLaunch);
      expect(secondId).not.toBe(firstId);
      await expect(dbgPanel).toContainText(/Paused/i, { timeout: 30_000 });
      const live = await liveSessions();
      expect(live.map((s) => s.id), 'only the new session is live on the daemon').toEqual([secondId]);
    } finally {
      await stopAll(started);
    }
  });

  test('I2-01: light popover; a README is not a launch target and Run explains instead of guessing', async ({ page }) => {
    test.setTimeout(90_000);
    await bootPolyglotIde(page, seed, 'light');
    await openPolyglotFile(page, 'pyapp', 'review.py');
    const selector = page.getByTestId('ide-run-config-select');
    await expect(selector).toContainText('Current file (review.py)');
    await selector.click();
    const pop = page.getByTestId('ide-run-config-popover');
    await expect(pop.getByTestId('ide-run-target-current-file')).toBeVisible();
    await shot(page, 'run-config-popover-light.png');
    await page.keyboard.press('Escape');

    await openPolyglotFile(page, 'README.md');
    await expect(selector).toContainText('Current file (README.md)');
    await selector.click();
    const row = pop.getByTestId('ide-run-target-current-file');
    await expect(row).toBeDisabled();
    await expect(row).toContainText('Can’t run this file type');
    await shot(page, 'run-config-popover-readme-light.png');
    await page.keyboard.press('Escape');

    let runs = 0;
    page.on('request', (r) => { if (r.method() === 'POST' && /\/api\/v1\/ide\/runs$/.test(r.url())) runs++; });
    await page.getByTestId('ide-run-btn').click();
    await expect(page.getByText('Nothing to run').first()).toBeVisible({ timeout: 10_000 });
    expect(runs).toBe(0);
  });
});
