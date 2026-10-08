/// <reference types="vite/client" />
import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { randomUUID } from 'node:crypto';
import { expectFullyInViewport } from './helpers';

// Home → Otto School (the Classrooms box), the REAL 3D path: Chromium gets a
// software WebGL (SwiftShader) and motion stays ON, the Blender-built assets
// load from ui/public/school/, and every flow is driven through the canvas
// the way a person does it — click the door, click a kid, use the card. The
// stage exposes its scene handle (`__ottoSchool`) so the spec can READ what
// the scene shows (view, characters + clips, what each monitor displays) and
// project a kid / door to the pixel to click.

test.use({
  launchOptions: { args: ['--use-angle=swiftshader', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist'] },
  serviceWorkers: 'block',
  reducedMotion: 'no-preference',
  viewport: { width: 1440, height: 900 },
});
// Software WebGL on a busy CI runner: the first mount (three + ~10 MB of
// models + the HDRI) alone can take most of Playwright's default 45 s.
test.describe.configure({ timeout: 150_000 });

const WS_NAME = `School E2E ${randomUUID()}`;
const MARKER = `OTTO_SCHOOL_${Date.now().toString(36).toUpperCase()}`;
let wsId = '';
let base = '';
let ctx: Awaited<ReturnType<typeof apiCtx>>['ctx'];
const ids: Record<string, string> = {};

async function shell(title: string): Promise<string> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'manual' } },
  });
  expect(r.ok()).toBeTruthy();
  return ((await r.json()) as { id: string }).id;
}

async function boot(page: Page, config: Record<string, unknown> = { view: '3d' }): Promise<void> {
  await page.addInitScript(({ id, config }) => {
    localStorage.setItem('otto_workspace', id);
    if (sessionStorage.getItem('otto_e2e_school_reset')) return;
    sessionStorage.setItem('otto_e2e_school_reset', '1');
    localStorage.removeItem('otto_home_active');
    localStorage.removeItem('otto_bar_spaces');
    localStorage.removeItem('otto_home_rotate');
    localStorage.setItem(
      'otto_home_views',
      JSON.stringify([{ id: 'v1', name: 'School', boxes: [{ id: 'cr1', kind: 'classrooms', w: 12, h: 8, config }] }]),
    );
  }, { id: wsId, config });
  await page.goto('/#/home');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  if (config.view === 'list') return;
  await expect(box(page).locator('.stage canvas')).toBeVisible({ timeout: 60_000 });
  await expect.poll(() => debug(page).then((d) => d?.frames ?? 0), { timeout: 60_000 }).toBeGreaterThan(0);
}

const box = (page: Page) => page.locator('section.hbox[data-kind="classrooms"]');
const card = (page: Page) => page.getByRole('group', { name: /— details$/ });

type Debug = {
  view: { kind: string; roomId?: string; kidId?: string };
  frames: number;
  missing: string[];
  kids: { id: string; clip: string; mode: string; visible: boolean }[];
  head: { clip: string; mode: string; target: string | null } | null;
  screens: Record<string, string>;
};

function debug(page: Page): Promise<Debug | null> {
  return page.evaluate(() => {
    const st = document.querySelector('section.hbox[data-kind="classrooms"] .stage') as (HTMLElement & { __ottoSchool?: { debug(): unknown } }) | null;
    return (st?.__ottoSchool?.debug() as Debug | undefined) ?? null;
  });
}

async function project(page: Page, pick: { kind: 'kid' | 'door' | 'head'; id: string }): Promise<{ x: number; y: number; visible: boolean } | null> {
  return page.evaluate((p) => {
    const st = document.querySelector('section.hbox[data-kind="classrooms"] .stage') as (HTMLElement & { __ottoSchool?: { project(x: unknown): unknown } }) | null;
    return (st?.__ottoSchool?.project(p) as { x: number; y: number; visible: boolean } | null) ?? null;
  }, pick);
}

