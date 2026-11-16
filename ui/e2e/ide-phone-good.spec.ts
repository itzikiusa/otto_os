import { test, expect, type Locator, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expectNoHorizontalOverflow } from './helpers';
import { daemonCall } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

// The IDE on a phone, "works" → "good" (round 5, IDE-PHONE):
//  - one file row on a phone (active file + "N open" + the editor ⋯), no split buttons;
//    on its side that row sits in the page header (≤ 2 chrome rows above the code);
//  - typing mode: with the keyboard up the page header (and on an iPhone SE the file row)
//    steps aside, and the line being edited stays inside the visible area;
//  - the editor's verbs by touch (⋯ → Go to Definition / Find Usages / … / Go to Line);
//  - paused in a real Python session: a 44 px step bar over the editor that steps.
// Playwright cannot open a keyboard: like the reviewer (and threads-phone-layout.spec.ts)
// the viewport shrinks to what a keyboard leaves while the code has focus;
// lib/viewportFit.ts reads that as the keyboard (html[data-kb="open"]).
// Screenshots (OTTO_E2E_SHOTS) → fix-IDE-PHONE/.

const PHONES = ['iphone-portrait', 'iphone-landscape', 'iphone-se'];
const LONG = '    result = compute_settlement_total(order_lines, discounts, currency="EUR", rounding="half-even")';
const SOURCE = [
  'def compute_settlement_total(lines, discounts, currency, rounding):',
  ...Array.from({ length: 60 }, (_, i) => (i % 7 === 3 ? LONG : `    value_${i + 2} = ${i + 2}  # line ${i + 2}`)),
  '',
].join('\n');
const LOOP = ['def main():', '    total = 0', '    for i in range(3):', '        total += i', '    return total', '', '', 'main()', ''].join('\n');

