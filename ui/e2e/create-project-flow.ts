// A PROJECT on the real productions server (the isolated E2E daemon's fixed
// planner; the fake engine makes stand-in pieces): a first scene is made, then
// "New scene in this project" plans the next one with every piece reused (the
// fixed planner reuses what the project offers, as the real one is told to),
// the plan says so and a piece can be switched to a new one and back, and Make
// finishes at once with nothing built. My creations shows the two together.
import { expect, type Page, type TestInfo } from '@playwright/test';
import { apiCtx } from './seed';
import { evidence, openCreate, type CreateSeed } from './create-fixture';

const V1 = '/api/v1';
type Prod = { id: string; status: string; project_id: string; plan: { items: { id: string; kind: string; details: { reuse?: { artifact_id: string } }; result?: { artifact_id?: string } }[] } };

async function production(id: string): Promise<Prod> {
  const { ctx, base } = await apiCtx();
  try {
    return (await (await ctx.get(`${base}${V1}/design/productions/${id}`)).json()) as Prod;
  } finally {
    await ctx.dispose();
  }
}

async function start(page: Page, description: string): Promise<string> {
  await page.getByTestId('create-idea').fill(description);
  const created = page.waitForResponse((r) => r.url().endsWith('/api/v1/design/productions') && r.request().method() === 'POST');
  await page.getByTestId('create-make').click();
  const resp = await created;
  expect(resp.status()).toBe(201);
  await expect(page.getByRole('heading', { name: 'Here’s my plan' })).toBeVisible({ timeout: 30_000 });
  return ((await resp.json()) as { id: string }).id;
}

export async function projectFlow(page: Page, seed: CreateSeed, info: TestInfo): Promise<void> {
  const shot = (n: string) => evidence(page, info.project.name, n);
  await openCreate(page, seed, '');
  const first = await start(page, 'A tired man next to a chest of gold.');
  await expect(page.getByTestId('create-reused'), 'a new project reuses nothing').toHaveCount(0);
  await page.getByTestId('create-make').click();
  await expect(page.getByRole('heading', { name: 'The man and the chest' })).toBeVisible({ timeout: 280_000 });
  const made = await production(first);
  expect(made.project_id, 'a creation started without a project is its own').toBe(first);

  // The next scene of the project: its people, things and place are offered.
  await page.getByTestId('create-new-scene').click();
  await expect(page.getByTestId('create-project')).toHaveValue(first);
  await expect(page.getByTestId('create-project-sub')).toContainText('“The man and the chest”');
  await shot('41-project-new-scene');
  const second = await start(page, 'Later that day the man counts the coins.');
  const reused = page.getByTestId('create-reused');
  await expect(reused).toContainText('3 of 3 pieces reused');
  await expect(page.getByTestId('create-item-from')).toHaveCount(3);
  await expect(page.getByTestId('create-item-from').first()).toHaveText('From “The man and the chest”');
  await shot('42-project-plan');
  // A piece switched to a new one, then back to the project's.
  const man = page.getByTestId('create-plan-item').filter({ hasText: 'The man' }).first();
  await man.getByTestId('create-item-switch').click();
  await expect(reused).toContainText('2 of 3 pieces reused');
  await expect(man.getByTestId('create-item-switch')).toHaveText('Use one from this project');
  await man.getByTestId('create-item-switch').click();
  await page.getByRole('button', { name: 'The man (from “The man and the chest”)' }).click();
  await expect(reused).toContainText('3 of 3 pieces reused');
  const t = Date.now();
  await page.getByTestId('create-make').click();
  await expect(page.getByRole('heading', { name: 'Later that day' })).toBeVisible({ timeout: 30_000 });
  expect(Date.now() - t, 'nothing is built: it is done at once').toBeLessThan(20_000);
  await expect(page.locator('[data-testid="create-model"] canvas').first()).toBeVisible({ timeout: 30_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  await shot('43-project-scene');
  const p = await production(second);
  expect(p.status).toBe('done');
  expect(p.project_id).toBe(first);
  const was = new Set(made.plan.items.map((i) => i.result?.artifact_id).filter(Boolean));
  const models = p.plan.items.filter((i) => i.kind === 'character' || i.kind === 'object');
  expect(models.length).toBe(3);
  for (const i of models) expect(was.has(i.result?.artifact_id), `${i.id} is the project's own model`).toBe(true);

  // My creations: the two scenes of the project stand together, after its name.
  await page.getByTestId('create-mine').click();
  const label = page.getByTestId('create-project-label').filter({ hasText: 'The man and the chest' });
  await expect(label).toBeVisible({ timeout: 20_000 });
  await shot('44-project-creations');
}
