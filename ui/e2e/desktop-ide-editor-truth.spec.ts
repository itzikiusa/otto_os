import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { bootPolyglotIde, daemonCall, openPolyglotFile, type PolyglotWorkspaceSeed } from './ide-polyglot-fixtures';

// What the editor shows while you edit and debug, on a REAL Python session (built-in bdb):
//  I2-02 breakpoints stay on the code they were set on: two lines inserted above move both,
//        the Breakpoints tab (the store) follows, and the session pauses on the right code.
//  I2-13 hovering a name while paused shows its value.
//  I2-21 right-clicking a line number opens the breakpoint editor; Escape leaves nothing.
//  I1-12 a dirty tab whose file changes on disk gets a banner (Compare / Keep mine); a clean
//        tab still reloads silently.
// The harness daemon is the installed build (no `selected_frame_*`, no unbound answers): those
// paths are unit-tested in unit/ideEditorTruth.test.ts.

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
  '    return total',
  '',
  '',
  'if __name__ == "__main__":',
  '    main()',
  '',
].join('\n');
const BP_A = 10; // `    total = 0`
const BP_B = 13; // `        total += len(order["items"])`

let seed: PolyglotWorkspaceSeed;

async function shot(page: Page, name: string): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-EDITOR-B'), { recursive: true });
  await page.screenshot({ path: join(dir, 'fix-EDITOR-B', name) });
}

function lineNumberEl(page: Page, line: number) {
  return page.locator('.ide-editor-area .cm-lineNumbers .cm-gutterElement').filter({ hasText: new RegExp(`^\\s*${line}\\s*$`) }).first();
}

async function clickLineNumber(page: Page, line: number, button: 'left' | 'right' = 'left'): Promise<void> {
  const el = lineNumberEl(page, line);
  await expect(el).toBeVisible({ timeout: 10_000 });
  const box = (await el.boundingBox())!;
  await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2, { button });
}

/** Lines the gutter's breakpoint markers sit on (from their own tooltips). */
async function gutterBreakpointLines(page: Page): Promise<number[]> {
  const titles = await page.locator('.ide-editor-area [data-testid="ide-bp-marker"]').evaluateAll((els) => els.map((e) => e.getAttribute('title') ?? ''));
  return titles.map((t) => Number(/line (\d+)/.exec(t)?.[1] ?? NaN)).sort((a, b) => a - b);
}

