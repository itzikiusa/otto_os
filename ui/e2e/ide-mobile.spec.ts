import { test, expect, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, realpathSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { daemonCall, openPolyglotFile, seedPolyglotFiles } from './ide-polyglot-fixtures';
import { apiCtx } from './seed';

// IDE on phones & tablets.
//
// The IDE picks its layout from its OWN size (shell/ideLayout.svelte.ts):
// under 700 px wide or 520 px tall it stacks one pane at a time (Files ·
// Editor · Tools · Agent), otherwise it is the workbench. A phone on its side
// (~814×380 here) is short, so it stays stacked; the shell's "short" rule
// turns the Navigator into a closable drawer and drops the status bar there.
//
// Guards the owner's report ("phone on its side: half the screen empty, the
// sidebar is open and I cannot close it") and the review's defects: rotation
// keeps the pane and file and never opens desktop docks, one ⋯ in the header,
// Run on screen, a tap on a file opens the editor, Terminal reachable, the
// Settings modal usable at phone width.
//
// Screenshots + a small JSON audit per capture go to OTTO_IDE_MOBILE_SHOTS_DIR
// (default: a temp folder) for visual review.

const SHOTS = process.env.OTTO_IDE_MOBILE_SHOTS_DIR ?? join(tmpdir(), 'otto-ide-mobile-shots');
const PORTRAIT = { width: 430, height: 740 };
const LANDSCAPE = { width: 814, height: 380 };
const DOCK_OPEN_KEY = 'otto.ide.dockOpen';
const PANES = ['files', 'editor', 'tools', 'agent'] as const;
type Pane = (typeof PANES)[number];

interface Seed { workspaceId: string; workspaceRoot: string }
let seed: Seed | null = null;
// Own seed (not createPolyglotSeed): that one also posts a run config with a
// FIXED id, which the daemon refuses for a second workspace.
async function getSeed(): Promise<Seed> {
  if (seed) return seed;
  const workspaceRoot = realpathSync(mkdtempSync(join(tmpdir(), 'otto-ide-mobile-')));
  seedPolyglotFiles(workspaceRoot);
  const c = await apiCtx();
  try {
    const ws = await daemonCall<{ id: string }>(c.ctx, c.base, c.token, 'POST', '/workspaces', {
      name: `IDE Mobile ${Date.now()}`,
      root_path: workspaceRoot,
    });
    await daemonCall(c.ctx, c.base, c.token, 'POST', '/repos', { path: workspaceRoot, workspace_id: ws.id }).catch(() => undefined);
    seed = { workspaceId: ws.id, workspaceRoot };
    return seed;
  } finally {
    await c.ctx.dispose();
  }
}

/** Boot #/ide on the seeded project (phone: the editor pane comes first). */
async function bootIde(page: Page, scheme: 'dark' | 'light' = 'dark'): Promise<void> {
  const s = await getSeed();
  await page.addInitScript(
    ({ wsId, root, uiScheme }) => {
      if (sessionStorage.getItem('__ide_mobile_init')) return;
      sessionStorage.setItem('__ide_mobile_init', '1');
      localStorage.setItem('otto_workspace', wsId);
      localStorage.setItem('otto_rail_expanded', '0');
      localStorage.setItem('otto_scheme', uiScheme);
      localStorage.setItem('otto_direction', 'ltr');
      localStorage.setItem('otto.ide.agent.provider', 'claude');
      localStorage.setItem(
        'otto.ide.namedProjects.v1',
        JSON.stringify([{ id: 'proj-polyglot', name: 'Otto IDE Polyglot Project', folders: [{ path: root, name: 'otto-polyglot' }], updatedAt: Date.now() }]),
      );
    },
    { wsId: s.workspaceId, root: s.workspaceRoot, uiScheme: scheme },
  );
  await page.goto('/#/ide');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('ide-editor-area').or(page.getByTestId('ide-project-tree')).first()).toBeVisible({ timeout: 20_000 });
}

/** Two frames: layout, ResizeObservers and Svelte effects have run. */
async function settle(page: Page): Promise<void> {
  await page.evaluate(() => new Promise<void>((r) => requestAnimationFrame(() => requestAnimationFrame(() => r()))));
}