/** Click a door / kid where the scene draws it (kids: a bit below the head). */
async function clickOn(page: Page, pick: { kind: 'kid' | 'door'; id: string }, dy = 0): Promise<void> {
  await expect.poll(async () => (await project(page, pick))?.visible ?? false, { timeout: 15_000 }).toBe(true);
  const p = (await project(page, pick))!;
  await page.mouse.click(p.x, p.y + dy);
}

async function enterOurRoom(page: Page): Promise<void> {
  const d = await debug(page);
  if (d?.view.kind !== 'corridor' && d?.view.roomId === wsId) return;
  if (d?.view.kind !== 'corridor') {
    await box(page).getByRole('button', { name: 'Corridor', exact: true }).click();
    await expect.poll(() => debug(page).then((x) => x?.view.kind), { timeout: 45_000 }).toBe('corridor');
  }
  await clickOn(page, { kind: 'door', id: wsId }, 60);
  await expect.poll(() => debug(page).then((x) => `${x?.view.kind}:${x?.view.roomId}`), { timeout: 45_000 }).toBe(`room:${wsId}`);
}

test.beforeAll(async () => {
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(c.ctx, c.base, WS_NAME);
  ids.idle1 = await shell('Idle Kid One');
  ids.idle2 = await shell('Idle Kid Two');
  ids.printer = await shell('Printer Kid');
  ids.kick = await shell('Kick Me Kid');
  ids.bench = await shell('Detention Kid');
  // Two quiet kids: a long sleep keeps the shell alive without output or a
  // prompt (idle — free to wander). A shell AT its prompt counts as "needs
  // you" (hand up), which is what Kick Me / Detention Kid stay at.
  for (const id of [ids.idle1, ids.idle2]) await ctx.post(`${base}/api/v1/sessions/${id}/input`, { data: { text: 'sleep 3600', submit: true } });
  // The printer keeps producing output (a "working" kid with a live screen).
  await ctx.post(`${base}/api/v1/sessions/${ids.printer}/input`, {
    data: { text: `while true; do echo ${MARKER} $(date +%s); sleep 1; done`, submit: true },
  });
});

test.afterAll(async () => {
  for (const id of Object.values(ids)) await ctx.delete(`${base}/api/v1/sessions/${id}`).catch(() => undefined);
});

test.beforeEach(async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
});

