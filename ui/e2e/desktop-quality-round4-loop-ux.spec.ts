import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';

test.use({ serviceWorkers: 'block' });
const criterion = { id: 'human-check', text: 'Inspect the report', verify: 'Read the generated report', verify_kind: 'human', verify_cmd: null };
function loop(id: string, workspace: string) {
  return { id, name: `Report ${id}`, workspace_id: workspace, status: 'blocked', phase: 'done', current_iteration: 1,
    iterations_started: 1, progress_pct: 50, elapsed_secs: 20, run_started_at: null,
    definition: { title: `Report ${id}`, summary: `Verify the independently generated report ${id}`, acceptance_criteria: [criterion] },
    config: { executors: [{ name: 'Executor', provider: 'codex', model: '' }], mode: 'build', allow_commits: false },
    limits: { max_iterations: 3, max_runtime_secs: 300 },
    ledger: { questions: [{ id: 'format', question: `Which format for ${id}?`, answer: null, answered_by: null }],
      verifications: [], next_action: 'Review the report', review_summary: '', review_passed: false } };
}
async function fixture(page: Page, scheme: string) {
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript(({ workspace, scheme }) => {
    localStorage.setItem('otto_workspace', workspace);
    localStorage.setItem('otto_scheme', scheme);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_rail_expanded', '0');
  }, { workspace, scheme });
  const loops = [loop('alpha', workspace), loop('beta', workspace)];
  await page.route(`**/workspaces/${workspace}/goal-loops`, route => route.fulfill({ json: loops }));
  await page.route('**/goal-loops/*?summary=true', route => {
    const id = new URL(route.request().url()).pathname.split('/').at(-1);
    return route.fulfill({ json: { loop: loops.find(item => item.id === id), iterations: [] } });
  });
  return loops;
}
for (const scheme of ['light', 'dark']) {
  test(`loop navigation isolates answer and verification drafts ${scheme}`, async ({ page }, info) => {
    await fixture(page, scheme);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto('/#/loops/alpha');
    await expect(page.getByRole('heading', { name: 'Report alpha', exact: true })).toBeVisible();
    await page.getByLabel('Answer question').fill('Alpha must use Markdown');
    await page.getByLabel('Evidence for human-check').fill('I inspected alpha only');
    await page.evaluate(() => { location.hash = '#/loops/beta'; });
    await expect(page.getByRole('heading', { name: 'Report beta', exact: true })).toBeVisible();
    await expect(page.getByLabel('Answer question')).toHaveValue('');
    await expect(page.getByLabel('Evidence for human-check')).toHaveValue('');
    await expect(page.getByRole('button', { name: 'Record answer', exact: true })).toBeDisabled();
    await expect(page.getByRole('button', { name: 'Record verification', exact: true })).toBeDisabled();
    await page.screenshot({ path: info.outputPath(`loop-desktop-${scheme}.png`) });
    await page.setViewportSize({ width: 390, height: 844 });
    await expectNoHorizontalOverflow(page);
    const answer = page.getByLabel('Answer question');
    await answer.scrollIntoViewIfNeeded();
    await expectFullyInViewport(page, answer);
    await answer.fill('Beta uses plain text');
    await expect(page.getByRole('button', { name: 'Record answer', exact: true })).toBeEnabled();
    await page.screenshot({ path: info.outputPath(`loop-phone-${scheme}.png`) });
    await page.evaluate(() => { location.hash = '#/loops/alpha'; });
    await expect(page.getByRole('heading', { name: 'Report alpha', exact: true })).toBeVisible();
    await expect(page.getByLabel('Answer question')).toHaveValue('');
    await expect(page.getByLabel('Evidence for human-check')).toHaveValue('');
  });
}

test('direct loop navigation closes the previous budget dialog', async ({ page }) => {
  await fixture(page, 'light');
  await page.goto('/#/loops/alpha');
  await page.getByRole('button', { name: 'Extend budget…', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Extend budget' })).toBeVisible();
  await page.getByLabel('Max iterations').fill('12');
  await page.evaluate(() => { location.hash = '#/loops/beta'; });
  await expect(page.getByRole('heading', { name: 'Report beta', exact: true })).toBeVisible();
  await expect(page.getByRole('dialog', { name: 'Extend budget' })).toHaveCount(0);
  await page.getByRole('button', { name: 'Extend budget…', exact: true }).click();
  await expect(page.getByLabel('Max iterations')).not.toHaveValue('12');
  await page.getByRole('button', { name: 'Cancel', exact: true }).press('Enter');
  await expect(page.getByRole('button', { name: 'Extend budget…', exact: true })).toBeFocused();
});

test('a failed goal detail offers inline keyboard recovery', async ({ page }) => {
  const loops = await fixture(page, 'dark');
  let fail = true;
  await page.route('**/goal-loops/alpha?summary=true', route => fail
    ? route.fulfill({ status: 503, json: { code: 'unavailable', message: 'Fixture detail unavailable' } })
    : route.fulfill({ json: { loop: loops[0], iterations: [] } }));
  await page.goto('/#/loops/alpha');
  await expect(page.getByTestId('load-error')).toBeVisible();
  fail = false;
  await page.getByRole('button', { name: 'Retry', exact: true }).press('Enter');
  await expect(page.getByRole('heading', { name: 'Report alpha', exact: true })).toBeVisible();
  await expect(page.getByLabel('Answer question')).toBeVisible();
});


test('a late delete for the previous loop cannot close the current detail', async ({ page }) => {
  await fixture(page, 'light');
  let release!: () => void;
  const held = new Promise<void>(resolve => { release = resolve; });
  let deleting = false;
  await page.route('**/goal-loops/alpha', async route => {
    expect(route.request().method()).toBe('DELETE');
    deleting = true;
    await held;
    await route.fulfill({ status: 204 });
  });
  try {
    await page.goto('/#/loops/alpha');
    await page.getByRole('button', { name: 'More actions', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Delete loop…', exact: true }).click();
    await page.getByRole('dialog', { name: 'Delete goal loop' }).getByRole('button', { name: 'Delete', exact: true }).click();
    await expect.poll(() => deleting).toBe(true);
    await page.evaluate(() => { location.hash = '#/loops/beta'; });
    await expect(page.getByRole('heading', { name: 'Report beta', exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Extend budget…', exact: true }).click();
    await page.getByLabel('Max iterations').fill('17');
    const response = page.waitForResponse(r => r.request().method() === 'DELETE' && r.url().endsWith('/goal-loops/alpha'));
    release();
    await response;
    await expect(page.getByText('Goal loop deleted', { exact: true })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Report beta', exact: true })).toBeVisible();
    await expect(page.getByRole('dialog', { name: 'Extend budget' })).toBeVisible();
    await expect(page.getByLabel('Max iterations')).toHaveValue('17');
    await expect(page).toHaveURL(/#\/loops\/beta$/);
  } finally { release(); }
});
