// Workflows screens (ux.md §1.1, §14.7): the addresses and the test ids the
// workflow specs drive. Runs is `#/workflows`, the definitions
// `#/workflows/all`, the editor `#/workflows/<wf>`, Optimize
// `#/workflows/<wf>/optimize`, a run `#/workflows/<wf>/runs/<run>[/<node>[/<child>]]`.
import { expect, type Locator, type Page } from '@playwright/test';

export const runsTab = (page: Page): Promise<unknown> => page.goto('/#/workflows');
export const workflowsTab = (page: Page): Promise<unknown> => page.goto('/#/workflows/all');

/** A Workflows-tab row. */
export const workflowRow = (page: Page, id: string): Locator => page.locator(`[data-testid="wf-workflow-row"][data-workflow-id="${id}"]`);
/** A Runs-inbox row. */
export const runRow = (page: Page, runId: string): Locator => page.locator(`[data-testid="wf-run-row"][data-run-id="${runId}"]`);

/** The editor of one workflow, painted (canvas on desktop, outline on a phone). */
export async function openEditor(page: Page, workflowId: string): Promise<void> {
  await page.goto(`/#/workflows/${workflowId}`);
  await expect(page.getByTestId('wf-editor-canvas').or(page.getByTestId('wf-phone-outline'))).toBeVisible({ timeout: 20_000 });
}

/** A run's page (optionally with a step or reviewer open). */
export async function openRun(page: Page, workflowId: string, runId: string, nodeId?: string, childId?: string): Promise<Locator> {
  const tail = nodeId ? `/${nodeId}${childId ? `/${childId}` : ''}` : '';
  await page.goto(`/#/workflows/${workflowId}/runs/${runId}${tail}`);
  const run = page.locator(`[data-testid="wf-run-page"][data-run-id="${runId}"]`);
  await expect(run).toBeVisible({ timeout: 20_000 });
  return run;
}

/** One step row of the run page's list. */
export const stepRow = (page: Page, nodeId: string): Locator => page.locator(`[data-testid="wf-step"][data-node-id="${nodeId}"]`);
/** The open step's pane (beside the list, or the pushed screen on a phone). */
export const stepPane = (page: Page): Locator => page.getByTestId('wf-step-pane');

/** The page header's ⋯ menu, then one of its items. */
export async function headerMenu(page: Page, item: string | RegExp): Promise<void> {
  await page.getByTestId('page-header').first().getByRole('button', { name: 'More actions' }).click();
  const menu = page.locator('.ctx-menu');
  await menu.getByRole('menuitem', { name: item }).or(menu.getByRole('menuitemcheckbox', { name: item })).first().click();
}

/** Poll a run over the API until it settles; its final status. */
export async function waitRunSettled(get: (path: string) => Promise<{ status: string }>, runId: string, timeoutMs = 60_000): Promise<string> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const run = await get(`/workflow-runs/${runId}`);
    if (run.status !== 'running' && run.status !== 'pending') return run.status;
    if (Date.now() > deadline) throw new Error(`run ${runId} did not settle`);
    await new Promise((r) => setTimeout(r, 300));
  }
}

/** A step card on the editor's canvas. */
export const canvasNode = (page: Page, nodeId: string): Locator => page.locator(`[data-testid="wf-node"][data-node-id="${nodeId}"]`);

/** Select a step on the editor's canvas; its inspector opens. */
export async function selectStep(page: Page, nodeId: string): Promise<void> {
  await canvasNode(page, nodeId).getByRole('button').first().click();
  await expect(page.getByTestId('wf-inspector')).toBeVisible({ timeout: 10_000 });
}

/** The editor header's Save. */
export async function saveEditor(page: Page): Promise<void> {
  await page.getByTestId('page-header').first().getByRole('button', { name: 'Save', exact: true }).click();
}

/** The header's Run… (the editor's or the Runs tab's): the start form. */
export async function openStartForm(page: Page): Promise<Locator> {
  await page.getByTestId('page-header').first().getByRole('button', { name: 'Run…' }).click();
  const form = page.getByTestId('wf-start-form');
  await expect(form).toBeVisible({ timeout: 10_000 });
  return form;
}

/** The run page of `runId` (whatever is in the address). */
export const runPage = (page: Page, runId: string): Locator => page.locator(`[data-testid="wf-run-page"][data-run-id="${runId}"]`);
