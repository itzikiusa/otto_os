import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedSwarm, seedGitRepo } from './seed';
import { expectNoHorizontalOverflow, expectFullyInViewport } from './helpers';

async function workspace(page: Page) {
  const { ctx, base } = await apiCtx();
  const id = await seedWorkspace(ctx, base);
  await ctx.dispose();
  await page.addInitScript(id => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_rail_expanded', '0');
  }, id);
  return id;
}
for (const route of ['personal-agents', 'loops', 'mission-control']) {
  test(`${route}: first run offers workspace setup`, async ({ page }) => {
    await page.route('**/api/v1/workspaces', r => r.fulfill({ json: [] }));
    await page.setViewportSize({ width: 375, height: 812 });
    await page.goto(`/#/${route}`);
    await expect(page.getByRole('heading', { name: 'Add a workspace to get started' })).toBeVisible();
    await page.getByTestId('page-empty').getByRole('button', { name: 'Add workspace', exact: true }).click();
    await expect(page.getByRole('dialog')).toBeVisible();
    await page.keyboard.press('Escape');
    await expectNoHorizontalOverflow(page);
  });
}
test('agent detail: arrow keys and End move focus with selection', async ({ page }) => {
  await workspace(page);
  await page.goto('/#/personal-agents');
  await page.locator('.pa-card .name').first().click();
  const overview = page.getByRole('tab', { name: 'Overview', exact: true });
  await overview.focus();
  await page.keyboard.press('ArrowRight');
  const schedules = page.getByRole('tab', { name: 'Schedules', exact: true });
  await expect(schedules).toHaveAttribute('aria-selected', 'true');
  await expect(schedules).toBeFocused();
  await page.keyboard.press('End');
  await expect(page.getByRole('tab', { name: 'Context', exact: true })).toBeFocused();
});
test('Mission Control: pending summary is not zero and load is explicit', async ({ page }) => {
  const id = await workspace(page);
  let release!: () => void;
  const pending = new Promise<void>(resolve => { release = resolve; });
  await page.route(`**/api/v1/workspaces/${id}/workgraph/**`, async route => {
    await pending;
    await route.continue();
  });
  await page.goto('/#/mission-control');
  try {
    await expect(page.getByLabel('Loading work items')).toBeVisible();
    await expect(page.locator('.t-val').first()).toHaveText('—');
  } finally { release(); }
  await expect(page.getByLabel('Loading work items')).toHaveCount(0);
});

test('agent form: losing workspace preserves typed fields and explains how to save', async ({ page }) => {
  await workspace(page);
  await page.goto('/#/personal-agents');
  await page.getByRole('button', { name: 'New agent', exact: true }).click();
  const dialog = page.getByRole('dialog');
  await dialog.getByLabel('Name', { exact: true }).fill('Keep my new agent');
  await page.evaluate(async () => {
    const path = '/src/lib/stores/workspace.svelte.ts';
    const { ws } = await import(/* @vite-ignore */ path);
    ws.currentId = null;
  });
  await dialog.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(dialog).toBeVisible();
  await expect(dialog.getByLabel('Name', { exact: true })).toHaveValue('Keep my new agent');
  await expect(dialog.getByRole('alert')).toContainText('workspace');
});

test('goal definition: losing workspace gives actionable recovery and retains goal', async ({ page }) => {
  await workspace(page);
  await page.goto('/#/loops');
  await page.getByRole('button', { name: 'New goal loop', exact: true }).click();
  await page.locator('#gl-seed').fill('Compare approaches and retain this draft');
  await page.locator('#gl-mode').selectOption('research');
  await page.evaluate(async () => {
    const path = '/src/lib/stores/workspace.svelte.ts';
    const { ws } = await import(/* @vite-ignore */ path);
    ws.currentId = null;
  });
  await expect(page.getByRole('button', { name: 'Define with AI', exact: true })).toBeDisabled();
  await expect(page.getByText('Add a workspace to define and launch this goal. Your draft stays here.')).toBeVisible();
  await expect(page.locator('#gl-seed')).toHaveValue('Compare approaches and retain this draft');
});

test('rooms: failed messages never claim an empty conversation and Retry recovers', async ({ page }) => {
  const id = await workspace(page);
  const { ctx, base } = await apiCtx();
  const response = await ctx.post(`${base}/api/v1/workspaces/${id}/agent-rooms`, { data: { name: 'Design decisions' } });
  expect(response.ok()).toBeTruthy();
  const room = await response.json();
  await ctx.post(`${base}/api/v1/agent-rooms/${room.id}/messages`, { data: { text: 'Keep this conversation visible' } });
  await ctx.dispose();
  let fail = true;
  await page.route(`**/api/v1/agent-rooms/${room.id}/messages*`, r => fail
    ? r.fulfill({ status: 503, json: { code: 'upstream', message: 'Messages temporarily unavailable' } })
    : r.continue());
  await page.goto('/#/personal-agents/rooms');
  await expect(page.getByText("Couldn't load room messages")).toBeVisible();
  await expect(page.getByText(/No messages yet/)).toHaveCount(0);
  fail = false;
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByText('Keep this conversation visible', { exact: true })).toBeVisible();
});

