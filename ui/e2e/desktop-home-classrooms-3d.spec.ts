import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
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
  reducedMotion: 'no-preference',
  viewport: { width: 1440, height: 900 },
});
// Software WebGL on a busy CI runner: the first mount (three + ~10 MB of
// models + the HDRI) alone can take most of Playwright's default 45 s.
test.describe.configure({ mode: 'serial', timeout: 150_000 });

const WS_NAME = `School E2E ${Date.now().toString(36)}`;
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

async function boot(page: Page): Promise<void> {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    if (sessionStorage.getItem('otto_e2e_school_reset')) return;
    sessionStorage.setItem('otto_e2e_school_reset', '1');
    localStorage.removeItem('otto_home_active');
    localStorage.removeItem('otto_bar_spaces');
    localStorage.removeItem('otto_home_rotate');
    localStorage.setItem(
      'otto_home_views',
      JSON.stringify([{ id: 'v1', name: 'School', boxes: [{ id: 'cr1', kind: 'classrooms', w: 12, h: 8, config: { view: '3d' } }] }]),
    );
  }, wsId);
  await page.goto('/#/home');
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
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
    await expect.poll(() => debug(page).then((x) => x?.view.kind)).toBe('corridor');
  }
  await clickOn(page, { kind: 'door', id: wsId }, 60);
  await expect.poll(() => debug(page).then((x) => `${x?.view.kind}:${x?.view.roomId}`), { timeout: 15_000 }).toBe(`room:${wsId}`);
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

test('click a kid → its card; look at its screen; open the session', async ({ page }) => {
  await boot(page);
  await enterOurRoom(page);
  // Wait for the printer to be seated and visible, then click it.
  await clickOn(page, { kind: 'kid', id: ids.printer }, 30);
  await expect(card(page)).toContainText('Printer Kid');
  await expectFullyInViewport(page, card(page), 'school kid card');
  await card(page).getByRole('button', { name: 'Look at screen' }).click();
  await expect.poll(() => debug(page).then((d) => `${d?.view.kind}:${d?.view.kidId}`), { timeout: 10_000 }).toBe(`screen:${ids.printer}`);
  await box(page).locator('.stage').screenshot({ path: test.info().outputPath('school-screen.png') });
  await card(page).getByRole('button', { name: 'Open session' }).click();
  await expect(page).toHaveURL(new RegExp(`#/agents/${ids.printer}$`));
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
  await expect.poll(async () => (await debug(page))?.kids.find((k) => k.id === ids.bench)?.mode, { timeout: 30_000 }).toBe('bench');
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
  await expect.poll(() => debug(page).then((d) => d?.view.kind), { timeout: 10_000 }).toBe('corridor');
  // The corridor camera stands at our door facing it after walking out.
  await page.keyboard.press('Enter');
  await expect.poll(() => debug(page).then((d) => `${d?.view.kind}:${d?.view.roomId}`), { timeout: 10_000 }).toBe(`room:${wsId}`);
});

test('dark theme renders the school too', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'dark' });
  await boot(page);
  await enterOurRoom(page);
  await box(page).locator('.stage').screenshot({ path: test.info().outputPath('school-classroom-dark.png') });
});
