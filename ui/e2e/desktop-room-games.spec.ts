import { test, expect, type Page, type TestInfo, type BrowserContext } from '@playwright/test';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';
import type { KartState } from '../src/modules/rooms/games/kart/sim';
import type { ShooterState } from '../src/modules/rooms/games/shooter/sim';
import { pointAt, trackFor } from '../src/modules/rooms/games/kart/tracks';
import { ordinal } from '../src/modules/rooms/games/common/math';

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
  if (await page.locator('.game-play').count()) await page.reload();
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
function wrap(a: number): number { return Math.atan2(Math.sin(a), Math.cos(a)); }
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
  await expect(page.locator('.games-environments > button')).toHaveCount(7);
  await expect(page.locator('.games-roster button')).toHaveCount(8);
  await expectNoHorizontalOverflow(page);
  await expect.poll(() => page.locator('.games-page img').evaluateAll(imgs => imgs.every(i => (i as HTMLImageElement).complete && (i as HTMLImageElement).naturalWidth > 0))).toBe(true);
  await capture(page, 'chooser-kart-light', info);
  await page.getByRole('button', { name: /Storm Royale/ }).click();
  await expect(page.getByRole('heading', { name: 'Choose an island' })).toBeVisible();
  await expect(page.locator('.games-roster button')).toHaveCount(12);
  await expect.poll(() => page.locator('.games-page img').evaluateAll(imgs => imgs.every(i => (i as HTMLImageElement).complete && (i as HTMLImageElement).naturalWidth > 0))).toBe(true);
  await capture(page, 'chooser-shooter-light', info);
  expect(errors).toEqual([]);
});

for (const track of [{ id: 'meadow', name: 'Sunny Meadow' }, { id: 'downtown', name: 'Downtown Dash' }, { id: 'canyon', name: 'Canyon Run' }, { id: 'frost', name: 'Frostbite Pass' }, { id: 'lagoon', name: 'Coral Lagoon' }, { id: 'volcano', name: 'Magma Peak' }, { id: 'skyway', name: 'Neon Skyway' }]) {
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
    const me = (s: KartState) => s.racers.find(r => r.human)!;
    // Follow the road until the race has run a few seconds and the kart is up to drifting speed
    // (some grids open on a bend, where a blind straight run ends in the barrier).
    const road = trackFor(track.id as Parameters<typeof trackFor>[0]);
    let steer: string | null = null;
    await expect.poll(async () => {
      const s = await snap<KartState>(page), r = me(s);
      const p = pointAt(road, r.lastS + 10 + Math.max(0, r.speed) * 0.45, 0);
      const angle = wrap(Math.atan2(p[0] - r.x, p[2] - r.z) - r.yaw);
      const want = angle > 0.05 ? 'a' : angle < -0.05 ? 'd' : null;
      if (want !== steer) { if (steer) await page.keyboard.up(steer); if (want) await page.keyboard.down(want); steer = want; }
      return s.raceTime > 4 && r.speed > 16;
    }, { timeout: 20_000, intervals: [50] }).toBe(true);
    if (steer) await page.keyboard.up(steer);
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
    // Calm lobby: this test checks the drop and the world, not survival.
    await page.getByRole('combobox', { name: /Computer difficulty/ }).selectOption('easy');
    await page.getByRole('slider', { name: /Bots on the island/ }).fill('9');
    await page.getByRole('button', { name: 'Drop in', exact: true }).click();
    await expect(page.locator('.br-play canvas').first()).toBeVisible({ timeout: 30_000 });
    await expect.poll(async () => (await snap<ShooterState>(page))?.players.length ?? 0, { timeout: 30_000 }).toBe(10);
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
    // Five hotbar slots plus the pickaxe.
    await expect(page.locator('.br-inventory .br-slot')).toHaveCount(6);
    expect(errors).toEqual([]);
  });
}