async function shot(page: Page, name: string, schemes: readonly ('light' | 'dark')[] = ['dark']): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-IDE-PHONE'), { recursive: true });
  for (const scheme of schemes) {
    await page.emulateMedia({ colorScheme: scheme });
    // The scheme is `data-scheme`; `data-theme` names the theme (native / warm / pro-dark).
    await page.evaluate((s) => { document.documentElement.dataset.scheme = s; }, scheme).catch(() => undefined);
    await page.screenshot({ path: join(dir, 'fix-IDE-PHONE', `${test.info().project.name}-${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.evaluate(() => { document.documentElement.dataset.scheme = 'dark'; }).catch(() => undefined);
}

async function onTop(loc: Locator): Promise<boolean> {
  const box = (await loc.boundingBox())!;
  return loc.evaluate((el, p) => {
    const hit = document.elementFromPoint(p.x, p.y);
    return !!hit && (hit === el || el.contains(hit));
  }, { x: box.x + box.width / 2, y: box.y + box.height / 2 });
}

async function settle(page: Page): Promise<void> {
  await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
}

async function seedWorkspace(files: Record<string, string>, label: string): Promise<{ root: string; wsId: string }> {
  const root = realpathSync(mkdtempSync(join(tmpdir(), `otto-e2e-ide-phone-${label}-`)));
  for (const [name, body] of Object.entries(files)) writeFileSync(join(root, name), body);
  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `IDE phone ${label} ${Date.now()}`, root_path: root });
    return { root, wsId: ws.id };
  } finally {
    await c.ctx.dispose();
  }
}

async function boot(page: Page, root: string, wsId: string, pane: 'editor' | 'tools', file?: string): Promise<void> {
  await page.addInitScript(({ ws, dir, p }) => {
    if (sessionStorage.getItem('__ide_phone_good')) return;
    sessionStorage.setItem('__ide_phone_good', '1');
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_scheme', 'dark');
    localStorage.setItem('otto_direction', 'ltr');
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto.ide.namedProjects.v1', JSON.stringify([{ id: 'proj-phone-good', name: 'Phone good', folders: [{ path: dir, name: 'phone' }], updatedAt: Date.now() }]));
    sessionStorage.setItem('otto.ide.stackPane', p);
    // Tools first = the debugger panel, which re-attaches a live session.
    if (p === 'tools') localStorage.setItem('otto.ide.dockTab', 'debug');
  }, { ws: wsId, dir: root, p: pane });
  const q = file ? `?root=${encodeURIComponent(root)}&file=${encodeURIComponent(join(root, file))}` : '';
  await page.goto(`/#/ide${q}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('ide-page')).toHaveClass(/\bstack\b/, { timeout: 20_000 });
}

/** Rows of code in the editor's visible band (its height above the keyboard / row height). */
async function visibleCodeLines(page: Page): Promise<number> {
  return page.evaluate(() => {
    const sc = document.querySelector('.ide-editor-area .cm-scroller');
    if (!sc) return 0;
    const r = sc.getBoundingClientRect();
    const bottom = Math.min(r.bottom, window.visualViewport?.height ?? innerHeight);
    const lh = parseFloat(getComputedStyle(sc.querySelector('.cm-content')!).lineHeight) || 18;
    return Math.floor(Math.max(0, bottom - r.top) / lh);
  });
}

/** The rect + text of the line holding the (primary) caret. */
async function caretLine(page: Page): Promise<{ top: number; bottom: number; text: string } | null> {
  return page.evaluate(() => {
    const cur = document.querySelector('.ide-editor-area .cm-cursor-primary, .ide-editor-area .cm-cursor')?.getBoundingClientRect();
    if (!cur) return null;
    const y = (cur.top + cur.bottom) / 2;
    const line = [...document.querySelectorAll('.ide-editor-area .cm-line')].find((l) => {
      const b = l.getBoundingClientRect();
      return b.top <= y && b.bottom >= y;
    });
    if (!line) return null;
    const b = line.getBoundingClientRect();
    return { top: b.top, bottom: b.bottom, text: line.textContent ?? '' };
  });
}

test.describe('IDE on a phone: reading, typing, touch commands', () => {
  let seed: { root: string; wsId: string } | null = null;
  test.afterAll(() => { if (seed) rmSync(seed.root, { recursive: true, force: true }); });

  test('one file row, typing keeps the caret line above the keyboard, ⋯ opens the editor verbs and Go to Line works', async ({ page }, info) => {
    test.skip(!PHONES.includes(info.project.name), 'phones');
    test.setTimeout(150_000);
    const p = info.project.name;
    seed ??= await seedWorkspace({ 'settlement.py': SOURCE }, 'read');
    await boot(page, seed.root, seed.wsId, 'editor', 'settlement.py');
    const editor = page.locator('.ide-editor-area .cm-editor').first();
    await expect(editor).toBeVisible({ timeout: 30_000 });
    await expect(page.locator('.ide-editor-area .cm-content')).toContainText('compute_settlement_total', { timeout: 30_000 });

    // ── reading: one file row, no split buttons ──
    const row = page.getByTestId('ide-phone-file-row');
    await expect(row).toHaveCount(1);
    await expect(row).toContainText('settlement.py');
    await expect(page.getByTestId('ide-split-right-btn')).toHaveCount(0);
    await expect(page.getByTestId('ide-split-down-btn')).toHaveCount(0);
    const vp = page.viewportSize()!;
    const landscape = vp.width > vp.height;
    const header = page.getByTestId('page-header').first();
    if (landscape && vp.width >= 700) {
      // On its side: the file row lives in the header — two chrome rows above the code.
      await expect(header.getByTestId('ide-phone-file-row')).toHaveCount(1);
      const top = await page.evaluate(() => {
        const bar = document.querySelector('.mtopbar')?.getBoundingClientRect();
        const ph = document.querySelector('[data-testid=page-header]')?.getBoundingClientRect();
        const cm = document.querySelector('.ide-editor-area .cm-editor')!.getBoundingClientRect();
        return { cm: cm.top, chrome: (bar && bar.height > 0 ? bar.height : 0) + (ph?.height ?? 0) };
      });
      expect(top.cm, `${p}: nothing but the shell bar and the page header above the code`).toBeLessThanOrEqual(top.chrome + 2);
    } else {
      await expect(page.locator('.ide-editor-area').getByTestId('ide-phone-file-row')).toHaveCount(1);
    }
    for (const id of ['ide-tabs-overflow-btn', 'ide-editor-touch-more']) {
      const b = (await page.getByTestId(id).boundingBox())!;
      expect(b.height, `${p}: ${id} height`).toBeGreaterThanOrEqual(36);
      expect(await onTop(page.getByTestId(id)), `${p}: ${id} on top`).toBe(true);
    }
    await expectNoHorizontalOverflow(page);
    // Soft-wrapped lines hang under their indent.
    const hang = await page.locator('.ide-editor-area .cm-line', { hasText: 'order_lines' }).first().evaluate((el) => getComputedStyle(el).textIndent);
    expect(parseFloat(hang), `${p}: wrapped line has a hanging indent`).toBeLessThan(0);
    await shot(page, 'reading', ['dark', 'light']);

    // ── typing: tap a line in the lower half, then the keyboard opens ──
    const target = page.locator('.ide-editor-area .cm-line', { hasText: '# line 30' }).first();
    await target.scrollIntoViewIfNeeded();
    await target.click();
    const before = await visibleCodeLines(page);
    const kb = landscape ? 180 : 336;
    await page.setViewportSize({ width: vp.width, height: vp.height - kb });
    await expect(page.locator('html')).toHaveAttribute('data-kb', 'open', { timeout: 5_000 });
    await expect(page.getByTestId('ide-page')).toHaveClass(/\btyping\b/);
    await expect(header).toBeHidden();
    await settle(page);
    await settle(page);
    const band = await page.evaluate(() => ({
      scTop: document.querySelector('.ide-editor-area .cm-scroller')!.getBoundingClientRect().top,
      visible: window.visualViewport?.height ?? innerHeight,
    }));
    const caret = await caretLine(page);
    expect(caret, `${p}: the caret line is rendered`).toBeTruthy();
    expect(caret!.text, `${p}: the caret is on the tapped line`).toContain('# line 30');
    expect(caret!.top, `${p}: caret line below the chrome`).toBeGreaterThanOrEqual(band.scTop - 1);
    expect(caret!.bottom, `${p}: caret line above the keyboard`).toBeLessThanOrEqual(band.visible);
    const typingLines = await visibleCodeLines(page);
    test.info().annotations.push({ type: 'code rows', description: `${p}: reading ${before}, keyboard up ${typingLines}` });
    console.log(`[ide-phone-good] ${p}: code rows reading=${before} keyboard=${typingLines}`);
    if (p === 'iphone-se') expect(typingLines, 'iPhone SE keeps ≥ 8 rows of code with the keyboard up').toBeGreaterThanOrEqual(8);
    await page.keyboard.type('Q');
    await expect.poll(async () => (await caretLine(page))?.text ?? '').toMatch(/value_30 .*Q|Q.*# line 30/);
    await shot(page, 'typing', ['dark', 'light']);

    // ── keyboard closes: chrome back; ⋯ → the touch verbs; Go to Line ──
    await page.keyboard.press('Backspace');
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
    await page.setViewportSize(vp);
    await expect(page.getByTestId('ide-page')).not.toHaveClass(/\btyping\b/);
    await page.getByTestId('ide-editor-touch-more').click();
    const menu = page.locator('.ctx-menu');
    for (const label of ['Go to Definition', 'Find Usages', 'Show Documentation', 'Rename…', 'Find in File…', 'Go to Line…']) {
      await expect(menu.getByRole('menuitem', { name: label, exact: true }).first(), `${p}: ${label}`).toBeVisible();
    }
    await expect(menu.getByText('⌘B')).toHaveCount(0);
    await shot(page, 'touch-menu');
    await menu.getByRole('menuitem', { name: 'Go to Line…', exact: true }).first().click();
    const input = page.getByTestId('ide-search-input');
    await expect(input).toBeVisible();
    await input.fill(':13');
    await input.press('Enter');
    await expect.poll(async () => (await caretLine(page))?.text ?? '', { timeout: 10_000 }).toContain('# line 13');
    await expectNoHorizontalOverflow(page);
  });
});

test.describe('IDE debugger on a phone: the step bar over the code', () => {
  test('paused: a 44 px step bar on top of the editor, clear of the paused line; Step over advances', async ({ page }, info) => {
    test.skip(!PHONES.includes(info.project.name), 'phones');
    test.setTimeout(150_000);
    const p = info.project.name;
    const { root, wsId } = await seedWorkspace({ 'loop.py': LOOP }, 'dbg');
    const c = await apiCtx();
    let sessionId = '';
    try {
      const target = { kind: 'local', workspace_id: wsId, root, connection_id: null };
      const program = join(root, 'loop.py');
      const started = await daemonCall<{ session: { id: string } }>(c.ctx, c.base, c.token, 'POST', '/ide/debug/sessions', {
        target, project_folders: [root], language: 'python',
        launch: { program, args: [], cwd: root, env: {}, inherit_shell_env: true },
        breakpoints: [{ path: program, line: 4, enabled: true }], break_on_exceptions: false,
      });
      sessionId = started.session.id;
      await expect.poll(async () => (await daemonCall<{ session: { state: string } }>(c.ctx, c.base, c.token, 'GET', `/ide/debug/sessions/${sessionId}`)).session.state, { timeout: 30_000 }).toBe('paused');

      // The debugger panel (Tools) re-attaches the live session; then back to the code.
      await boot(page, root, wsId, 'tools');
      await expect(page.getByTestId('ide-bottom-dock')).toBeVisible({ timeout: 20_000 });
      const dockDebug = page.getByTestId('ide-tab-debug');
      if (await dockDebug.count()) await dockDebug.click();
      await expect(page.getByTestId('ide-debug-status')).toContainText('Paused', { timeout: 30_000 });
      const editorTab = page.getByTestId('ide-stack-tab-editor');
      if ((await editorTab.getAttribute('aria-pressed')) !== 'true') await editorTab.click();

      const bar = page.getByTestId('ide-touch-debug-bar');
      await expect(bar).toBeVisible({ timeout: 20_000 });
      const where = page.getByTestId('ide-touch-debug-where');
      await expect(where).toHaveText('loop.py:4');
      for (const id of ['ide-touch-debug-resume', 'ide-touch-debug-step-over', 'ide-touch-debug-step-into', 'ide-touch-debug-step-out', 'ide-touch-debug-stop']) {
        const b = (await page.getByTestId(id).boundingBox())!;
        expect(b.width, `${p}: ${id} width`).toBeGreaterThanOrEqual(43.9);
        expect(b.height, `${p}: ${id} height`).toBeGreaterThanOrEqual(43.9);
        expect(await onTop(page.getByTestId(id)), `${p}: ${id} is on top`).toBe(true);
      }
      // The paused line is not under the bar.
      await expect.poll(async () => page.evaluate(() => {
        const line = document.querySelector('.ide-editor-area .cm-ide-paused-line')?.getBoundingClientRect();
        const b = document.querySelector('[data-testid=ide-touch-debug-bar]')!.getBoundingClientRect();
        return line ? line.bottom <= b.top : false;
      }), { timeout: 10_000 }).toBe(true);
      await expectNoHorizontalOverflow(page);
      await shot(page, 'debug-paused', ['dark', 'light']);

      await page.getByTestId('ide-touch-debug-step-over').click();
      await expect(where).not.toHaveText('loop.py:4', { timeout: 20_000 });
      await expect(where).toHaveText(/^loop\.py:\d+$/, { timeout: 20_000 });
      await expect(bar).toBeVisible();
      await shot(page, 'debug-stepped');
    } finally {
      if (sessionId) await c.ctx.post(`${c.base}/api/v1/ide/debug/sessions/${encodeURIComponent(sessionId)}/command`, { data: { action: 'stop' } }).catch(() => undefined);
      await c.ctx.dispose();
      rmSync(root, { recursive: true, force: true });
    }
  });
});

test.describe('IDE on a touch tablet', () => {
  test('iPad landscape keeps the workbench with finger-sized editor tabs', async ({ page }, info) => {
    test.skip(info.project.name !== 'ipad-landscape', 'touch tablet');
    test.setTimeout(90_000);
    const { root, wsId } = await seedWorkspace({ 'settlement.py': SOURCE }, 'ipad');
    try {
      await page.addInitScript(({ ws, dir }) => {
        localStorage.setItem('otto_workspace', ws);
        localStorage.setItem('otto_scheme', 'dark');
        localStorage.setItem('otto.ide.namedProjects.v1', JSON.stringify([{ id: 'proj-phone-ipad', name: 'Phone good', folders: [{ path: dir, name: 'phone' }], updatedAt: Date.now() }]));
      }, { ws: wsId, dir: root });
      await page.goto(`/#/ide?root=${encodeURIComponent(root)}&file=${encodeURIComponent(join(root, 'settlement.py'))}`);
      await expect(page.getByTestId('ide-page')).not.toHaveClass(/\bstack\b/, { timeout: 20_000 });
      await expect(page.getByTestId('ide-tabs-strip').locator('.ide-tab.active')).toContainText('settlement.py', { timeout: 30_000 });
      const h = (await page.locator('.ide-tabs-row').boundingBox())!.height;
      expect(h, 'editor tab row on a coarse pointer').toBeGreaterThanOrEqual(40);
      await expectNoHorizontalOverflow(page);
      await shot(page, 'workbench');
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });
});
