import { test, expect, type APIRequestContext, type CDPSession, type Locator, type Page } from '@playwright/test';
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// The Threads composer on a phone, on REAL fake-agent threads (otto-fake-codex).
//   • The first letter typed into a composer that got its focus from the
//     thread opening (no tap on it yet) stays first: "i also saw" was sent as
//     "also sawi" — the caret went back to the empty composer's saved {0,0}
//     (lib/stores/runtimeDraftCursor.ts). Like threads-phone-layout.spec.ts,
//     the keyboard is the viewport shrinking while the composer has focus.
//   • A vertical drag that nothing under the finger can scroll — on the
//     composer, the conversation at its end, the header — stays put while
//     the keyboard is up (lib/holdTouchScroll.ts, for the whole document):
//     it went on to the page — under the keyboard that pans the visual viewport,
//     which lib/viewportFit.ts snaps back: the screen jumped. Playwright has
//     no on-screen keyboard and WebKit no touch drags, so on Chromium (CDP
//     touches) a pinch-zoom stands in for the keyboard: it too leaves the
//     visual viewport smaller than the page, and a drag that reaches the page
//     pans it. A long draft and the conversation still scroll.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

const MOBILE = /^(iphone-|ipad-|android-)/;

let ctx: APIRequestContext;
let base = '';
let wsId = '';
let fixtureDir = '';

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  fixtureDir = mkdtempSync(join(tmpdir(), 'otto-e2e-threads-composer-'));
  writeFileSync(join(fixtureDir, 'README.md'), '# Threads composer on a phone\n');
  const r = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Composer Mobile WS', root_path: fixtureDir } });
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
  await page.addInitScript((id) => localStorage.setItem('otto_workspace', id as string), wsId);
});

async function newThread(title: string): Promise<string> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runtime-sessions`, { data: { provider: 'codex', cwd: fixtureDir, title } });
  expect(r.ok(), await r.text()).toBeTruthy();
  return (await r.json()).id as string;
}

async function send(thread: Locator, text: string): Promise<void> {
  await thread.getByTestId('runtime-composer').fill(text);
  await thread.getByTestId('runtime-send').click();
  await expect(thread.getByText(`echo: ${text.slice(0, 24)}`, { exact: false }).last()).toBeVisible({ timeout: 20_000 });
}

/** What the on-screen keyboard leaves (threads-phone-layout.spec.ts). */
function withKeyboard(page: Page): { width: number; height: number } {
  const { width: w, height: h } = page.viewportSize()!;
  return { width: w, height: w > h ? h - 180 : Math.round(h * 0.55) };
}

test('the first letter typed into a thread just opened stays first', async ({ page }, info) => {
  test.skip(!MOBILE.test(info.project.name), 'phones and tablets');
  test.setTimeout(120_000);
  const a = await newThread('Composer caret A');
  const b = await newThread('Composer caret B');
  await page.goto(`/#/threads/${a}`);
  const thread = page.getByTestId(`runtime-thread-${a}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  // A message went out: the composer is empty again (its saved caret {0,0}).
  await send(thread, 'hello from the phone');
  await expect(thread.getByTestId('runtime-composer')).toHaveValue('');
  // Another thread, then back: the composer is new and the page focuses it.
  await page.goto(`/#/threads/${b}`);
  await expect(page.getByTestId(`runtime-thread-${b}`)).toBeVisible();
  await page.goto(`/#/threads/${a}`);
  const composer = thread.getByTestId('runtime-composer');
  await expect(composer).toBeFocused();
  const node = await composer.elementHandle();
  // The first letter, then the keyboard finishes opening, then the rest.
  await page.keyboard.type('i');
  await page.setViewportSize(withKeyboard(page));
  await expect(page.locator('html[data-kb="open"]')).toHaveCount(1);
  await page.keyboard.type(' also saw some issue on mobile');
  await expect(composer).toHaveValue('i also saw some issue on mobile');
  expect(await composer.evaluate((el: HTMLTextAreaElement) => [el.selectionStart, el.selectionEnd])).toEqual([31, 31]);
  expect(await composer.evaluate((el, n) => el === n, node), 'the same textarea throughout').toBe(true);
});

/** A one-finger vertical drag through CDP (real touch scrolling), at a point
 *  of the visual viewport (CSS px, zoom not applied). */
