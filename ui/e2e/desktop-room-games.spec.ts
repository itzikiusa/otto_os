import { test, expect, type Page, type TestInfo } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';
import type { GameState, GameKind } from '../src/modules/rooms/games/types';

// Real UI, assets, simulation, HTTP and WebSockets. Only the failure test aborts
// one asset download; no API replies or game state are fabricated.
// OTTO_E2E_SLOT=games OTTO_E2E_PORT=7898 OTTO_E2E_PW_PORT=5298 \
// OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=../target/debug/ottod \
// npx playwright test --config e2e/room-games.config.ts
const evidenceDir = resolve('..', 'docs/testing/room-games/screenshots');
const maps = [
  { kind: 'shooter', id: 'station', name: 'Orbital Station' },
  { kind: 'shooter', id: 'foundry', name: 'Ember Foundry' },
  { kind: 'shooter', id: 'dunes', name: 'Sunken Dunes' },
  { kind: 'kart', id: 'coast', name: 'Coral Coast' },
  { kind: 'kart', id: 'forest', name: 'Fernwood Rally' },
  { kind: 'kart', id: 'neon', name: 'Neon Overdrive' },
] as const;
interface RenderStats { frames: number; drawCalls: number; triangles: number; frameMs: number }
interface GameDiagnostic { snapshot(): GameState; stats(): RenderStats }
let workspace = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  try { workspace = await seedWorkspace(ctx, base, 'Room Games E2E'); }
  finally { await ctx.dispose(); }
  mkdirSync(evidenceDir, { recursive: true });
});

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'room-games', 'Run with e2e/room-games.config.ts for isolated guest HTTP/WS proxy');
});

async function boot(page: Page, scheme: 'light' | 'dark' = 'dark') {
  await page.addInitScript(({ id, scheme }) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_scheme', scheme);
  }, { id: workspace, scheme });
  await page.goto('/#/rooms/games');
  await expect(page.getByRole('button', { name: 'Play computer', exact: true })).toBeVisible();
}

function observe(page: Page) {
  const errors: string[] = [], assets: { path: string; status: number }[] = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
  page.on('response', response => {
    const path = new URL(response.url()).pathname;
    if (path.startsWith('/room-games/')) assets.push({ path, status: response.status() });
  });
  return { errors, assets };
}

async function snapshot(page: Page) {
  return page.locator('.game-stage').evaluate(el =>
    (el as HTMLDivElement & { __ottoGame: GameDiagnostic }).__ottoGame.snapshot());
}
async function stats(page: Page) {
  return page.locator('.game-stage').evaluate(el =>
    (el as HTMLDivElement & { __ottoGame: GameDiagnostic }).__ottoGame.stats());
}
async function playing(page: Page) {
  await expect(page.locator('.game-stage canvas')).toBeVisible();
  await expect(page.locator('.game-play')).toHaveAttribute('data-phase', 'playing', { timeout: 30_000 });
  await expect.poll(async () => (await stats(page)).frames).toBeGreaterThan(5);
  await expect(page.locator('.game-play [role=alert]')).toHaveCount(0);
}
async function startComputer(page: Page, kind: GameKind, mapName: string) {
  await page.getByRole('button', { name: kind === 'shooter' ? /Arena Duel Find/ : /Circuit Clash Hold/ }).click();
  await page.getByRole('combobox', { name: /Difficulty/ }).selectOption('easy');
  await page.getByRole('button', { name: new RegExp(mapName) }).click();
  await page.getByRole('button', { name: 'Play computer', exact: true }).click();
  await playing(page);
}
function distance(a: { x: number; z: number }, b: { x: number; z: number }) {
  return Math.hypot(a.x - b.x, a.z - b.z);
}
async function move(page: Page, player = 0) {
  const before = (await snapshot(page)).players[player];
  await page.locator('.game-stage canvas').focus();
  await page.keyboard.down('w');
  try {
    await expect.poll(async () => distance((await snapshot(page)).players[player], before), { timeout: 8_000 }).toBeGreaterThan(1);
  } finally { await page.keyboard.up('w'); }
}
async function capture(page: Page, file: string, info: TestInfo) {
  const path = resolve(evidenceDir, `${file}.png`);
  await page.screenshot({ path, fullPage: true });
  await info.attach(file, { path, contentType: 'image/png' });
}

for (const scheme of ['light', 'dark'] as const) {
  test(`chooser previews and controls in ${scheme}`, async ({ page }, info) => {
    const observed = observe(page);
    await boot(page, scheme);
    const previews = page.locator('.games-titles img');
    await expect(previews).toHaveCount(2);
    await expect.poll(() => previews.evaluateAll(images => images.every(image => (image as HTMLImageElement).naturalWidth > 100))).toBe(true);
    await expect(page.getByRole('button', { name: 'Computer', exact: true })).toHaveAttribute('aria-pressed', 'true');
    await expect(page.getByRole('button', { name: /Orbital Station/ })).toHaveAttribute('aria-pressed', 'true');
    await expectNoHorizontalOverflow(page);
    await capture(page, `chooser-${scheme}`, info);
    expect(observed.errors).toEqual([]);
  });
}

