// Runtime custom plugins E2E (desktop-browser project only): installs BOTH
// example plugins from this repo into the ISOLATED test daemon (plugins-home +
// secrets live in the throwaway data dir — see global-setup), wires a mock
// Jira + a scripted git repo, then drives the real dashboards through the
// plugin iframe.
//
// The dora-metrics sidecar is a Rust crate compiled on first enable; the
// beforeAll prebuilds it with an explicit compile-sized timeout. On machines
// without cargo the DORA half self-skips (team-performance still runs).
import { test, expect, type APIRequestContext, type FrameLocator, type Page } from '@playwright/test';
import { execSync } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import {
  startMockJira,
  makeFixtureRepo,
  installPlugin,
  installedPlugins,
  uninstallPlugin,
  enablePlugin,
  waitPluginHealthy,
  type MockJira,
} from './fixtures/plugins-fixtures';

test.describe.configure({ mode: 'serial' });

const EXAMPLES = join(process.cwd(), '..', 'examples', 'plugins');

let hasCargo = false;
try {
  execSync('cargo --version', { stdio: 'ignore' });
  hasCargo = true;
} catch {
  /* dora half will self-skip */
}

let api: APIRequestContext;
let base: string;
let mockJira: MockJira;
let repoDir: string;
/** Plugins THIS spec installed (absent before it ran) — removed in afterAll so
 *  they don't leak into the shared daemon (an enabled plugin adds a sidebar
 *  section every later spec would see). */
const installedHere = new Set<string>();
let preinstalled = new Set<string>();

async function install(source: string): Promise<string> {
  const slug = await installPlugin(api, base, source);
  if (!preinstalled.has(slug)) installedHere.add(slug);
  return slug;
}

function pluginsHome(): string {
  const slot = process.env.OTTO_E2E_SLOT ?? '0';
  const meta = JSON.parse(
    readFileSync(join(process.cwd(), 'e2e', `.auth-${slot}`, 'daemon.json'), 'utf8'),
  ) as { dataDir: string };
  return join(meta.dataDir, 'plugins-home');
}

async function pluginFrame(page: Page, slug: string): Promise<FrameLocator> {
  await page.goto(`/#/plugin/${slug}`);
  await expect(page.locator('.shell')).toBeVisible({ timeout: 15_000 });
  const frame = page.frameLocator(`iframe[data-plugin="${slug}"]`);
  return frame;
}

test.beforeAll(async ({}, testInfo) => {
  // Guard here as well as in beforeEach: without it this hook runs (and
  // installs plugins / spawns fixtures) once per device project in a full
  // suite run — 6 concurrent copies racing each other.
  if (testInfo.project.name !== 'desktop-browser') return;
  testInfo.setTimeout(120_000);
  const a = await apiCtx();
  api = a.ctx;
  base = a.base;

  mockJira = await startMockJira();
  repoDir = makeFixtureRepo();

  // Register the fixture repo + the Jira account pointing at the mock.
  const ws = await seedWorkspace(api, base);
  const repo = await api.post(`${base}/api/v1/workspaces/${ws}/repos`, {
    data: { path: repoDir, name: 'plugins-fixture' },
  });
  expect(repo.ok(), await repo.text()).toBeTruthy();
  const acct = await api.post(`${base}/api/v1/issue/accounts`, {
    data: { provider: 'jira', label: 'E2E Jira', email: 'e2e@otto.local', base_url: mockJira.baseUrl, token: 'e2e-token' },
  });
  expect(acct.ok(), await acct.text()).toBeTruthy();

  preinstalled = new Set(await installedPlugins(api, base));

  // team-performance: install + enable (Node sidecar — instant).
  await install(join(EXAMPLES, 'team-performance'));
  await enablePlugin(api, base, 'team-performance');
  await waitPluginHealthy(api, base, 'team-performance', 20_000);
});

test.afterAll(async () => {
  try {
    for (const slug of installedHere) await uninstallPlugin(api, base, slug);
    installedHere.clear();
  } finally {
    await mockJira?.close();
  }
});

test.beforeEach(async ({}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser project only');
});

