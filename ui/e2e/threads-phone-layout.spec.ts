import { test, expect, type APIRequestContext, type Locator, type Page } from '@playwright/test';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// ─────────────────────────────────────────────────────────────────────────────
// Threads on a phone with the on-screen keyboard up, on a REAL fake-agent
// thread (otto-fake-codex: echo turns with a code block and a diff, then `ask`
// = a pending approval). Playwright cannot open a keyboard: like the reviewer
// we shrink the viewport to what a keyboard leaves while the composer has
// focus — lib/viewportFit.ts reads exactly that (a text field focused + the
// visible height dropping at the same width) and sets html[data-kb="open"].
//   • the composer's Send is fully on screen and on top;
//   • the timeline keeps ≥ 3 lines of conversation;
//   • the BottomNav is gone; no horizontal page scroll;
//   • landscape: one header row, no duplicated toggles;
//   • code wraps on a phone, a diff is red/green;
//   • iPad portrait: thread list and conversation side by side.
// Screenshots (OTTO_E2E_SHOTS) → fix-T-PHONE/.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

const PHONES = /^(iphone-portrait|iphone-landscape|iphone-se|android-phone)$/;

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let fixtureDir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-threads-phone-'));
  writeFileSync(join(fixtureDir, 'README.md'), '# Threads on a phone\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Phone Layout WS', root_path: fixtureDir } });
  expect(r.ok(), await r.text()).toBeTruthy();
  wsId = (await r.json()).id as string;
});