test.describe('Original School flows', () => {
  test.describe.configure({ mode: 'serial' });
test('the school loads its Blender assets and opens in the corridor, one door per workspace', async ({ page }) => {
  await boot(page);
  const d = (await debug(page))!;
  expect(d.missing, 'every kit node / character file loaded').toEqual([]);
  expect(d.view.kind).toBe('corridor');
  // Our door is drawn and labelled on hover.
  const door = await project(page, { kind: 'door', id: wsId });
  expect(door?.visible).toBe(true);
  await page.mouse.move(door!.x, door!.y + 60);
  await expect(box(page).locator('.hover-tag')).toContainText(WS_NAME);
  await box(page).locator('.stage').screenshot({ path: test.info().outputPath('school-corridor.png') });
});

test('clicking the door walks into the classroom: one kid per session, the headmaster at the board', async ({ page }) => {
  await boot(page);
  await enterOurRoom(page);
  await expect(box(page).getByRole('navigation', { name: /Where you are/ })).toContainText(WS_NAME);
  const d = (await debug(page))!;
  for (const id of Object.values(ids)) expect(d.kids.map((k) => k.id)).toContain(id);
  expect(d.head).not.toBeNull();
  await box(page).locator('.stage').screenshot({ path: test.info().outputPath('school-classroom.png') });
});

test('a PC monitor shows the session’s live terminal output', async ({ page }) => {
  await boot(page);
  await enterOurRoom(page);
  await expect
    .poll(async () => Object.values((await debug(page))?.screens ?? {}).some((s) => s.includes(MARKER)), { timeout: 30_000 })
    .toBe(true);
});

test('kids live: idle kids get up and wander, the headmaster walks over and inspects screens', async ({ page }) => {
  test.setTimeout(240_000);
  await boot(page);
  await enterOurRoom(page);
  const seen = new Set<string>();
  const headModes = new Set<string>();
  await expect
    .poll(
      async () => {
        const d = (await debug(page))!;
        for (const k of d.kids) if (['walking', 'visiting', 'returning'].includes(k.mode)) seen.add(k.id);
        if (d.head?.mode === 'inspecting' && d.head.target) headModes.add(d.head.target);
        return seen.size > 0 && headModes.size > 0;
      },
      { timeout: 120_000, intervals: [500] },
    )
    .toBe(true);
  // Only idle kids wander — the printer (always working) stays at its desk.
  expect(seen.has(ids.printer)).toBe(false);
});

test.describe('Screen navigation', () => {
  // Preserve the CSS viewport, animation and real canvas interactions while
  // drawing one quarter as many pixels. Full-resolution asset/visual journeys
  // above stay separate: SwiftShader capture stalled this flow for 35 s in CI.
  test.use({ deviceScaleFactor: 0.5 });
test('click a kid → its card; look at its screen; open the session', async ({ page }) => {
  await boot(page);
  await enterOurRoom(page);
  // Wait for the printer to be seated and visible, then click it.
  await clickOn(page, { kind: 'kid', id: ids.printer }, 30);
  await expect(card(page)).toContainText('Printer Kid');
  await expectFullyInViewport(page, card(page), 'school kid card');
  await card(page).getByRole('button', { name: 'Look at screen' }).click();
  await expect.poll(() => debug(page).then((d) => `${d?.view.kind}:${d?.view.kidId}`), { timeout: 30_000 }).toBe(`screen:${ids.printer}`);
  await box(page).locator('.stage').screenshot({ path: test.info().outputPath('school-screen.png') });
  await card(page).getByRole('button', { name: 'Open session' }).click();
  await expect(page).toHaveURL(new RegExp(`#/agents/${ids.printer}$`));
});
});

test('kick out: confirm, the kid walks out of the door, the session is deleted', async ({ page }) => {
  await boot(page);
  await enterOurRoom(page);
  await clickOn(page, { kind: 'kid', id: ids.kick }, 30);
  await expect(card(page)).toContainText('Kick Me Kid');
  await card(page).getByRole('button', { name: 'Kick out…' }).click();
  await page.getByRole('dialog').getByRole('button', { name: 'Kick out', exact: true }).click();
  await expect.poll(async () => (await debug(page))?.kids.find((k) => k.id === ids.kick)?.mode ?? 'gone', { timeout: 5_000 }).toMatch(/standing|leaving|gone/);
  await expect(page.locator('.toast').filter({ hasText: 'Kicked out Kick Me Kid' })).toBeVisible({ timeout: 20_000 });
  await expect
    .poll(async () => {
      const r = await ctx.get(`${base}/api/v1/sessions?ids=${ids.kick}`);
      return r.ok() ? ((await r.json()) as unknown[]).length : -1;
    })
    .toBe(0);
  delete ids.kick;
});

test('detention: the kid walks to the bench and the session is archived; release brings it back', async ({ page }) => {
  await boot(page);
  await enterOurRoom(page);
  await clickOn(page, { kind: 'kid', id: ids.bench }, 30);
  await card(page).getByRole('button', { name: 'Detention' }).click();
  await expect
    .poll(async () => {
      const r = await ctx.get(`${base}/api/v1/sessions?ids=${ids.bench}&archived=true`);
      return r.ok() ? ((await r.json()) as { archived: boolean }[])[0]?.archived ?? false : false;
    }, { timeout: 15_000 })
    .toBe(true);
  await expect.poll(async () => (await debug(page))?.kids.find((k) => k.id === ids.bench)?.mode, { timeout: 90_000 }).toBe('bench');
  // Release from the bench.
  await clickOn(page, { kind: 'kid', id: ids.bench }, 30);
  await card(page).getByRole('button', { name: 'Release from detention' }).click();
  await expect
    .poll(async () => {
      const r = await ctx.get(`${base}/api/v1/sessions?ids=${ids.bench}`);
      return r.ok() ? ((await r.json()) as { archived: boolean }[])[0]?.archived : true;
    }, { timeout: 15_000 })
    .toBe(false);
});

test('keyboard: Escape walks back out to the corridor; Enter walks through the door you face', async ({ page }) => {
  await boot(page);
  await enterOurRoom(page);
  await box(page).locator('.stage').focus();
  await page.keyboard.press('Escape');
  await expect.poll(() => debug(page).then((d) => d?.view.kind), { timeout: 45_000 }).toBe('corridor');
  // The corridor camera stands at our door facing it after walking out.
  await page.keyboard.press('Enter');
  await expect.poll(() => debug(page).then((d) => `${d?.view.kind}:${d?.view.roomId}`), { timeout: 45_000 }).toBe(`room:${wsId}`);
});

test('dark theme renders the school too', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'dark' });
  await boot(page);
  await enterOurRoom(page);
  await box(page).locator('.stage').screenshot({ path: test.info().outputPath('school-classroom-dark.png') });
});


});

