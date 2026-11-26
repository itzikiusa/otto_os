// Older goal loops: read-only, Continue only where allowed (goal.md §5.2-5.3,
// test-plan.md GL21.3). Continue opens the Goal form on Workflows, filled from
// the loop. Needs a first real run: the Workflows page must mount GoalHost.
import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';

function loop(ws: string, id: string, status: string) {
  return {
    id, workspace_id: ws, name: `Loop ${id}`, repo_path: '/src/app', status, phase: 'done', current_iteration: 2, iterations_started: 2, progress_pct: 40,
    elapsed_secs: 1800, cost_usd: 0, context_digest: '', worktree_path: `/data/goal-loops/${id}/work`, created_by: 'u', created_at: '2026-11-01T09:00:00Z', updated_at: '2026-11-01T10:00:00Z',
    definition: { title: `Loop ${id}`, summary: 'Keep the nightly build green', objectives: [], constraints: [], out_of_scope: [], success_signal: '',
      acceptance_criteria: [{ id: 'build', text: 'The build passes', verify: 'exit 0', verify_kind: 'command', verify_cmd: 'make build' }] },
    limits: { max_iterations: 6, max_runtime_secs: 7200, per_phase_timeout_secs: 900, max_attempts_per_executor: 3 },
    config: { mode: 'build', executors: [{ name: 'Executor', provider: 'codex', model: '', prompt_extra: '' }], planner: {}, evaluator: { provider: 'claude', model: '', prompt: '' }, digester: {}, definer: {} },
    ledger: { questions: [], verifications: [], next_action: '', last_failure_signature: '', repeated_failures: 0, review_summary: '', review_passed: false },
  };
}

test('the history is read-only and Continue opens a filled Goal form', async ({ page }) => {
  const { ctx, base } = await apiCtx();
  const ws = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript((ws) => localStorage.setItem('otto_workspace', ws), ws);
  const paused = loop(ws, 'P', 'paused');
  const done = loop(ws, 'S', 'succeeded');
  const writes: string[] = [];
  await page.route('**/api/v1/**', async (route) => {
    const req = route.request();
    const path = new URL(req.url()).pathname.replace('/api/v1', '');
    if (req.method() !== 'GET' && path.includes('goal')) { writes.push(path); return route.fulfill({ status: 500, json: {} }); }
    if (path === `/workspaces/${ws}/goal-loops`) return route.fulfill({ json: [paused, done] });
    if (path === '/goal-loops/P') return route.fulfill({ json: { loop: paused, iterations: [] } });
    if (path === '/goal-loops/S') return route.fulfill({ json: { loop: done, iterations: [] } });
    if (path === '/goal-loops/S/diff' || path === '/goal-loops/P/diff') return route.fulfill({ json: { loop_id: 'P', available: false, branch: null, stat: '', patch: '', truncated: false, untracked: [] } });
    if (path === `/workspaces/${ws}/repos`) return route.fulfill({ json: [{ id: 'r1', workspace_id: ws, name: 'app', path: '/src/app', remote_url: null, provider: null, git_account_id: null, created_at: '' }] });
    return route.fallback();
  });
  await page.goto('/#/loops');
  await expect(page.getByRole('heading', { name: 'Older goal loops' })).toBeVisible();
  await expect(page.locator('.loop-card')).toHaveCount(2);
  // The sidebar has no Goal Loops entry any more.
  await expect(page.getByRole('button', { name: 'Goal Loops', exact: true })).toHaveCount(0);

  await page.goto('/#/loops/S');
  await expect(page.getByRole('heading', { name: 'Loop S', exact: true })).toBeVisible();
  await expect(page.getByTestId('loop-continue')).toHaveCount(0);

  await page.goto('/#/loops/P');
  await expect(page.getByRole('heading', { name: 'Loop P', exact: true })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.getByTestId('loop-continue').click();
  await expect(page).toHaveURL(/#\/workflows/);
  await expect(page.getByTestId('wf-goal-form')).toBeVisible();
  await expect(page.getByTestId('wf-goal-field-title')).toHaveValue('Loop P');
  await expect(page.getByTestId('wf-goal-field-work')).toContainText('/data/goal-loops/P/work');
  await expect(page.getByText('Earlier person verifications are not carried over')).toBeVisible();
  expect(writes).toEqual([]);
});
