import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

test('workflow preflight identifies a broken step and cron editing preserves disabled state', async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser only');
  const { ctx, base } = await apiCtx();
  try {
    const workspace = await seedWorkspace(ctx, base);
    const r = await ctx.post(`${base}/api/v1/workspaces/${workspace}/workflows`, { data: {
      name: 'Recovery workflow', graph: { nodes: [{ id: 'missing-url', kind: 'http_request', name: 'Missing endpoint', params: {} }], edges: [] },
    } });
    expect(r.ok(), await r.text()).toBeTruthy();
    const workflow = (await r.json()).id;
    const invalid = await ctx.post(`${base}/api/v1/workflows/${workflow}/validate`, { data: {} });
    expect(invalid.ok()).toBeTruthy();
    expect(await invalid.json()).toMatchObject({ valid: false, issues: [expect.objectContaining({ node_id: 'missing-url', field: 'url' })] });
    const trigger = await ctx.post(`${base}/api/v1/workflows/${workflow}/triggers`, { data: {
      kind: 'schedule', enabled: false, spec: { cadence: 'cron', expr: '0 9 * * 1-5', timezone: 'UTC', future_extension: 'preserved' },
    } });
    expect(trigger.ok(), await trigger.text()).toBeTruthy();
    await page.addInitScript((id) => { localStorage.setItem('otto_workspace', id); localStorage.setItem('otto_rail_expanded', '0'); }, workspace);
    await page.goto('/#/workflows');
    await page.getByTestId(`wf-row-${workflow}`).click();
    await page.getByRole('button', { name: 'Validate', exact: true }).click();
    await expect(page.locator('.preflight[role="alert"]')).toContainText('url');
    await page.getByRole('button', { name: 'Triggers', exact: true }).click();
    await page.getByTitle('Edit trigger', { exact: true }).click();
    await page.getByLabel('Cron (5 fields)', { exact: true }).fill('15 10 * * 1-5');
    await page.getByLabel('Timezone (IANA)', { exact: true }).fill('Asia/Jerusalem');
    await page.getByRole('button', { name: 'Preview / validate', exact: true }).click();
    await expect(page.locator('.add-form')).toContainText('Next fires (Asia/Jerusalem)');
    await page.getByRole('button', { name: 'Save trigger', exact: true }).click();
    await expect(page.locator('.trig-spec')).toContainText('15 10 * * 1-5 (Asia/Jerusalem)');
    const saved = await (await ctx.get(`${base}/api/v1/workflows/${workflow}/triggers`)).json();
    expect(saved).toHaveLength(1);
    expect(saved[0]).toMatchObject({ enabled: false, spec: { expr: '15 10 * * 1-5', timezone: 'Asia/Jerusalem', future_extension: 'preserved' } });
  } finally { await ctx.dispose(); }
});

test('loop run exposes durable attempts and retains its pinned version after editing', async ({}, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop browser only');
  const { ctx, base } = await apiCtx();
  try {
    const workspace = await seedWorkspace(ctx, base);
    const graph = { nodes: [{ id: 'loop', kind: 'loop', name: 'Checkpointed loop', params: {
      max_iterations: 2, until: 'last.done == true', steps: [{ kind: 'transform', name: 'Result', params: { json: { done: true } } }],
    } }], edges: [] };
    const response = await ctx.post(`${base}/api/v1/workspaces/${workspace}/workflows`, { data: { name: 'Checkpoint run', graph } });
    expect(response.ok(), await response.text()).toBeTruthy();
    const workflow = (await response.json()).id;
    const started = await ctx.post(`${base}/api/v1/workflows/${workflow}/run`, { data: {} });
    expect(started.ok(), await started.text()).toBeTruthy();
    const runId = (await started.json()).id;
    await expect.poll(async () => (await (await ctx.get(`${base}/api/v1/workflow-runs/${runId}`)).json()).status, { timeout: 30_000 }).toBe('success');
    const run = await (await ctx.get(`${base}/api/v1/workflow-runs/${runId}`)).json();
    expect(run.workflow_version).toBe(1);
    expect(run.checkpoints).toHaveLength(2);
    expect(run.checkpoints.find((step: { node_id: string }) => step.node_id === 'loop#1.0')).toMatchObject({ status: 'success', attempts: 1, output: { done: true, _iteration: 1 } });
    expect(run.checkpoints.find((step: { node_id: string }) => step.node_id === 'loop')).toMatchObject({ status: 'success', attempts: 1, output: { iterations: 1, satisfied: true } });
    const changed = await ctx.patch(`${base}/api/v1/workflows/${workflow}`, { data: { graph: { nodes: [{ id: 'next', kind: 'log', name: 'New version' }], edges: [] } } });
    expect(changed.ok(), await changed.text()).toBeTruthy();
    expect((await changed.json()).version).toBe(2);
    expect((await (await ctx.get(`${base}/api/v1/workflow-runs/${runId}`)).json()).workflow_version).toBe(1);
  } finally { await ctx.dispose(); }
});