test('phone and tablet chooser fit; touch controls drive a real kart', async ({ page, browser, baseURL }, info) => {
  const context = await browser.newContext({
    baseURL, storageState: await page.context().storageState(),
    viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true,
    deviceScaleFactor: 1, serviceWorkers: 'block',
  });
  const mobile = await context.newPage(), observed = observe(mobile);
  try {
    await boot(mobile, 'light');
    for (const viewport of [{ width: 390, height: 844 }, { width: 768, height: 1024 }]) {
      await mobile.setViewportSize(viewport);
      await expect(mobile.getByRole('button', { name: /Circuit Clash Hold/ })).toBeVisible();
      await expectNoHorizontalOverflow(mobile);
      await capture(mobile, `chooser-touch-${viewport.width}`, info);
    }
    await mobile.setViewportSize({ width: 390, height: 844 });
    await startComputer(mobile, 'kart', 'Coral Coast');
    const forward = mobile.getByRole('button', { name: 'Move up', exact: true });
    await expect(forward).toBeVisible();
    await expect(mobile.getByRole('button', { name: 'Drift', exact: true })).toBeVisible();
    await expectNoHorizontalOverflow(mobile);
    const bounds = (await forward.boundingBox())!;
    const start = (await snapshot(mobile)).players[0];
    const cdp = await context.newCDPSession(mobile);
    await cdp.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: bounds.x + bounds.width / 2, y: bounds.y + bounds.height / 2, id: 1 }] });
    try {
      await expect.poll(async () => distance((await snapshot(mobile)).players[0], start)).toBeGreaterThan(1);
    } finally { await cdp.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] }); }
    await cdp.detach();
    await capture(mobile, 'kart-coast-touch-phone', info);
    await mobile.getByRole('button', { name: 'Menu', exact: true }).tap();
    await expect(mobile.getByRole('button', { name: 'Resume', exact: true })).toBeVisible();
    await mobile.getByRole('button', { name: 'Leave match', exact: true }).tap();
    await expect(mobile.getByRole('button', { name: 'Play computer', exact: true })).toBeVisible();
    expect(observed.errors).toEqual([]);
  } finally { await context.close(); }
});