const variants = [
  ['native', 'light', 1440, 900, false], ['native', 'dark', 1440, 900, false],
  ['warm', 'light', 375, 812, false], ['warm', 'dark', 1024, 768, true],
  ['pro-dark', 'dark', 1440, 900, false],
] as const;

test.describe('loaded automation visual and interaction fixtures', () => {
  let wsId = '';
  let workflowId = '';
  test.beforeAll(async () => {
    const { ctx, base } = await apiCtx();
    wsId = await seedWorkspace(ctx, base);
    await seedSwarm(ctx, base, wsId);
    await ctx.post(`${base}/api/v1/workspaces/${wsId}/workgraph/backfill`);
    const room = await (await ctx.post(`${base}/api/v1/workspaces/${wsId}/agent-rooms`, { data: { name: 'Design decisions — cross-team rollout and follow-up' } })).json();
    for (let i = 0; i < 8; i++) await ctx.post(`${base}/api/v1/agent-rooms/${room.id}/messages`, { data: { text: `Decision ${i + 1}: ${'A long discussion with a concrete acceptance condition. '.repeat(5)}` } });
    await ctx.post(`${base}/api/v1/workspaces/${wsId}/scheduled-tasks`, { data: { name: 'Weekly review — security and release readiness', prompt: 'Review release readiness', enabled: false, schedule: { cadence: 'interval', every_min: 60 }, destination: { type: 'none' } } });
    const wf = await (await ctx.post(`${base}/api/v1/workspaces/${wsId}/workflows`, { data: { name: 'Release readiness review', description: 'Review deterministic offline evidence', graph: { nodes: [
      { id: 'start', kind: 'manual_trigger', name: 'Start review', x: 0, y: 0, params: {} },
      { id: 'transform', kind: 'transform', name: 'Prepare the release evidence', x: 300, y: 0, params: { json: { result: 'ready' } } },
      { id: 'log', kind: 'log', name: 'Record the result', x: 600, y: 0, params: {} },
    ], edges: [{ id: 'e1', source: 'start', target: 'transform' }, { id: 'e2', source: 'transform', target: 'log' }] } } })).json();
    workflowId = wf.id;
    await ctx.post(`${base}/api/v1/workflows/${workflowId}/run`, { data: {} });
    const repo = await seedGitRepo(ctx, base, wsId);
    const run = await ctx.post(`${base}/api/v1/workspaces/${wsId}/runs`, { data: {
      source_kind: 'channel', source_ref: 'automation-review', seed_text: 'Add a short project note.',
      mode: 'single_agent', repo_id: repo.repoId, title: 'Release evidence ready for review',
    } });
    expect(run.ok()).toBeTruthy();
    await ctx.dispose();
  });
  for (const [theme, scheme, width, height, rtl] of variants) {
    test(`loaded surfaces ${theme} ${scheme} ${width} RTL=${rtl}`, async ({ page }) => {
      test.setTimeout(120_000);
      await page.setViewportSize({ width, height });
      await page.addInitScript(({ wsId, theme, scheme }) => {
        localStorage.setItem('otto_workspace', wsId);
        localStorage.setItem('otto_rail_expanded', '0');
        localStorage.setItem('otto_theme', theme);
        localStorage.setItem('otto_scheme', scheme);
      }, { wsId, theme, scheme });
      const shot = async (name: string) => {
        await expectNoHorizontalOverflow(page);
        await page.screenshot({ path: `/tmp/otto-ux-r2-automation-screens/${theme}-${scheme}-${width}-${name}.png` });
      };
      await page.goto('/#/swarm');
      if (rtl) await page.evaluate(() => document.documentElement.dir = 'rtl');
      if (width <= 640 || width > 1024) await page.locator('.swarm-item', { hasText: 'E2E Swarm' }).first().click();
      await expect(page.getByRole('tab', { name: 'Org', exact: true })).toBeVisible();
      for (const view of ['Graph', 'Board', 'Feed']) {
        const tab = page.locator('.switcher .seg', { hasText: view }).first();
        await tab.click();
        await expect(tab).toHaveAttribute('aria-selected', 'true');
        await shot(`swarm-${view}`);
      }
      await page.goto('/#/mission-control');
      await expect(page.locator('.t-val').first()).not.toHaveText('—');
      await page.getByRole('tab', { name: 'Graph', exact: true }).click();
      await expect(page.getByRole('tab', { name: 'Graph', exact: true })).toHaveAttribute('aria-selected', 'true');
      await shot('mission-control');
      await page.goto('/#/personal-agents/rooms');
      await expect(page.getByRole('log', { name: 'Room messages' }).getByText('Decision 8:', { exact: false })).toBeVisible();
      await page.getByLabel('Message to the room').fill('Draft kept while reading the discussion');
      await shot('rooms');
      await page.goto('/#/personal-agents');
      await expect(page.locator('.pa-card').first()).toBeVisible();
      await page.locator('.pa-card .name').first().click();
      await page.getByRole('tab', { name: 'Schedules', exact: true }).click();
      await shot('agent-schedules');
      await page.goto('/#/scheduled-tasks');
      await expect(page.getByText('Weekly review — security and release readiness', { exact: true }).first()).toBeVisible();
      await shot('scheduled-tasks');
      await page.goto('/#/workflows');
      await page.getByTestId(`wf-row-${workflowId}`).locator('.row-main').click();
      await shot('workflow');
      if (width <= 640) {
        await page.getByRole('button', { name: 'More actions', exact: true }).click();
        await page.getByRole('menuitem', { name: 'Runs', exact: true }).click();
      } else await page.getByRole('button', { name: 'Runs', exact: true }).click();
      await page.getByTestId('run-item').first().click();
      await expect(page.locator('.timeline')).toBeVisible();
      await shot('workflow-run');
      await page.route(`**/api/v1/workspaces/${wsId}/goal-loops`, route => route.fulfill({ json: Array.from({ length: 12 }, (_, i) => ({
        id: `loop-${i}`, workspace_id: wsId, name: `Release readiness ${i + 1} ${'long goal title '.repeat(8)}`, definition: { summary: 'Verify every delivery condition and preserve the evidence. '.repeat(6), acceptance_criteria: [] },
        limits: { max_iterations: 5 }, status: i % 2 ? 'running' : 'succeeded', phase: 'executing', progress_pct: i % 2 ? 50 : 100, current_iteration: 2, updated_at: '2026-09-25T12:00:00Z',
      })) }));
      await page.goto('/#/run-with-otto');
      await page.locator('.runs .run').first().click();
      await expect(page.getByRole('region', { name: 'Run detail' })).toBeVisible();
      await shot('run-with-otto');
      await page.goto('/#/proof');
      await page.locator('.pack-item').first().click();
      await expect(page.locator('.badges-row')).toBeVisible();
      await shot('proof');
      await page.goto('/#/loops');
      await expect(page.locator('.loop-card')).toHaveCount(12);
      await shot('loops');
    });
  }
});