test('enabled plugins are listed and the section hosts the iframe', async ({ page }) => {
  const list = await (await api.get(`${base}/api/v1/plugins`)).json();
  const slugs = list.map((p: { slug: string }) => p.slug);
  expect(slugs).toContain('team-performance');

  const frame = await pluginFrame(page, 'team-performance');
  // The version badge carries the running plugin's version (from its /config), so
  // the iframe is proven to talk to THIS install's sidecar, not just render.
  const { version } = JSON.parse(
    readFileSync(join(EXAMPLES, 'team-performance', 'otto-plugin.json'), 'utf8'),
  ) as { version: string };
  await expect(frame.getByRole('heading', { level: 1 })).toHaveText('Team Performance');
  await expect(frame.locator('#tp-version')).toBeVisible();
  await expect(frame.locator('#tp-version')).toHaveText(`v${version}`);
});

test('team-performance: scan → team dashboard with bars, predictions, estimation guide', async ({ page }) => {
  test.setTimeout(90_000);
  const frame = await pluginFrame(page, 'team-performance');

  // Account/project preselected from fixtures (other specs may add accounts —
  // assert ours exists rather than pinning the count).
  await expect(frame.locator('#account option', { hasText: 'E2E Jira' })).toHaveCount(1);
  // The project picker is a multi-select button labelled with the selection.
  await expect(frame.locator('#proj-btn')).toContainText('TP');

  await frame.getByRole('tab', { name: 'Overview', exact: true }).click();
  await frame.locator('#scan').click();
  // Overview, People, Flow and Estimates now have separate tabs. Wait for
  // scanned data, then follow the same visible navigation as a user.
  const completedTile = frame.locator('.tile').filter({ has: frame.getByText('Completed tickets', { exact: true }) });
  await expect(completedTile.locator('.value')).toHaveText('6', { timeout: 45_000 });
  await expect(completedTile).toContainText('2 still open');
  await frame.getByRole('tab', { name: 'People', exact: true }).click();
  const people = frame.getByRole('table', { name: /^2 people ·/ });
  await expect(people.locator('tbody tr')).toHaveCount(2);
  await expect(people.getByRole('button', { name: 'Alice', exact: true })).toBeVisible();
  await expect(people.getByRole('button', { name: 'Bob', exact: true })).toBeVisible();

  await frame.getByRole('tab', { name: 'Flow', exact: true }).click();
  const phases = frame.getByRole('region', { name: 'Cycle time by phase', exact: true });
  await expect(phases.getByRole('img', { name: /^Cycle time by phase/ })).toBeVisible();
  await expect(phases.locator('svg > rect').first()).toBeVisible();
  await expect(phases.locator('.legend')).toContainText('Dev');
  await expect(phases.locator('.legend')).toContainText('Design');
  await phases.getByText('Show as table', { exact: true }).click();
  const phaseTable = phases.getByRole('table', { name: /^Median working days per phase/ });
  await expect(phaseTable.getByRole('columnheader', { name: 'Design', exact: true })).toBeVisible();
  await expect(phaseTable.getByRole('columnheader', { name: 'Dev', exact: true })).toBeVisible();
  await expect(phaseTable.locator('tbody tr').first()).toContainText(/\dd/);

  // Check the actual predicted duration and date cells, not merely an elapsed
  // duration elsewhere on the row (which would pass with predictions missing).
  const openRows = frame.getByRole('table', { name: '2 open tickets with predicted timelines', exact: true }).locator('tbody tr');
  await expect(openRows).toHaveCount(2);
  const aliceOpen = openRows.filter({ has: frame.getByRole('link', { name: 'TP-7', exact: true }) });
  await expect(aliceOpen.locator('td').nth(4)).toContainText(/\dd/);
  await expect(aliceOpen.locator('td').nth(6)).toHaveText(/\d{4}-\d{2}-\d{2}/);

  await frame.getByRole('tab', { name: 'Estimates', exact: true }).click();
  const guide = frame.getByRole('table', { name: 'Estimation guide', exact: true });
  await expect(guide.locator('tbody tr').first()).toBeVisible();
  await expect(guide).toContainText('Story');
  await expect(guide.getByRole('columnheader', { name: 'Implementation', exact: true })).toBeVisible();
  await expect(guide.locator('tbody tr').first().locator('td').last()).toContainText(/\dd/);
});

async function openAlice(frame: FrameLocator): Promise<void> {
  await frame.getByRole('tab', { name: 'People', exact: true }).click();
  const people = frame.getByRole('table', { name: /^2 people ·/ });
  await expect(people.locator('tbody tr')).toHaveCount(2, { timeout: 20_000 });
  await people.getByRole('button', { name: 'Alice', exact: true }).click();
}