for (const map of maps) {
  test(`${map.kind} ${map.id}: assets, movement, focus loss, menu and rendering`, async ({ page }, info) => {
    const observed = observe(page);
    await boot(page, map.id === 'station' || map.id === 'coast' ? 'light' : 'dark');
    await startComputer(page, map.kind, map.name);
    await expect(page.locator('.game-play')).toHaveAttribute('data-map', map.id);
    await expect(page.locator('.game-score')).toContainText(map.kind === 'shooter' ? 'First to 7' : 'Lap / 3');
    await expect(page.locator('.game-bottom-hud')).toContainText(map.kind === 'shooter' ? 'Health' : 'Speed');
    for (const file of ['environment-kit.glb', `${map.kind === 'shooter' ? 'fighter' : 'kart'}-azure.glb`, `${map.kind === 'shooter' ? 'fighter' : 'kart'}-ember.glb`, ...(map.kind === 'shooter' ? ['rifle.glb'] : [])]) {
      expect(observed.assets.some(asset => asset.path.endsWith(file) && asset.status === 200), `${file} loaded successfully`).toBe(true);
    }
    await move(page);

    if (map.id === 'station') {
      // Chromium headless rejects native pointer lock (standalone reproduction:
      // WrongDocumentError). Exercise the shipped click/drag fallback instead;
      // no pointer-lock API is mocked, and Escape must release fire before up.
      const canvas = page.locator('.game-stage canvas');
      await canvas.click({ position: { x: 350, y: 250 } });
      const bounds = (await canvas.boundingBox())!;
      const yaw = (await snapshot(page)).players[0].yaw;
      await page.mouse.down();
      try {
        await page.mouse.move(bounds.x + 410, bounds.y + 270, { steps: 4 });
        await expect.poll(async () => Math.abs((await snapshot(page)).players[0].yaw - yaw)).toBeGreaterThan(.01);
        await expect.poll(async () => (await snapshot(page)).players[0].ammo).toBeLessThan(24);
        await page.keyboard.press('Escape');
        await expect.poll(() => page.evaluate(() => document.pointerLockElement === null)).toBe(true);
        const ammo = (await snapshot(page)).players[0].ammo;
        await page.waitForTimeout(350); // ui-guards: allow — prove Escape clears held fire before pointerup arrives.
        expect((await snapshot(page)).players[0].ammo).toBe(ammo);
      } finally { await page.mouse.up(); }
    }

    // Lose canvas focus while W remains physically held. The simulation must
    // release the input before keyup, not merely rely on eventual keyup delivery.
    await page.locator('.game-stage canvas').focus();
    await page.keyboard.down('w');
    await expect.poll(async () => (await snapshot(page)).players[0].moving).toBe(true);
    await page.getByRole('button', { name: 'Mute game' }).focus();
    const released = (await snapshot(page)).players[0];
    await page.waitForTimeout(450); // ui-guards: allow — prove held movement is absent after focus loss, before keyup.
    const afterBlur = (await snapshot(page)).players[0];
    if (map.kind === 'shooter') expect(distance(afterBlur, released)).toBeLessThan(.2);
    else expect(Math.abs(afterBlur.speed)).toBeLessThan(Math.abs(released.speed));
    await page.keyboard.up('w');
    // Keyboard input outside the game cannot restart movement.
    await page.keyboard.down('w');
    await page.waitForTimeout(250); // ui-guards: allow — prove keys outside the canvas cannot restart movement.
    const outside = (await snapshot(page)).players[0];
    if (map.kind === 'shooter') expect(distance(outside, afterBlur)).toBeLessThan(.2);
    else expect(Math.abs(outside.speed)).toBeLessThanOrEqual(Math.abs(afterBlur.speed));
    await page.keyboard.up('w');

    await page.getByRole('button', { name: 'Mute game' }).click();
    await expect(page.getByRole('button', { name: 'Unmute game' })).toBeVisible();
    if (map.kind === 'shooter') {
      await page.getByRole('button', { name: 'Third person', exact: true }).click();
      await expect(page.locator('.game-play')).toHaveAttribute('data-camera', 'first');
      await capture(page, `${map.id}-first-person`, info);
      await page.getByRole('button', { name: 'First person', exact: true }).click();
      await expect(page.locator('.game-play')).toHaveAttribute('data-camera', 'third');
    }
    await page.getByRole('button', { name: 'Menu', exact: true }).click();
    await expect(page.getByText('Game paused', { exact: true })).toBeVisible();
    const paused = await snapshot(page);
    await page.waitForTimeout(350); // ui-guards: allow — prove simulation ticks remain frozen throughout the pause.
    expect((await snapshot(page)).tick).toBe(paused.tick);
    await page.getByRole('button', { name: 'Resume', exact: true }).click();
    await expect.poll(async () => (await snapshot(page)).tick).toBeGreaterThan(paused.tick);
    await move(page);
    const before = await stats(page), started = Date.now();
    await page.waitForTimeout(1200); // ui-guards: allow — fixed measurement window for observed rendering FPS.
    const after = await stats(page);
    const fps = (after.frames - before.frames) * 1000 / (Date.now() - started);
    expect(after.drawCalls).toBeGreaterThan(0);
    expect(after.triangles).toBeGreaterThan(0);
    expect(fps).toBeGreaterThan(0);
    await capture(page, `${map.kind}-${map.id}`, info);
    const renderer = await page.locator('.game-stage canvas').evaluate(el => {
      const gl = (el as HTMLCanvasElement).getContext('webgl2');
      const extension = gl?.getExtension('WEBGL_debug_renderer_info');
      return extension ? String(gl!.getParameter(extension.UNMASKED_RENDERER_WEBGL)) : 'unavailable';
    });
    const report = { map, renderer, fps, ...after, ...observed };
    writeFileSync(resolve(evidenceDir, `${map.id}-render-report.json`), JSON.stringify(report, null, 2));
    await info.attach('render-and-console-report', { body: JSON.stringify(report), contentType: 'application/json' });
    await page.getByRole('button', { name: 'Menu', exact: true }).click();
    await page.getByRole('button', { name: 'Leave match', exact: true }).click();
    await expect(page.getByRole('button', { name: 'Play computer', exact: true })).toBeVisible();
    await expect(page.locator('.game-stage canvas')).toHaveCount(0);
    expect(observed.errors).toEqual([]);
  });
}

test('failed model download offers Retry and successfully loads the real asset', async ({ page }) => {
  await boot(page);
  await page.route('**/room-games/fighter-azure.glb', route => route.abort('failed'));
  await page.getByRole('button', { name: 'Play computer', exact: true }).click();
  const error = page.locator('.game-play').getByTestId('load-error');
  await expect(error).toContainText('Game models could not load');
  await expect(page.locator('.game-stage canvas')).toHaveCount(0);
  await page.unroute('**/room-games/fighter-azure.glb');
  await error.getByRole('button', { name: 'Retry', exact: true }).click();
  await playing(page);
  await expect(error).toHaveCount(0);
  await move(page);
});