async function shot(page: Page, name: string): Promise<void> {
  mkdirSync(SHOTS, { recursive: true });
  await settle(page);
  await page.screenshot({ path: join(SHOTS, `${name}.png`), fullPage: true });
  const audit = await page.evaluate(() => {
    const box = (sel: string): string => {
      const el = [...document.querySelectorAll(sel)].find((e) => e.getBoundingClientRect().height > 0);
      if (!el) return '-';
      const r = el.getBoundingClientRect();
      return `${Math.round(r.width)}x${Math.round(r.height)} @${Math.round(r.left)},${Math.round(r.top)}`;
    };
    return {
      vw: innerWidth,
      vh: innerHeight,
      hScroll: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      topbar: box('.mtopbar'),
      header: box('[data-testid=page-header]'),
      navigator: box('.shell-main > .sidebar'),
      editor: box('.ide-editor-area .cm-editor'),
      tree: box('[data-testid=ide-project-tree]'),
      dock: box('[data-testid=ide-bottom-dock]'),
      agent: box('[data-testid=ide-agent-panel]'),
      bottomNav: box('.bottomnav'),
      statusBar: box('.statusbar'),
      gutters: [...document.querySelectorAll('.ide-editor-area .cm-gutter')].map((g) => `${g.className.replace(/\s+/g, '.')}=${(g as HTMLElement).offsetWidth}`),
    };
  });
  writeFileSync(join(SHOTS, `${name}.json`), JSON.stringify(audit, null, 2));
}

async function showPane(page: Page, pane: Pane): Promise<void> {
  const tab = page.getByTestId(`ide-stack-tab-${pane}`);
  await tab.click();
  await expect(tab).toHaveAttribute('aria-pressed', 'true');
}

async function expectPane(page: Page, pane: Pane): Promise<void> {
  await expect(page.getByTestId(`ide-stack-tab-${pane}`)).toHaveAttribute('aria-pressed', 'true');
  if (pane === 'files') await expect(page.getByTestId('ide-project-tree')).toBeVisible();
  if (pane === 'editor') await expect(page.locator('.ide-editor-area .cm-editor').first()).toBeVisible();
  if (pane === 'tools') await expect(page.getByTestId('ide-bottom-dock')).toBeVisible();
  if (pane === 'agent') await expect(page.getByTestId('ide-agent-panel')).toBeVisible();
}

async function heightOf(page: Page, selector: string): Promise<number> {
  const box = await page.locator(selector).first().boundingBox();
  return box?.height ?? 0;
}

/** One toolbar row: a single ⋯, every header control on screen, Run visible. */
async function expectHeaderFits(page: Page, what: string): Promise<void> {
  const header = page.getByTestId('page-header').first();
  await expect(header.getByRole('button', { name: 'More actions' }), `${what}: exactly one ⋯ in the header`).toHaveCount(1);
  await expectFullyInViewport(page, page.getByTestId('ide-more-btn'), `${what}: header ⋯`);
  await expectFullyInViewport(page, page.getByTestId('ide-run-btn').first(), `${what}: Run`);
}

async function openFileByTap(page: Page): Promise<void> {
  await showPane(page, 'files');
  const tree = page.getByTestId('ide-project-tree');
  const dir = tree.locator('button.tree-item[title="app"], button.tree-item[title$="/app"]').first();
  await expect(dir).toBeVisible({ timeout: 15_000 });
  if ((await dir.getAttribute('aria-expanded')) !== 'true') await dir.click();
  const file = tree.locator('button.tree-item[title="app/main.go"], button.tree-item[title$="/app/main.go"]').first();
  await expect(file).toBeVisible();
  // ONE tap — a phone has no double-click.
  await file.click();
}

async function dockOpenPref(page: Page): Promise<string | null> {
  return page.evaluate((k) => localStorage.getItem(k), DOCK_OPEN_KEY);
}

