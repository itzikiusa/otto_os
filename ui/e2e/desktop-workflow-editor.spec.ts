import { test, expect } from '@playwright/test';
import { agentStep, chain, createWorkflow, wfApi, type WfApi, node } from './workflowsFixture';
import { canvasNode, selectStep } from './workflowsUi';

// The editor (ux.md §9, §16): opened fitted with every card visible; model
// and effort saved from the Threads picker; a condition has no model; continue
// from lists only same-provider ancestors; an uninstalled skill blocks Save.

let api: WfApi;
test.beforeAll(async ({}, info) => {
  if (!(info.project.name === 'desktop-browser')) return;
  api = await wfApi();
});
test.afterAll(async () => {
  await api?.ctx.dispose();
});
test.beforeEach(async ({ page }, info) => {
  test.skip(!(info.project.name === 'desktop-browser'), 'not this project');
  await page.addInitScript((ws) => {
    localStorage.setItem('otto_workspace', ws as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, api.ws);
});


test('opens fitted: every card is inside the canvas', async ({ page }) => {
  const steps = Array.from({ length: 11 }, (_, i) => agentStep(`s${i + 1}`, `Step ${i + 1}`));
  const wf = await createWorkflow(api, 'E2E Twelve', chain(...steps));
  await page.goto(`/#/workflows/${wf}`);
  const canvas = page.getByTestId('wf-editor-canvas');
  await expect(canvas).toBeVisible({ timeout: 20_000 });
  const c = (await canvas.boundingBox())!;
  for (const n of await page.getByTestId('wf-node').all()) {
    const b = (await n.boundingBox())!;
    expect(b.x >= c.x && b.y >= c.y && b.x + b.width <= c.x + c.width && b.y + b.height <= c.y + c.height, 'card inside the fitted canvas').toBeTruthy();
  }
  await expect(page.getByTestId('wf-node')).toHaveCount(12);
});

test('a condition step has no model control; an agent step has one', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Condition', chain(agentStep('a', 'Agent'), node('c', 'condition', 'Passed?', { expr: 'true' })));
  await page.goto(`/#/workflows/${wf}`);
  await selectStep(page, 'c');
  await expect(page.getByTestId('wf-step-model')).toHaveCount(0);
  await selectStep(page, 'a');
  await expect(page.getByTestId('wf-step-model')).toHaveCount(1);
});

test('choosing a model and effort saves them on the step', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Model pick', chain(agentStep('a', 'Agent')));
  await page.goto(`/#/workflows/${wf}`);
  await selectStep(page, 'a');
  const saved = page.waitForRequest((r) => r.method() === 'PATCH' && r.url().endsWith(`/workflows/${wf}`));
  // The model: the first one the picker lists that is not the default.
  await page.getByTestId('wf-step-model').click();
  const picker = page.getByTestId('model-picker');
  await expect(picker).toBeVisible();
  await picker.locator('.mp-row:not([disabled])').filter({ hasNotText: 'Default model' }).first().click();
  await page.keyboard.press('Escape');
  await expect(picker).toBeHidden();
  // The effort, from the same picker.
  await page.getByTestId('wf-step-effort').click();
  await picker.getByRole('group', { name: 'Reasoning effort' }).getByRole('button', { name: /^high$/i }).click();
  await page.keyboard.press('Escape');
  await page.keyboard.press('Meta+s');
  const params = (await saved).postDataJSON().graph.nodes.find((n: { id: string }) => n.id === 'a').params;
  expect(params.model, 'a model was chosen').toEqual(expect.any(String));
  expect(params.model).not.toBe('');
  expect(params.effort).toBe('high');
});

test('continue from lists only same-provider ancestors', async ({ page }) => {
  const g = chain(agentStep('a', 'Claude one'), node('b', 'agent_prompt', 'Codex one', { provider: 'codex', prompt: 'x' }), agentStep('c', 'Claude two'));
  const wf = await createWorkflow(api, 'E2E Continue', g);
  await page.goto(`/#/workflows/${wf}`);
  await selectStep(page, 'c');
  const pick = page.getByTestId('wf-continue-from');
  await pick.getByRole('radio', { name: /Continue the thread of an earlier step/ }).check();
  // ux.md §9.6: the select lists the same-provider ancestors; others disabled with the reason.
  const from = pick.getByRole('combobox', { name: 'The step whose thread this step continues' });
  await expect(from).toHaveValue('a');
  await expect(from.locator('option[value="a"]')).toBeEnabled();
  await expect(from.locator('option[value="b"]')).toBeDisabled();
  await expect(pick).toContainText('Codex one is a Codex step');
});

test('a skill that is not installed shows the problem and Save sends nothing', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Skill', chain(agentStep('a', 'Agent', [], { skills: ['not-installed-skill'] })));
  await page.goto(`/#/workflows/${wf}`);
  await selectStep(page, 'a');
  let patched = false;
  page.on('request', (r) => { if (r.method() === 'PATCH' && r.url().endsWith(`/workflows/${wf}`)) patched = true; });
  await canvasNode(page, 'a').getByRole('button').first().click();
  await page.getByTestId('page-header').first().getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText(/1 problem/)).toBeVisible();
  expect(patched).toBe(false);
});
