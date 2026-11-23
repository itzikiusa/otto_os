// The making screen at real-life pace on the real productions server
// (create-progress-fixture.ts), run by desktop-create and mobile-create: thinking with a clock, a long step that
// visibly moves while the server's percent stands still, each piece's
// picture on its row, "See the scene so far" before the end, a made piece
// opened (moves playing, its skeleton shown) while the rest keeps going,
// and two creations at once on the "In progress" strip, the second waiting.
import { expect, type Locator, type Page, type TestInfo } from '@playwright/test';
import type { Production } from '../src/lib/api/types';
import { evidence, expectPlain, openCreate, type CreateSeed } from './create-fixture';
import { apiCtx, seedWorkspace } from './seed';
import { Pace, holdPlanning } from './create-progress-fixture';

/** A Create route (`createHref` in plan.ts). */
const createPath = (...parts: string[]): string => ['design/create', ...parts.map(encodeURIComponent)].join('/');

/** A row of the making list by the piece's name. */
const row = (page: Page, name: string): Locator => page.getByTestId('create-plan-item').filter({ has: page.locator('b').getByText(name, { exact: true }) });
const barNow = async (r: Locator): Promise<number> => Number(await r.getByTestId('create-item-bar').getAttribute('aria-valuenow'));

/** How many pixels differ between two pictures of the same element, taken `gapMs` apart. */
async function moved(page: Page, el: Locator, gapMs: number): Promise<number> {
  const a = (await el.screenshot()).toString('base64');
  await page.waitForTimeout(gapMs); // ui-guards: allow — the gap IS the measurement (does the picture change over time?)
  const b = (await el.screenshot()).toString('base64');
  return page.evaluate(async ([x, y]) => {
    const read = async (s: string) => {
      const img = new Image();
      img.src = `data:image/png;base64,${s}`;
      await img.decode();
      const c = document.createElement('canvas');
      c.width = img.width;
      c.height = img.height;
      const g = c.getContext('2d')!;
      g.drawImage(img, 0, 0);
      return g.getImageData(0, 0, c.width, c.height).data;
    };
    const [p, q] = [await read(x), await read(y)];
    let n = 0;
    for (let i = 0; i < Math.min(p.length, q.length); i += 4) if (Math.abs(p[i] - q[i]) + Math.abs(p[i + 1] - q[i + 1]) + Math.abs(p[i + 2] - q[i + 2]) > 30) n++;
    return n;
  }, [a, b]);
}

