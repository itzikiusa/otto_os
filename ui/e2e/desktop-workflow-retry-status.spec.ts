import { test, expect } from '@playwright/test';
import { mkdir } from 'node:fs/promises';
import type { Workflow, WorkflowRun } from '../src/lib/api/types';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// The real Workflows page and RunAgents component, with a synthetic running
// snapshot at the API boundary. Only the workspace is seeded in the isolated
// E2E daemon; no workflow, review, session or provider is started.
test('workflow agents show the current attempt and retry reason before the run finishes', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop workflow sidebar');
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
      localStorage.setItem('otto_wf_ctx_open', '1');
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

    await page.goto('/#/workflows');
    await page.getByTestId(`wf-row-${workflow.id}`).locator('.row-main').click();
    await page.getByRole('button', { name: 'More actions', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Runs', exact: true }).click();
    await page.getByTestId('run-item').first().click();
    await page.getByTestId('ctx-tab-agents').click();

    const agents = page.getByTestId('run-agents');
    await expect(agents.getByText('Attempt 2', { exact: true })).toBeVisible();
    await expect(agents.getByText('Review changes · earlier attempt', { exact: true })).toBeVisible();
    const explanation = agents.getByTestId('workflow-recovery-status');
    await expect(explanation).toHaveText(retry);
    await expect(explanation).not.toContainText('temporary failure');
    await expect(agents.locator('.grp-h')).toContainText('Running');

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
      await expectFullyInViewport(page, explanation, `${scheme} retry explanation`);
      const textLayout = await explanation.evaluate((element) => {
        const style = getComputedStyle(element);
        return { scrollWidth: element.scrollWidth, clientWidth: element.clientWidth,
          whiteSpace: style.whiteSpace, textOverflow: style.textOverflow };
      });
      expect(textLayout.scrollWidth, `${scheme} retry reason must fit without clipping`).toBeLessThanOrEqual(textLayout.clientWidth);
      expect(textLayout.whiteSpace, `${scheme} retry reason must wrap`).not.toBe('nowrap');
      expect(textLayout.textOverflow, `${scheme} retry reason must remain readable`).not.toBe('ellipsis');
      await expectNoHorizontalOverflow(page);
      const screenshot = `${screenshotDir}/workflow-retry-status-${scheme}.png`;
      await page.screenshot({ path: screenshot, fullPage: true, animations: 'disabled' });
      await info.attach(`Workflow retry status — ${scheme}`, { path: screenshot, contentType: 'image/png' });
    }
  } finally {
    await ctx.dispose();
  }
});
