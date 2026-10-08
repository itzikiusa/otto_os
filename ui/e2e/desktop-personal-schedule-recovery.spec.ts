import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

test.use({ serviceWorkers: 'block' });
test('schedule refresh failure retains cadence and Retry recovers in light, dark and phone layouts', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const workspace = await seedWorkspace(ctx, base); await ctx.dispose();
  await page.addInitScript(id => { localStorage.setItem('otto_workspace', id); localStorage.setItem('otto_scheme', 'light'); }, workspace);
  const agent = { id: 'r09-scout', name: 'Review Scout', workspace_id: workspace, avatar: '', soul_md: '', provider: 'claude', model: '', cwd: '', browser: false, delivery: { type: 'none' }, enabled: true, created_at: '2026-10-08T00:00:00Z', updated_at: '2026-10-08T00:00:00Z' };
  let fail = false;
  await page.route(`**/api/v1/workspaces/${workspace}/personal-agents`, r => r.fulfill({ json: [agent] }));
  await page.route('**/api/v1/personal-agents/r09-scout/autonomy', r => r.fulfill({ json: { primary: false, proactive: { enabled: false }, goals: [], rules: [] } }));
  await page.route('**/api/v1/personal-agents/r09-scout/schedules', r => r.fulfill(fail
    ? { status: 503, json: { code: 'upstream', message: 'Fixture schedule service unavailable' } }
    : { json: [{ id: 'r09-daily', agent_id: agent.id, schedule: { cadence: 'daily', at: '09:00' }, timezone: 'UTC', directive: 'Daily fixture recap', enabled: true, permission: 'read_only', last_run_at: null, next_run_at: '2026-10-09T09:00:00Z' }] }));
  await page.goto('/#/personal-agents/r09-scout/schedules');
  await expect(page.getByText('Daily fixture recap', { exact: true })).toBeVisible();
  fail = true;
  await page.evaluate(async () => {
    const path = '/src/lib/stores/personalAgents.svelte.ts';
    await (await import(path)).personalAgents.loadSchedules('r09-scout');
  });
  await expect(page.getByTestId('load-stale')).toContainText('Couldn’t refresh this agent’s schedules');
  await expect(page.getByText('Daily fixture recap', { exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'No schedules', exact: true })).toHaveCount(0);
  for (const [scheme, width] of [['light', 1440], ['dark', 1440], ['dark', 390]] as const) {
    await page.setViewportSize({ width, height: 900 });
    await page.evaluate(s => { document.documentElement.dataset.scheme = s; }, scheme);
    await expect(page.getByTestId('load-stale').getByRole('button', { name: 'Retry' })).toBeVisible();
    await page.screenshot({ path: `../docs/reviews/quality-20261008/evidence/R09-schedules-${scheme}-${width}.png` });
  }
  fail = false;
  await page.getByTestId('load-stale').getByRole('button', { name: 'Retry' }).click();
  await expect(page.getByTestId('load-stale')).toHaveCount(0);
  await expect(page.getByText('Daily fixture recap', { exact: true })).toBeVisible();
});
