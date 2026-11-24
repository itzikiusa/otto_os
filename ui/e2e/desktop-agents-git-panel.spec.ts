import { test, expect } from '@playwright/test';
import { apiCtx } from './seed';
import { expectFullyInViewport } from './helpers';
import { remoteSubject, seedCardRepo, stubForge, type CardRepo } from './git-commit-card-fixture';

// The Agents right panel's Git tab is the Changes view with the commit card
// on top (P5): a session working in a dirty checkout opens the panel and
// commits and pushes from the card (a local bare remote — nothing leaves the
// machine); "Open in Git" goes to the full page; nothing git is read while
// the panel is closed.

test.use({ serviceWorkers: 'block' });
const SHOTS = process.env.OTTO_E2E_SHOTS;
let repo: CardRepo;
let wsId = '';
let sid = '';

test.beforeAll(async () => {
  repo = await seedCardRepo('e2e-agents-git-panel');
  const { ctx, base } = await apiCtx();
  const w = await ctx.post(`${base}/api/v1/workspaces`, { data: { name: 'Git Panel WS', root_path: repo.dir } });
  expect(w.ok(), await w.text()).toBeTruthy();
  wsId = (await w.json()).id as string;
  const s = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, { data: { kind: 'agent', provider: 'shell', title: 'Rivera', cwd: repo.dir, meta: { origin: 'e2e' } } });
  expect(s.ok(), await s.text()).toBeTruthy();
  sid = (await s.json()).id as string;
  await ctx.dispose();
});

test('a dirty session folder: the Git tab is Changes + the commit card; Commit and push from it', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.setViewportSize({ width: 1440, height: 900 });
  await stubForge(page);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_firstrun_dismissed', '1');
    localStorage.setItem('otto_nav_all_ws', '0');
  }, wsId);
  const gitReads: string[] = [];
  page.on('request', (r) => { if (/\/api\/v1\/repos\/[^/]+\/(status|log|diff)/.test(r.url())) gitReads.push(r.url()); });
  await page.goto(`/#/agents/${sid}`);
  await expect(page.locator(`[data-pane-key][data-session="${sid}"]`)).toBeVisible({ timeout: 30_000 });
  await page.waitForLoadState('networkidle');
  expect(gitReads, 'no git reads while the panel is closed').toEqual([]);

  await page.locator('.rstrip').getByRole('button', { name: 'Git', exact: true }).click();
  const card = page.getByTestId('commit-card');
  await expect(card).toBeVisible({ timeout: 20_000 });
  await expectFullyInViewport(page, card, 'commit card');
  await expect(card.getByTestId('commit-card-included')).toHaveText('2 files included');
  await expect(card.locator('.subject-input')).toHaveValue('feat: ship the commit card', { timeout: 15_000 });
  await expect(card.locator('.btn.primary')).toHaveCount(1);
  // The whole Git page is gone from the panel: no graph / PR tab set.
  await expect(page.locator('.rv-tab')).toHaveCount(0);
  if (SHOTS) for (const s of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: s });
    await expect(page.locator('html')).toHaveAttribute('data-scheme', s);
    await page.screenshot({ path: `${SHOTS}/agents-git-panel-1440-${s}.png`, animations: 'disabled' });
  }
  await page.emulateMedia({ colorScheme: 'light' });

  // A file's diff on click.
  await page.locator('.wip-panel').getByText('app.txt', { exact: true }).click();
  await expect(page.locator('.wip-panel .wp-diff')).toBeVisible({ timeout: 15_000 });
  await card.locator('.wc-commit').click();
  await expect.poll(() => remoteSubject(repo), { timeout: 20_000 }).toBe('feat: ship the commit card');
  await expect(page.getByTestId('commit-card-primary')).toHaveAttribute('data-kind', 'create-pr', { timeout: 15_000 });

  await page.getByTestId('git-panel-open').click();
  await expect.poll(() => page.evaluate(() => location.hash)).toMatch(/^#\/git\//);
});