test.describe('IDE on phones and tablets', () => {
  test.describe.configure({ mode: 'serial' });

  test('stacked phone layout: one ⋯, Run on screen, a tap opens the file, every pane reachable', async ({ page }, info) => {
    test.skip(!['iphone-portrait', 'iphone-landscape', 'iphone-se', 'android-phone'].includes(info.project.name), 'phones');
    test.setTimeout(150_000);
    const p = info.project.name;
    await bootIde(page);
    await expect(page.getByTestId('ide-page')).toHaveClass(/\bstack\b/);
    await expect(page.getByTestId('ide-stack-switcher')).toBeVisible();
    await expectHeaderFits(page, p);
    await expectNoHorizontalOverflow(page);
    // Finger-sized pane switcher.
    for (const pane of PANES) {
      const h = (await page.getByTestId(`ide-stack-tab-${pane}`).boundingBox())?.height ?? 0;
      expect(h, `${p}: ${pane} tab height`).toBeGreaterThanOrEqual(36);
    }
    const dockPrefBefore = await dockOpenPref(page);

    await openFileByTap(page);
    await expectPane(page, 'editor');
    await expect(page.locator('.ide-editor-area [aria-label="Open files"]').getByText('main.go').first()).toBeVisible();
    const vh = page.viewportSize()!.height;
    const editorH = await heightOf(page, '.ide-editor-area .cm-editor');
    expect(editorH, `${p}: editor height`).toBeGreaterThanOrEqual(vh < 520 ? Math.min(200, vh * 0.5) : vh * 0.5);
    // Slim gutter on a phone: no fold column, a narrow breakpoint lane.
    const gutterW = (await page.locator('.ide-editor-area .cm-gutters').first().boundingBox())?.width ?? 0;
    const gutters = await page.locator('.ide-editor-area .cm-gutter').evaluateAll((els) =>
      els.map((g) => `${g.className}=${(g as HTMLElement).offsetWidth}`).join(', '),
    );
    expect(gutterW, `${p}: gutter width (${gutters})`).toBeLessThanOrEqual(60);
    await shot(page, `${p}--03-editor-dark`);

    await showPane(page, 'files');
    await shot(page, `${p}--02-files-dark`);

    await showPane(page, 'tools');
    const terminalTab = page.getByTestId('ide-tab-terminal');
    await expectFullyInViewport(page, terminalTab, `${p}: Terminal tab`);
    await terminalTab.click();
    await expect(page.getByTestId('ide-terminal-panel')).toBeVisible({ timeout: 15_000 });
    await expect(page.locator('.ide-editor-area')).toBeHidden();
    expect(await dockOpenPref(page), `${p}: the Tools pane never writes the desktop dock preference`).toBe(dockPrefBefore);
    await shot(page, `${p}--05-tools-terminal-dark`);

    await showPane(page, 'agent');
    await expectPane(page, 'agent');
    await shot(page, `${p}--07-agent-dark`);
    await showPane(page, 'editor');

    // ⋯ holds what the one-row header doesn't show; no desktop-only toggles.
    await page.getByTestId('ide-more-btn').click();
    const menu = page.locator('.ctx-menu').first();
    await expectFullyInViewport(page, menu, `${p}: ⋯ menu`);
    await expect(menu.getByRole('menuitem', { name: /^Debug/ })).toBeVisible();
    await expect(menu.getByRole('menuitem', { name: /Hide project explorer|bottom tool dock/ })).toHaveCount(0);
    await shot(page, `${p}--08-more-menu-dark`);
    await menu.getByRole('menuitem', { name: /Project SDKs & Toolchains/ }).click();
    const modal = page.getByTestId('ide-settings-modal');
    await expect(modal).toBeVisible();
    // The settings page gets most of the modal (narrow: all of it, the
    // category list is a screen of its own) — not a 100 px column.
    const right = await page.locator('.st-right').first().boundingBox();
    const whole = await modal.boundingBox();
    expect(right?.width ?? 0, `${p}: settings page width`).toBeGreaterThanOrEqual((whole?.width ?? 0) * 0.6);
    await shot(page, `${p}--12-modal-settings-sdks-dark`);
    if (await page.getByTestId('ide-settings-categories-btn').isVisible()) {
      await page.getByTestId('ide-settings-categories-btn').click();
      await expect(page.locator('.st-left')).toBeVisible();
      await shot(page, `${p}--12b-modal-settings-list-dark`);
    }
    await page.keyboard.press('Escape');
    await expect(modal).toBeHidden();
    await expectNoHorizontalOverflow(page);
  });

  test('light theme: stacked panes', async ({ page }, info) => {
    test.skip(info.project.name !== 'iphone-portrait', 'one phone is enough for colour');
    test.setTimeout(90_000);
    await bootIde(page, 'light');
    for (const pane of PANES) {
      await showPane(page, pane);
      await expectPane(page, pane);
      await shot(page, `iphone-portrait--1x-${pane}-light`);
    }
  });

  test('rotation keeps the pane and file, never opens desktop docks, and the navigator closes', async ({ page }, info) => {
    test.skip(info.project.name !== 'iphone-portrait', 'rotation is driven from the iphone-portrait project');
    test.setTimeout(180_000);
    await bootIde(page);
    await openFileByTap(page);
    await expectPane(page, 'editor');
    const dockPrefBefore = await dockOpenPref(page);

    for (const pane of PANES) {
      await page.setViewportSize(PORTRAIT);
      await showPane(page, pane);
      await page.setViewportSize(LANDSCAPE);
      await settle(page);
      // Still stacked, same pane, no desktop chrome in ~380 px of height.
      await expect(page.getByTestId('ide-page')).toHaveClass(/\bstack\b/);
      await expectPane(page, pane);
      await expect(page.locator('.shell-main > .sidebar')).toHaveCount(0);
      await expect(page.locator('.statusbar')).toHaveCount(0);
      await expect(page.locator('.ide-agent-wrap')).toHaveCount(pane === 'agent' ? 1 : 0);
      await expect(page.getByRole('button', { name: 'Open navigator' })).toBeVisible();
      await expectHeaderFits(page, `landscape from ${pane}`);
      if (pane === 'editor') {
        expect(await heightOf(page, '.ide-editor-area .cm-editor'), 'landscape editor height').toBeGreaterThanOrEqual(200);
      }
      await expectNoHorizontalOverflow(page);
      await shot(page, `rotate--${pane}-2-landscape`);
      await page.setViewportSize(PORTRAIT);
      await expectPane(page, pane);
      await shot(page, `rotate--${pane}-3-back-portrait`);
    }
    await showPane(page, 'editor');
    await expect(page.locator('.ide-editor-area [aria-label="Open files"]').getByText('main.go').first()).toBeVisible();
    expect(await dockOpenPref(page), 'no pane or rotation wrote the desktop dock preference').toBe(dockPrefBefore);

    // The navigator drawer: rotation closes it; on its side it opens, carries
    // the palette + machine row the hidden bottom nav / status bar held, closes.
    const drawer = page.getByRole('dialog', { name: 'Navigator' });
    await page.getByRole('button', { name: 'Open navigator' }).click();
    await expect(drawer).toBeVisible();
    await page.setViewportSize(LANDSCAPE);
    await expect(drawer).toBeHidden();
    await page.getByRole('button', { name: 'Open navigator' }).click();
    await expect(drawer).toBeVisible();
    await expectFullyInViewport(page, drawer.getByTestId('short-nav-extras').getByRole('button', { name: 'Commands' }), 'drawer Commands row');
    await shot(page, 'rotate--drawer-2-landscape');
    await page.keyboard.press('Escape');
    await expect(drawer).toBeHidden();
    await page.setViewportSize(PORTRAIT);
    await expect(drawer).toBeHidden();
    await shot(page, 'rotate--drawer-3-back-portrait');

    // On-screen keyboard (approximation: the app follows visualViewport's
    // height, lib/viewportFit.ts): typing hides the IDE header on a tiny page.
    await showPane(page, 'editor');
    await page.locator('.ide-editor-area .cm-content').first().click();
    await page.setViewportSize({ width: PORTRAIT.width, height: 404 });
    await settle(page);
    expect(await heightOf(page, '.ide-editor-area .cm-editor'), 'portrait + keyboard editor height').toBeGreaterThanOrEqual(150);
    await shot(page, 'rotate--keyboard-portrait-editor');
    await page.setViewportSize({ width: LANDSCAPE.width, height: 150 });
    await settle(page);
    expect(await heightOf(page, '.ide-editor-area .cm-editor'), 'landscape + keyboard editor height').toBeGreaterThanOrEqual(60);
    await shot(page, 'rotate--keyboard-landscape-editor');
  });

  test('tablet: Run on screen, one ⋯, at most one side panel beside the editor', async ({ page }, info) => {
    test.skip(!['ipad-portrait', 'ipad-landscape'].includes(info.project.name), 'tablets');
    test.setTimeout(120_000);
    const p = info.project.name;
    await bootIde(page);
    await expectHeaderFits(page, p);
    await expectNoHorizontalOverflow(page);
    const stacked = /\bstack\b/.test((await page.getByTestId('ide-page').getAttribute('class')) ?? '');
    if (stacked) {
      await openFileByTap(page);
      await expectPane(page, 'editor');
      await shot(page, `${p}--20-stack-editor`);
      await showPane(page, 'agent');
      await expectPane(page, 'agent');
      await shot(page, `${p}--21-stack-agent`);
    } else {
      await openPolyglotFile(page, 'app', 'main.go');
      await page.getByTestId('ide-toggle-agent').click();
      await expect(page.getByTestId('ide-agent-panel')).toBeVisible();
      const editorW = (await page.locator('.ide-editor-area').first().boundingBox())?.width ?? 0;
      expect(editorW, `${p}: editor width with the agent open`).toBeGreaterThanOrEqual(360);
      await shot(page, `${p}--21-workbench-agent`);
    }
  });
});
