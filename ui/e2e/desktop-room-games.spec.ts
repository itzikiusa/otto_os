import { test, expect, type Page, type TestInfo, type BrowserContext } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';
import type { KartState } from '../src/modules/rooms/games/kart/sim';
import type { ShooterState } from '../src/modules/rooms/games/shooter/sim';

// Real UI, real Kenney CC0 assets, real simulation, HTTP and WebSockets against
// the isolated test daemon. Keyboard/mouse input is driven through the page.
// OTTO_E2E_SLOT=games OTTO_E2E_PORT=7898 OTTO_E2E_PW_PORT=5298 \
// OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_BIN=../target/debug/ottod \
// npx playwright test --config e2e/room-games.config.ts
const evidence = resolve('..', 'docs/testing/room-games/screenshots');
interface Stats { frames: number; fps: number; frameMs: number; drawCalls: number; triangles: number }
interface Diagnostic<T> { snapshot(): T; stats(): Stats; look?(yaw: number, pitch: number): void }
let workspace = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  try { workspace = await seedWorkspace(ctx, base, 'Room Games E2E'); } finally { await ctx.dispose(); }
  mkdirSync(evidence, { recursive: true });
});
test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'room-games', 'Run with e2e/room-games.config.ts (isolated daemon + guest proxy)');
});

async function boot(page: Page, scheme: 'light' | 'dark' = 'dark') {
  await page.addInitScript(({ id, scheme }) => { localStorage.setItem('otto_workspace', id); localStorage.setItem('otto_scheme', scheme); }, { id: workspace, scheme });
  await page.goto('/#/rooms/games');
  await expect(page.getByRole('heading', { name: 'Choose a track' })).toBeVisible({ timeout: 30_000 });
}
function observe(page: Page) {
  const errors: string[] = [];
  page.on('pageerror', e => errors.push(e.message));
  page.on('console', m => { if (m.type() === 'error' && !/WebGL|GPU stall|AudioContext/.test(m.text())) errors.push(m.text()); });
  return errors;
}
// Resolves null until the renderer has published its diagnostic hook.
const snap = <T>(page: Page) => page.locator('.game-play').first().locator('div').first().evaluate(el => (el as HTMLDivElement & { __ottoGame?: Diagnostic<T> }).__ottoGame?.snapshot() ?? null) as Promise<T>;
const stats = (page: Page) => page.locator('.game-play div').first().evaluate(el => (el as HTMLDivElement & { __ottoGame: Diagnostic<unknown> }).__ottoGame.stats());
async function capture(page: Page, name: string, info: TestInfo) {
  const path = resolve(evidence, `${name}.png`);
  await page.screenshot({ path });
  await info.attach(name, { path, contentType: 'image/png' });
}
async function measure(page: Page, ms: number): Promise<{ fps: number; drawCalls: number; triangles: number; frameMs: number }> {
  const a = await stats(page), t0 = Date.now();
  await expect.poll(async () => (await stats(page)).frames - a.frames, { timeout: ms + 10_000 }).toBeGreaterThan(Math.floor(ms / 1000 * 25));
  await expect.poll(() => Date.now() - t0, { timeout: ms + 5000 }).toBeGreaterThan(ms);
  const b = await stats(page);
  return { fps: (b.frames - a.frames) * 1000 / (Date.now() - t0), drawCalls: b.drawCalls, triangles: b.triangles, frameMs: b.frameMs };
}

test('chooser shows both games, tracks, islands and rosters', async ({ page }, info) => {
  const errors = observe(page);
  await boot(page, 'light');
  await expect(page.locator('.games-titles img')).toHaveCount(2);
  await expect(page.locator('.games-environments > button')).toHaveCount(4);
  await expect(page.locator('.games-roster button')).toHaveCount(8);
  await expectNoHorizontalOverflow(page);
  await capture(page, 'chooser-kart-light', info);
  await page.getByRole('button', { name: /Storm Royale/ }).click();
  await expect(page.getByRole('heading', { name: 'Choose an island' })).toBeVisible();
  await expect(page.locator('.games-roster button')).toHaveCount(12);
  await capture(page, 'chooser-shooter-light', info);
  expect(errors).toEqual([]);
});