test.describe('School regressions', () => {
  test.describe.configure({ mode: 'default', timeout: 90_000 });
  // Functional ownership/restoration checks need real WebGL, not animation load.
  // Original journeys above retain their full-resolution animated coverage.
  // Keep the CSS viewport unchanged while rendering and capturing one quarter
  // as many pixels. The software-renderer cap must preserve this lower scale.
  test.use({ reducedMotion: 'reduce', viewport: { width: 1100, height: 800 }, deviceScaleFactor: 0.5 });
test('software rendering respects display scales below one', async ({ page }) => {
  await boot(page, { view: 'list' });
  const dimensions = await page.evaluate(async () => {
    const scenePath = '/src/modules/home/school/scene.ts';
    const { mountSchool } = await import(/* @vite-ignore */ scenePath) as typeof import('../src/modules/home/school/scene');
    const host = document.createElement('div');
    host.style.cssText = 'position:fixed;inset:0;width:320px;height:240px;z-index:10000';
    document.body.append(host);
    let scene: Awaited<ReturnType<typeof mountSchool>> | undefined;
    try {
      scene = await mountSchool(host, { reducedMotion: true, dark: true, label: 'Drawing buffer regression' });
      await new Promise<void>((resolve) => scene!.onFrame(resolve));
      const canvas = host.querySelector('canvas')!;
      const gl = canvas.getContext('webgl2')!;
      const info = gl.getExtension('WEBGL_debug_renderer_info')!;
      return {
        renderer: String(gl.getParameter(info.UNMASKED_RENDERER_WEBGL)),
        scale: window.devicePixelRatio,
        css: [canvas.clientWidth, canvas.clientHeight],
        buffer: [gl.drawingBufferWidth, gl.drawingBufferHeight],
      };
    } finally {
      scene?.destroy();
      host.remove();
    }
  });
  expect(dimensions.renderer).toMatch(/swiftshader|llvmpipe|softpipe|software/i);
  expect(dimensions.scale).toBe(0.5);
  expect(dimensions.css).toEqual([320, 240]);
  expect(dimensions.buffer).toEqual([160, 120]);
});
test('new corridor doors can be picked before their first rendered frame', async ({ page }) => {
  await boot(page, { view: 'list' });
  const result = await page.evaluate(async () => {
    const scenePath = '/src/modules/home/school/scene.ts';
    const modelPath = '/src/modules/home/school/model.ts';
    const { mountSchool } = await import(/* @vite-ignore */ scenePath) as typeof import('../src/modules/home/school/scene');
    const { buildSchool } = await import(/* @vite-ignore */ modelPath) as typeof import('../src/modules/home/school/model');
    const host = document.createElement('div');
    host.style.cssText = 'position:fixed;inset:0;width:1000px;height:650px;z-index:10000';
    document.body.append(host);
    let scene: Awaited<ReturnType<typeof mountSchool>> | undefined;
    try {
      scene = await mountSchool(host, { reducedMotion: true, dark: true, label: 'Picking regression' });
      await new Promise<void>((resolve) => scene!.onFrame(resolve));
      // Data can arrive between animation frames. Keep the real renderer and
      // camera, but prevent a render from incidentally updating hit matrices.
      scene.setActive(false);
      const frames = scene.debug().frames;
      scene.update(buildSchool({ workspaces: [{ id: 'picking-room', name: 'Picking room' }], currentId: 'picking-room', sessions: [] }));
      const point = scene.project({ kind: 'door', id: 'picking-room' })!;
      return { visible: point.visible, picked: scene.pick(point.x, point.y + 60), frames, after: scene.debug().frames };
    } finally {
      scene?.destroy();
      host.remove();
    }
  });
  expect(result.visible).toBe(true);
  expect(result.after, 'picking must not depend on another render').toBe(result.frames);
  expect(result.picked).toEqual({ kind: 'door', id: 'picking-room' });
});
for (const key of ['Enter', 'Space']) {
  test(`native card controls retain ${key} activation inside the stage`, async ({ page }) => {
    // Check on deliberately does not animate in reduced-motion mode.
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await page.addInitScript((scheme) => localStorage.setItem('otto_scheme', scheme), key === 'Enter' ? 'light' : 'dark');
    await boot(page);
    await enterOurRoom(page);
    // Select through the keyboard companion for this keyboard-controls test.
    // A projected world point can sit behind the fixed Otto bar: CI clicked
    // that bar at y=731 and opened its palette instead of selecting the kid.
    // The original journeys above independently exercise real canvas picking.
    const printerRow = box(page).getByRole('region', { name: `Classroom ${WS_NAME}`, exact: true })
      .getByRole('button', { name: /^Open Printer Kid,/ });
    await printerRow.focus();
    const printerCard = page.getByRole('group', { name: 'Printer Kid — details', exact: true });
    await expect(printerCard).toBeVisible();
    const activate = async (name: string) => {
      const button = printerCard.getByRole('button', { name, exact: true });
      await button.focus();
      await expect(button).toBeFocused();
      await button.press(key);
    };
    await activate('Look at screen');
    await expect.poll(() => debug(page).then((d) => d?.view.kind)).toBe('screen');
    await activate('Back to the room');
    await expect.poll(() => debug(page).then((d) => d?.view.kind)).toBe('room');
    await activate('Check on');
    await expect.poll(() => debug(page).then((d) => d?.head?.target)).toBe(ids.printer);
    await expect(card(page)).toBeVisible();
    await box(page).screenshot({ path: test.info().outputPath(`school-card-${key === 'Enter' ? 'light' : 'dark'}.png`) });
    await activate('Kick out…');
    await expect(page.getByRole('dialog')).toContainText('Printer Kid');
    await page.getByRole('dialog').getByRole('button', { name: 'Cancel', exact: true }).click();
    await activate('Close details');
    await expect(card(page)).toHaveCount(0);
    // Check on moves the headmaster beside this kid and can occlude a canvas
    // hit. Keyboard focus is the supported accessible way to reselect them.
    await printerRow.focus();
    await activate('Open session');
    await expect(page).toHaveURL(new RegExp(`#/agents/${ids.printer}$`));
  });
}

test('keyboard companion reveals the focused row and its menu in 3D', async ({ page }) => {
  await boot(page);
  const list = box(page).getByRole('region', { name: 'School list', exact: true });
  // Other workers legitimately have a Printer Kid in their own classroom.
  const room = list.getByRole('region', { name: `Classroom ${WS_NAME}`, exact: true });
  const row = room.getByRole('button', { name: /^Open Printer Kid/ });
  await row.focus();
  await expect(row).toBeFocused();
  await expect.poll(() => list.evaluate((el) => el.getBoundingClientRect().height)).toBeGreaterThan(50);
  await expectFullyInViewport(page, row, 'focused school row');
  await page.keyboard.press('Tab');
  const more = room.getByRole('button', { name: 'More actions for Printer Kid' });
  await expect(more).toBeFocused();
  await expectFullyInViewport(page, more, 'focused school menu control');
  await box(page).screenshot({ path: test.info().outputPath('school-keyboard-focus.png') });
  await more.press('Enter');
  await expect(page.getByRole('menu')).toBeVisible();
  await page.keyboard.press('Escape');
  await row.focus();
  await row.press('Enter');
  await expect(page).toHaveURL(new RegExp(`#/agents/${ids.printer}$`));
});

const savedRoom = (page: Page) => page.evaluate(() => {
  const views = JSON.parse(localStorage.getItem('otto_home_views') ?? '[]');
  return views.find((v: { id: string }) => v.id === 'v1')?.boxes.find((b: { id: string }) => b.id === 'cr1')?.config.room ?? null;
});

test('saved classroom survives delayed assets and restores before navigation is persisted', async ({ page }) => {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => { release = resolve; });
  let requested = false;
  await page.route('**/school/school-kit.glb', async (route) => { requested = true; await gate; await route.continue(); });
  const booting = boot(page, { view: '3d', room: wsId });
  void booting.catch(() => {}); // preserve the original error at the await below
  try {
    await expect.poll(() => requested, { timeout: 15_000 }).toBe(true);
    await expect.poll(() => savedRoom(page)).toBe(wsId);
  } finally { release(); }
  await booting;
  await expect.poll(() => debug(page).then((d) => d?.view.roomId), { timeout: 30_000 }).toBe(wsId);
  await expect.poll(() => savedRoom(page)).toBe(wsId);
  await box(page).getByRole('button', { name: 'Corridor', exact: true }).click();
  await expect.poll(() => savedRoom(page)).toBe(null);
});

test('list-first mount and list/3D remount retain the last classroom', async ({ page }) => {
  await boot(page, { view: 'list', room: wsId });
  await expect(box(page).getByRole('region', { name: 'School list', exact: true })).toBeVisible();
  await expect.poll(() => savedRoom(page)).toBe(wsId);
  await box(page).getByRole('button', { name: '3D', exact: true }).click();
  await expect.poll(() => debug(page).then((d) => d?.view.roomId), { timeout: 60_000 }).toBe(wsId);
  await box(page).getByRole('button', { name: 'List', exact: true }).click();
  await expect.poll(() => savedRoom(page)).toBe(wsId);
  await box(page).getByRole('button', { name: '3D', exact: true }).click();
  await expect.poll(() => debug(page).then((d) => d?.view.roomId), { timeout: 60_000 }).toBe(wsId);
});


test('scene initialization errors say load and Retry recovers a real scene', async ({ page }) => {
  let failed = true;
  await page.route('**/school/school-kit.glb', (route) => failed ? route.abort('failed') : route.continue());
  await boot(page, { view: 'list' });
  await box(page).getByRole('button', { name: '3D', exact: true }).click();
  await expect(box(page).getByRole('alert')).toContainText('Couldn’t load the 3D school');
  await expect(box(page)).not.toContainText('last good load');
  failed = false;
  await box(page).getByRole('button', { name: 'Retry', exact: true }).click();
  await expect.poll(() => debug(page).then((d) => d?.frames ?? 0), { timeout: 60_000 }).toBeGreaterThan(0);
  await expect(box(page).getByRole('alert')).toHaveCount(0);
});

test('rendered room visits and individual removal release owned skeleton textures', async ({ page }) => {
  await page.emulateMedia({ reducedMotion: 'reduce' });
  let threeUrl = '';
  page.on('request', (request) => {
    if (new URL(request.url()).pathname.endsWith('/three.js')) threeUrl = request.url();
  });
  ids.resource = await shell('Resource removal kid');
  await boot(page);
  // Import the exact optimized module already requested by the scene, so the
  // instrumented prototype is the one used by the renderer, not a second copy.
  // Capture the request outside the page: its default 250-entry resource
  // timing buffer can fill before the late-loaded scene imports Three.
  await page.evaluate(async (url) => {
    if (!url) throw new Error('Scene Three module was not loaded');
    const { Skeleton } = await import(url);
    const live = new Set<object>();
    const stats = { allocated: 0, disposed: 0, live: 0 };
    const allocate = Skeleton.prototype.computeBoneTexture;
    const dispose = Skeleton.prototype.dispose;
    Skeleton.prototype.computeBoneTexture = function (this: object) {
      const result = allocate.call(this);
      if (!live.has(this)) { live.add(this); stats.allocated++; }
      stats.live = live.size;
      return result;
    };
    Skeleton.prototype.dispose = function (this: object) {
      if (live.delete(this)) stats.disposed++;
      stats.live = live.size;
      return dispose.call(this);
    };
    (window as unknown as { __schoolSkeletons: typeof stats }).__schoolSkeletons = stats;
  }, threeUrl);
  const resources = () => page.evaluate(() => (window as unknown as { __schoolSkeletons: { allocated: number; disposed: number; live: number } }).__schoolSkeletons);
  for (let visit = 0; visit < 10; visit++) {
    // The accessible room action is stable across repeated camera returns;
    // a world-space door can be outside the current corridor viewport.
    const walkIn = box(page).getByRole('region', { name: `Classroom ${WS_NAME}`, exact: true }).getByRole('button', { name: 'Walk in', exact: true });
    await walkIn.focus();
    await walkIn.press('Enter');
    await expect.poll(() => debug(page).then((d) => d?.view.roomId)).toBe(wsId);
    await expect.poll(() => resources().then((r) => r.live)).toBeGreaterThan(0);
    expect((await debug(page))?.head).not.toBeNull();
    if (visit === 0) {
      await expect.poll(() => debug(page).then((d) => d?.kids.some((k) => k.id === ids.resource))).toBe(true);
      const before = (await resources()).disposed;
      expect((await ctx.delete(`${base}/api/v1/sessions/${ids.resource}`)).ok()).toBe(true);
      await expect.poll(() => debug(page).then((d) => d?.kids.some((k) => k.id === ids.resource))).toBe(false);
      await expect.poll(() => resources().then((r) => r.disposed)).toBeGreaterThan(before);
      delete ids.resource;
    }
    await box(page).getByRole('button', { name: 'Corridor', exact: true }).click();
    await expect.poll(() => debug(page).then((d) => d?.view.kind)).toBe('corridor');
    await expect.poll(() => resources().then((r) => r.live)).toBe(0);
  }
  const result = await resources();
  expect(result.allocated).toBeGreaterThan(10);
  expect(result.disposed).toBe(result.allocated);
});


test('moving focus away from stage releases every held movement key', async ({ page }) => {
  await boot(page);
  const stage = box(page).locator('.stage');
  await stage.focus();
  const position = () => page.evaluate(() => {
    const el = document.querySelector('.school .stage') as HTMLElement & { __ottoSchool: { debug(): { camera: { x: number } } } };
    return el.__ottoSchool.debug().camera.x;
  });
  const initial = await position();
  await page.keyboard.down('d');
  await expect.poll(position).toBeGreaterThan(initial + 0.1);
  await box(page).getByRole('button', { name: 'List', exact: true }).focus();
  await page.keyboard.up('d');
  const stopped = await position();
  await page.evaluate(async () => { for (let frame = 0; frame < 12; frame++) await new Promise<void>((resolve) => requestAnimationFrame(() => resolve())); });
  expect(await position()).toBeCloseTo(stopped, 5);
});

});

test('hidden document suspends the School render loop and resumes without losing its room', async ({ page }) => {
  await boot(page);
  await enterOurRoom(page);
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => true });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  const before = (await debug(page))!;
  await page.waitForTimeout(350); // ui-guards: allow — prove no hidden frames occur over several frame intervals.
  expect((await debug(page))!.frames, 'hidden native webviews must do no School rendering').toBe(before.frames);
  await page.evaluate(() => {
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => false });
    document.dispatchEvent(new Event('visibilitychange'));
  });
  await expect.poll(() => debug(page).then(d => d?.frames ?? 0)).toBeGreaterThan(before.frames);
  expect((await debug(page))!.view).toEqual(before.view);
});