export async function progressFlow(page: Page, seed: CreateSeed, info: TestInfo): Promise<void> {
  const shot = (n: string, keepScroll = false) => evidence(page, info.project.name, n, { keepScroll });
  const pace = new Pace(seed);
  const { ctx, base } = await apiCtx();
  const read = async (id: string) => (await (await ctx.get(`${base}/api/v1/design/productions/${id}`)).json()) as Production;
  const item = async (id: string, it: string) => (await read(id)).plan!.items.find((i) => i.id === it)!;
  const ours = (p: Page) => p.getByTestId('create-in-progress-item').filter({ hasText: 'The man and the chest' });
  const ids: string[] = [];
  let hold = holdPlanning();
  try {
    // ── Thinking: about a minute, a clock that ticks, and the way out when it is slow. Its own
    // window (a browser context of its own: a clock belongs to its context), whose clock the test
    // moves on two and a half minutes while the daemon's planner is held.
    const u = info.project.use;
    const thinking = await page.context().browser()!.newContext({
      ...{ viewport: u.viewport, userAgent: u.userAgent, deviceScaleFactor: u.deviceScaleFactor, isMobile: u.isMobile, hasTouch: u.hasTouch, baseURL: u.baseURL },
      storageState: await page.context().storageState(),
      serviceWorkers: 'block',
    });
    const think = await thinking.newPage();
    await think.clock.install();
    await openCreate(think, seed, '');
    await think.getByTestId('create-idea').fill('A tired man next to a chest of gold. He says the chest is his.');
    const posted = think.waitForResponse((r) => r.url().endsWith('/api/v1/design/productions') && r.request().method() === 'POST');
    await think.getByTestId('create-make').click();
    ids.push(((await (await posted).json()) as { id: string }).id);
    await expect(think.getByTestId('create-planning')).toContainText('This takes about a minute.');
    const clock = think.getByTestId('create-planning-clock');
    const first = await clock.innerText();
    await expect.poll(() => clock.innerText(), { timeout: 5000 }).not.toBe(first);
    await expect(think.getByTestId('create-planning-slow')).toHaveCount(0);
    await expectPlain(think, 'thinking');
    await evidence(think, info.project.name, 'p01-thinking');
    await think.clock.fastForward('02:30');
    await expect(think.getByTestId('create-planning-slow')).toContainText('taking longer than usual', { timeout: 5000 });
    await expect(think.getByTestId('create-planning-slow')).toContainText('In progress');
    await expect(clock).toHaveText(/^2:3\d$/);
    await evidence(think, info.project.name, 'p02-thinking-long');
    await thinking.close();
    // The same creation in this window, from the "In progress" strip; the planner finishes.
    await openCreate(page, seed, '');
    const planning = page.locator(`[data-testid="create-in-progress-item"][data-production="${ids[0]}"]`);
    await expect(planning.getByTestId('create-in-progress-state')).toHaveText('Planning');
    await planning.click();
    await expect(page.getByTestId('create-planning')).toBeVisible();
    hold.release();
    await expect(page.getByRole('heading', { name: 'Here’s my plan' })).toBeVisible({ timeout: 30_000 });
    await page.getByTestId('create-make').click();

    // ── A long step: the server's percent stands still, the clock and the bar do not.
    await expect(page.getByTestId('create-now')).toHaveText('Now making: The man', { timeout: 30_000 });
    // The man's line may already be spoken (spoken lines come right after the pictures).
    await expect(page.getByTestId('create-left')).toContainText(/[01] of 4 ready · about/);
    const man = row(page, 'The man');
    expect(await pace.at('man')).toBe('concept');
    await expect(man.getByTestId('create-item-state')).toContainText('Drawing the picture…');
    await expect(man).toContainText('Step 1 of 5');
    // The server has the engine's line of this step (its percent is that line's, and stays).
    await expect.poll(async () => (await item(ids[0], 'man')).message ?? '').toContain('concept picture');
    const pct = (await item(ids[0], 'man')).progress_pct;
    const c0 = await man.getByTestId('create-item-clock').innerText();
    const b0 = await barNow(man);
    await expect.poll(() => man.getByTestId('create-item-clock').innerText(), { message: 'the clock ticks', timeout: 5000 }).not.toBe(c0);
    await expect.poll(() => barNow(man), { message: 'the bar moves inside the step', timeout: 10_000 }).toBeGreaterThan(b0);
    expect((await item(ids[0], 'man')).progress_pct, 'while the server percent did not move').toBe(pct);
    await expectPlain(page, 'making, drawing');
    await shot('p03-making-drawing');
    // The picture is drawn: it shows on the row as soon as the next step starts.
    await pace.next('man');
    await expect(man.getByTestId('create-thumb').locator('img')).toBeVisible({ timeout: 10_000 });
    await pace.until('man', 'generate');
    await expect(man).toContainText('Building the 3D shape…');
    await expect(man).toContainText('Step 2 of 5');
    const b1 = await barNow(man);
    expect(b1, 'the next step starts at its own mark').toBeGreaterThanOrEqual(20);
    await expect.poll(() => barNow(man), { timeout: 10_000 }).toBeGreaterThan(b1);
    expect(await barNow(man), 'never at the end of the step before the step ends').toBeLessThan(40);
    await shot('p04-making-picture');

    // ── The man stands in the scene (his skeleton comes once every model is made); the chest is coming.
    await pace.through('man', 'gameready');
    await expect(man.getByTestId('create-item-state')).toHaveText('In the scene · its skeleton comes next', { timeout: 10_000 });
    await expect(row(page, 'Treasure chest')).toContainText('Drawing the picture…', { timeout: 10_000 });
    await expect(page.getByTestId('create-so-far')).toBeVisible({ timeout: 10_000 });
    await expect(page.getByTestId('create-still-coming')).toContainText('Treasure chest');
    await expectPlain(page, 'making, scene so far offered');
    await shot('p05-so-far-offered');
    await man.getByTestId('create-open-piece').click();
    const look = page.getByTestId('create-look');
    await expect(look).toContainText('The rest keeps going while you look.');
    await expect(page.getByTestId('create-rig-still')).toHaveText(/Moves are still being added\. It stands still until then\./, { timeout: 30_000 });
    await shot('p06-open-skeleton-coming');
    await page.keyboard.press('Escape');
    await expect(look).toBeHidden();

    // ── The scene so far, while the chest is still coming.
    await page.getByTestId('create-so-far').click();
    await expect(page.getByText('Here is what I made so far. The rest is still coming.')).toBeVisible();
    await expect(page.getByTestId('create-coming')).toContainText('Treasure chest');
    await expect(page.locator('[data-testid="create-model"] canvas').first()).toBeVisible({ timeout: 30_000 });
    await expect(page.getByTestId('create-piece').filter({ hasText: 'The man' })).toBeVisible();
    await expect(page.getByTestId('create-steer')).toHaveCount(0);
    await page.waitForLoadState('networkidle').catch(() => {});
    await expectPlain(page, 'scene so far');
    await shot('p10-scene-so-far');
    // The chest arrives: the scene takes it in, still "so far" (the coins are coming).
    await pace.through('chest', 'gameready');
    await expect(page.getByTestId('create-coming')).not.toContainText('Treasure chest', { timeout: 10_000 });
    await expect(page.getByTestId('create-piece').filter({ hasText: 'Treasure chest' })).toBeVisible();
    await page.getByTestId('create-back').click();
    await expect(page.getByRole('heading', { name: 'Making your scene…' })).toBeVisible();

    // ── Two at once: Back keeps it going, a second one takes its turn.
    await page.getByTestId('create-back').click();
    await expect(ours(page)).toHaveCount(1);
    await expect(ours(page).getByTestId('create-in-progress-state')).toHaveText(['Making 3 of 4']);
    await page.getByTestId('create-idea').fill('A second chest of gold, smaller.');
    await expect(page.getByTestId('create-takes-turn')).toBeVisible();
    await shot('p11-home-one-in-progress');
    hold = holdPlanning();
    const second = page.waitForResponse((r) => r.url().endsWith('/api/v1/design/productions') && r.request().method() === 'POST');
    await page.getByTestId('create-make').click();
    ids.push(((await (await second).json()) as { id: string }).id);
    await expect(page.getByTestId('create-planning')).toBeVisible();
    hold.release();
    await expect(page.getByTestId('create-takes-turn')).toBeVisible({ timeout: 30_000 });
    await page.getByTestId('create-make').click();
    await expect(page.getByTestId('create-now')).toHaveText('Waiting for its turn');
    await expect(page.getByTestId('create-left')).toContainText('One other creation is being made first.');
    expect((await read(ids[1])).queue_position, 'the server says it waits').toBe(1);
    await page.getByTestId('create-back').click();
    await expect(ours(page).getByTestId('create-in-progress-state')).toHaveText(['Making 3 of 4', 'Waiting, 1 ahead']);
    await expect(ours(page).first().locator('img')).toBeVisible({ timeout: 10_000 });
    await expectPlain(page, 'home, two in progress');
    await shot('p12-home-two-in-progress');

    // ── The other Product tabs stay usable while it is making.
    for (const tab of ['Stories', 'Designs', 'Voice', 'Learnings']) {
      await page.getByRole('tab', { name: tab }).click();
      await expect(page.getByRole('tab', { name: tab, selected: true })).toBeVisible();
      await expect(page.locator('.page-body').first()).toBeVisible();
    }
    await page.getByRole('tab', { name: 'Create' }).click();
    await expect(ours(page)).toHaveCount(2);

    // ── My creations shows them too, from the server's shelf.
    await page.getByTestId('create-mine').click();
    const card = (state: string) => page.getByTestId('create-gallery').locator('li').filter({ has: page.getByTestId('create-gallery-state').filter({ hasText: state }) });
    await expect(card('Making 3 of 4')).toBeVisible({ timeout: 15_000 });
    await expect(card('Waiting, 1 ahead')).toBeVisible();
    await shot('p13-my-creations');

    // ── The last piece is made; the man gets his skeleton and moves while he is looked at.
    await card('Making 3 of 4').locator('button').first().click();
    await expect(page.getByRole('heading', { name: 'Making your scene…' })).toBeVisible({ timeout: 15_000 });
    await pace.through('coins', 'gameready');
    expect(await pace.at('man'), 'his skeleton is being added').toBe('rig');
    await expect(man.getByTestId('create-item-state')).toContainText('Adding a skeleton…', { timeout: 10_000 });
    await expect(man).toContainText('Step 4 of 5');
    await man.getByTestId('create-open-piece').click();
    await expect(look).toContainText('The rest keeps going while you look.');
    await expect(page.getByTestId('create-rig-still')).toHaveText(/Moves are still being added\./, { timeout: 30_000 });
    await pace.next('man');
    await pace.next('man');
    expect(await pace.at('man'), 'his moves are being added').toBe('animate');
    await pace.next('man');
    await pace.next('man');
    // Made: the open look takes the man with his moves, playing the first one.
    await expect(look.getByTestId('create-rig-words')).toHaveText('Skeleton added · 3 moves', { timeout: 30_000 });
    const moves = look.getByTestId('create-move');
    await expect(moves).toHaveText(['Stand', 'Walk', 'Run']);
    await expect(moves.first()).toHaveAttribute('aria-pressed', 'true');
    // A new skeleton shows for a moment the first time, then hides (the button says which).
    const bones = look.getByTestId('create-skeleton');
    await expect(bones).toHaveText('Hide skeleton');
    await shot('p08-look-skeleton-moment');
    await expect(bones).toHaveText('Show skeleton', { timeout: 6000 });
    const canvas = look.locator('canvas').first();
    await look.getByRole('button', { name: 'Walk', exact: true }).click();
    await expect(look.getByRole('button', { name: 'Walk', exact: true })).toHaveAttribute('aria-pressed', 'true');
    // A busy machine draws fewer frames: sample until a change shows (a stopped pose changes nothing, see below).
    await expect.poll(() => moved(page, canvas, 600), { message: 'a move plays: the pose changes', timeout: 15_000 }).toBeGreaterThan(50);
    await bones.click();
    await expect(bones).toHaveText('Hide skeleton');
    await shot('p09-look-walking-skeleton');
    await look.getByRole('button', { name: 'Walk', exact: true }).click();
    await expect.poll(() => moved(page, canvas, 500), { message: 'stopped: the pose holds', timeout: 5000 }).toBeLessThan(30);
    await page.keyboard.press('Escape');
    // Everything is made: the finished scene, the man in it with his moves.
    await expect(page.getByTestId('create-piece').filter({ hasText: 'The man' }).getByTestId('create-rig-words')).toHaveText('Skeleton added · 3 moves', { timeout: 20_000 });
    await shot('p07-man-ready');
    expect((await read(ids[0])).status, 'looking never stops it').toBe('done');

    // ── Finished and already looked at, the first leaves "In progress"; the second is being made now.
    expect(await pace.at('man'), 'the second one has its turn').toBe('concept');
    await page.goto(`/#/${createPath()}`);
    await expect(ours(page).getByTestId('create-in-progress-state')).toHaveText([/^Making [12] of 4$/], { timeout: 15_000 });
    await shot('p14-home-first-done');
    // In a window that has not looked at it yet, it says "Ready to look" and opens to the scene.
    const fresh = await page.context().browser()!.newContext({
      ...{ viewport: u.viewport, userAgent: u.userAgent, deviceScaleFactor: u.deviceScaleFactor, isMobile: u.isMobile, hasTouch: u.hasTouch, baseURL: u.baseURL },
      storageState: await page.context().storageState(),
      serviceWorkers: 'block',
    });
    try {
      const other = await fresh.newPage();
      await other.addInitScript(() => localStorage.removeItem('otto.create.looked'));
      await openCreate(other, seed, '');
      await expect(ours(other).getByTestId('create-in-progress-state')).toHaveText(['Ready to look', /^Making [12] of 4$/], { timeout: 15_000 });
      await evidence(other, info.project.name, 'p14-home-ready-to-look');
      await ours(other).first().click();
      await expect(other.getByRole('heading', { name: 'The man and the chest' })).toBeVisible({ timeout: 20_000 });
    } finally {
      await fresh.close();
    }
  } finally {
    hold.release();
    pace.stop();
    for (const id of ids) await ctx.post(`${base}/api/v1/design/productions/${id}/cancel`).catch(() => undefined);
    await ctx.dispose();
  }
}

