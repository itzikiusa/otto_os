import { test, expect } from '@playwright/test';
import { mkdir } from 'node:fs/promises';
import type { Workflow, WorkflowRun } from '../src/lib/api/types';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { openRun, stepPane, stepRow } from './workflowsUi';

// The real run page, with a synthetic running snapshot at the API boundary.
// Only the workspace is seeded in the isolated E2E daemon; no workflow, review,
// session or provider is started. A step between attempts is still a working
// step of a running run (never "Failed"), and its earlier attempt stays
// reachable from the step before the run finishes.
test('a retrying agent step stays working and keeps its earlier attempt before the run finishes', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop run page');
  const { ctx, base } = await apiCtx();
  try {
    const workspace = await seedWorkspace(ctx, base, 'Recovery browser fixture');
    const now = new Date().toISOString();
    const retry = 'Review changes — ↻ retry 3/5 in 20000ms (provider overloaded: 529)';
    const workflow: Workflow = {
      id: 'wf-retry-status', workspace_id: workspace, name: 'Review recovery',
      description: 'Retry visibility fixture', instructions: '', version: 1,
      created_by: 'fixture', created_at: now, updated_at: now,
      graph: { nodes: [{ id: 'review', kind: 'agent_prompt', name: 'Review changes',
        x: 0, y: 0, params: { prompt: 'Review the changes' } }], edges: [] },
    };
    const run: WorkflowRun = {
      id: 'run-retry-status', workflow_id: workflow.id, workspace_id: workspace,
      status: 'running', started_at: now, finished_at: null, input: {}, rev: 2,
      context_dir: '/tmp/otto-recovery-browser-fixture', checkpoints: [],
      nodes: [{ node_id: 'review', status: 'running', attempts: 2,
        sessions: ['retry-status-previous'], review_ids: [],
        logs: ['Review changes — attempt 1/3', 'Review changes — ↻ retry 2/3 in 2000ms (temporary failure)',
          'Review changes — attempt 2/3', retry], output: null }],
    };
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.addInitScript((id) => {
      localStorage.setItem('otto_workspace', id);
      localStorage.setItem('otto_firstrun_dismissed', '1');
      localStorage.setItem('otto_theme', 'native');
      localStorage.setItem('otto_rail_expanded', '0');
    }, workspace);
    await page.route('**/api/v1/workspaces/*/workflows', (route) => route.fulfill({ json: [workflow] }));
    await page.route('**/api/v1/workflows/wf-retry-status', (route) => route.fulfill({ json: workflow }));
    await page.route('**/api/v1/workflows/wf-retry-status/runs?summary=true', (route) => route.fulfill({ json: [run] }));
    await page.route('**/api/v1/workflow-runs/run-retry-status**', (route) => {
      const path = new URL(route.request().url()).pathname;
      if (path.endsWith('/progress')) return route.fulfill({ json: { changed: true, rev: run.rev, run } });
      if (path.endsWith('/checkpoints')) return route.fulfill({ json: { items: [], generation: 0, checkpoint_rev: 0, next_cursor: null } });
      if (path.endsWith('/nodes/review')) return route.fulfill({ json: { rev: run.rev, detail_version: 'retry-2', body: run.nodes[0] } });
      return route.fulfill({ json: run });
    });
    await page.route('**/api/v1/sessions/retry-status-previous', (route) => route.fulfill({ json: {
      id: 'retry-status-previous', workspace_id: workspace, kind: 'agent', provider: 'codex',
      title: 'Review changes · earlier attempt', status: 'exited', created_at: now,
      cwd: '/tmp/otto-recovery-browser-fixture', meta: { source: 'workflow', run_id: run.id },
    } }));

    const runView = await openRun(page, workflow.id, run.id, 'review');
    await expect(runView).toHaveAttribute('data-state', 'running');

    // Between attempts the step is working, not failed.
    const step = stepRow(page, 'review');
    await expect(step).toHaveAttribute('data-status', 'running');
    await expect(step).toContainText('Working');
    await expect(page.locator('[data-testid="wf-step"][data-status="error"]')).toHaveCount(0);
    await expect(page.getByTestId('wf-run-card')).toHaveCount(0);

    // The earlier attempt's session stays one click away on the step.
    const pane = stepPane(page);
    await expect(pane).toHaveAttribute('data-node-id', 'review');
    await expect(pane.getByTestId('wf-step-tab-conversation')).toHaveAttribute('aria-selected', 'true');
    const earlier = pane.getByRole('listitem').filter({ hasText: 'Session' }).filter({ hasText: 'evious' });
    await expect(earlier).toBeVisible();
    await expect(earlier.getByRole('button', { name: 'Open in Agents' })).toBeVisible();

    const screenshotDir = '/tmp/otto-recovery-screenshots';
    await mkdir(screenshotDir, { recursive: true });
    for (const scheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
      await page.evaluate((value) => {
        localStorage.setItem('otto_scheme', value);
        document.documentElement.dataset.theme = 'native';
        document.documentElement.dataset.scheme = value;
      }, scheme);
      await expect(page.locator('html')).toHaveAttribute('data-scheme', scheme);
      await expectFullyInViewport(page, step, `${scheme} retrying step`);
      const textLayout = await step.evaluate((element) => ({ scrollWidth: element.scrollWidth, clientWidth: element.clientWidth }));
      expect(textLayout.scrollWidth, `${scheme} step row must fit without clipping`).toBeLessThanOrEqual(textLayout.clientWidth);
      await expectNoHorizontalOverflow(page);
      const screenshot = `${screenshotDir}/workflow-retry-status-${scheme}.png`;
      await page.screenshot({ path: screenshot, fullPage: true, animations: 'disabled' });
      await info.attach(`Workflow retrying step — ${scheme}`, { path: screenshot, contentType: 'image/png' });
    }
  } finally {
    await ctx.dispose();
  }
});