test('team-performance: developer drill-down — verdicts, phase durations, evidence, goals', async ({ page }) => {
  test.setTimeout(60_000);
  const frame = await pluginFrame(page, 'team-performance');
  await openAlice(frame);

  // The current person view replaces bullet charts with explicit phase cells.
  // Preserve the measured-time and verdict checks on Alice's three tickets.
  const completed = frame.getByRole('table', { name: '3 completed tickets — estimate → actual, by phase', exact: true });
  await expect(completed.locator('tbody tr')).toHaveCount(3);
  for (const name of ['Estimate', 'Actual', 'Design', 'Dev', 'Verdict']) {
    await expect(completed.getByRole('columnheader', { name, exact: true })).toBeVisible();
  }
  const ticket = completed.locator('tbody tr').filter({ has: frame.getByRole('link', { name: 'TP-1', exact: true }) });
  // Ticket is a row header; the following cells are Type, Estimate, Actual,
  // Design, Dev, Review, Deploy, Rework, Verdict and Details.
  await expect(ticket.locator('td').nth(2)).toHaveText(/^[\d.]+[dh]$/);
  await expect(ticket.locator('td').nth(3)).toHaveText(/^[\d.]+[dh]$/);
  await expect(ticket.locator('td').nth(4)).toHaveText(/^[\d.]+[dh]$/);
  await expect(ticket.locator('td').nth(8).locator('.badge')).toHaveText(/^(fast|on track|slow)$/);

  await ticket.getByRole('button', { name: 'Details for TP-1', exact: true }).click();
  const evidence = frame.getByRole('dialog', { name: 'TP-1 — evidence and corrections', exact: true });
  await evidence.getByText('Status timeline', { exact: true }).click();
  const history = evidence.getByRole('table', { name: 'Jira status intervals', exact: true });
  await expect(history.getByRole('columnheader', { name: 'Status', exact: true })).toBeVisible();
  await expect(history.getByRole('columnheader', { name: 'Phase', exact: true })).toBeVisible();
  await expect(history.locator('tbody tr').first()).toBeVisible();
  await expect(history).toContainText('In Progress');
  await evidence.getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(evidence).toHaveCount(0);

  const open = frame.getByRole('table', { name: 'In progress, with predictions', exact: true });
  await expect(open.getByRole('link', { name: 'TP-7', exact: true })).toBeVisible();
  await expect(open.locator('tbody tr')).toHaveCount(1);
  await expect(open.locator('tbody td').nth(3)).toHaveText(/^[\d.]+[dh]$/);

  const goalRow = frame.getByRole('table', { name: 'Goals for Alice', exact: true }).locator('tr[data-metric="median_cycle_days"]');
  await expect(goalRow.getByText('suggested', { exact: true })).toBeVisible();
  await goalRow.getByRole('spinbutton', { name: 'Target for Median cycle (days)', exact: true }).fill('3.5');
  const saved = page.waitForResponse((r) => r.request().method() === 'PUT' && /\/goals(\?|$)/.test(r.url()));
  await frame.getByRole('button', { name: 'Save goals', exact: true }).click();
  const response = await saved;
  expect(response.ok(), 'PUT /goals succeeded').toBe(true);
  expect(response.request().postDataJSON()).toMatchObject({
    assignee: 'u-alice', goals: expect.arrayContaining([{ metric: 'median_cycle_days', target: 3.5 }]),
  });
  await expect(goalRow.getByRole('spinbutton')).toHaveValue('3.5');
  await expect(goalRow.getByText('suggested', { exact: true })).toHaveCount(0);
});

test('team-performance: goal target persists across a full reload', async ({ page }) => {
  test.setTimeout(60_000);
  const frame = await pluginFrame(page, 'team-performance');
  await openAlice(frame);
  const goalRow = frame.getByRole('table', { name: 'Goals for Alice', exact: true }).locator('tr[data-metric="median_cycle_days"]');
  await expect(goalRow.getByRole('spinbutton', { name: 'Target for Median cycle (days)', exact: true })).toHaveValue('3.5');
  await expect(goalRow.getByText('suggested', { exact: true })).toHaveCount(0);
});

