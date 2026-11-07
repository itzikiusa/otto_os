import { test, expect } from '@playwright/test';
import type { Workflow, WorkflowRun } from '../src/lib/api/types';
import { apiCtx, seedWorkspace } from './seed';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';

// Real run UI, mocked workflow boundary: no provider or outbound comment runs.
test('re-review keeps the previous report and opens the new iteration after confirmation', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop workflow run bar');
  const { ctx, base } = await apiCtx();
  try {
    const workspace = await seedWorkspace(ctx, base, 'Re-review fixture');
    const now = new Date().toISOString();
    const workflow: Workflow = {
      id: 'wf-rereview', workspace_id: workspace, name: 'PR review', description: '', instructions: '', version: 1,
      created_by: 'fixture', created_at: now, updated_at: now,
      graph: { nodes: [{ id: 'review', kind: 'review_run', name: 'Review PR', x: 0, y: 0, params: {} }], edges: [] },
    };
    const previous: WorkflowRun = {
      id: 'review-old', workflow_id: workflow.id, workspace_id: workspace, status: 'success',
      started_at: now, finished_at: now, input: { pr: 15 }, rev: 2, checkpoints: [],
      nodes: [{ node_id: 'review', status: 'success', sessions: [], review_ids: ['finding-review'],
        logs: [], output: { review_id: 'finding-review', score: 60, passed: false } }],
    };
    const next: WorkflowRun = { ...previous, id: 'review-new', status: 'pending', finished_at: null, rev: 0,
      input: { pr: 15, review_previous_run_id: previous.id }, nodes: [] };
    let requests = 0;
    await page.addInitScript((id) => {
      localStorage.setItem('otto_workspace', id);
      localStorage.setItem('otto_firstrun_dismissed', '1');
      localStorage.setItem('otto_rail_expanded', '0');
      localStorage.setItem('otto_wf_ctx_open', '0');
    }, workspace);
    await page.route('**/api/v1/workspaces/*/workflows', route => route.fulfill({ json: [workflow] }));
    await page.route('**/api/v1/workflows/wf-rereview', route => route.fulfill({ json: workflow }));
    await page.route('**/api/v1/workflows/wf-rereview/runs?summary=true', route => route.fulfill({ json: [previous] }));
    await page.route('**/api/v1/reviews/finding-review**', route => route.fulfill({ json: { id: 'finding-review', status: 'done', agents: [], comments: [] } }));
    await page.route(/\/api\/v1\/workflow-runs\/review-(old|new)(?:\/.*)?(?:\?.*)?$/, route => {
      const path = new URL(route.request().url()).pathname;
      const run = path.includes('review-new') ? next : previous;
      if (path.endsWith('/re-review')) { requests++; return route.fulfill({ json: next }); }
      if (path.endsWith('/progress')) return route.fulfill({ json: { changed: true, rev: run.rev, run } });
      if (path.endsWith('/checkpoints')) return route.fulfill({ json: { items: [], generation: 0, checkpoint_rev: 0, next_cursor: null } });
      if (path.endsWith('/nodes/review')) return route.fulfill({ json: { rev: run.rev, detail_version: '1', body: previous.nodes[0] } });
      return route.fulfill({ json: run });
    });
    await page.goto('/#/workflows');
    await page.getByTestId(`wf-row-${workflow.id}`).locator('.row-main').click();
    await page.getByRole('button', { name: 'More actions', exact: true }).click();
    await page.getByRole('menuitem', { name: 'Runs', exact: true }).click();
    await page.getByTestId('run-item').first().click();
    const action = page.getByTestId('workflow-re-review');
    await expect(action).toBeVisible();
    for (const scheme of ['light', 'dark'] as const) {
      await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
      await page.evaluate(value => { document.documentElement.dataset.scheme = value; }, scheme);
      await expectFullyInViewport(page, action, `${scheme} re-review`);
      await expectNoHorizontalOverflow(page);
      await info.attach(`workflow-rereview-${scheme}`, { body: await page.screenshot({ fullPage: true }), contentType: 'image/png' });
    }
    await action.click();
    await expect(page.getByRole('dialog')).toContainText('configured notifications and PR comments');
    expect(requests).toBe(0);
    await page.getByRole('dialog').getByRole('button', { name: 'Cancel', exact: true }).click();
    expect(requests).toBe(0);
    await action.click();
    await page.getByRole('dialog').getByRole('button', { name: 'Re-review', exact: true }).click();
    await expect.poll(() => requests).toBe(1);
    await expect(action).toHaveCount(0);
    await expect(page.getByTestId('run-cancel')).toBeVisible();
    expect(previous.status).toBe('success');
  } finally { await ctx.dispose(); }
});