for (const track of [{ id: 'meadow', name: 'Sunny Meadow' }, { id: 'downtown', name: 'Downtown Dash' }, { id: 'canyon', name: 'Canyon Run' }, { id: 'frost', name: 'Frostbite Pass' }]) {
  test(`kart ${track.id}: 8-racer grid, countdown, driving, drifting and HUD`, async ({ page }, info) => {
    const errors = observe(page);
    await boot(page);
    await page.getByRole('button', { name: new RegExp(track.name) }).click();
    await page.getByRole('button', { name: 'Start race', exact: true }).click();
    await expect(page.locator('.kart-play canvas').first()).toBeVisible({ timeout: 30_000 });
    await expect.poll(async () => (await snap<KartState>(page))?.racers.length ?? 0, { timeout: 30_000 }).toBe(8);
    await capture(page, `kart-${track.id}-intro`, info);
    await page.locator('.kart-play canvas').first().click();
    await page.keyboard.down('w');
    await expect(page.locator('.kart-play')).toHaveAttribute('data-phase', 'race', { timeout: 15_000 });
    await expect.poll(async () => (await snap<KartState>(page)).raceTime, { timeout: 15_000 }).toBeGreaterThan(4);
    const me = (s: KartState) => s.racers.find(r => r.human)!;
    expect(me(await snap<KartState>(page)).travel).toBeGreaterThan(20);
    // Drift through the next bend: hold Space while steering.
    await page.keyboard.down('d'); await page.keyboard.down('Space');
    await expect.poll(async () => me(await snap<KartState>(page)).driftCharge, { timeout: 6000 }).toBeGreaterThan(0.3);
    await capture(page, `kart-${track.id}-drift`, info);
    await page.keyboard.up('Space'); await page.keyboard.up('d');
    const perf = await measure(page, 3000);
    await capture(page, `kart-${track.id}-race`, info);
    await page.keyboard.up('w');
    writeFileSync(resolve(evidence, `kart-${track.id}-perf.json`), JSON.stringify(perf, null, 2));
    await expect(page.locator('.kart-ranking li')).toHaveCount(8);
    await expect(page.locator('.kart-lap')).toContainText('Lap');
    expect(errors).toEqual([]);
  });
}

for (const map of [{ id: 'isle', name: 'Palm Isle' }, { id: 'mesa', name: 'Red Mesa' }]) {
  test(`shooter ${map.id}: drop, glide, land, loot and fight`, async ({ page }, info) => {
    test.setTimeout(150_000);
    const errors = observe(page);
    await boot(page);
    await page.getByRole('button', { name: /Storm Royale/ }).click();
    await page.getByRole('button', { name: new RegExp(map.name) }).click();
    await page.getByRole('button', { name: 'Drop in', exact: true }).click();
    await expect(page.locator('.br-play canvas').first()).toBeVisible({ timeout: 30_000 });
    await expect.poll(async () => (await snap<ShooterState>(page))?.players.length ?? 0, { timeout: 30_000 }).toBeGreaterThan(8);
    await capture(page, `shooter-${map.id}-balloon`, info);
    const canvas = page.locator('.br-play canvas').first();
    await canvas.focus();
    await expect.poll(async () => (await snap<ShooterState>(page)).bus.t, { timeout: 15_000 }).toBeGreaterThan(4);
    await page.keyboard.press('Space');
    const me = (s: ShooterState) => s.players.find(p => p.human)!;
    await expect.poll(async () => me(await snap<ShooterState>(page)).mode, { timeout: 10_000 }).not.toBe('bus');
    await page.locator('.br-play div').first().evaluate(el => (el as HTMLDivElement & { __ottoGame: Diagnostic<unknown> }).__ottoGame.look?.(0, -0.6));
    await capture(page, `shooter-${map.id}-skydive`, info);
    await expect.poll(async () => me(await snap<ShooterState>(page)).mode, { timeout: 45_000 }).toBe('ground');
    await page.locator('.br-play div').first().evaluate(el => (el as HTMLDivElement & { __ottoGame: Diagnostic<unknown> }).__ottoGame.look?.(0.6, -0.12));
    await capture(page, `shooter-${map.id}-landed`, info);
    // Walk forward and sprint around to find loot.
    await page.keyboard.down('w'); await page.keyboard.down('Shift');
    const start = me(await snap<ShooterState>(page));
    await expect.poll(async () => { const p = me(await snap<ShooterState>(page)); return Math.hypot(p.x - start.x, p.z - start.z); }, { timeout: 10_000 }).toBeGreaterThan(5);
    await page.keyboard.up('Shift'); await page.keyboard.up('w');
    const perf = await measure(page, 3000);
    writeFileSync(resolve(evidence, `shooter-${map.id}-perf.json`), JSON.stringify(perf, null, 2));
    await capture(page, `shooter-${map.id}-ground`, info);
    await expect(page.locator('.br-vitals')).toBeVisible();
    await expect(page.locator('.br-inventory .br-slot')).toHaveCount(3);
    expect(errors).toEqual([]);
  });
}