test.afterAll(async () => {
  if (ctx && wsId) {
    const rows = (await (await ctx.get(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`)).json()) as { id: string }[];
    for (const s of rows) await ctx.post(`${base}/api/v1/runtime-sessions/${s.id}/close`).catch(() => {});
  }
  await ctx?.dispose();
  if (fixtureDir) rmSync(fixtureDir, { recursive: true, force: true });
});

test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
  }, wsId);
});

async function shot(page: Page, name: string, schemes: readonly ('light' | 'dark')[] = ['light']): Promise<void> {
  const dir = process.env.OTTO_E2E_SHOTS;
  if (!dir) return;
  mkdirSync(join(dir, 'fix-T-PHONE'), { recursive: true });
  for (const scheme of schemes) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.screenshot({ path: join(dir, 'fix-T-PHONE', `${test.info().project.name}-${name}-${scheme}.png`), animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });
}

async function onTop(loc: Locator): Promise<boolean> {
  const box = (await loc.boundingBox())!;
  return loc.evaluate((el, p) => {
    const hit = document.elementFromPoint(p.x, p.y);
    return !!hit && (hit === el || el.contains(hit));
  }, { x: box.x + box.width / 2, y: box.y + box.height / 2 });
}

const CODE = [
  'code',
  '```ts',
  'const aVeryLongIdentifierNameForTheTest = someFunctionCall(argumentNumberOne, argumentNumberTwo, argumentNumberThree);',
  '```',
  '```diff',
  '@@ -1,2 +1,2 @@',
  '-const total = items.length - 1;',
  '+const total = items.length;',
  '```',
].join('\n');

/** A thread with three finished turns (one with a code block and a diff) and
 *  a pending approval. */
async function richThread(page: Page): Promise<{ id: string; thread: Locator; composer: Locator }> {
  const created = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, {
    data: { provider: 'codex', cwd: fixtureDir, title: 'Phone layout thread' },
  });
  expect(created.ok(), await created.text()).toBeTruthy();
  const id = (await created.json()).id as string;
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  const composer = thread.getByTestId('runtime-composer');
  for (const [prompt, done] of [
    ['First, look at the failing invoice rounding test and tell me what it checks.', 'echo: First'],
    ['Then explain the off-by-one in a short paragraph so I can follow along on the phone.', 'echo: Then'],
  ] as const) {
    await composer.fill(prompt);
    await thread.getByTestId('runtime-send').click();
    await expect(thread.getByText(done, { exact: false }).last()).toBeVisible({ timeout: 20_000 });
  }
  await composer.fill(CODE);
  await thread.getByTestId('runtime-send').click();
  await expect(thread.locator('.code-block[data-lang="diff"]')).toBeVisible({ timeout: 20_000 });
  await composer.fill('ask');
  await thread.getByTestId('runtime-send').click();
  await expect(thread.getByTestId('runtime-attention')).toHaveAttribute('data-kind', 'approval', { timeout: 20_000 });
  return { id, thread, composer };
}

/** What a keyboard leaves: ~45 % of a phone held upright, 200 px on its side
 *  (a 430 px landscape iPhone with its ~200 px keyboard leaves about this in
 *  the browser's 380 px). */
function visibleWithKeyboard(w: number, h: number): number {
  return w > h ? h - 180 : Math.round(h * 0.55);
}

async function openKeyboard(page: Page, composer: Locator): Promise<{ width: number; height: number }> {
  const vp = page.viewportSize()!;
  await composer.click();
  await composer.fill('Looks fine — go ahead and also re-run the tests');
  await page.setViewportSize({ width: vp.width, height: visibleWithKeyboard(vp.width, vp.height) });
  await expect(page.locator('html[data-kb="open"]')).toHaveCount(1);
  // The layout settles (resize observers, the follow pin) before measuring.
  await page.waitForFunction(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(() => r(true)))));
  return vp;
}

async function timelineLines(thread: Locator): Promise<{ height: number; line: number }> {
  return thread.locator('.rt-scroll').evaluate((el) => {
    const p = el.querySelector('p') ?? el;
    const lh = parseFloat(getComputedStyle(p).lineHeight) || 20;
    return { height: el.getBoundingClientRect().height, line: lh };
  });
}

test('phone: with the keyboard up the composer is on screen and the conversation keeps 3+ lines', async ({ page }, info) => {
  test.skip(!PHONES.test(info.project.name), 'phones');
  test.setTimeout(120_000);
  const { thread, composer } = await richThread(page);
  const landscape = info.project.name === 'iphone-landscape';

  // Keyboard closed: one header row and no duplicated toggles.
  await expect(page.locator('.mtopbar')).toHaveCount(0);
  await expect(page.locator('[data-testid="threads-page"] > .page-header-bar')).toHaveCount(1);
  await expect(page.getByTestId('threads-toggle-inspector')).toHaveCount(1);
  await expect(page.getByTestId('threads-toggle-sidebar')).toHaveCount(1);
  await expect(page.locator('[aria-label="Open navigator"]')).toHaveCount(landscape ? 1 : 0);
  if (landscape) await expectFullyInViewport(page, page.getByTestId('threads-open-navigator'), 'navigator button');
  const closed = await timelineLines(thread);
  console.log(`[T-PHONE] ${info.project.name} keyboard closed: timeline ${Math.round(closed.height)} px`);
  await expectNoHorizontalOverflow(page);
  // The composer's foot is one row even at iPhone SE width (no pile of chips).
  const foot = (await thread.locator('.rt-foot').boundingBox())!;
  expect(foot.height, 'composer foot is one row').toBeLessThanOrEqual(60);
  await shot(page, 'kb-closed', ['light', 'dark']);

  // The code block wraps on a phone; the diff keeps its lines and its colours.
  const ts = thread.locator('.code-block[data-lang="ts"] pre');
  const tsOverflow = await ts.evaluate((el) => el.scrollWidth - el.clientWidth);
  expect(tsOverflow, 'a ts block wraps on a phone').toBeLessThanOrEqual(1);
  const add = thread.locator('.code-block[data-lang="diff"] .hljs-addition').first();
  if (await add.count()) {
    const bg = await add.evaluate((el) => getComputedStyle(el).backgroundColor);
    expect(bg, 'an added diff line is tinted').not.toBe('rgba(0, 0, 0, 0)');
  }

  const gapOf = (): Promise<number> => thread.locator('.rt-scroll').evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight);
  const gapBefore = await gapOf();
  const vp = await openKeyboard(page, composer);
  const send = thread.getByTestId('runtime-send');
  await expectFullyInViewport(page, send, 'Send with the keyboard up');
  expect(await onTop(send), 'Send is on top').toBe(true);
  await expectFullyInViewport(page, composer, 'the composer with the keyboard up');
  await expect(page.locator('nav.bottomnav')).toBeHidden();
  const open = await timelineLines(thread);
  console.log(`[T-PHONE] ${info.project.name} keyboard open (${page.viewportSize()!.height} px): timeline ${Math.round(open.height)} px, line ${open.line}`);
  expect(open.height, 'three lines of conversation above the composer').toBeGreaterThanOrEqual(3 * open.line);
  // The timeline's end stays where it was as the viewport shrinks (pinned
  // stays pinned): the keyboard never scrolls the conversation away.
  const gap = await gapOf();
  console.log(`[T-PHONE] ${info.project.name} distance from the end: ${gapBefore} px before, ${gap} px with the keyboard`);
  expect(gap, 'the keyboard does not scroll the conversation away from its end').toBeLessThanOrEqual(gapBefore + 48);
  // No page-level scroll jump when the composer took focus.
  expect(await page.evaluate(() => window.scrollY)).toBe(0);
  await expectNoHorizontalOverflow(page);
  await shot(page, 'kb-open', ['light', 'dark']);

  // Keyboard down: everything returns.
  await page.setViewportSize(vp);
  await composer.blur();
  await expect(page.locator('html[data-kb="open"]')).toHaveCount(0);
  await expect(page.locator('[data-testid="threads-page"] > .page-header-bar')).toBeVisible();
});

test('phone: the ⋯ menu is an opaque surface over the conversation', async ({ page }, info) => {
  test.skip(info.project.name !== 'iphone-portrait', 'iPhone portrait');
  test.setTimeout(120_000);
  await richThread(page);
  await page.getByTestId('threads-thread-menu').click();
  const menu = page.locator('.ctx-menu').first();
  await expectFullyInViewport(page, menu, 'the ⋯ menu');
  const bg = await menu.evaluate((el) => {
    const s = getComputedStyle(el);
    return { bg: s.backgroundColor, blur: s.backdropFilter || (s as unknown as { webkitBackdropFilter?: string }).webkitBackdropFilter || 'none' };
  });
  expect(bg.bg.startsWith('rgba') && !bg.bg.endsWith(', 1)'), `opaque menu (${bg.bg})`).toBe(false);
  expect(bg.blur).toBe('none');
  // A checkable row with an icon has no empty check column before its icon.
  const terminal = menu.getByRole('menuitemcheckbox', { name: /Terminal drawer/ });
  if (await terminal.count()) await expect(terminal.locator('.ctx-icon-gap')).toHaveCount(0);
  await shot(page, 'menu', ['light', 'dark']);
  await page.keyboard.press('Escape');

  // A diff block, close up.
  const dir = process.env.OTTO_E2E_SHOTS;
  if (dir) {
    for (const scheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: scheme });
      await page.locator('.code-block[data-lang="diff"]').first().screenshot({ path: join(dir, 'fix-T-PHONE', `diff-block-${scheme}.png`) });
    }
    await page.emulateMedia({ colorScheme: 'light' });
  }
});

test('iPad: the thread list is a column beside the conversation; Otto folds to its rail', async ({ page }, info) => {
  test.skip(!/^ipad-(portrait|landscape)$/.test(info.project.name), 'iPad');
  test.setTimeout(120_000);
  const { thread } = await richThread(page);
  const list = page.locator('.th-left-col');
  await expect(list).toBeVisible();
  const lb = (await list.boundingBox())!;
  const tb = (await thread.boundingBox())!;
  expect(lb.x + lb.width, 'list left of the conversation').toBeLessThanOrEqual(tb.x + 2);
  expect(tb.width, 'the conversation keeps a usable width').toBeGreaterThanOrEqual(480);
  // One header row; the shell's own bar steps aside on Threads.
  await expect(page.locator('.mtopbar')).toHaveCount(0);
  await expect(page.getByTestId('threads-toggle-inspector')).toHaveCount(1);
  await expectNoHorizontalOverflow(page);
  await shot(page, 'side-by-side', ['light', 'dark']);
  // Other pages keep the tablet Navigator column.
  if (info.project.name === 'ipad-portrait') {
    await page.goto('/#/home');
    await expect(page.locator('.msidebar:not(.mfolded)')).toBeVisible({ timeout: 15_000 });
  }
});