test('Mission Control: keyboard switches view tabs with focus', async ({ page }) => {
  await workspace(page);
  await page.goto('/#/mission-control');
  const list = page.getByRole('tab', { name: 'List', exact: true });
  await list.focus();
  await page.keyboard.press('ArrowRight');
  const graph = page.getByRole('tab', { name: 'Graph', exact: true });
  await expect(graph).toHaveAttribute('aria-selected', 'true');
  await expect(graph).toBeFocused();
  await page.keyboard.press('Home');
  await expect(list).toBeFocused();
});

test('agent detail: RTL arrow keys follow visual tab direction', async ({ page }) => {
  await workspace(page);
  await page.goto('/#/personal-agents');
  await page.locator('.pa-card .name').first().click();
  await page.evaluate(() => document.documentElement.dir = 'rtl');
  await page.getByRole('tab', { name: 'Overview', exact: true }).focus();
  await page.keyboard.press('ArrowLeft');
  await expect(page.getByRole('tab', { name: 'Schedules', exact: true })).toBeFocused();
});

test('rooms: phone conversation keeps its header and composer in view', async ({ page }) => {
  const id = await workspace(page);
  const { ctx, base } = await apiCtx();
  const room = await (await ctx.post(`${base}/api/v1/workspaces/${id}/agent-rooms`, { data: { name: 'Long discussion' } })).json();
  await ctx.post(`${base}/api/v1/agent-rooms/${room.id}/messages`, { data: { text: 'A detailed decision with evidence. '.repeat(100) } });
  await ctx.dispose();
  await page.setViewportSize({ width: 375, height: 812 });
  await page.goto('/#/personal-agents/rooms');
  await page.getByRole('button', { name: 'Back to rooms' }).click();
  await page.getByRole('button', { name: 'Long discussion 0 agents' }).click();
  await page.getByLabel('Message to the room').fill('Follow-up draft');
  await expectFullyInViewport(page, page.locator('.detail-title'));
  await expectFullyInViewport(page, page.getByLabel('Message to the room'));
  await expectFullyInViewport(page, page.getByRole('button', { name: 'Send', exact: true }));
  await expect.poll(() => page.getByRole('log', { name: 'Room messages' }).evaluate(el => el.scrollHeight > el.clientHeight)).toBe(true);
});

