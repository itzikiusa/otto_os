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
  const frame = page.frameLocator(`iframe[title="${slug}"]`);
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

  // team-performance: install + enable (Node sidecar — instant).
  await installPlugin(api, base, join(EXAMPLES, 'team-performance'));
  await enablePlugin(api, base, 'team-performance');
  await waitPluginHealthy(api, base, 'team-performance', 20_000);
});

test.afterAll(async () => {
  await mockJira?.close();
});

test.beforeEach(async ({}, testInfo) => {
  test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-browser project only');
});

test('enabled plugins are listed and the section hosts the iframe', async ({ page }) => {
  const list = await (await api.get(`${base}/api/v1/plugins`)).json();
  const slugs = list.map((p: { slug: string }) => p.slug);
  expect(slugs).toContain('team-performance');

  const frame = await pluginFrame(page, 'team-performance');
  await expect(frame.locator('h1')).toHaveText('Team Performance');
});

test('team-performance: scan → team dashboard with bars, predictions, estimation guide', async ({ page }) => {
  test.setTimeout(90_000);
  const frame = await pluginFrame(page, 'team-performance');

  // Account/project preselected from fixtures (other specs may add accounts —
  // assert ours exists rather than pinning the count).
  await expect(frame.locator('#account option', { hasText: 'E2E Jira' })).toHaveCount(1);
  await expect(frame.locator('#project')).toContainText('TP');

  await frame.locator('#scan').click();
  // Team view renders once the scan lands: 2 developers.
  await expect(frame.locator('#assignee-table tr.clickable')).toHaveCount(2, { timeout: 45_000 });

  // Phase-split bars (SVG marks) + legend.
  await expect(frame.locator('#assignee-bars svg rect').first()).toBeVisible();
  await expect(frame.locator('#assignee-bars .legend')).toContainText('design');

  // Team-level open tasks carry predictions + projected dates.
  const openRows = frame.locator('#open-tasks tbody tr');
  await expect(openRows).toHaveCount(2);
  await expect(openRows.filter({ hasText: 'TP-7' })).toContainText(/\dd/);

  // The estimation guide (baseline buckets) exists — the lead's timeline table.
  await expect(frame.locator('#estimation-guide tbody tr').first()).toBeVisible();
  await expect(frame.locator('#estimation-guide')).toContainText('Story');

  // KPI tiles.
  await expect(frame.locator('.kpi-tile').first()).toContainText('Completed');
});

test('team-performance: developer drill-down — verdicts, bullet bars, evidence, goals', async ({ page }) => {
  test.setTimeout(60_000);
  const frame = await pluginFrame(page, 'team-performance');
  await expect(frame.locator('#assignee-table tr.clickable')).toHaveCount(2, { timeout: 20_000 });

  await frame.locator('#assignee-table tr.clickable', { hasText: 'Alice' }).click();

  // Completed tasks with per-phase verdict badges + bullet bars.
  await expect(frame.locator('#completed-table tr.task-row')).toHaveCount(3);
  await expect(frame.locator('#completed-table .badge').first()).toBeVisible();
  await expect(frame.locator('#completed-table td.bullet svg').first()).toBeVisible();

  // Evidence drill-down: click a row → stored status intervals appear.
  await frame.locator('#completed-table tr.task-row').first().click();
  await expect(frame.locator('.evidence.open')).toContainText('status history');
  await expect(frame.locator('.evidence.open table tbody tr').first()).toBeVisible();

  // Open task prediction for Alice (TP-7).
  await expect(frame.locator('#dev-open')).toContainText('TP-7');

  // Goals: suggested rows exist; edit the cycle target and save.
  const goalRow = frame.locator('#goals .goal[data-metric="median_cycle_days"]');
  await expect(goalRow).toBeVisible();
  await goalRow.locator('input.goal-target').fill('3.5');
  await frame.locator('#save-goals').click();
  await expect(frame.locator('#goals-msg')).toHaveText('saved');
});

test('team-performance: goal target persists across a full reload', async ({ page }) => {
  test.setTimeout(60_000);
  const frame = await pluginFrame(page, 'team-performance');
  await expect(frame.locator('#assignee-table tr.clickable')).toHaveCount(2, { timeout: 20_000 });
  await frame.locator('#assignee-table tr.clickable', { hasText: 'Alice' }).click();

  const goalRow = frame.locator('#goals .goal[data-metric="median_cycle_days"]');
  await expect(goalRow.locator('input.goal-target')).toHaveValue('3.5');
  // A saved goal is no longer marked as a suggestion.
  await expect(goalRow).not.toContainText('(suggested)');
});

test.describe('dora-metrics (needs cargo)', () => {
  test.beforeAll(async ({}, testInfo) => {
    if (testInfo.project.name !== 'desktop-browser') return;
    test.skip(!hasCargo, 'cargo not on PATH — dora sidecar cannot compile');
    // Install, prebuild (compile-sized budget), then enable → health is fast.
    testInfo.setTimeout(600_000);
    await installPlugin(api, base, join(EXAMPLES, 'dora-metrics'));
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
