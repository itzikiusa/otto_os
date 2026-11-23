// The Create flows, written once and run by desktop-create and mobile-create.
import { expect, type Page, type TestInfo } from '@playwright/test';
import { join } from 'node:path';
import { expectFullyInViewport } from './helpers';
import { apiCtx } from './seed';
import { ProductionMock, OWNER_DESCRIPTION, evidence, expectColourful, expectPlain, openCreate, type CreateSeed } from './create-fixture';

const V1 = '/api/v1';

/** Tap the 3D view until a note box appears (only the on-screen part takes taps). */
export async function tapToPin(page: Page): Promise<void> {
  const canvas = page.locator('.cr-stage canvas').first();
  await expect(canvas).toBeVisible({ timeout: 20_000 });
  await canvas.scrollIntoViewIfNeeded();
  const before = await page.getByTestId('create-pin-note').count();
  const full = (await canvas.boundingBox())!;
  const vh = page.viewportSize()!.height;
  const top = Math.max(full.y, 0);
  const box = { x: full.x, width: full.width, y: top, height: Math.min(full.y + full.height, vh) - top };
  const spots: [number, number][] = [[0.5, 0.5]];
  for (const fy of [0.45, 0.55, 0.35, 0.65, 0.25, 0.75]) for (const fx of [0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8]) spots.push([fx, fy]);
  for (const [fx, fy] of spots) {
    await page.mouse.click(box.x + box.width * fx, box.y + box.height * fy);
    const took = await expect.poll(() => page.getByTestId('create-pin-note').count(), { timeout: 1200 }).toBeGreaterThan(before).then(() => true, () => false);
    if (took) return;
  }
  throw new Error('no place on the model took a pin');
}

