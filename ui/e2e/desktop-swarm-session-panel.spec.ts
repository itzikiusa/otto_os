import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace, seedSwarm } from './seed';

// Desktop BROWSER: the Swarm page's inline session panel must actually SHOW the
// session.
//
// REGRESSION (the bug this pins): `.session-panel` is a flex row and SessionView's
// `.pane` was a plain flex item with no `flex`, so it was sized by its intrinsic
// width — except it HAS none: the pane header is a `container-type: inline-size`
// query container (size-contained on the inline axis) and the body is a terminal
// canvas. The pane therefore shrink-wrapped to its padding, ~20px: opening a
// session from Runs/Org/Graph produced a sliver with a lone status dot and no
// terminal, on an otherwise blank 480px panel. Splits/TiledView never hit this
// because their parents are CSS grids, where items stretch.
//
// Seeds a swarm plus a shell session TAGGED to one of its agents (meta.swarm_id +
// meta.agent_id) — that is exactly what the Org tree lists and what makes the
// panel openable without a live agent CLI.
//
// Only meaningful on the desktop-browser project; self-skips elsewhere.

test.setTimeout(120_000);

let api: APIRequestContext;
let base = '';
let workspaceId = '';
const swarmName = 'E2E Swarm';
const sessionTitle = 'E2E Swarm Pane';

test.beforeAll(async () => {
  const a = await apiCtx();
  api = a.ctx;
  base = a.base;
  workspaceId = await seedWorkspace(api, base);
  const { swarmId } = await seedSwarm(api, base, workspaceId);

  const detail = await (await api.get(`${base}/api/v1/swarm/swarms/${swarmId}`)).json();
  const agentId = (detail.agents as { id: string }[])[0]?.id;
  expect(agentId, 'preset must seed at least one agent').toBeTruthy();

  // A session the Org tree will hang under that agent (it filters on
  // meta.swarm_id + meta.agent_id). `shell` needs no agent CLI installed.
  const res = await api.post(`${base}/api/v1/workspaces/${workspaceId}/sessions`, {
    data: {
      kind: 'agent',
      provider: 'shell',
      title: sessionTitle,
      cwd: '/tmp',
      meta: { origin: 'e2e', swarm_id: swarmId, agent_id: agentId },
    },
  });
  expect(res.ok(), `seed session -> ${res.status()}`).toBeTruthy();
});

test.afterAll(async () => {
  await api?.dispose();
});

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
});

async function openSwarm(page: Page): Promise<void> {
  await page.goto('/#/swarm');
  await expect(page.locator('.swarm-page')).toBeVisible({ timeout: 30_000 });
  const item = page.locator('.swarm-item', { hasText: swarmName }).first();
  await expect(item).toBeVisible({ timeout: 20_000 });
  await item.click();
  await expect(page.locator('.swarm-head')).toBeVisible({ timeout: 20_000 });
}

test('session panel: the opened session fills the panel (not a 20px sliver)', async ({ page }) => {
  await openSwarm(page);

  // Org is the default view; agent nodes start expanded, so the tagged session
  // is listed straight away.
  const row = page.locator('.session-row', { hasText: sessionTitle }).first();
  await expect(row).toBeVisible({ timeout: 20_000 });
  await row.click();

  const panel = page.locator('.session-panel');
  const pane = page.locator('.session-panel .pane');
  await expect(panel).toBeVisible({ timeout: 20_000 });
  await expect(pane).toBeVisible();

  const panelBox = await panel.boundingBox();
  const paneBox = await pane.boundingBox();
  expect(panelBox, 'session panel must have a box').toBeTruthy();
  expect(paneBox, 'session pane must have a box').toBeTruthy();

  // The pane fills its panel — the collapse this guards against left it at ~20px
  // inside a several-hundred-px panel.
  expect(paneBox!.width).toBeGreaterThan(200);
  expect(Math.abs(paneBox!.width - panelBox!.width)).toBeLessThanOrEqual(2);

  // And it is a real session view, not just a wide empty box.
  await expect(pane.getByText(sessionTitle).first()).toBeVisible();
});