async function guestPage(context: BrowserContext, invite: string) {
  const guest = await context.newPage();
  await guest.goto(invite);
  return guest;
}

test('multiplayer kart: host + guest race with CPUs filling the grid', async ({ page, browser, baseURL }, info) => {
  test.setTimeout(120_000);
  const errors = observe(page);
  await boot(page);
  await page.getByRole('button', { name: 'Invite people', exact: true }).click();
  await page.getByRole('button', { name: 'Create game room', exact: true }).click();
  const invite = await page.getByLabel('Game invitation').inputValue();
  expect(invite).toContain('#/game-room/');
  const guestCtx = await browser.newContext({ baseURL, viewport: { width: 1280, height: 860 } });
  const guest = await guestPage(guestCtx, invite.replace(/^https?:\/\/[^/]+/, baseURL!));
  const guestErrors = observe(guest);
  try {
    await guest.getByLabel('Your display name').fill('Sam');
    await guest.getByRole('button', { name: 'Join game', exact: true }).click();
    await expect(page.getByText('Sam')).toBeVisible({ timeout: 15_000 });
    await guest.getByRole('button', { name: /Rosa/ }).click();
    await guest.getByRole('button', { name: 'Ready', exact: true }).click();
    await page.getByRole('button', { name: 'Ready', exact: true }).click();
    await page.getByRole('button', { name: 'Start match', exact: true }).click();
    await expect.poll(async () => (await snap<KartState>(guest))?.racers.length ?? 0, { timeout: 30_000 }).toBe(8);
    await guest.locator('.kart-play canvas').first().click();
    await guest.keyboard.down('w');
    await expect.poll(async () => (await snap<KartState>(page)).raceTime, { timeout: 20_000 }).toBeGreaterThan(3);
    await expect.poll(async () => (await snap<KartState>(page)).racers.find(r => r.human && r.slot === 1)?.travel ?? -99, { timeout: 10_000 }).toBeGreaterThan(5);
    expect((await snap<KartState>(page)).racers.find(r => r.human && r.slot === 1)?.racer).toBe('rosa');
    await capture(page, 'multiplayer-kart-host', info);
    await capture(guest, 'multiplayer-kart-guest', info);
    await guest.keyboard.up('w');
    expect(errors).toEqual([]);
    expect(guestErrors).toEqual([]);
  } finally { await guestCtx.close(); }
});

test('multiplayer shooter: host + guest drop onto the island with bots', async ({ page, browser, baseURL }, info) => {
  test.setTimeout(120_000);
  const errors = observe(page);
  await boot(page);
  await page.getByRole('button', { name: /Storm Royale/ }).click();
  await page.getByRole('button', { name: 'Invite people', exact: true }).click();
  await page.getByRole('button', { name: 'Create game room', exact: true }).click();
  const invite = await page.getByLabel('Game invitation').inputValue();
  const guestCtx = await browser.newContext({ baseURL, viewport: { width: 1280, height: 860 } });
  const guest = await guestPage(guestCtx, invite.replace(/^https?:\/\/[^/]+/, baseURL!));
  const guestErrors = observe(guest);
  try {
    await guest.getByLabel('Your display name').fill('Sam');
    await guest.getByRole('button', { name: 'Join game', exact: true }).click();
    await expect(page.getByText('Sam')).toBeVisible({ timeout: 15_000 });
    await guest.getByRole('button', { name: 'Ready', exact: true }).click();
    await page.getByRole('button', { name: 'Ready', exact: true }).click();
    await page.getByRole('button', { name: 'Start match', exact: true }).click();
    await expect.poll(async () => (await snap<ShooterState>(guest))?.players.length ?? 0, { timeout: 30_000 }).toBeGreaterThan(2);
    await expect.poll(async () => (await snap<ShooterState>(page)).bus.t, { timeout: 15_000 }).toBeGreaterThan(3);
    await guest.locator('.br-play canvas').first().focus();
    await guest.keyboard.press('Space');
    await expect.poll(async () => (await snap<ShooterState>(page)).players.find(p => p.human && p.slot === 1)?.mode, { timeout: 10_000 }).not.toBe('bus');
    await capture(page, 'multiplayer-shooter-host', info);
    await capture(guest, 'multiplayer-shooter-guest', info);
    expect(errors).toEqual([]);
    expect(guestErrors).toEqual([]);
  } finally { await guestCtx.close(); }
});