/** Wait until the 3D picture has drawn its pieces (the camera has framed them). */
async function settled(page: Page): Promise<void> {
  await expect(page.locator('[data-testid="create-model"] canvas').first()).toBeVisible({ timeout: 30_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
}

/** Smart mode with the owner's own scene: describe → plan → change the plan in words → make → scene → change it → before/after. */
export async function smartFlow(page: Page, seed: CreateSeed, info: TestInfo): Promise<ProductionMock> {
  const shot = (n: string, keepScroll = false) => evidence(page, info.project.name, n, { keepScroll });
  const mock = new ProductionMock(seed);
  await mock.install(page);
  await openCreate(page, seed, '');
  await expect(page.getByRole('heading', { name: 'What do you want to make?' })).toBeVisible();
  // Create is a normal Product tab: the app frame and the mode tabs stay.
  await expect(page.getByRole('tab', { name: 'Create', selected: true })).toBeVisible();
  await expectPlain(page, 'smart home');
  await shot('01-home');
  await page.getByTestId('create-idea').fill(OWNER_DESCRIPTION);
  await page.getByTestId('create-make').click();
  await expect(page.getByTestId('create-planning')).toBeVisible();
  await shot('02-thinking');
  await expect(page.getByRole('heading', { name: 'Here’s my plan' })).toBeVisible({ timeout: 15_000 });
  for (const g of ['People', 'Things', 'Voices']) await expect(page.getByRole('heading', { name: g })).toBeVisible();
  await expect(page.getByRole('switch')).toHaveCount(0);
  await expect(page.getByTestId('create-not-yet')).toContainText('Sounds and music aren’t ready yet');
  await expect(page.getByTestId('create-total')).toContainText('This will take about');
  // The time and "Make it" show without scrolling, the time right above the button.
  await expect(page.getByTestId('create-make')).toBeInViewport({ ratio: 1 });
  await expect(page.getByTestId('create-total')).toBeInViewport({ ratio: 1 });
  const totalY = (await page.getByTestId('create-total').boundingBox())!.y;
  expect((await page.getByTestId('create-make').boundingBox())!.y).toBeGreaterThan(totalY);
  const planTime = (await page.getByTestId('create-total').innerText()).match(/about ([^.]+)\./i)?.[1];
  await expectPlain(page, 'plan');
  await shot('03-plan');
  // Changing the plan is said in words, with the same box as later.
  await page.getByTestId('create-steer-open').click();
  await page.getByTestId('create-steer').fill('Make the coins a bigger pile.');
  await page.getByTestId('create-steer-go').click();
  await expect(page.getByText('I’ll make a bigger pile of coins.')).toBeVisible({ timeout: 15_000 });
  await expect(page.getByText('A big pile of shiny gold coins next to the chest.')).toBeVisible();
  await shot('04-plan-changed');
  await page.getByTestId('create-make').click();
  await expect(page.getByRole('heading', { name: 'Making your scene…' })).toBeVisible();
  await expect(page.getByTestId('create-stop')).toBeVisible();
  await expect(page.getByTestId('create-now')).toBeVisible();
  // One estimate: the making screen starts from the plan's time and only counts down.
  const making = (await page.locator('.cr-sub').filter({ hasText: ' left.' }).first().innerText()).match(/about ([^.]+?) left/i)?.[1];
  const mins = (t?: string) => (!t ? NaN : /an hour/.test(t) ? 60 : Number(t.match(/\d+/)?.[0] ?? NaN));
  expect(mins(making), `making time "${making}" vs plan "${planTime}"`).toBeLessThanOrEqual(mins(planTime));
  expect(mins(making)).toBeGreaterThanOrEqual(mins(planTime) * 0.75);
  await expectPlain(page, 'making');
  await shot('05-making');
  // Leave and come back: the work keeps going and the screen picks it up.
  await page.getByTestId('create-mine').click();
  await expect(page.getByRole('heading', { name: 'My creations' })).toBeVisible();
  await page.goBack();
  await expect(page.getByRole('heading', { name: 'The sword, the monster and the gold' })).toBeVisible({ timeout: 60_000 });
  await settled(page);
  // The key didn't come out the first time: it is named with a reason and can be tried again.
  await expect(page.getByTestId('create-missing-piece').filter({ hasText: 'Key' })).toContainText('didn’t come out right');
  // The picture holds only what was made: no key in it, and none named as in it.
  expect(mock.scene).toBe(seed.scenes.noKey);
  await expect(page.getByTestId('create-piece').filter({ hasText: 'Key' })).toHaveCount(0);
  await expect(page.getByTestId('create-audio').first()).toBeVisible();
  await expectPlain(page, 'scene with a missing piece');
  await shot('06-scene-missing');
  await page.getByTestId('create-retry').click();
  await expect(page.getByRole('heading', { name: 'Making your scene…' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'The sword, the monster and the gold' })).toBeVisible({ timeout: 60_000 });
  await settled(page);
  await expect(page.getByTestId('create-missing-piece')).toHaveCount(0);
  await expect(page.getByText('Here is everything together.')).toBeVisible();
  // A small piece is findable by its name and where it is.
  await expect(page.getByTestId('create-piece').filter({ hasText: 'Key, on the monster’s neck' })).toBeVisible();
  await expectColourful(page, 'whole scene');
  await expectPlain(page, 'scene');
  await shot('06-scene');
  // Play the whole scene: the line is said with its speaker, then it ends and can play again.
  const play = page.getByTestId('create-play-scene');
  await expect(play).toBeEnabled({ timeout: 15_000 });
  await expect(play).toBeInViewport();
  await play.click();
  await expect(play).toContainText('Stop');
  await expect(page.getByTestId('create-play-caption')).toContainText('This sword is mine now.', { timeout: 15_000 });
  await shot('06-scene-playing');
  await expect(play).toContainText('Play again', { timeout: 60_000 });
  await expect(page.getByTestId('create-play-caption')).toHaveText('');
  // Change it: words plus a pin on the scene.
  await page.getByTestId('create-steer').fill('Make the man bigger.');
  await tapToPin(page);
  await page.getByTestId('create-pin-note').last().fill('a stronger look');
  await shot('07-change-it', true);
  await page.getByTestId('create-steer-go').click();
  await expect(page.getByTestId('create-revision')).toContainText('bigger', { timeout: 15_000 });
  await expect(page.getByTestId('create-revision')).toContainText('a stronger look');
  const steer = mock.calls.filter((c) => c.path.endsWith('/steer')).at(-1)!.body as { comment: string; pins: { note: string; point: number[]; item_id?: string }[] };
  expect(steer.comment).toBe('Make the man bigger.');
  expect(steer.pins[0].note).toBe('a stronger look');
  expect(steer.pins[0].point).toHaveLength(3);
  // The words say what will really change: the pinned piece is the one made again.
  const pinned = (mock.items.find((i) => i.id === (steer.pins[0].item_id ?? 'man'))!.name as string).toLowerCase();
  await expect(page.getByTestId('create-revision')).toContainText(new RegExp(`make ${pinned} again`));
  await expectPlain(page, 'what I will change');
  await shot('08-what-i-will-change');
  await page.getByTestId('create-make-changes').click();
  // Each part is said: the size is done, and the piece made again for the note came back the
  // same, with the next step right there. No empty choice.
  const said = page.getByTestId('create-said');
  await expect(said).toContainText('Done: The man is bigger.', { timeout: 60_000 });
  await expect(said).toContainText(`${pinned.charAt(0).toUpperCase()}${pinned.slice(1)}’s new look came out the same. Try saying it differently.`);
  await expect(page.getByRole('heading', { name: 'Which one do you like?' })).toHaveCount(0);
  await settled(page);
  expect(mock.scene).toBe(seed.scenes.bigMan);
  await expectPlain(page, 'changed scene');
  await shot('09-changed');
  await page.getByTestId('create-say-again').click();
  await expect(page.getByTestId('create-steer')).toBeFocused();
  await expect(page.getByTestId('create-steer')).toBeInViewport();
  await shot('09b-say-it-differently', true);
  return mock;
}

/** One character, made for real by the (fake) local engine through the one-button chain. */
export async function characterFlow(page: Page, seed: CreateSeed, info: TestInfo): Promise<string> {
  const shot = (n: string, keepScroll = false) => evidence(page, info.project.name, n, { keepScroll });
  await openCreate(page, seed, 'kinds');
  await expectPlain(page, 'kinds');
  await shot('11-kinds');
  await page.getByTestId('create-kind-character').click();
  await expect(page.getByRole('heading', { name: 'Describe your person' })).toBeVisible();
  await page.getByRole('button', { name: 'A brave knight in blue armor' }).click();
  await expectPlain(page, 'describe');
  await shot('12-describe');
  await page.getByTestId('create-make').click();
  await expect(page.getByRole('heading', { name: 'Making your person…' })).toBeVisible({ timeout: 15_000 });
  await expect(page.getByTestId('create-steps')).toContainText('Giving it joints');
  await expectPlain(page, 'working');
  await shot('13-working');
  const jobUrl = page.url();
  await page.getByTestId('create-mine').click();
  await expect(page.getByText('Still making…').first()).toBeVisible();
  await page.goto(jobUrl);
  await expect(page.getByRole('heading', { name: 'Brave knight in blue armor' })).toBeVisible({ timeout: 90_000 });
  const id = decodeURIComponent(page.url().split('/design/create/a/')[1].split(/[/?]/)[0]);
  await expect(page.getByRole('button', { name: 'Walk', exact: true })).toBeVisible({ timeout: 20_000 });
  await settled(page);
  await expectPlain(page, 'result');
  await shot('14-result');
  await page.getByRole('button', { name: 'Walk', exact: true }).click();
  await page.getByTestId('create-save').click();
  await expect(page.getByText('It’s in your game’s folder.')).toBeVisible();
  // The note goes away by itself before the next step (a pointer resting on it would hold it).
  await page.mouse.move(1, 1);
  await expect(page.getByText('It’s in your game’s folder.')).toBeHidden({ timeout: 20_000 });
  // Change something: the same box, a pin and a note → "Here's what I'll change" → a new version.
  await page.getByTestId('create-act-change').click();
  await expect(page.getByRole('heading', { name: 'Change something' })).toBeVisible();
  await settled(page);
  await tapToPin(page);
  await page.getByTestId('create-pin-note').first().fill('make the armor red');
  // The pin is a whole, numbered marker inside the picture.
  const pin = (await page.getByTestId('create-pin').first().boundingBox())!;
  const stage = (await page.locator('[data-testid="create-model"]').first().boundingBox())!;
  expect(pin.x >= stage.x - 1 && pin.y >= stage.y - 1 && pin.x + pin.width <= stage.x + stage.width + 1 && pin.y + pin.height <= stage.y + stage.height + 1, 'pin fully inside the picture').toBe(true);
  await expectPlain(page, 'change');
  await shot('15-change', true);
  await page.getByTestId('create-steer-go').click();
  await expect(page.getByTestId('create-revision')).toContainText('make the armor red');
  await expectPlain(page, 'what I will change, one thing');
  await shot('16-what-i-will-change-one');
  const { ctx, base } = await apiCtx();
  try {
    const versionsBefore = Number((await ctx.get(`${base}${V1}/design/artifacts/${id}/versions`)).headers()['x-total-count']);
    await page.getByTestId('create-apply').click();
    await expect.poll(async () => ((await (await ctx.get(`${base}${V1}/design/artifacts/${id}/comments`)).json()) as { pin_3d: unknown; body: string }[]).filter((c) => c.pin_3d && c.body === 'make the armor red').length).toBe(1);
    // There is no real red-armoured knight, so the engine gives the same knight back:
    // the same content is no new version, and Create says so instead of offering two equal pictures.
    await expect(page.getByRole('heading', { name: 'It came out the same' })).toBeVisible({ timeout: 90_000 });
    await expect(page.getByTestId('create-same')).toContainText('Try saying it differently.');
    const versionsAfter = Number((await ctx.get(`${base}${V1}/design/artifacts/${id}/versions`)).headers()['x-total-count']);
    expect(versionsAfter, 'the same knight is no new version').toBe(versionsBefore);
    const jobs = (await (await ctx.get(`${base}${V1}/design/3d/jobs?artifact_id=${id}`)).json()) as { prompt: string; then_stages: string[] }[];
    expect(jobs[0].prompt).toContain('Changes: make the armor red');
    expect(jobs[0].then_stages).toEqual(['rig', 'animate']);
  } finally {
    await ctx.dispose();
  }
  await settled(page);
  await expectColourful(page, 'the same one');
  await expectPlain(page, 'came out the same, one thing');
  await shot('17-same-one');
  await page.getByRole('button', { name: 'Keep it like this' }).click();
  await expect(page.getByRole('heading', { name: 'Brave knight in blue armor' })).toBeVisible();
  return id;
}

export async function objectFlow(page: Page, seed: CreateSeed): Promise<void> {
  await openCreate(page, seed, 'new/object');
  await expect(page.getByRole('heading', { name: 'Describe your thing' })).toBeVisible();
  await page.getByTestId('create-idea').fill('A treasure chest');
  await page.getByTestId('create-make').click();
  await expect(page.getByTestId('create-steps')).toContainText('Finishing it');
  await expect(page.getByTestId('create-steps')).not.toContainText('Giving it joints');
  await expect(page.getByRole('heading', { name: 'Treasure chest' })).toBeVisible({ timeout: 90_000 });
  await expect(page.getByTestId('create-act-voice')).toHaveCount(0);
  // The Forge chest (with its collision hulls and lower details inside the file) draws as wood, not white shards.
  await expectColourful(page, 'Forge chest');
  await expectPlain(page, 'thing result');
}

export async function voiceFlow(page: Page, seed: CreateSeed, info: TestInfo): Promise<void> {
  await openCreate(page, seed, `voice/${seed.man}`);
  await expect(page.getByRole('heading', { name: 'Who is talking?' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'The man', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await page.getByRole('radio', { name: /Old storyteller/ }).click();
  await page.getByTestId('create-lines').fill('I found the sword.\nNow I will open the chest.');
  await expectPlain(page, 'voice');
  await evidence(page, info.project.name, '18-voice');
  const before = await page.getByTestId('create-bubble').count();
  await page.getByTestId('create-talk').click();
  await expect(page.getByTestId('create-bubble')).toHaveCount(before + 2, { timeout: 20_000 });
  await expect(page.getByText('Now I will open the chest.').first()).toBeVisible();
  await expect(page.getByRole('button', { name: 'Say it again' }).first()).toBeVisible();
  await expectPlain(page, 'voice made');
}

export async function soundAndKit(page: Page, seed: CreateSeed, info: TestInfo): Promise<void> {
  await openCreate(page, seed, 'sound');
  const waiting = page.getByTestId('create-sound-waiting');
  const form = page.getByTestId('create-sound-words');
  await expect(waiting.or(form)).toBeVisible({ timeout: 20_000 });
  if (await waiting.isVisible()) {
    await expect(page.getByText('Sounds aren’t ready yet')).toBeVisible();
    await expectPlain(page, 'sound');
    await evidence(page, info.project.name, '19-sound');
  } else {
    // The real sound maker is there: make one sound and one piece of music, for real.
    info.setTimeout(info.timeout + 1_200_000);
    await expectPlain(page, 'sound');
    await evidence(page, info.project.name, '19-sound');
    const before = await page.getByTestId('create-sound-clip').count();
    await page.getByRole('button', { name: 'A windy cave' }).click();
    await page.getByRole('button', { name: 'Footsteps on stones' }).click();
    await page.getByTestId('create-make').click();
    await expect(page.getByTestId('create-sound-making')).toBeVisible({ timeout: 20_000 });
    await evidence(page, info.project.name, '19b-sound-making');
    await expect(page.getByTestId('create-sound-clip')).toHaveCount(before + 1, { timeout: 600_000 });
    await page.getByRole('button', { name: 'Music' }).click();
    await page.getByTestId('create-sound-words').fill('Slow, happy music with a flute');
    await page.getByTestId('create-make').click();
    await expect(page.getByTestId('create-sound-clip')).toHaveCount(before + 2, { timeout: 600_000 });
    await expectPlain(page, 'sounds made');
    await evidence(page, info.project.name, '19c-sounds-made');
  }
  await page.route('**/api/v1/design/3d/providers', async (route) => {
    const r = await route.fetch();
    const list = ((await r.json()) as { id: string; ready: boolean }[]).map((p) => ({ ...p, ready: false }));
    await route.fulfill({ response: r, json: list });
  });
  await openCreate(page, seed, '');
  await page.reload();
  await expect(page.getByTestId('create-kit-missing')).toBeVisible();
  await expect(page.getByTestId('create-kit-missing').getByRole('button', { name: 'Go back' })).toBeVisible();
  await expectPlain(page, 'maker missing');
  await evidence(page, info.project.name, '20-maker-missing');
}

/** Smart mode on the REAL productions server and executor: the isolated E2E
 *  daemon plans from its fixed file (the owner's scene) and makes the pieces
 *  through the fake local engine, which hands back only real Forge results
 *  (the knight, the chest) and says it couldn't make the rest. */
export async function smartRealServer(page: Page, seed: CreateSeed, info: TestInfo): Promise<void> {
  const shot = (n: string) => evidence(page, info.project.name, n);
  await openCreate(page, seed, '');
  await page.getByTestId('create-idea').fill(OWNER_DESCRIPTION);
  const created = page.waitForResponse((r) => r.url().endsWith('/api/v1/design/productions') && r.request().method() === 'POST');
  await page.getByTestId('create-make').click();
  const resp = await created;
  expect(resp.status(), 'the real server takes the description').toBe(201);
  const { id } = (await resp.json()) as { id: string };
  await expect(page.getByRole('heading', { name: 'Here’s my plan' })).toBeVisible({ timeout: 30_000 });
  await expect(page.getByTestId('create-not-yet')).toBeVisible();
  await expectPlain(page, 'real plan');
  await shot('21-real-plan');
  await page.getByTestId('create-make').click();
  await expect(page.getByRole('heading', { name: 'Making your scene…' })).toBeVisible();
  await expectPlain(page, 'real making');
  await shot('22-real-making');
  await expect(page.getByRole('heading', { name: 'The sword, the monster and the gold' })).toBeVisible({ timeout: 280_000 });
  await settled(page);
  await expect(page.getByTestId('create-missing-piece')).toHaveCount(0);
  await expectColourful(page, 'real scene');
  // The small pieces are named where they really are: the coins in the chest (the server puts
  // a piece "inside" on its container's floor).
  // The stand-in monster has no neck bone, so the key hangs at its front (and says so).
  await expect(page.getByTestId('create-piece').filter({ hasText: 'Key, at the front of the monster' })).toBeVisible();
  await expect(page.getByTestId('create-piece').filter({ hasText: 'Gold coins, in the treasure chest' })).toBeVisible();
  await expectPlain(page, 'real scene');
  await shot('23-real-scene');  const { ctx, base } = await apiCtx();
  try {
    const p = (await (await ctx.get(`${base}${V1}/design/productions/${id}`)).json()) as { status: string; scene_artifact_id?: string; plan: { items: { id: string; status: string; result?: { artifact_id?: string; clip_id?: string } }[] } };
    expect(p.status).toBe('done');
    expect(p.scene_artifact_id, 'the server assembled one scene').toBeTruthy();
    expect(p.plan.items.find((i) => i.id === 'man')!.result?.artifact_id).toBeTruthy();
    for (const id of ['monster', 'sword', 'chest', 'key', 'coins']) expect(p.plan.items.find((i) => i.id === id)!.result?.artifact_id, `${id} made`).toBeTruthy();
    expect(p.plan.items.find((i) => i.id === 'line1')!.result?.clip_id).toBeTruthy();
  } finally {
    await ctx.dispose();
  }
  // "Make it bigger" is a size: the server re-plans and shows the bigger monster at once, nothing made again.
  const bodyOf = async () => {
    const { ctx: c, base: b } = await apiCtx();
    try {
      return (await (await c.get(`${b}${V1}/design/productions/${id}`)).json()) as { status: string; scene_artifact_id: string; plan: { items: { id: string; result?: { version_id?: string }; previous_result?: unknown }[] } };
    } finally {
      await c.dispose();
    }
  };
  const monster = (p: Awaited<ReturnType<typeof bodyOf>>) => p.plan.items.find((i) => i.id === 'monster')!;
  const first = await bodyOf();
  await page.getByTestId('create-steer').fill('Make the monster bigger.');
  await page.getByTestId('create-steer-go').click();
  await expect(page.getByTestId('create-said')).toContainText('Done: The monster is bigger.', { timeout: 30_000 });
  await settled(page);
  const bigger = await bodyOf();
  expect(bigger.status).toBe('done');
  expect(monster(bigger).result, 'the monster was not made again').toEqual(monster(first).result);
  expect(monster(bigger).previous_result).toBeUndefined();
  await expectColourful(page, 'bigger monster');
  await expectPlain(page, 'real scene, bigger');
  await shot('24-real-bigger');
  // A look: "older and tired" makes the man again with the real engine's new words, and the
  // real result differs (the fixture's real before/after pair), so both are shown to choose from.
  const man = (p: Awaited<ReturnType<typeof bodyOf>>) => p.plan.items.find((i) => i.id === 'man')!;
  await page.getByTestId('create-steer').fill('The man should look older and tired.');
  await page.getByTestId('create-steer-go').click();
  await expect(page.getByTestId('create-revision')).toContainText('older', { timeout: 30_000 });
  await expectPlain(page, 'real what I will change');
  await shot('25-real-what-i-will-change');
  await page.getByTestId('create-make-changes').click();
  await expect(page.getByRole('heading', { name: 'Which one do you like?' })).toBeVisible({ timeout: 300_000 });
  await expect(page.getByTestId('create-compare')).toHaveCount(1);
  await expect(page.getByTestId('create-compare')).toContainText('The man');
  const older = await bodyOf();
  expect(man(older).result, 'a new man').not.toEqual(man(older).previous_result);
  expect(monster(older).result, 'the monster was not made again').toEqual(monster(first).result);
  await expect(page.locator('[data-testid="create-compare"] canvas')).toHaveCount(2, { timeout: 30_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  await expectPlain(page, 'real before and after');
  await shot('26-real-before-after');
  await page.getByTestId('create-keep-new').click();
  await expect(page.getByRole('heading', { name: 'The sword, the monster and the gold' })).toBeVisible({ timeout: 30_000 });
  await settled(page);
  await expectPlain(page, 'real scene, older man');
  await shot('27-real-older');
  // The scene's picture is kept for My creations while it is looked at, and its card on the
  // server's shelf shows it (review v4: the card had lost its picture).
  const { ctx: c, base: b } = await apiCtx();
  try {
    const scene = (await bodyOf()).scene_artifact_id;
    const thumb = async () => ((await (await c.get(`${b}${V1}/design/artifacts/${scene}`)).json()) as { artifact: { thumb_blob?: string | null } }).artifact.thumb_blob ?? null;
    await expect.poll(thumb, { timeout: 20_000 }).toBeTruthy();
  } finally {
    await c.dispose();
  }
  await page.getByTestId('create-mine').click();
  await expect(page.getByTestId('create-gallery').locator(`[data-production="${id}"] img`)).toBeVisible({ timeout: 20_000 });
  await shot('10-my-creations');
}

/** Who plans it: the chip beside "Use a picture" names the default, the
 *  shared model panel picks another model, the production is created with it
 *  and the daemon stores it, and the pick is back on the next visit. The chip
 *  never pushes "Make it" off a phone screen. Closed and open, light and dark,
 *  into OTTO_CREATE_EVIDENCE. */
export async function agentPickFlow(page: Page, seed: CreateSeed, info: TestInfo): Promise<void> {
  const shot = async (name: string) => {
    const dir = process.env.OTTO_CREATE_EVIDENCE;
    const theme = await page.evaluate(() => localStorage.getItem('otto_scheme') ?? 'light');
    if (dir) await page.screenshot({ path: join(dir, `${info.project.name}-${theme}-${name}.png`), animations: 'disabled' });
  };
  await openCreate(page, seed, '', { theme: 'light' });
  const chip = page.getByTestId('create-agent');
  const panel = page.getByTestId('model-picker');
  await expect(chip).toContainText('· Default');
  const picture = page.getByRole('button', { name: 'Use a picture' });
  // Polled: the project picker above the box arrives with the creations list and moves both.
  await expect.poll(async () => Math.abs((await chip.boundingBox())!.y - (await picture.boundingBox())!.y), { message: 'one row with Use a picture' }).toBeLessThan(4);
  await expect(page.getByTestId('create-make')).toBeInViewport({ ratio: 1 });
  await shot('agent-closed');
  await chip.click();
  await expect(panel).toBeVisible();
  await expectFullyInViewport(page, panel, 'model panel');
  await shot('agent-open');
  // A model that is not the default, of the first agent the panel can start.
  const models = panel.locator('.mp-row:enabled', { hasNotText: 'Default model' });
  if ((await models.count()) === 0) await panel.locator('.mp-prov:enabled').nth(1).click();
  await models.first().click();
  // Effort is offered the way Threads offers it, in the same panel: pick one, and the chip says it in plain words.
  const effortRow = panel.getByTestId('picker-effort');
  let effort: string | null = null;
  if (await effortRow.isVisible()) {
    const lvl = effortRow.getByRole('button', { name: 'High', exact: true });
    if (await lvl.count()) {
      await lvl.click();
      effort = 'high';
    }
  }
  await shot('agent-effort');
  await page.keyboard.press('Escape');
  await expect(panel).toBeHidden();
  const label = (await chip.innerText()).trim();
  expect(label).not.toContain('· Default');
  if (effort) expect(label).toMatch(/ · High$/);
  await page.getByTestId('create-idea').fill('A small grey rock.');
  const created = page.waitForResponse((r) => r.url().endsWith(`${V1}/design/productions`) && r.request().method() === 'POST');
  await page.getByTestId('create-make').click();
  const resp = await created;
  expect(resp.status(), 'the daemon takes the pick').toBe(201);
  const sent = resp.request().postDataJSON() as { provider?: string; model?: string; effort?: string };
  expect(sent.provider && sent.model, `the request carries the pick: ${JSON.stringify(sent)}`).toBeTruthy();
  expect(sent.effort ?? null, 'the request carries the effort picked').toBe(effort);
  const { id } = (await resp.json()) as { id: string };
  const { ctx, base } = await apiCtx();
  try {
    const stored = (await (await ctx.get(`${base}${V1}/design/productions/${id}`)).json()) as { provider?: string; model?: string; effort?: string };
    expect({ provider: stored.provider, model: stored.model, effort: stored.effort ?? null }).toEqual({ provider: sent.provider, model: sent.model, effort });
  } finally {
    await ctx.dispose();
  }
  // Remembered for this workspace, and the same chip in the dark.
  await page.evaluate(() => localStorage.setItem('otto_scheme', 'dark'));
  await page.goto('/#/design/create');
  await expect(chip).toHaveText(label);
  await shot('agent-closed');
  await chip.click();
  await expect(panel).toBeVisible();
  await expectFullyInViewport(page, panel, 'model panel');
  await shot('agent-open');
}
