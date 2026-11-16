import { test, expect, type Locator, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { bootPolyglotIde, daemonCall, GO_MAIN_SOURCE, openPolyglotFile, type PolyglotWorkspaceSeed } from './ide-polyglot-fixtures';

// The debugger panel during a REAL Python session (built-in bdb adapter):
//  I2-09 one toolbar row that never scrolls; the step buttons are on top.
//  I2-04 `print("tick", i)` is one row; an evaluation sits between the output
//        before and after it.  I2-12 Clear, then new output still shows.
//  I2-08 an expanded variable stays expanded after a step.
//  I2-11 the tree's Breakpoints tab is THE list; ⋯ › Breakpoints opens it.
//  I2-10 a refused Go start (no Delve here) shows a card with "Install Delve".
// The harness daemon is the installed build (no `seq`/`error_kind` yet), so this
// also proves the client fallbacks (client sequence counter, error-text kind).

const REVIEW_SOURCE = [
  'import json, sys',
  '',
  '',
  'def build(n):',
  '    order = {"id": n, "items": [{"sku": "a", "qty": 1}, {"sku": "b", "qty": 2}], "meta": {"region": "eu"}}',
  '    return order',
  '',
  '',
  'def main():',
  '    total = 0',
  '    for i in range(4):',
  '        order = build(i)',
  '        total += len(order["items"])',
  '        print("tick", i, flush=True)',
  '        print("warn", i, file=sys.stderr, flush=True)',
  '    print(json.dumps({"level": "info", "msg": "done", "total": total}), flush=True)',
  '    return total',
  '',
  '',
  'if __name__ == "__main__":',
  '    main()',
  '',
].join('\n');
const BP_LINE = 13;

let seed: PolyglotWorkspaceSeed;

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-DBG-PANEL'), { recursive: true });
  await page.screenshot({ path: join(dir, 'fix-DBG-PANEL', name) });
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

async function stopAllSessions(): Promise<void> {
  const c = await apiCtx();
  try {
    const r = await daemonCall<{ sessions: { id: string; state: string }[] }>(c.ctx, c.base, c.token, 'GET', '/ide/debug/sessions');
    for (const s of r.sessions) {
      if (s.state === 'terminated' || s.state === 'error') continue;
      await c.ctx.post(`${c.base}/api/v1/ide/debug/sessions/${encodeURIComponent(s.id)}/command`, { data: { action: 'stop' } }).catch(() => undefined);
    }
  } finally {
    await c.ctx.dispose();
  }
}

/** The control at its own centre is the topmost element (nothing covers it). */
async function expectOnTop(loc: Locator, what: string): Promise<void> {
  const ok = await loc.evaluate((el) => {
    const r = el.getBoundingClientRect();
    const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
    return Boolean(hit && (hit === el || el.contains(hit))) && r.right <= innerWidth && r.left >= 0;
  });
  expect(ok, `${what} is on top and on screen`).toBe(true);
}

async function waitPausedWith(page: Page, name: string, value: string): Promise<void> {
  const panel = page.getByTestId('ide-debugger-panel');
  await expect(panel.getByTestId('ide-debug-status')).toContainText('Paused', { timeout: 30_000 });
  await expect(panel.locator(`[data-testid="ide-debug-var-row"][data-var-name="${name}"]`).first()).toContainText(value, { timeout: 15_000 });
}

/** Console rows (output lines and evaluations) in DOM order, as short strings. */
async function consoleRows(page: Page): Promise<string[]> {
  return page.getByTestId('ide-debug-console-full').locator('[data-testid="ide-console-line"], [data-testid="ide-console-repl-entry"]').evaluateAll((els) =>
    els.map((el) => (el.getAttribute('data-testid') === 'ide-console-repl-entry' ? `> ${(el.querySelector('.dbg-repl-expr')?.textContent ?? '').replace(/^>\s*/, '').trim()}` : (el.textContent ?? '').trim())),
  );
}

test.describe('Desktop IDE — the debugger panel during a session', () => {
  test.describe.configure({ mode: 'serial' });
  test.use({ viewport: { width: 1280, height: 800 } });

  test.beforeAll(async () => {
    test.setTimeout(60_000);
    const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-dbgpanel-')));
    mkdirSync(join(root, 'pyapp'), { recursive: true });
    mkdirSync(join(root, 'app'), { recursive: true });
    writeFileSync(join(root, 'pyapp', 'review.py'), REVIEW_SOURCE);
    writeFileSync(join(root, 'go.mod'), 'module example.com/dbgpanel\n\ngo 1.22\n');
    writeFileSync(join(root, 'app', 'main.go'), GO_MAIN_SOURCE);
    execFileSync('git', ['init', '-q'], { cwd: root });
    const c = await apiCtx();
    try {
      const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `IDE debugger panel ${Date.now()}`, root_path: root });
      seed = { base: c.base, token: c.token, workspaceId: ws.id, workspaceRoot: root, target: { kind: 'local', workspace_id: ws.id, root, connection_id: null } };
    } finally {
      await c.ctx.dispose();
    }
  });

  test.afterAll(async () => {
    await stopAllSessions().catch(() => undefined);
    if (seed?.workspaceRoot) rmSync(seed.workspaceRoot, { recursive: true, force: true });
  });

  test.beforeEach(async ({}, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop debugger panel');
  });

  test('one-row bar, stitched console, evaluations in time, expansion kept, Clear, Breakpoints tab', async ({ page }) => {
    test.setTimeout(180_000);
    try {
      await bootPolyglotIde(page, seed, 'dark');
      await openPolyglotFile(page, 'pyapp', 'review.py');
      await expect(page.locator('.ide-editor-area .cm-content').first()).toContainText('total += len');
      await clickLineNumber(page, BP_LINE);
      await expect(page.locator('.ide-editor-area [data-testid="ide-bp-marker"]')).toHaveCount(1);

      await page.getByTestId('ide-debug-btn').click();
      const panel = page.getByTestId('ide-debugger-panel');
      await waitPausedWith(page, 'i', '0');

      // I2-09: one row, never scrolls; the step buttons are visible and on top.
      const bar = panel.getByTestId('ide-debug-bar');
      const dims = await bar.evaluate((el) => ({ sw: el.scrollWidth, cw: el.clientWidth, h: el.getBoundingClientRect().height }));
      expect(dims.sw, 'the debugger bar does not overflow').toBeLessThanOrEqual(dims.cw);
      expect(dims.h).toBeLessThanOrEqual(33);
      for (const id of ['ide-debug-resume', 'ide-debug-step-over', 'ide-debug-step-into', 'ide-debug-step-out', 'ide-debug-stop', 'ide-debug-more']) {
        await expectOnTop(panel.getByTestId(id), id);
      }
      await expect(panel.getByTestId('ide-debug-location')).toHaveText(`review.py:${BP_LINE}`);
      await expect(panel.getByTestId('ide-debug-status')).toHaveAttribute('title', /python/);

      // I2-08: expand order → items, then a step to the next stop keeps both open.
      await panel.getByRole('button', { name: 'Expand order' }).click();
      await expect(panel.locator('[data-testid="ide-debug-var-row"][data-var-name="items"]')).toBeVisible({ timeout: 10_000 });
      await panel.getByRole('button', { name: 'Expand items' }).click();
      await expect(panel.locator('[data-testid="ide-debug-var-row"][data-var-name="0"]')).toBeVisible({ timeout: 10_000 });
      await shot(page, 'paused-1280-dark.png');

      await panel.getByTestId('ide-debug-resume').click();
      await waitPausedWith(page, 'i', '1');
      await expect(panel.locator('[data-testid="ide-debug-var-row"][data-var-name="items"]'), 'order stays expanded after the step').toBeVisible({ timeout: 10_000 });
      await expect(panel.locator('[data-testid="ide-debug-var-row"][data-var-name="0"]'), 'items stays expanded too').toBeVisible({ timeout: 10_000 });
      await expect(panel.locator('[data-testid="ide-debug-var-row"][data-var-name="total"]'), 'total 0 → 2 is marked changed').toHaveClass(/is-changed/);
      await shot(page, 'after-step-expansion-kept-dark.png');

      // Evaluate between "tick 0" and "tick 1".
      const evalInput = panel.getByRole('textbox', { name: 'Evaluate expression' });
      await evalInput.fill('i * 10');
      await evalInput.press('Enter');
      await expect(panel.locator('.dbg-quick-repl')).toContainText('10');
      await panel.getByTestId('ide-debug-resume').click();
      await waitPausedWith(page, 'i', '2');

      await panel.getByTestId('ide-debug-subtab-console').click();
      const con = page.getByTestId('ide-debug-console-full');
      await expect(con.locator('[data-testid="ide-console-line"]', { hasText: /^tick 1$/ })).toHaveCount(1, { timeout: 10_000 });
      const rows = await consoleRows(page);
      const t0 = rows.indexOf('tick 0');
      const ev = rows.indexOf('> i * 10');
      const t1 = rows.indexOf('tick 1');
      expect(t0, `"tick 0" is one row: ${JSON.stringify(rows)}`).toBeGreaterThanOrEqual(0);
      expect(ev, 'the evaluation follows the output before it').toBeGreaterThan(t0);
      expect(t1, 'and precedes the output after it').toBeGreaterThan(ev);
      expect(rows.filter((r) => r === '' || r === '0' || r === 'tick'), 'no fragment rows').toEqual([]);
      await expect(con.locator('[data-testid="ide-console-line"]', { hasText: /^warn 0$/ })).toHaveClass(/is-err/);
      await shot(page, 'console-1280-dark.png');

      // I2-12: Clear, then more output shows.
      await con.getByTestId('ide-console-clear').click();
      await expect(con.locator('[data-testid="ide-console-line"]')).toHaveCount(0);
      await panel.getByTestId('ide-debug-resume').click();
      await expect(con.locator('[data-testid="ide-console-line"]', { hasText: /^tick 2$/ })).toHaveCount(1, { timeout: 20_000 });
      await expect(con.locator('[data-testid="ide-console-line"]', { hasText: /^tick 1$/ })).toHaveCount(0);

      // I2-11: ⋯ › Breakpoints opens the tree tab; the list shows and removes it.
      await panel.getByTestId('ide-debug-more').click();
      await page.getByRole('menuitem', { name: /Breakpoints \(1\)/ }).click();
      const list = page.getByTestId('ide-breakpoints-list');
      await expect(list).toBeVisible({ timeout: 10_000 });
      const row = list.getByTestId('ide-bp-row');
      await expect(row).toHaveCount(1);
      await expect(row).toContainText(String(BP_LINE));
      await expect(row).toContainText('total += len(order["items"])');
      await shot(page, 'breakpoints-tab-dark.png');
      await row.getByTestId('ide-bp-remove').click();
      await expect(list.getByTestId('ide-bp-row')).toHaveCount(0);
      await expect(page.locator('.ide-editor-area [data-testid="ide-bp-marker"]')).toHaveCount(0);
    } finally {
      await stopAllSessions();
    }
  });

  test('light: paused bar, console and the Breakpoints tab', async ({ page }) => {
    test.setTimeout(150_000);
    try {
      await bootPolyglotIde(page, seed, 'light');
      await openPolyglotFile(page, 'pyapp', 'review.py');
      await expect(page.locator('.ide-editor-area .cm-content').first()).toContainText('total += len');
      await clickLineNumber(page, BP_LINE);
      await page.getByTestId('ide-debug-btn').click();
      const panel = page.getByTestId('ide-debugger-panel');
      await waitPausedWith(page, 'i', '0');
      const dims = await panel.getByTestId('ide-debug-bar').evaluate((el) => ({ sw: el.scrollWidth, cw: el.clientWidth }));
      expect(dims.sw).toBeLessThanOrEqual(dims.cw);
      await shot(page, 'paused-1280-light.png');
      await panel.getByTestId('ide-debug-resume').click();
      await waitPausedWith(page, 'i', '1');
      await panel.getByTestId('ide-debug-subtab-console').click();
      await expect(page.getByTestId('ide-debug-console-full').locator('[data-testid="ide-console-line"]', { hasText: /^tick 0$/ })).toHaveCount(1, { timeout: 10_000 });
      await shot(page, 'console-1280-light.png');
      await panel.getByTestId('ide-debug-more').click();
      await page.getByRole('menuitem', { name: /Breakpoints \(1\)/ }).click();
      await expect(page.getByTestId('ide-breakpoints-list').getByTestId('ide-bp-row')).toHaveCount(1);
      await shot(page, 'breakpoints-tab-light.png');
      await page.getByTestId('ide-breakpoints-list').getByTestId('ide-bp-remove').click();
    } finally {
      await stopAllSessions();
    }
  });

  for (const scheme of ['dark', 'light'] as const) {
    test(`a Go start without Delve shows the failure card with Install Delve (${scheme})`, async ({ page }) => {
      test.setTimeout(120_000);
      let installs = 0;
      // Never download anything from the harness: count the click instead.
      await page.route('**/api/v1/ide/toolchains/install', async (route) => {
        installs++;
        await route.fulfill({ json: { job: { id: 'job-dlv', ecosystem: 'go', component: 'dap', target_os: 'darwin', target_arch: 'arm64', status: 'running', progress_pct: 30, log_lines: [] } } });
      });
      try {
        await bootPolyglotIde(page, seed, scheme);
        await openPolyglotFile(page, 'app', 'main.go');
        await expect(page.locator('.ide-editor-area .cm-content').first()).toContainText('package main');
        await page.getByTestId('ide-debug-btn').click();
        const card = page.getByTestId('ide-debug-error-card');
        await expect(card).toBeVisible({ timeout: 45_000 });
        await expect(card).toContainText('Delve');
        const install = card.getByTestId('ide-debug-error-primary');
        await expect(install).toHaveText('Install Delve');
        const msg = card.locator('.dbg-err-msg');
        await expect(msg).not.toHaveCSS('text-overflow', 'ellipsis');
        expect(await msg.textContent()).not.toMatch(/^not found:/);
        const bar = page.getByTestId('ide-debug-bar');
        const dims = await bar.evaluate((el) => ({ sw: el.scrollWidth, cw: el.clientWidth }));
        expect(dims.sw).toBeLessThanOrEqual(dims.cw);
        await shot(page, `error-card-${scheme}.png`);
        await install.click();
        await expect(card.getByTestId('ide-debug-error-install-status')).toContainText('Installing', { timeout: 10_000 });
        expect(installs).toBe(1);
        await card.getByTestId('ide-debug-error-dismiss').click();
        await expect(card).toHaveCount(0);
      } finally {
        await stopAllSessions();
      }
    });
  }
});