/** Hotbar items that are guns (the rest are heals and shields). */
const GUNS: string[] = ['pistol', 'smg', 'rifle', 'shotgun', 'sniper', 'rocket'];
const look = (page: Page, yaw: number, pitch: number) => page.locator('.game-play div').first().evaluate((el, [y, p]) => (el as HTMLDivElement & { __ottoGame: Diagnostic<unknown> }).__ottoGame.look?.(y, p), [yaw, pitch] as const);

test('kart full Grand Prix: autopilot races 3 laps, uses items and reaches the results', async ({ page }, info) => {
  test.setTimeout(300_000);
  const errors = observe(page);
  await boot(page);
  await page.getByRole('combobox', { name: /Computer difficulty/ }).selectOption('easy');
  await page.getByRole('button', { name: /Sunny Meadow/ }).click();
  await page.getByRole('button', { name: 'Start race', exact: true }).click();
  await expect.poll(async () => (await snap<KartState>(page))?.racers.length ?? 0, { timeout: 30_000 }).toBe(8);
  await page.locator('.kart-play canvas').first().click();
  await page.keyboard.down('w');
  const track = trackFor('meadow');
  let held: string | null = null, usedItem = false, sawItem = false, shot = false;
  const deadline = Date.now() + 260_000;
  for (;;) {
    const s = await snap<KartState>(page);
    if (s.phase === 'done' || Date.now() > deadline) break;
    const me = s.racers.find(r => r.human)!;
    const p = pointAt(track, me.lastS + 10 + Math.max(0, me.speed) * 0.45, 0);
    const angle = wrap(Math.atan2(p[0] - me.x, p[2] - me.z) - me.yaw);
    const want = angle > 0.05 ? 'a' : angle < -0.05 ? 'd' : null;
    if (want !== held) { if (held) await page.keyboard.up(held); if (want) await page.keyboard.down(want); held = want; }
    if (me.item && me.roll <= 0) {
      sawItem = true;
      if (!shot) { await capture(page, 'kart-meadow-item', info); shot = true; }
      await page.keyboard.press('e'); usedItem = true;
    }
  }
  if (held) await page.keyboard.up(held);
  await page.keyboard.up('w');
  const final = await snap<KartState>(page);
  expect(final.phase).toBe('done');
  const me = final.racers.find(r => r.human)!;
  expect(me.lapTimes).toHaveLength(3);
  expect(sawItem && usedItem).toBe(true);
  await expect(page.locator('.kart-results tbody tr')).toHaveCount(8);
  await expect(page.locator('.kart-results h2')).toContainText(me.place === 1 ? 'You win' : ordinal(me.place));
  await capture(page, 'kart-meadow-results', info);
  expect(errors).toEqual([]);
});

