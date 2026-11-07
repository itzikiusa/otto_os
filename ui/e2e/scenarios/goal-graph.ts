import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from '../seed';
import { openPage, expectNoHorizontalOverflow, expectFullyInViewport } from '../helpers';
import type { CreateGoalLoopReq, GoalLoop } from '../../src/lib/api/types';

export function goalGraphScenario(): void {
  test.use({ serviceWorkers: 'block' });
for (const scheme of ['light', 'dark']) {
  test(`templates submit the edited graph and render saved-response states after reload ${scheme}`, async ({ page }, info) => {
    const { ctx, base } = await apiCtx();
    const workspace = await seedWorkspace(ctx, base);
    await ctx.dispose();
    await page.addInitScript(({ workspace, scheme }) => {
      localStorage.setItem('otto_workspace', workspace);
      localStorage.setItem('otto_scheme', scheme);
      localStorage.setItem('otto_rail_expanded', '0');
      localStorage.setItem('otto_firstrun_dismissed', '1');
    }, { workspace, scheme });
    let saved: GoalLoop | null = null;
    let submitted: CreateGoalLoopReq | null = null;
    let defineCalls = 0;
    await page.route('**/goal-loops/define', async route => {
      defineCalls++;
      await route.fulfill({ status: 500, json: { message: 'Template selection must not call an agent' } });
    });
    // Isolate the provider boundary. The real form, serialization, detail store,
    // route navigation and persisted-response rendering remain mounted.
    await page.route(`**/workspaces/${workspace}/goal-loops`, async route => {
      if (route.request().method() === 'GET') return route.fulfill({ json: saved ? [saved] : [] });
      submitted = route.request().postDataJSON() as CreateGoalLoopReq;
      const nodes = submitted.config.graph!.nodes;
      saved = { ...submitted, id: 'template-loop', workspace_id: workspace, status: 'blocked', phase: 'done',
        current_iteration: 2, iterations_started: 2, progress_pct: 33, elapsed_secs: 20, cost_usd: 0,
        context_digest: '', created_by: 'user', created_at: new Date().toISOString(), updated_at: new Date().toISOString(),
        ledger: { questions: [], verifications: [], next_action: 'Repair the streaming check', review_summary: '', review_passed: false,
          repeated_failures: 0, last_failure_signature: '', graph_nodes: nodes.map((node, i) => ({ node_id: node.id,
            status: i === 0 ? 'passed' : i === 1 ? 'blocked' : 'pending', iteration: i === 2 ? 0 : i + 1,
            evidence: i === 0 ? 'Producer and consumers traced at export.rs:12' : i === 1 ? 'Regression check still fails' : '', criteria: [] })) } };
      await route.fulfill({ json: saved });
    });
    await page.route('**/goal-loops/template-loop?summary=true', route => route.fulfill({ json: { loop: saved, iterations: [] } }));
    if (info.project.name === 'desktop-browser') await page.setViewportSize({ width: 1440, height: 900 });
    await openPage(page, 'loops');
    await page.getByRole('button', { name: /New goal loop|Define your first goal/ }).first().click();
    const cycle = page.getByRole('region', { name: 'Goal loop execution' });
    await expect(cycle.locator('[data-edge="digesting:planning"]')).toHaveCount(1);
    await cycle.getByRole('button', { name: /^Plan/ }).focus();
    await page.keyboard.press('ArrowRight');
    await expect(cycle.getByRole('button', { name: /^Execute/ })).toBeFocused();
    await page.keyboard.press('Enter');
    await expect(cycle.locator('.phase-detail')).toContainText('configured executors work sequentially');
    await cycle.scrollIntoViewIfNeeded();
    await page.screenshot({ path: info.outputPath(`goal-loop-cycle-${scheme}.png`) });
    await page.locator('#gl-execution').selectOption('graph');
    await expect(page.locator('#gl-template option')).toHaveCount(8);
    await page.locator('#gl-template').selectOption('general');
    await expect(page.getByRole('button', { name: 'Launch loop', exact: true })).toBeDisabled();
    await page.locator('#gl-seed').fill('Stream the CSV export without buffering all rows');
    await page.locator('#gl-repo').fill('/tmp/test-repository');
    const editor = page.getByRole('region', { name: 'Engineering graph', exact: true });
    await editor.getByRole('button', { name: /Implement the change/ }).click();
    await editor.getByLabel('Task title', { exact: true }).fill('Stream rows incrementally');
    await page.locator('#gl-execution').selectOption('loop');
    await page.getByRole('dialog').getByRole('button', { name: 'Use goal loop', exact: true }).click();
    await expect(cycle).toBeVisible();
    await page.locator('#gl-execution').selectOption('graph');
    await editor.getByRole('button', { name: /Stream rows incrementally/ }).click();
    await expect(editor.getByLabel('Task title', { exact: true })).toHaveValue('Stream rows incrementally');
    await page.locator('#gl-template').selectOption('angular');
    await page.getByRole('dialog').getByRole('button', { name: /Cancel|Keep editing/ }).click();
    await expect(editor.getByLabel('Task title', { exact: true })).toHaveValue('Stream rows incrementally');
    await expect(page.locator('#gl-template')).toHaveValue('general');
    await editor.getByRole('button', { name: /Understand the change/ }).click();
    await editor.getByLabel('Review and verify', { exact: true }).check();
    await expect(editor.locator('.error')).toContainText('cycle');
    await expect(page.getByRole('button', { name: 'Launch loop', exact: true })).toBeDisabled();
    await editor.getByLabel('Review and verify', { exact: true }).uncheck();
    await expect(editor.locator('[data-edge="task-1:task-2"]')).toHaveCount(1);
    await expect(editor.locator('[data-edge="task-2:task-3"]')).toHaveCount(1);
    await editor.scrollIntoViewIfNeeded();
    await page.screenshot({ path: info.outputPath(`goal-graph-editor-${scheme}.png`) });
    await page.setViewportSize({ width: 390, height: 844 });
    await expectNoHorizontalOverflow(page);
    await editor.scrollIntoViewIfNeeded();
    await page.screenshot({ path: info.outputPath(`goal-graph-phone-${scheme}.png`) });
    await page.getByRole('button', { name: 'Launch loop', exact: true }).click();
    const graph = page.getByTestId('goal-graph-state');
    await expect(graph).toBeVisible();
    await expect(graph.locator('[data-diagram-node="task-1"]')).toContainText('Passed');
    await expect(graph.locator('[data-diagram-node="task-2"]')).toContainText('Blocked');
    await expect(graph.locator('[data-diagram-node="task-3"]')).toContainText('Pending');
    expect(defineCalls).toBe(0);
    expect((submitted as CreateGoalLoopReq | null)?.config.graph?.nodes[1].title).toBe('Stream rows incrementally');
    await page.reload();
    await expect(graph.locator('[data-diagram-node="task-2"]')).toContainText('Stream rows incrementally');
    await graph.locator('[data-diagram-node="task-2"]').click();
    await expect(graph).toContainText('Regression check still fails');
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath(`goal-graph-state-phone-${scheme}.png`) });
    // Pause retains the attempt ledger; it must not advertise live execution.
    const paused = saved as GoalLoop | null;
    expect(paused).not.toBeNull();
    paused!.status = 'paused';
    paused!.ledger!.graph_nodes![1].status = 'running';
    await page.reload();
    await expect(graph.locator('[data-diagram-node="task-2"]')).toContainText('Paused');
    await expect(graph.locator('[data-diagram-node="task-2"]')).not.toContainText('Running');
    // The same persisted response surface also visualizes a whole-goal loop.
    paused!.config.graph = null;
    paused!.phase = 'executing';
    await page.reload();
    await expect(cycle).toBeVisible();
    await expect(cycle.getByRole('button', { name: /^Execute/ })).toContainText('Paused');
    const header = page.getByTestId('page-header');
    await expectFullyInViewport(page, header.getByRole('button', { name: 'Resume', exact: true }));
    await expectFullyInViewport(page, header.getByRole('button', { name: 'More actions', exact: true }).last());
    await expect(cycle.locator('[data-edge="digesting:planning"]')).toHaveCount(1);
    await expectNoHorizontalOverflow(page);
    await cycle.scrollIntoViewIfNeeded();
    await page.screenshot({ path: info.outputPath(`goal-loop-cycle-phone-${scheme}.png`) });
  });
}
}
