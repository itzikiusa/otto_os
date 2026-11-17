import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { daemonCall } from './ide-polyglot-fixtures';

// The paused debugger on a phone (I4-04, I4-17): the IDE stacks by its OWN
// size, the debugger follows that flag; the bar never scrolls sideways and the
// step controls are 44 px tap targets. A real Python session (built-in bdb
// adapter) is started on the daemon and re-attached by the panel.

const SOURCE = ['def main():', '    total = 0', '    for i in range(3):', '        total += i', '    return total', '', '', 'main()', ''].join('\n');

interface Snap { id: string; state: string }
let root = '';
let wsId = '';

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-DBG-PANEL'), { recursive: true });
  await page.screenshot({ path: join(dir, 'fix-DBG-PANEL', name) });
}

test.describe('IDE debugger on a phone', () => {
  test.beforeEach(async ({}, info) => {
    test.skip(info.project.name !== 'iphone-portrait', 'one phone shape is enough for this check');
  });

  test('paused: one non-scrolling bar with 44 px step controls', async ({ page }) => {
    test.setTimeout(120_000);
    root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-dbg-mobile-')));
    writeFileSync(join(root, 'loop.py'), SOURCE);
    const c = await apiCtx();
    let sessionId = '';
    try {
      wsId = (await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `IDE debugger mobile ${Date.now()}`, root_path: root })).id;
      const target = { kind: 'local', workspace_id: wsId, root, connection_id: null };
      const program = join(root, 'loop.py');
      const started = await daemonCall<{ session: Snap }>(c.ctx, c.base, c.token, 'POST', '/ide/debug/sessions', {
        target, project_folders: [root], language: 'python',
        launch: { program, args: [], cwd: root, env: {}, inherit_shell_env: true },
        breakpoints: [{ path: program, line: 4, enabled: true }], break_on_exceptions: false,
      });
      sessionId = started.session.id;
      await expect.poll(async () => (await daemonCall<{ session: Snap }>(c.ctx, c.base, c.token, 'GET', `/ide/debug/sessions/${sessionId}`)).session.state, { timeout: 30_000 }).toBe('paused');

      await page.addInitScript(({ ws, dir }) => {
        localStorage.setItem('otto_workspace', ws);
        localStorage.setItem('otto_scheme', 'dark');
        localStorage.setItem('otto_direction', 'ltr');
        localStorage.setItem('otto.ide.namedProjects.v1', JSON.stringify([{ id: 'proj-dbg-mobile', name: 'Debugger mobile', folders: [{ path: dir, name: 'dbg' }], updatedAt: Date.now() }]));
        sessionStorage.setItem('otto.ide.stackPane', 'tools');
        localStorage.setItem('otto.ide.dockTab', 'debug');
      }, { ws: wsId, dir: root });
      await page.goto('/#/ide');
      await expect(page.getByTestId('ide-bottom-dock')).toBeVisible({ timeout: 20_000 });
      const panel = page.getByTestId('ide-debugger-panel');
      await expect(panel.getByTestId('ide-debug-status')).toContainText('Paused', { timeout: 30_000 });
      // The re-attach opens the paused file in the editor pane; come back to Tools.
      const tools = page.getByTestId('ide-stack-tab-tools');
      if ((await tools.getAttribute('aria-pressed')) !== 'true') await tools.click();
      await page.getByTestId('ide-tab-debug').click();
      await expect(panel).toBeVisible();

      const bar = panel.getByTestId('ide-debug-bar');
      const dims = await bar.evaluate((el) => ({ sw: el.scrollWidth, cw: el.clientWidth }));
      expect(dims.sw, 'the bar never scrolls sideways').toBeLessThanOrEqual(dims.cw);
      for (const id of ['ide-debug-resume', 'ide-debug-step-over', 'ide-debug-step-into', 'ide-debug-step-out', 'ide-debug-stop']) {
        const box = await panel.getByTestId(id).boundingBox();
        expect(box, id).toBeTruthy();
        expect(box!.width, `${id} width`).toBeGreaterThanOrEqual(43.9);
        expect(box!.height, `${id} height`).toBeGreaterThanOrEqual(43.9);
        expect(box!.x + box!.width, `${id} on screen`).toBeLessThanOrEqual(page.viewportSize()!.width);
      }
      const docScroll = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
      expect(docScroll).toBeLessThanOrEqual(0);
      await shot(page, 'iphone-portrait-paused-dark.png');
    } finally {
      if (sessionId) await c.ctx.post(`${c.base}/api/v1/ide/debug/sessions/${encodeURIComponent(sessionId)}/command`, { data: { action: 'stop' } }).catch(() => undefined);
      await c.ctx.dispose();
      if (root) rmSync(root, { recursive: true, force: true });
    }
  });
});