async function drag(cdp: CDPSession, page: Page, at: { x: number; y: number }, dy: number, dx = 0): Promise<void> {
  const frame = (): Promise<unknown> => page.evaluate(() => new Promise((r) => requestAnimationFrame(r)));
  let { x, y } = at;
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x, y }] });
  for (let i = 0; i < 10; i++) {
    x += dx / 10;
    y += dy / 10;
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x, y }] });
    await frame();
  }
  await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
  await frame();
}

/** Zoom in on `loc` until the page is twice the screen. */
async function zoomOn(cdp: CDPSession, page: Page, loc: Locator): Promise<void> {
  const c = (await loc.boundingBox())!;
  await cdp.send('Input.synthesizePinchGesture', { x: c.x + c.width / 2, y: c.y + c.height / 2, scaleFactor: 2, gestureSourceType: 'touch' });
  await expect.poll(() => page.evaluate(() => window.visualViewport!.scale)).toBeGreaterThan(1.5);
}

/** The centre of `loc` in the visual viewport. */
function centre(loc: Locator): Promise<{ x: number; y: number }> {
  return loc.evaluate((el) => {
    const r = el.getBoundingClientRect();
    const v = window.visualViewport!;
    return { x: r.left + r.width / 2 - v.offsetLeft, y: r.top + r.height / 2 - v.offsetTop };
  });
}