test('goal research flow: draft, edit, launch and inspect within a phone viewport', async ({ page }) => {
  const id = await workspace(page);
  await page.setViewportSize({ width: 375, height: 812 });
  const agent = { name: 'Researcher', provider: 'claude', model: '', prompt_extra: '' };
  const draft = {
    definition: { title: 'Research release options', summary: 'Compare two options with evidence.', acceptance_criteria: [{ id: 'c1', text: 'Compare the options', verify: 'Read the report', verify_kind: 'agent', verify_cmd: null }] },
    suggested_limits: { max_iterations: 3, max_runtime_secs: 600, per_phase_timeout_secs: 300, max_cost_usd: null, max_attempts_per_executor: 3 },
    suggested_config: { executors: [agent], planner: agent, evaluator: agent, digester: agent, definer: agent, mode: 'research' },
  };
  let launched: Record<string, unknown> | null = null;
  const loop = { id: 'review-loop', workspace_id: id, name: draft.definition.title, repo_path: '', definition: draft.definition, limits: draft.suggested_limits, config: draft.suggested_config, status: 'paused', phase: 'planning', current_iteration: 0, iterations_started: 0, progress_pct: 0, context_digest: '', elapsed_secs: 0, cost_usd: 0, created_by: 'review', created_at: '2026-09-25T12:00:00Z', updated_at: '2026-09-25T12:00:00Z' };
  await page.route(`**/api/v1/workspaces/${id}/goal-loops/define`, r => r.fulfill({ json: draft }));
  await page.route(`**/api/v1/workspaces/${id}/goal-loops`, async r => {
    if (r.request().method() === 'POST') { launched = r.request().postDataJSON(); await r.fulfill({ json: loop }); }
    else await r.fulfill({ json: launched ? [loop] : [] });
  });
  await page.route('**/api/v1/goal-loops/review-loop', r => r.fulfill({ json: { loop, iterations: [] } }));
  await page.goto('/#/loops');
  await page.getByRole('button', { name: 'New goal loop', exact: true }).click();
  await page.locator('#gl-seed').fill('Compare release options');
  await page.locator('#gl-mode').selectOption('research');
  await page.getByRole('button', { name: 'Define with AI', exact: true }).click();
  await expect(page.locator('#gl-name')).toHaveValue('Research release options');
  await page.locator('#gl-name').fill('Reviewed research plan');
  await expectNoHorizontalOverflow(page);
  await page.getByRole('button', { name: 'Launch loop', exact: true }).click();
  await expect.poll(() => launched?.name).toBe('Reviewed research plan');
  await expect(page.getByRole('heading', { name: 'Research release options', exact: true })).toBeVisible();
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: '/tmp/otto-ux-r2-automation-screens/phone-goal-detail.png' });
});

test('proof: optional successes never count as completed requirements', async ({ page }) => {
  const id = await workspace(page);
  const { ctx, base } = await apiCtx();
  const pack = await (await ctx.post(`${base}/api/v1/workspaces/${id}/proof-packs`, { data: { work_item_kind: 'manual', work_item_id: `contract-${Date.now()}`, title: 'Incomplete required evidence' } })).json();
  const detail = await (await ctx.get(`${base}/api/v1/proof-packs/${pack.id}`)).json();
  await ctx.dispose();
  detail.done_contract = {
    score: 58, satisfied: 3, required: 3,
    items: [
      { key: 'diff', label: 'Code diff captured', required: true, satisfied: true, weight: 1, detail: 'diff present' },
      { key: 'tests', label: 'Tests passed', required: true, satisfied: false, weight: 1, detail: 'no passing recognized test' },
      { key: 'failures', label: 'No failed evidence', required: true, satisfied: true, weight: 1, detail: 'nothing failing' },
      { key: 'review', label: 'Agent self-review', required: false, satisfied: true, weight: 1, detail: 'self-review present' },
    ],
  };
  await page.route(`**/api/v1/proof-packs/${pack.id}`, r => r.fulfill({ json: detail }));
  await page.goto('/#/proof');
  await expect(page.getByRole('region', { name: 'Done contract' }).getByText('2/3 required met')).toBeVisible();
});