async function pointAtWord(page: Page, lineText: string, word: string): Promise<{ x: number; y: number }> {
  const pt = await page.evaluate(([lt, w]) => {
    const line = [...document.querySelectorAll('.ide-editor-area .cm-line')].find((l) => l.textContent?.includes(lt));
    if (!line) return null;
    const walker = document.createTreeWalker(line, NodeFilter.SHOW_TEXT);
    for (let n = walker.nextNode(); n; n = walker.nextNode()) {
      const i = n.textContent?.indexOf(w) ?? -1;
      if (i < 0) continue;
      const r = document.createRange(); r.setStart(n, i); r.setEnd(n, i + w.length);
      const b = r.getBoundingClientRect();
      return { x: b.left + b.width / 2, y: b.top + b.height / 2 };
    }
    return null;
  }, [lineText, word] as const);
  expect(pt, `"${word}" on the line "${lineText}"`).toBeTruthy();
  return pt!;
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

test.describe('Desktop IDE — the editor tells the truth', () => {
  test.describe.configure({ mode: 'serial' });
  test.use({ viewport: { width: 1280, height: 800 } });

  test.beforeAll(async () => {
    test.setTimeout(60_000);
    const root = realpathSync(mkdtempSync(join(tmpdir(), 'otto-e2e-ide-truth-')));
    mkdirSync(join(root, 'pyapp'), { recursive: true });
    writeFileSync(join(root, 'pyapp', 'review.py'), REVIEW_SOURCE);
    writeFileSync(join(root, 'pyapp', 'notes.py'), 'NOTES = 1\n');
    writeFileSync(join(root, 'pyapp', 'clean.py'), 'CLEAN = 1\n');
    execFileSync('git', ['init', '-q'], { cwd: root });
    const c = await apiCtx();
    try {
      const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `IDE editor truth ${Date.now()}`, root_path: root });
      seed = { base: c.base, token: c.token, workspaceId: ws.id, workspaceRoot: root, target: { kind: 'local', workspace_id: ws.id, root, connection_id: null } };
    } finally {
      await c.ctx.dispose();
    }
  });

  test.afterAll(async () => {
    await stopAllSessions().catch(() => undefined);
    if (seed?.workspaceRoot) rmSync(seed.workspaceRoot, { recursive: true, force: true });
  });

  test.beforeEach(async ({ page }, info) => {
    test.skip(info.project.name !== 'desktop-browser', 'desktop editor');
    // Unsaved edits must stay unsaved for the disk-change case (autosave is on by default).
    await page.addInitScript(() => {
      const key = 'otto.ide.preferences.v1';
      let cur: Record<string, unknown> = {};
      try { cur = JSON.parse(localStorage.getItem(key) || '{}'); } catch { /* fresh */ }
      localStorage.setItem(key, JSON.stringify({ ...cur, autoSave: false }));
    });
  });

  test('breakpoints follow inserted lines into the store and the session; right-click + Escape leaves none', async ({ page }) => {
    test.setTimeout(180_000);
    try {
      await bootPolyglotIde(page, seed, 'dark');
      await openPolyglotFile(page, 'pyapp', 'review.py');
      const content = page.locator('.ide-editor-area .cm-content').first();
      await expect(content).toContainText('total += len');
      await clickLineNumber(page, BP_A);
      await clickLineNumber(page, BP_B);
      await expect.poll(() => gutterBreakpointLines(page)).toEqual([BP_A, BP_B]);
      await shot(page, 'gutter-pending-dark.png');

      // I2-21: right-click a line NUMBER → the breakpoint editor; Escape → no breakpoint left.
      await clickLineNumber(page, 5, 'right');
      await expect(page.getByTestId('ide-breakpoint-popover')).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(page.getByTestId('ide-breakpoint-popover')).toHaveCount(0);
      await expect.poll(() => gutterBreakpointLines(page)).toEqual([BP_A, BP_B]);

      // I2-02: two lines inserted under `def main():` (above both breakpoints).
      await content.locator('.cm-line', { hasText: 'def main():' }).click();
      await page.keyboard.press('End');
      await page.keyboard.press('Enter');
      await page.keyboard.type('# one');
      await page.keyboard.press('Enter');
      await page.keyboard.type('# two');
      await expect.poll(() => gutterBreakpointLines(page)).toEqual([BP_A + 2, BP_B + 2]);
      // The store (the Breakpoints tab) follows once typing settles, and keeps following.
      await page.getByTestId('ide-tree-tab-breakpoints').click();
      const rows = page.getByTestId('ide-breakpoints-list').getByTestId('ide-bp-row');
      await expect.poll(() => rows.locator('.bp-line').allTextContents(), { timeout: 5_000 }).toEqual([String(BP_A + 2), String(BP_B + 2)]);
      await expect(rows.first()).toContainText('total = 0');
      await page.waitForTimeout(800); // ui-guards: allow — proves the store update is not re-applied as an old line (no jump back)
      expect(await gutterBreakpointLines(page)).toEqual([BP_A + 2, BP_B + 2]);
      await shot(page, 'breakpoints-moved-dark.png');

      // The session gets the new lines: it pauses on `total = 0`, now line 12.
      await page.keyboard.press('Meta+s');
      await page.getByTestId('ide-debug-btn').click();
      const panel = page.getByTestId('ide-debugger-panel');
      await expect(panel.getByTestId('ide-debug-status')).toContainText('Paused', { timeout: 30_000 });
      await expect(panel.getByTestId('ide-debug-location')).toHaveText(`review.py:${BP_A + 2}`);
      await expect(page.locator('.ide-editor-area .cm-ide-paused-line').first()).toContainText('total = 0');
      await panel.getByTestId('ide-debug-resume').click();
      await expect(panel.getByTestId('ide-debug-location')).toHaveText(`review.py:${BP_B + 2}`, { timeout: 20_000 });
      await expect(page.locator('.ide-editor-area .cm-ide-paused-line').first()).toContainText('total += len');

      // I2-13: hover a name while paused → its value.
      const pt = await pointAtWord(page, 'total += len', 'total');
      await page.mouse.move(pt.x, pt.y);
      await expect(page.getByTestId('ide-debug-hover')).toContainText('total = 0', { timeout: 10_000 });
      await shot(page, 'paused-hover-dark.png');
    } finally {
      await stopAllSessions();
    }
  });

  test('a dirty tab whose file changes on disk gets the banner; a clean tab reloads silently', async ({ page }) => {
    test.setTimeout(120_000);
    await bootPolyglotIde(page, seed, 'light');
    await openPolyglotFile(page, 'pyapp', 'notes.py');
    const content = page.locator('.ide-editor-area .cm-content').first();
    await expect(content).toContainText('NOTES = 1');
    await content.locator('.cm-line', { hasText: 'NOTES = 1' }).click();
    await page.keyboard.press('End');
    await page.keyboard.type('  # mine');

    writeFileSync(join(seed.workspaceRoot, 'pyapp', 'notes.py'), 'NOTES = 2  # an agent wrote this\n');
    const banner = page.getByTestId('ide-disk-changed-banner');
    await expect(banner).toBeVisible({ timeout: 15_000 });
    await expect(banner).toContainText('pyapp/notes.py changed on disk.');
    await expect(content, 'the unsaved edit is still there').toContainText('NOTES = 1  # mine');
    const keep = banner.getByTestId('ide-disk-keep');
    const onTop = await keep.evaluate((el) => {
      const r = el.getBoundingClientRect();
      const hit = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
      return Boolean(hit && (hit === el || el.contains(hit)));
    });
    expect(onTop, 'Keep mine is on top').toBe(true);
    await shot(page, 'disk-changed-banner-light.png');

    await banner.getByTestId('ide-disk-compare').click();
    const cmp = page.getByTestId('ide-comparison-modal');
    await expect(cmp).toBeVisible();
    await expect(cmp).toContainText('an agent wrote this');
    await page.keyboard.press('Escape');
    await expect(cmp).toHaveCount(0);

    await keep.click();
    await expect(banner).toHaveCount(0);
    await expect(content).toContainText('NOTES = 1  # mine');

    // A clean tab still picks up the disk change with no banner.
    await openPolyglotFile(page, 'pyapp', 'clean.py');
    await expect(content).toContainText('CLEAN = 1');
    writeFileSync(join(seed.workspaceRoot, 'pyapp', 'clean.py'), 'CLEAN = 2\n');
    await expect(content).toContainText('CLEAN = 2', { timeout: 20_000 });
    await expect(banner).toHaveCount(0);
  });

  test('dark: the banner reads in dark too', async ({ page }) => {
    test.setTimeout(90_000);
    await bootPolyglotIde(page, seed, 'dark');
    await openPolyglotFile(page, 'pyapp', 'clean.py');
    const content = page.locator('.ide-editor-area .cm-content').first();
    await expect(content).toContainText('CLEAN');
    await content.locator('.cm-line').first().click();
    await page.keyboard.press('End');
    await page.keyboard.type('  # mine');
    writeFileSync(join(seed.workspaceRoot, 'pyapp', 'clean.py'), 'CLEAN = 3\n');
    await expect(page.getByTestId('ide-disk-changed-banner')).toBeVisible({ timeout: 15_000 });
    await shot(page, 'disk-changed-banner-dark.png');
  });
});