test('a drag that starts on the composer stays in it; a long draft and the conversation still scroll', async ({ page, browserName }, info) => {
  test.skip(!MOBILE.test(info.project.name) || browserName !== 'chromium', 'touch drags need CDP (Chromium phones and tablets)');
  test.setTimeout(150_000);
  const id = await newThread('Composer drag');
  await page.goto(`/#/threads/${id}`);
  const thread = page.getByTestId(`runtime-thread-${id}`);
  await expect(thread).toBeVisible({ timeout: 30_000 });
  const line = 'a long line of conversation so the timeline has more than a screen of text to scroll through';
  for (let i = 0; i < 5; i++) await send(thread, `turn ${i}: ${line} ${line} ${line}`);
  const wide = `const x = [${Array.from({ length: 30 }, (_, i) => `"item_${i}_wide_value"`).join(', ')}];`;
  await thread.getByTestId('runtime-composer').fill(`wide code\n\n\`\`\`js\n${wide}\n${wide}\n\`\`\``);
  await thread.getByTestId('runtime-send').click();
  await expect(thread.locator('.rt-scroll .code-block').last()).toBeVisible({ timeout: 20_000 });
  const composer = thread.getByTestId('runtime-composer');
  await composer.click();
  await page.setViewportSize(withKeyboard(page));
  await expect(page.locator('html[data-kb="open"]')).toHaveCount(1);
  const cdp = await page.context().newCDPSession(page);
  const vv = (): Promise<{ top: number; left: number }> =>
    page.evaluate(() => ({ top: Math.round(window.visualViewport!.offsetTop), left: Math.round(window.visualViewport!.offsetLeft) }));

  // Zoomed in on the composer: the page is bigger than the screen, as under the keyboard.
  await zoomOn(cdp, page, composer);
  const start = await vv();
  for (const dy of [90, -90]) {
    await drag(cdp, page, await centre(composer), dy);
    await page.waitForTimeout(300); // ui-guards: allow — absence: the page did not pan after the drag
    expect(await vv(), `a drag of ${dy} px on the composer leaves the page where it was`).toEqual(start);
  }
  await cdp.send('Emulation.setPageScaleFactor', { pageScaleFactor: 1 });
  await expect.poll(() => page.evaluate(() => Math.round(window.visualViewport!.scale * 100))).toBe(100);

  // Anything else nothing can scroll that way is held too while the keyboard is up: the conversation
  // at its end and the header. Seen on Android Chrome with the real keyboard: a drag up there panned
  // the page by up to 312 px. Here the first move of the drag must come back cancelled.
  const scroller = thread.locator('.rt-scroll');
  await page.evaluate(() => {
    const w = window as unknown as { held: boolean | null };
    w.held = null;
    addEventListener('touchmove', (e) => void (w.held ??= e.defaultPrevented), { passive: true });
  });
  const held = async (at: { x: number; y: number }, dy: number, dx = 0): Promise<boolean | null> => {
    await page.evaluate(() => void ((window as unknown as { held: boolean | null }).held = null));
    await drag(cdp, page, at, dy, dx);
    return page.evaluate(() => (window as unknown as { held: boolean | null }).held);
  };
  await scroller.evaluate((el) => (el.scrollTop = el.scrollHeight));
  const nearEnd = await scroller.evaluate((el) => {
    const r = el.getBoundingClientRect();
    return { x: r.left + r.width / 2, y: r.bottom - 30 };
  });
  expect(await held(nearEnd, -60), 'a drag up on the conversation at its end is held').toBe(true);
  expect(await held(await centre(page.getByTestId('page-header')), -60), 'a drag up on the header is held').toBe(true);
  expect(await held(nearEnd, 60), 'a drag down on the conversation scrolls it').toBe(false);
  // A mostly sideways drag, slightly up, on the last code block (wrapping off) at the conversation's
  // end still scrolls the code sideways: the hold must not take it for a vertical drag.
  const code = thread.locator('.rt-scroll .code-block').last();
  const pre = code.locator('pre');
  // (clicked from script: a real tap would take the focus, and the keyboard, off the composer)
  if (await pre.evaluate((el) => el.scrollWidth <= el.clientWidth + 1)) await code.locator('[data-code-act=wrap]').evaluate((b: HTMLElement) => b.click());
  await expect.poll(() => pre.evaluate((el) => el.scrollWidth > el.clientWidth + 1), { message: 'the code scrolls sideways' }).toBe(true);
  await scroller.evaluate((el) => (el.scrollTop = el.scrollHeight));
  await pre.evaluate((el) => (el.scrollLeft = 0));
  await expect(page.locator('html[data-kb="open"]')).toHaveCount(1);
  // (a phone on its side shows a strip of conversation: bring the block into it)
  await pre.evaluate((el) => {
    const s = el.closest('.rt-scroll')!.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    if (r.top < s.top || r.bottom > s.bottom) el.scrollIntoView({ block: 'center' });
  });
  await page.evaluate(() => new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r))));
  const onCode = await pre.evaluate((el) => {
    const r = el.getBoundingClientRect();
    const x = r.left + r.width / 2;
    let y = r.bottom - 4;
    while (y > r.top && !el.contains(document.elementFromPoint(x, y))) y -= 4; // the first point not under the dock
    return { x, y };
  });
  expect(await pre.evaluate((el, p) => el.contains(document.elementFromPoint(p.x, p.y)), onCode), 'a point on the code block shows').toBe(true);
  expect(await held(onCode, -12, -160), 'a sideways drag on a code block is not held').toBe(false);
  await expect.poll(() => pre.evaluate((el) => el.scrollLeft), { message: 'the code block scrolled sideways' }).toBeGreaterThan(40);

  // A draft taller than its box scrolls inside the composer.
  await composer.fill(Array.from({ length: 30 }, (_, i) => `draft line ${i + 1}`).join('\n'));
  await composer.evaluate((el) => (el.scrollTop = el.scrollHeight));
  const draftTop = await composer.evaluate((el) => el.scrollTop);
  expect(draftTop, 'the draft overflows its box').toBeGreaterThan(40);
  await drag(cdp, page, await centre(composer), 60);
  await expect.poll(() => composer.evaluate((el) => el.scrollTop), { message: 'dragging down scrolls the draft up' }).toBeLessThan(draftTop);

  // The conversation scrolls as before.
  await scroller.evaluate((el) => (el.scrollTop = el.scrollHeight));
  const convTop = await scroller.evaluate((el) => el.scrollTop);
  expect(convTop, 'the conversation overflows').toBeGreaterThan(40);
  await drag(cdp, page, await centre(scroller), 120);
  await expect.poll(() => scroller.evaluate((el) => el.scrollTop), { message: 'dragging the conversation scrolls it' }).toBeLessThan(convTop);
});

test('a drag that starts on the New-thread composer stays in it', async ({ page, browserName }, info) => {
  test.skip(!MOBILE.test(info.project.name) || browserName !== 'chromium', 'touch drags need CDP (Chromium phones and tablets)');
  test.setTimeout(60_000);
  await page.goto('/#/threads/new');
  const input = page.getByTestId('new-thread-input');
  await expect(input).toBeVisible({ timeout: 30_000 });
  await input.click();
  await page.setViewportSize(withKeyboard(page));
  await expect(page.locator('html[data-kb="open"]')).toHaveCount(1);
  const cdp = await page.context().newCDPSession(page);
  await zoomOn(cdp, page, input);
  const top = (): Promise<number> => page.evaluate(() => Math.round(window.visualViewport!.offsetTop));
  const start = await top();
  for (const dy of [90, -90]) {
    await drag(cdp, page, await centre(input), dy);
    await page.waitForTimeout(300); // ui-guards: allow — absence: the page did not pan after the drag
    expect(await top(), `a drag of ${dy} px on the New-thread composer leaves the page where it was`).toBe(start);
  }
});
