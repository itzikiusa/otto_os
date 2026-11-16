import { test, expect, type Locator, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expectNoHorizontalOverflow } from './helpers';
import { daemonCall } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

// The IDE's dialogs on a phone (round 5, IDE-PHONE-2):
//  - Local History is list → detail: the diff gets ≥ 60 % of the visible height, the
//    snapshots are a step of their own ("‹ Snapshots"), the ✕ is a 44 px target on top;
//  - the disk-changed banner's Compare reads "On disk", and the banner wraps at 375 px;
//  - the run-configuration footer is one row (⋯ + Save), nothing scrolls sideways;
//  - the phone ⋯ chooses the run target, and Run launches it;
//  - quick-open on a phone on its side is a top sheet that shows ≥ 4 results;
//  - on an iPad the IDE folds Otto's navigator to its rail and gets the workbench.
// Screenshots (OTTO_E2E_SHOTS) → fix-IDE-PHONE-2/.

const PHONES = ['iphone-portrait', 'iphone-landscape', 'iphone-se'];
const TABLETS = ['ipad-portrait', 'ipad-landscape'];
const MAIN = ['def main():', ...Array.from({ length: 30 }, (_, i) => `    value_${i} = ${i}`), '', 'main()', ''].join('\n');

async function shot(page: Page, name: string, schemes: readonly ('light' | 'dark')[] = ['dark']): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-IDE-PHONE-2'), { recursive: true });
  for (const scheme of schemes) {
    await page.emulateMedia({ colorScheme: scheme });
    // The scheme is `data-scheme`; `data-theme` names the theme (native / warm / pro-dark).
    await page.evaluate((s) => { document.documentElement.dataset.scheme = s; }, scheme).catch(() => undefined);
    await page.screenshot({ path: join(dir, 'fix-IDE-PHONE-2', `${test.info().project.name}-${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.evaluate(() => { document.documentElement.dataset.scheme = 'dark'; }).catch(() => undefined);
}

async function onTop(loc: Locator): Promise<boolean> {
  const box = (await loc.boundingBox())!;
  return loc.evaluate((el, p) => {
    const hit = document.elementFromPoint(p.x, p.y);
    return !!hit && (hit === el || el.contains(hit));
  }, { x: box.x + box.width / 2, y: box.y + box.height / 2 });
}

interface Seed { root: string; wsId: string; target: Record<string, unknown>; cfgId: string }

async function seedWorkspace(label: string): Promise<Seed> {
  const root = realpathSync(mkdtempSync(join(tmpdir(), `otto-e2e-ide-dialogs-${label}-`)));
  writeFileSync(join(root, 'main.py'), MAIN);
  for (let i = 0; i < 12; i++) writeFileSync(join(root, `handler_${i}.py`), `def handler_${i}():\n    return ${i}\n`);
  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', { name: `IDE dialogs ${label} ${Date.now()}`, root_path: root });
    const target = { kind: 'local', workspace_id: ws.id, root, connection_id: null };
    // Two recorded versions → Local History has snapshots to compare.
    await daemonCall(c.ctx, c.base, c.token, 'PUT', '/ide/fs/file', { target, path: 'main.py', content: `${MAIN}# v1\n` });
    await daemonCall(c.ctx, c.base, c.token, 'PUT', '/ide/fs/file', { target, path: 'main.py', content: `${MAIN}# v2\n` });
    // Config ids are global on the daemon: one per seed.
    const cfgId = `phone-main-${Date.now()}`;
    await daemonCall(c.ctx, c.base, c.token, 'POST', '/ide/run-configs', {
      target, id: cfgId, name: 'Phone Main Script', runner: 'python', command: 'python3', args: [], cwd: root,
      target_path: join(root, 'main.py'),
    });
    return { root, wsId: ws.id, target, cfgId };
  } finally {
    await c.ctx.dispose();
  }
}

async function boot(page: Page, s: Seed, file = 'main.py'): Promise<void> {
  await page.addInitScript(({ ws, dir }) => {
    if (sessionStorage.getItem('__ide_phone_dialogs')) return;
    sessionStorage.setItem('__ide_phone_dialogs', '1');
    localStorage.setItem('otto_workspace', ws);
    localStorage.setItem('otto_scheme', 'dark');
    localStorage.setItem('otto_direction', 'ltr');
    localStorage.setItem('otto_rail_expanded', '0');
    localStorage.setItem('otto.ide.namedProjects.v1', JSON.stringify([{ id: 'proj-phone-dialogs', name: 'Phone dialogs', folders: [{ path: dir, name: 'dialogs' }], updatedAt: Date.now() }]));
    sessionStorage.setItem('otto.ide.stackPane', 'editor');
  }, { ws: s.wsId, dir: s.root });
  await page.goto(`/#/ide?root=${encodeURIComponent(s.root)}&file=${encodeURIComponent(join(s.root, file))}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await expect(page.locator('.ide-editor-area .cm-content').first()).toContainText('value_3', { timeout: 20_000 });
}

async function moreMenu(page: Page): Promise<Locator> {
  await page.getByTestId('ide-more-btn').click();
  const menu = page.locator('.ctx-menu').first();
  await expect(menu).toBeVisible();
  return menu;
}

test.describe('IDE dialogs on a phone', () => {
  let seed: Seed | null = null;
  test.afterAll(() => { if (seed) rmSync(seed.root, { recursive: true, force: true }); });

  test('Local History: the diff fills the sheet, snapshots are a step, ✕ is 44 px on top', async ({ page }, info) => {
    test.skip(!PHONES.includes(info.project.name), 'phones');
    test.setTimeout(120_000);
    seed ??= await seedWorkspace('lh');
    await boot(page, seed);
    await expect(page.getByTestId('ide-page')).toHaveClass(/\bstack\b/);

    await (await moreMenu(page)).getByRole('menuitem', { name: /^Show local history/ }).click();
    const modal = page.getByTestId('ide-comparison-modal');
    await expect(modal).toBeVisible({ timeout: 15_000 });
    const pane = modal.getByTestId('ide-compare-diff-pane');
    await expect(pane).toBeVisible({ timeout: 15_000 });
    await expect(pane.getByTestId('ide-compare-mode-split'), 'unified only on a phone').toHaveCount(0);

    const vh = await page.evaluate(() => innerHeight);
    const paneBox = (await pane.boundingBox())!;
    expect(paneBox.height / vh, `diff ${paneBox.height}px of ${vh}px`).toBeGreaterThanOrEqual(0.6);
    const sheet = page.locator('.sheet[role="dialog"]').last();
    expect((await sheet.boundingBox())!.width, 'the diff is the full sheet width').toBeLessThanOrEqual(paneBox.width + 4); // the sheet's 1 px borders

    const close = sheet.getByRole('button', { name: 'Close' }).first();
    const cb = (await close.boundingBox())!;
    // 43.9: a 44 px control can measure 43.99999… in layout pixels.
    expect(Math.min(cb.width, cb.height)).toBeGreaterThanOrEqual(43.9);
    expect(await onTop(close), '✕ is on top').toBe(true);
    const restore = modal.getByTestId('ide-local-history-restore');
    await expect(restore).toBeVisible();
    expect(await onTop(restore)).toBe(true);
    await expect(sheet.locator('footer'), 'no wrapping footer: the action sits in the step bar').toHaveCount(0);
    await expectNoHorizontalOverflow(page);
    await shot(page, '01-local-history-diff', ['dark', 'light']);

    // ‹ Snapshots → the list fills the sheet; a pick returns to its diff.
    await modal.getByTestId('ide-compare-back').click();
    const rows = modal.getByTestId('ide-local-history-row');
    await expect(rows.first()).toBeVisible();
    await expect(pane).toHaveCount(0);
    expect((await rows.first().boundingBox())!.height).toBeGreaterThanOrEqual(43.9);
    await shot(page, '02-local-history-list', ['dark', 'light']);
    await rows.nth(1).click();
    await expect(modal.getByTestId('ide-compare-diff-pane')).toBeVisible();

    await close.click();
    await expect(modal).toHaveCount(0);
  });

  test('disk-changed banner: Compare reads "On disk" and the banner wraps at phone width', async ({ page }, info) => {
    test.skip(!PHONES.includes(info.project.name), 'phones');
    test.setTimeout(120_000);
    seed ??= await seedWorkspace('lh');
    await boot(page, seed);
    const content = page.locator('.ide-editor-area .cm-content').first();
    await content.locator('.cm-line', { hasText: 'value_3 = 3' }).click();
    await page.keyboard.press('End');
    await page.keyboard.type('  # mine');
    writeFileSync(join(seed.root, 'main.py'), `${MAIN}# an agent wrote this\n`);
    const banner = page.getByTestId('ide-disk-changed-banner');
    await expect(banner).toBeVisible({ timeout: 15_000 });
    await page.locator(':focus').evaluate((el) => (el as HTMLElement).blur()).catch(() => undefined);
    const bb = (await banner.boundingBox())!;
    for (const id of ['ide-disk-compare', 'ide-disk-reload', 'ide-disk-keep']) {
      const b = (await banner.getByTestId(id).boundingBox())!;
      expect(b.x + b.width, `${id} inside the banner`).toBeLessThanOrEqual(bb.x + bb.width + 1);
    }
    await expectNoHorizontalOverflow(page);
    await shot(page, '03-disk-banner');
    await banner.getByTestId('ide-disk-compare').click();
    const sheet = page.locator('.sheet[role="dialog"]').last();
    await expect(sheet).toHaveAttribute('aria-label', /^On disk vs current — main\.py/);
    await expect(sheet.getByTestId('ide-compare-diff-pane')).toBeVisible({ timeout: 15_000 });
    await shot(page, '04-disk-compare');
    await sheet.getByRole('button', { name: 'Close' }).first().click();
    await page.getByTestId('ide-disk-keep').click();
  });

  test('run configurations: one-row footer with ⋯; run target from the phone ⋯ is what Run launches', async ({ page }, info) => {
    test.skip(!PHONES.includes(info.project.name), 'phones');
    test.setTimeout(120_000);
    seed ??= await seedWorkspace('lh');
    await boot(page, seed);

    await (await moreMenu(page)).getByRole('menuitem', { name: /^Edit run configurations/ }).click();
    const sheet = page.locator('.sheet[role="dialog"]').last();
    await expect(sheet).toBeVisible();
    await sheet.getByRole('button', { name: /Phone Main Script/ }).first().click();
    const footer = sheet.locator('footer');
    const save = footer.getByTestId('ide-cfg-save');
    const more = footer.getByTestId('ide-cfg-more');
    await expect(save).toBeVisible();
    await expect(footer.getByRole('button', { name: 'Cancel' })).toHaveCount(0);
    const tops = await footer.locator('button').evaluateAll((els) => els.map((e) => Math.round(e.getBoundingClientRect().top)));
    expect(new Set(tops).size, `one row: ${tops}`).toBe(1);
    const fb = (await footer.boundingBox())!;
    expect(fb.height).toBeLessThanOrEqual(64);
    const sideways = await sheet.locator('.sheet-body').evaluate((el) => {
      const right = el.getBoundingClientRect().right;
      const wide = [...el.querySelectorAll('*')].filter((c) => c.getBoundingClientRect().right > right + 1).slice(0, 3)
        .map((c) => `${c.tagName}.${[...c.classList].join('.')}`);
      return { over: el.scrollWidth - el.clientWidth, wide };
    });
    expect(sideways.over, `the form fits the sheet: ${sideways.wide.join(', ')}`).toBeLessThanOrEqual(1);
    await expectNoHorizontalOverflow(page);
    await shot(page, '05-run-configs');
    await more.click();
    const menu = page.locator('.ctx-menu').first();
    for (const label of ['Save & run', 'Save & debug', 'Duplicate', 'Delete…']) await expect(menu.getByRole('menuitem', { name: label })).toBeVisible();
    await shot(page, '06-run-configs-more');
    await page.keyboard.press('Escape');
    await expect(menu).toHaveCount(0);
    await sheet.getByRole('button', { name: 'Close' }).first().click();
    await expect(sheet).toHaveCount(0);

    // ⋯ → "Run target: <what Run launches>" › a target. Opening a saved config in the
    // editor above selected it (as on the desktop), so go back to Current file first.
    const pick = async (name: RegExp, shotName?: string): Promise<void> => {
      const m = await moreMenu(page);
      await m.getByRole('menuitem', { name: /^Run target: / }).click();
      const item = page.locator('.ctx-menu').getByRole('menuitemcheckbox', { name }).first();
      await expect(item).toBeVisible();
      if (shotName) await shot(page, shotName);
      await item.click();
      await expect(page.locator('.ctx-menu')).toHaveCount(0);
    };
    const expectTarget = async (label: string): Promise<void> => {
      const m = await moreMenu(page);
      await expect(m.getByRole('menuitem', { name: label })).toBeVisible();
      await page.keyboard.press('Escape');
      await expect(page.locator('.ctx-menu')).toHaveCount(0);
    };
    await pick(/^Current file/);
    await expectTarget('Run target: Current file (main.py)');
    await pick(/^Phone Main Script$/, '07-run-target-menu');
    await expectTarget('Run target: Phone Main Script');

    const runReq = page.waitForRequest((r) => r.url().includes('/ide/runs') && r.method() === 'POST');
    await page.getByTestId('ide-run-btn').first().click();
    expect((await runReq).postDataJSON().config_id).toBe(seed.cfgId);
  });

  test('quick-open on a phone: a top sheet with several results', async ({ page }, info) => {
    test.skip(!PHONES.includes(info.project.name), 'phones');
    test.setTimeout(90_000);
    seed ??= await seedWorkspace('lh');
    await boot(page, seed);
    await (await moreMenu(page)).getByRole('menuitem', { name: /^Search everywhere/ }).click();
    const qo = page.getByTestId('ide-quick-open');
    await expect(qo).toBeVisible();
    await page.getByTestId('ide-search-input').fill('handler');
    const rows = qo.getByTestId('ide-qo-row');
    await expect(rows.nth(4)).toBeAttached({ timeout: 15_000 });
    const box = (await qo.boundingBox())!;
    const vw = await page.evaluate(() => innerWidth);
    expect(box.width, 'full-width sheet').toBeGreaterThanOrEqual(vw - 2);
    expect(box.y, 'anchored at the top').toBeLessThanOrEqual(60);
    const shown = await rows.evaluateAll((els) => {
      const list = (els[0]?.closest('.se-results') as HTMLElement | null)?.getBoundingClientRect();
      return list ? els.filter((e) => { const r = e.getBoundingClientRect(); return r.top >= list.top - 1 && r.bottom <= list.bottom + 1; }).length : 0;
    });
    expect(shown, 'results fully visible').toBeGreaterThanOrEqual(4);
    await shot(page, '08-quick-open');
    await page.keyboard.press('Escape');
  });

  test('tablet: the IDE folds the navigator to its rail and gets the workbench; leaving restores it', async ({ page }, info) => {
    test.skip(!TABLETS.includes(info.project.name), 'tablets');
    test.setTimeout(90_000);
    seed ??= await seedWorkspace('lh');
    await boot(page, seed);
    // iPad landscape (1194 px) already gets the desktop shell; portrait has the tablet one.
    const tabletShell = info.project.name === 'ipad-portrait';
    if (tabletShell) await expect(page.locator('.sidebar.msidebar')).toHaveClass(/\bmfolded\b/);
    await expect(page.getByTestId('ide-page')).not.toHaveClass(/\bstack\b/, { timeout: 10_000 });
    await expect(page.getByTestId('ide-run-config-select'), 'the workbench toolbar').toBeVisible();
    await expectNoHorizontalOverflow(page);
    await shot(page, '09-tablet-workbench', ['dark', 'light']);
    await page.goto('/#/agents');
    if (tabletShell) await expect(page.locator('.sidebar.msidebar')).not.toHaveClass(/\bmfolded\b/, { timeout: 10_000 });
    await shot(page, '10-tablet-left-ide');
  });
});