for (const kind of ['shooter', 'kart'] as const) {
  test(`${kind}: real invitation, unauthenticated guest, ready, input relay and ${kind === 'shooter' ? 'leave' : 'disconnect'}`, async ({ page, browser, baseURL }, info) => {
    const hostObserved = observe(page);
    const guestContext = await browser.newContext({ baseURL, storageState: { cookies: [], origins: [] }, viewport: { width: 1440, height: 1000 } });
    const guest = await guestContext.newPage(), guestObserved = observe(guest);
    const guestRequests: { path: string; authorization: string | undefined }[] = [];
    guest.on('request', request => {
      const path = new URL(request.url()).pathname;
      if (path.startsWith('/api/')) guestRequests.push({ path, authorization: request.headers().authorization });
    });
    try {
      await boot(page);
      await page.getByRole('button', { name: kind === 'shooter' ? /Arena Duel Find/ : /Circuit Clash Hold/ }).click();
      await page.getByRole('button', { name: 'Another person', exact: true }).click();
      await page.getByLabel('Your display name').fill('Games host');
      await page.getByRole('button', { name: 'Create game room', exact: true }).click();
      await expect(page.getByRole('button', { name: 'Start match', exact: true })).toBeDisabled();
      const invitation = await page.getByLabel('Game invitation', { exact: true }).inputValue();
      // Change only the origin to our real test proxy; keep the server-issued fragment.
      await guest.goto(`/${new URL(invitation).hash}`);
      await expect(guest.getByRole('heading', { name: 'Join a game', exact: true })).toBeVisible();
      await expect(guest).not.toHaveURL(/invite=/);
      await guest.getByLabel('Your display name').fill('Games guest');
      await guest.getByRole('button', { name: 'Join game', exact: true }).click();
      await expect(page.locator('.game-members')).toContainText('Games guest');
      await page.getByRole('button', { name: 'Ready', exact: true }).click();
      await expect(page.getByRole('button', { name: 'Start match', exact: true })).toBeDisabled();
      await guest.getByRole('button', { name: 'Ready', exact: true }).click();
      await expect(page.getByRole('button', { name: 'Start match', exact: true })).toBeEnabled();
      await page.getByRole('button', { name: 'Start match', exact: true }).click();
      await Promise.all([playing(page), playing(guest)]);
      await expect(guest.locator('.game-score')).toContainText('Games host');
      await expect(guest.locator('.game-score')).toContainText('Games guest');
      const guestBefore = (await snapshot(page)).players[1];
      await move(guest, 1);
      await expect.poll(async () => distance((await snapshot(page)).players[1], guestBefore)).toBeGreaterThan(1);
      const hostBefore = (await snapshot(guest)).players[0];
      await move(page, 0);
      await expect.poll(async () => distance((await snapshot(guest)).players[0], hostBefore)).toBeGreaterThan(1);
      // Guest menu releases controls but host-owned simulation keeps advancing.
      await guest.getByRole('button', { name: 'Menu', exact: true }).click();
      await expect(guest.getByText('Your player is idle; the match continues for your opponent.')).toBeVisible();
      const active = await snapshot(page);
      await expect.poll(async () => (await snapshot(page)).tick).toBeGreaterThan(active.tick + 3);
      await capture(page, `multiplayer-${kind}-host`, info);
      await guest.getByRole('button', { name: 'Resume', exact: true }).click();
      await capture(guest, `multiplayer-${kind}-guest`, info);
      expect(await guest.evaluate(() => localStorage.getItem('otto_token'))).toBeNull();
      expect(guestRequests).toEqual([{ path: '/api/v1/game-room-join', authorization: undefined }]);
      if (kind === 'shooter') {
        await guest.getByRole('button', { name: 'Menu', exact: true }).click();
        await guest.getByRole('button', { name: 'Leave match', exact: true }).click();
        await expect(guest.getByRole('heading', { name: 'You left the match' })).toBeVisible();
        await expect(page.getByRole('heading', { name: 'Match ended' })).toBeVisible();
      } else {
        await guestContext.close();
        await expect(page.getByRole('heading', { name: 'Waiting for connection' })).toBeVisible();
        const paused = await snapshot(page);
        await page.waitForTimeout(400); // ui-guards: allow — prove disconnected matches remain paused across frames.
        expect((await snapshot(page)).tick).toBe(paused.tick);
        await page.getByRole('button', { name: 'Leave match', exact: true }).click();
        await expect(page.getByRole('button', { name: 'Create game room', exact: true })).toBeVisible();
      }
      expect(hostObserved.errors).toEqual([]);
      expect(guestObserved.errors).toEqual([]);
    } finally {
      await info.attach('multiplayer-console-and-requests', { body: JSON.stringify({ hostObserved, guestObserved, guestRequests }), contentType: 'application/json' });
      await guestContext.close();
    }
  });
}