/** The owner's bug (2d8ff87e8): a creation being made was missing from My creations ("Nothing made
 *  yet") — another workspace was active, or an older answer landed last. Creations are not a
 *  workspace's: one started while workspace A is active is listed and opens with B active, on the
 *  REAL productions server and its shelf (`GET /design/creations`), and an older shelf answer
 *  (the server's own, from before it was started) that lands after a newer one changes nothing. */
export async function inProgressListed(first: Page, seed: CreateSeed, info: TestInfo): Promise<void> {
  let page = first;
  const { ctx, base } = await apiCtx();
  const [a, b] = await Promise.all([seedWorkspace(ctx, base, `Game A ${info.project.name}`), seedWorkspace(ctx, base, `Game B ${info.project.name}`)]);
  // The server's shelf before this creation exists: the answer held back below.
  const older = await (await ctx.get(`${base}/api/v1/design/creations`)).text();
  await ctx.dispose();
  let shelfCalls = 0;
  let hold = false;
  let landed = false;
  await page.context().route('**/api/v1/design/creations', async (route) => {
    shelfCalls++;
    // Once armed, the next answer is held back and is the shelf from before anything was started: it lands last.
    if (!hold) return route.continue();
    hold = false;
    await new Promise((r) => setTimeout(r, 12_000));
    await route.fulfill({ contentType: 'application/json', body: older });
    landed = true;
  });
  await openCreate(page, { ...seed, wsId: a }, '');
  await page.getByTestId('create-idea').fill('A small grey rock next to a tree.');
  const created = page.waitForResponse((r) => r.url().endsWith('/api/v1/design/productions') && r.request().method() === 'POST');
  await page.getByTestId('create-make').click();
  const { id } = (await (await created).json()) as { id: string };
  // Planned and waiting for "Make it": the shelf is the only way to it now (nothing is watching it).
  await expect(page.getByRole('heading', { name: 'Here’s my plan' })).toBeVisible({ timeout: 60_000 });
  const opener = page;
  try {
    // The owner moves to another workspace (here: a fresh window with B active, as after his refresh).
    page = await opener.context().newPage();
    await page.addInitScript(([w, theme]) => {
      localStorage.setItem('otto_workspace', w as string);
      if (theme) localStorage.setItem('otto_scheme', theme as string);
    }, [b, process.env.OTTO_CREATE_THEME ?? null]);
    await opener.close();
    hold = true;
    const before = shelfCalls;
    await page.goto('/#/design/create/mine');
    await expect(page.getByRole('heading', { name: 'My creations' })).toBeVisible({ timeout: 20_000 });
    if (info.project.name === 'desktop-browser') {
      // While that first answer is still out, the active workspace changes in the app (⌘K → Switch to workspace).
      await page.keyboard.press('ControlOrMeta+k');
      const bar = page.getByRole('combobox', { name: 'Ask Otto or search commands' });
      await bar.fill(`Switch to workspace Game A ${info.project.name}`);
      await page.getByRole('option', { name: new RegExp(`Switch to workspace Game A ${info.project.name}`) }).first().click();
      await expect.poll(() => page.evaluate(() => localStorage.getItem('otto_workspace')), { timeout: 10_000 }).toBe(a);
      await expect(page.getByRole('heading', { name: 'My creations' })).toBeVisible();
    }
    // Its card shows (from the newer answer), the empty state never does, and the held-back older answer changes nothing.
    const cards = page.getByTestId('create-gallery').locator(`li:has([data-production="${id}"])`);
    await expect(cards).toHaveCount(1, { timeout: 20_000 });
    await expect(page.getByText('Nothing made yet')).toHaveCount(0);
    await expect.poll(() => shelfCalls, { timeout: 20_000 }).toBeGreaterThan(before + 1);
    await expect.poll(() => landed, { timeout: 20_000 }).toBe(true);
    await page.waitForTimeout(1000); // ui-guards: allow — gives the held-back older answer time to be (wrongly) applied
    // Read once, no retrying: a later refresh must not hide an older answer having emptied the shelf.
    expect(await cards.count(), 'the older answer that landed last changed nothing').toBe(1);
    expect(await page.getByText('Nothing made yet').count()).toBe(0);
    await evidence(page, info.project.name, 'p20-my-creations-other-workspace');
    // It opens and is made from its own page, whatever workspace is active, and never changes it.
    const activeWs = await page.evaluate(() => localStorage.getItem('otto_workspace'));
    await cards.first().locator('button').first().click();
    await page.getByTestId('create-make').click({ timeout: 30_000 });
    await expect(page.getByRole('heading', { name: 'Making your scene…' })).toBeVisible({ timeout: 15_000 });
    expect(await page.evaluate(() => localStorage.getItem('otto_workspace')), 'opening it never changes the active workspace').toBe(activeWs);
    await page.getByTestId('create-back').click();
    await expect(page.getByRole('heading', { name: 'My creations' })).toBeVisible();
    const state = cards.getByTestId('create-gallery-state');
    await expect(state).toHaveText(/^Making \d+ of \d+$/, { timeout: 15_000 });
    const was = await state.innerText();
    await expect.poll(() => state.innerText(), { message: 'it moves on without a reload', timeout: 120_000 }).not.toBe(was);
    await expect(page.getByText('Nothing made yet')).toHaveCount(0);
    // The Create home lists it too, with B active.
    await page.goto('/#/design/create');
    await expect(page.locator(`[data-testid="create-in-progress-item"][data-production="${id}"]`).getByTestId('create-in-progress-state')).toHaveText(/^Making \d+ of \d+$/);
    await evidence(page, info.project.name, 'p21-home-other-workspace');
  } finally {
    const c = await apiCtx();
    await c.ctx.post(`${c.base}/api/v1/design/productions/${id}/cancel`).catch(() => undefined);
    await c.ctx.dispose();
  }
}