async function selectFixtureRepo(frame: FrameLocator): Promise<void> {
  const value = await frame.locator('#repo option', { hasText: 'plugins-fixture' }).getAttribute('value');
  await frame.locator('#repo').selectOption(value ?? '');
}

test.describe('dora-metrics (needs cargo)', () => {
  test.beforeAll(async ({}, testInfo) => {
    if (testInfo.project.name !== 'desktop-browser') return;
    test.skip(!hasCargo, 'cargo not on PATH — dora sidecar cannot compile');
    // Install, prebuild (compile-sized budget), then enable → health is fast.
    testInfo.setTimeout(600_000);
    await install(join(EXAMPLES, 'dora-metrics'));
    execSync('cargo build --release', {
      cwd: join(pluginsHome(), 'dora-metrics'),
      stdio: 'pipe',
      timeout: 540_000,
    });
    await enablePlugin(api, base, 'dora-metrics');
    await waitPluginHealthy(api, base, 'dora-metrics', 60_000);
  });

  test.beforeEach(async ({}, testInfo) => {
    test.skip(!hasCargo, 'cargo not on PATH');
    test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser project only');
  });

  test('DORA dashboard: KPI tiles with tiers, weekly trends, suggestions, methodology', async ({ page }) => {
    test.setTimeout(90_000);
    const frame = await pluginFrame(page, 'dora-metrics');

    // Repo preselected; metrics load on boot (or via Refresh).
    await expect(frame.locator('#repo option', { hasText: 'plugins-fixture' })).toHaveCount(1, { timeout: 20_000 });
    // The shared e2e daemon lists other specs' repos too; the preselected
    // (first) one need not be the fixture with deploy tags.
    await selectFixtureRepo(frame);
    await frame.locator('#run').click();

    // 4 KPI tiles, each with a printed tier word.
    await expect(frame.locator('.kpi-tile')).toHaveCount(4, { timeout: 20_000 });
    const badges = frame.locator('.kpi-tile .tier-badge');
    await expect(badges).toHaveCount(4);
    for (const text of await badges.allTextContents()) {
      expect(text.trim().length).toBeGreaterThan(0);
    }

    // 2×2 weekly trend small-multiples with real SVG marks + table twins.
    await expect(frame.locator('.trend-chart')).toHaveCount(4);
    expect(await frame.locator('.trend-chart svg').count()).toBeGreaterThanOrEqual(4);
    await frame.locator('.trend-chart .toggle-table').first().click();
    await expect(frame.locator('.trend-chart .twin').first()).toBeVisible();

    // Deterministic suggestions: fixture CFR is 1/3 → a change-failure warning fires.
    await expect(frame.locator('#suggestions .suggestion').first()).toBeVisible();
    await expect(frame.locator('#suggestions')).toContainText(/failure|hotfix|deploy/i);

    // Methodology footnote documents signals + tiers.
    await expect(frame.locator('#methodology')).toContainText(/tier|deploy/i);
  });

  test('DORA config round-trip: tag pattern drives the deploy signal', async ({ page }) => {
    test.setTimeout(90_000);
    const frame = await pluginFrame(page, 'dora-metrics');
    await expect(frame.locator('#repo option', { hasText: 'plugins-fixture' })).toHaveCount(1, { timeout: 20_000 });
    // The shared e2e daemon lists other specs' repos too; the preselected
    // (first) one need not be the fixture with deploy tags.
    await selectFixtureRepo(frame);
    await frame.locator('#run').click();
    await expect(frame.locator('.kpi-tile')).toHaveCount(4, { timeout: 20_000 });

    // A pattern that matches nothing → no deploys → "no data" tiers.
    await frame.locator('#gear').click();
    await frame.locator('#tag-pattern').fill('zzz-no-such-tag');
    await frame.locator('#save-config').click();
    await frame.locator('#run').click();
    await expect(frame.locator('.kpi-tile .tier-badge.none').first()).toBeVisible({ timeout: 20_000 });

    // Restore the default and the tiers come back.
    await frame.locator('#gear').click();
    await frame.locator('#tag-pattern').fill('deploy');
    await frame.locator('#save-config').click();
    await frame.locator('#run').click();
    await expect(frame.locator('.kpi-tile .tier-badge:not(.none)').first()).toBeVisible({ timeout: 20_000 });
  });
});
