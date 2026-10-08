import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace, seedDirtyRepo } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });

for (const scheme of ['light', 'dark'] as const) {
  test(`pending score never presents old Approval as current evidence ${scheme}`, async ({ page }, info) => {
    const { ctx, base } = await apiCtx();
    const workspace = await seedWorkspace(ctx, base);
    const repo = await seedDirtyRepo(ctx, base, workspace);
    const post = async (path: string, data: unknown) => {
      const response = await ctx.post(`${base}/api/v1${path}`, { data });
      expect(response.ok(), await response.text()).toBe(true);
      return response.json();
    };
    try {
      const golden = await post(`/workspaces/${workspace}/golden-tasks`, {
        name: 'Round 5 pending evidence', prompt: 'Preserve truthful proof presentation.', skill: 'pending-evidence',
        test_cmd: 'true # cargo test', lint_cmd: 'true # cargo clippy',
      });
      const created = await post(`/workspaces/${workspace}/skill-evaluations`, {
        source: { kind: 'library', reference: '' }, task: '', impl_cli: '', validations: [], iterations: 1,
        mode: 'score_only', golden_task_id: golden.id, target: { kind: 'path', path: repo.dir },
      });
      let run: any;
      await expect.poll(async () => {
        run = await (await ctx.get(`${base}/api/v1/skill-evaluations/${created.id}`)).json();
        return run.status;
      }, { timeout: 30_000 }).not.toBe('running');
      const iteration = run.iterations[0].id;
      run = await post(`/skill-evaluations/${run.id}/iterations/${iteration}/rate`, { rating: 5, note: 'Published rating' });
      let pending: any = null;
      const ratePath = `**/skill-evaluations/${run.id}/iterations/${iteration}/rate`;
      await page.route(ratePath, async route => {
        pending = structuredClone(run);
        pending.best_iteration = null;
        pending.best_score = null;
        Object.assign(pending.iterations[0], { human_rating: 1, score: 0 });
        Object.assign(pending.iterations[0].scoring, { proof_status: 'pending', done_score: 0 });
        Object.assign(pending.iterations[0].scoring.human, { rating: 1, score: 20 });
        await route.fulfill({ status: 503, json: { code: 'unavailable', message: 'Fixture proof publication failed' } });
      });
      await page.route(`**/skill-evaluations/${run.id}`, route => pending ? route.fulfill({ json: pending }) : route.continue());
      await page.addInitScript(({ workspace, scheme }) => {
        localStorage.setItem('otto_workspace', workspace);
        localStorage.setItem('otto_scheme', scheme);
        localStorage.setItem('otto_theme', 'native');
        localStorage.setItem('otto_firstrun_dismissed', '1');
        localStorage.setItem('otto_rail_expanded', '0');
      }, { workspace, scheme });
      await page.setViewportSize({ width: 1440, height: 900 });
      await page.goto('/#/skills-eval');
      await page.getByTestId('tab-evaluator').click();
      await page.getByTestId('scorecard-proofpack-btn').click();
      const approval = page.locator('.scorecard .artifact').filter({ hasText: 'Human rating' });
      await expect(approval).toContainText('rating: 5/5');
      await page.getByRole('button', { name: 'Rate 1 of 5', exact: true }).click();
      await expect(page.locator('.rated')).toHaveText('1/5');
      await expect(page.getByTestId('scorecard-proof')).toContainText('Pending');
      await expect(approval).toHaveCount(0);
      await expect(page.getByTestId('proof-pending-evidence')).toBeVisible();
      await page.screenshot({ path: info.outputPath(`pending-evidence-desktop-${scheme}.png`) });
      await page.setViewportSize({ width: 390, height: 844 });
      await page.getByTestId('proof-pending-evidence').scrollIntoViewIfNeeded();
      await expectNoHorizontalOverflow(page);
      await expectFullyInViewport(page, page.getByTestId('proof-pending-evidence'));
      await page.screenshot({ path: info.outputPath(`pending-evidence-phone-${scheme}.png`) });
      await page.unroute(ratePath);
      pending = null;
      await page.locator('.score-pending').getByRole('button', { name: 'Retry', exact: true }).press('Enter');
      await expect(page.getByTestId('scorecard-proof')).toContainText('Passed');
      await expect(approval).toContainText('rating: 1/5');
      await expect(approval).not.toContainText('rating: 5/5');
      await expect(page.getByTestId('proof-pending-evidence')).toHaveCount(0);
    } finally { await ctx.dispose(); }
  });
}