test('shooter: loot a weapon, aim at a bot and fire', async ({ page }, info) => {
  test.setTimeout(180_000);
  const errors = observe(page);
  await boot(page);
  await page.getByRole('button', { name: /Storm Royale/ }).click();
  await page.getByRole('button', { name: /Red Mesa/ }).click();
  // A quiet island so the scripted player can loot before anyone finds it.
  await page.getByRole('combobox', { name: /Computer difficulty/ }).selectOption('easy');
  await page.getByRole('slider', { name: /Bots on the island/ }).fill('2');
  await page.getByRole('button', { name: 'Drop in', exact: true }).click();
  await expect.poll(async () => (await snap<ShooterState>(page))?.players.length ?? 0, { timeout: 30_000 }).toBe(3);
  const canvas = page.locator('.br-play canvas').first();
  await canvas.focus();
  const me = (s: ShooterState) => s.players.find(p => p.human)!;
  await expect.poll(async () => (await snap<ShooterState>(page)).bus.t, { timeout: 15_000 }).toBeGreaterThan(2);
  await page.keyboard.press('Space');
  await expect.poll(async () => me(await snap<ShooterState>(page)).mode, { timeout: 60_000 }).toBe('ground');
  // Walk to the nearest floor weapon until it is in the inventory.
  let walking = false, targetId = -1, best = Infinity, since = Date.now();
  const skip = new Set<number>();
  const deadline = Date.now() + 90_000;
  for (;;) {
    const s = await snap<ShooterState>(page), p = me(s);
    if (p.weapons.some(w => w && GUNS.includes(w.w)) || Date.now() > deadline || p.mode === 'dead') break;
    const loot = s.loot.filter(l => l.kind === 'weapon' && !skip.has(l.id)).sort((a, b) => Math.hypot(a.x - p.x, a.z - p.z) - Math.hypot(b.x - p.x, b.z - p.z))[0];
    if (!loot) break;
    // Give up on an item that is not getting closer (blocked by a prop) and try the next.
    const dist = Math.hypot(loot.x - p.x, loot.z - p.z);
    if (loot.id !== targetId) { targetId = loot.id; best = dist; since = Date.now(); }
    else if (dist < best - 0.5) { best = dist; since = Date.now(); }
    else if (Date.now() - since > 4000) { skip.add(loot.id); await page.keyboard.press('Space'); continue; }
    await look(page, Math.atan2(loot.x - p.x, loot.z - p.z), -0.25);
    if (!walking) { await page.keyboard.down('w'); walking = true; }
    if (Math.hypot(loot.x - p.x, loot.z - p.z) < 2.2) await page.keyboard.press('e');
  }
  if (walking) await page.keyboard.up('w');
  const armed = me(await snap<ShooterState>(page));
  expect(armed.weapons.some(w => w && GUNS.includes(w.w))).toBe(true);
  await capture(page, 'shooter-mesa-armed', info);
  // Aim at the nearest living fighter and fire a burst.
  const s = await snap<ShooterState>(page), p = me(s);
  const target = s.players.filter(q => q.id !== p.id && q.mode !== 'dead').sort((a, b) => Math.hypot(a.x - p.x, a.z - p.z) - Math.hypot(b.x - p.x, b.z - p.z))[0];
  const d = Math.hypot(target.x - p.x, target.z - p.z);
  await look(page, Math.atan2(target.x - p.x, target.z - p.z), Math.atan2(target.y + 1 - (p.y + 1.8), d));
  const before = (await snap<ShooterState>(page)).events.length;
  const box = (await canvas.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await expect.poll(async () => { const now = me(await snap<ShooterState>(page)); const w = now.weapons[now.current]; return w ? w.mag : 99; }, { timeout: 8_000 }).toBeLessThan(armed.weapons[armed.current]?.mag ?? 99);
  await capture(page, 'shooter-mesa-firing', info);
  await page.mouse.up();
  void before;
  expect(errors).toEqual([]);
});

test('shooter: the storm closes in phase by phase, eliminations end the match and show results', async ({ page }, info) => {
  test.setTimeout(240_000);
  const errors = observe(page);
  // Test hook: the storm runs at 6% of its schedule (all seven phases in about 25 s instead of
  // seven minutes), so the final circle ends the match on its own whatever the fight does.
  await page.addInitScript(() => { (window as Window & { __ottoGameTest?: { stormScale: number } }).__ottoGameTest = { stormScale: 0.06 }; });
  await boot(page);
  await page.getByRole('button', { name: /Storm Royale/ }).click();
  await page.getByRole('button', { name: /Palm Isle/ }).click();
  await page.getByRole('combobox', { name: /Computer difficulty/ }).selectOption('hard');
  await page.getByRole('slider', { name: /Bots on the island/ }).fill('15');
  await page.getByRole('button', { name: 'Drop in', exact: true }).click();
  await expect.poll(async () => (await snap<ShooterState>(page))?.players.length ?? 0, { timeout: 30_000 }).toBe(16);
  await page.locator('.br-play canvas').first().focus();
  // Wait until the flight passes the island centre, then drop into the action.
  await expect.poll(async () => { const s = await snap<ShooterState>(page); return s.bus.t / s.bus.duration; }, { timeout: 30_000 }).toBeGreaterThan(0.45);
  await page.keyboard.press('Space');
  const me = (s: ShooterState) => s.players.find(p => p.human)!;
  await expect.poll(async () => me(await snap<ShooterState>(page)).mode, { timeout: 60_000 }).toBe('ground');
  // Head for the nearest survivor while the storm closes; record every storm phase seen.
  await page.keyboard.down('w');
  let stormShot = false, feedShot = false;
  const stages: number[] = [];
  const deadline = Date.now() + 120_000;
  for (;;) {
    const s = await snap<ShooterState>(page), p = me(s);
    if (stages.at(-1) !== s.storm.stage) stages.push(s.storm.stage);
    if (s.phase === 'done' || Date.now() > deadline) break;
    if (p.mode === 'ground') {
      const target = s.players.filter(q => q.id !== p.id && q.mode === 'ground').sort((a, b) => Math.hypot(a.x - p.x, a.z - p.z) - Math.hypot(b.x - p.x, b.z - p.z))[0];
      if (target) await look(page, Math.atan2(target.x - p.x, target.z - p.z), -0.18);
    }
    if (!stormShot && s.storm.stage >= 0) { stormShot = true; await capture(page, 'shooter-isle-storm', info); }
    if (!feedShot && await page.locator('.br-feed li').count()) { feedShot = true; await capture(page, 'shooter-isle-killfeed', info); }
  }
  await page.keyboard.up('w');
  // Phases only ever advance, and the storm formed before the match ended.
  expect(stages.every((v, i) => i === 0 || v > stages[i - 1]), `storm stages ${stages.join(' → ')}`).toBe(true);
  expect(Math.max(...stages)).toBeGreaterThanOrEqual(0);
  await expect(page.locator('.br-play')).toHaveAttribute('data-phase', 'done', { timeout: 15_000 });
  await expect(page.locator('.br-results tbody tr').first()).toBeVisible();
  await expect(page.locator('.br-results h2')).toContainText(/Victory Royale|You placed/);
  await capture(page, 'shooter-isle-results', info);
  expect(errors).toEqual([]);
});

test('shooter duos: a bot teammate, the team panel and team colours', async ({ page }, info) => {
  test.setTimeout(120_000);
  const errors = observe(page);
  await boot(page);
  await page.getByRole('button', { name: /Storm Royale/ }).click();
  await page.getByRole('button', { name: /Red Mesa/ }).click();
  await page.getByRole('button', { name: 'Duos', exact: true }).click();
  await page.getByRole('combobox', { name: /Computer difficulty/ }).selectOption('easy');
  await page.getByRole('slider', { name: /Bots on the island/ }).fill('7');
  await page.getByRole('button', { name: 'Drop in', exact: true }).click();
  await expect.poll(async () => (await snap<ShooterState>(page))?.players.length ?? 0, { timeout: 30_000 }).toBe(8);
  const s = await snap<ShooterState>(page);
  expect(s.squad).toBe(2);
  // Four teams of two; the human's teammate is the first bot.
  expect(s.players.map(p => p.team)).toEqual([0, 0, 1, 1, 2, 2, 3, 3]);
  await expect(page.locator('.br-team li')).toHaveCount(1, { timeout: 30_000 });
  await expect(page.locator('.br-stats')).toContainText('teams');
  await capture(page, 'shooter-duos-team-panel', info);
  expect(errors).toEqual([]);
});

test('phone: chooser fits and touch controls drive a kart', async ({ page: desktop, browser, baseURL }, info) => {
  const context = await browser.newContext({ baseURL, storageState: await desktop.context().storageState(), viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, deviceScaleFactor: 2 });
  const page = await context.newPage();
  try {
    await boot(page, 'light');
    await expectNoHorizontalOverflow(page);
    await capture(page, 'chooser-phone', info);
    await page.getByRole('button', { name: 'Start race', exact: true }).click();
    await expect.poll(async () => (await snap<KartState>(page))?.phase ?? '', { timeout: 30_000 }).toBe('race');
    await expect(page.getByRole('button', { name: 'Drift', exact: true })).toBeVisible();
    await expectNoHorizontalOverflow(page);
    const before = (await snap<KartState>(page)).racers.find(r => r.human)!.travel;
    // A tap on the steering pad (the kart accelerates by itself on touch); holding it would loop the kart round.
    const left = page.getByRole('button', { name: 'Steer left', exact: true });
    await left.dispatchEvent('pointerdown', { pointerId: 1 });
    await expect.poll(async () => (await snap<KartState>(page)).racers.find(r => r.human)!.steer, { timeout: 5000 }).not.toBe(0);
    await left.dispatchEvent('pointerup', { pointerId: 1 });
    await expect.poll(async () => (await snap<KartState>(page)).racers.find(r => r.human)!.travel, { timeout: 10_000 }).toBeGreaterThan(before + 10);
    await capture(page, 'kart-phone', info);
  } finally { await context.close(); }
});

// Menu art: OTTO_GAMES_ART=1 renders HUD-free frames of every track and island
// into docs/testing/room-games/art (converted to ui/public/room-games/*.webp).
test('menu art from live frames', async ({ page }) => {
  test.skip(process.env.OTTO_GAMES_ART !== '1', 'Art generator: set OTTO_GAMES_ART=1');
  test.setTimeout(300_000);
  const art = resolve('..', 'docs/testing/room-games/art');
  mkdirSync(art, { recursive: true });
  await page.setViewportSize({ width: 1600, height: 1000 });
  const hide = () => page.addStyleTag({ content: '.kart-top,.kart-ranking,.kart-place,.kart-minimap,.kart-intro,.kart-hint,.kart-notice,.kart-drift,.br-top,.br-mapbox,.br-vitals,.br-inventory,.br-prompt,.br-toast,.br-cross,.br-hint,.br-overlay{display:none!important}' });
  for (const [id, name] of [['meadow', 'Sunny Meadow'], ['downtown', 'Downtown Dash'], ['canyon', 'Canyon Run'], ['frost', 'Frostbite Pass'], ['lagoon', 'Coral Lagoon'], ['volcano', 'Magma Peak'], ['skyway', 'Neon Skyway']]) {
    await boot(page);
    await page.getByRole('button', { name: new RegExp(name) }).click();
    await page.getByRole('button', { name: 'Start race', exact: true }).click();
    await expect.poll(async () => (await snap<KartState>(page))?.clock ?? 0, { timeout: 30_000 }).toBeGreaterThan(0.5);
    await hide();
    await page.locator('.kart-stage').screenshot({ path: resolve(art, `map-${id}.png`) });
    await page.locator('.kart-play canvas').first().click();
    await page.keyboard.down('w');
    await expect.poll(async () => (await snap<KartState>(page)).raceTime, { timeout: 20_000 }).toBeGreaterThan(id === 'meadow' ? 2.2 : 1.5);
    await page.keyboard.up('w');
    if (id === 'meadow') await page.locator('.kart-stage').screenshot({ path: resolve(art, 'cover-kart.png') });
  }
  for (const [id, name] of [['isle', 'Palm Isle'], ['mesa', 'Red Mesa']]) {
    await boot(page);
    await page.getByRole('button', { name: /Storm Royale/ }).click();
    await page.getByRole('button', { name: new RegExp(name) }).click();
    await page.getByRole('button', { name: 'Drop in', exact: true }).click();
    await expect.poll(async () => (await snap<ShooterState>(page))?.bus.t ?? 0, { timeout: 30_000 }).toBeGreaterThan(5);
    await hide();
    await page.locator('.br-stage').screenshot({ path: resolve(art, `map-${id}.png`) });
    if (id === 'isle') {
      await page.locator('.br-play canvas').first().focus();
      await page.keyboard.press('Space');
      await expect.poll(async () => (await snap<ShooterState>(page)).players.find(p => p.human)!.mode, { timeout: 60_000 }).toBe('ground');
      await look(page, 2.2, -0.08);
      await expect.poll(async () => (await snap<ShooterState>(page)).clock, { timeout: 5000 }).toBeGreaterThan(0);
      await page.locator('.br-stage').screenshot({ path: resolve(art, 'cover-shooter.png') });
    }
  }
});

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
