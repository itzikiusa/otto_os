import { test, expect, type APIRequestContext, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { apiCtx, seedWorkspace, seedGitRepo } from './seed';

// ─────────────────────────────────────────────────────────────────────────────
// Behavioural journeys for the S20 UX fixes (S20-306): the iteration-2 fixes
// were pinned only by source-regex unit tests, so a refactor that kept the
// strings but broke the flow (an inverted `{#if}`, a lost button `type`) still
// passed. These drive the real UI against the isolated e2e daemon:
//   • Share: on the loopback-only daemon a minted link warns "only works on
//     this Mac" instead of offering a phone QR; a LAN link (reach "lan") keeps
//     the QR and says it is Wi-Fi-only behind a self-signed certificate.
//   • Run with Otto approval gate: "Open findings" opens THIS run's review
//     read-only (not the clean-slate Review tab), and "View branch diff"
//     selects the run branch's tip on the graph (S20-301).
//   • Scheduled task form: the "Next fires" preview comes from the daemon.
// ─────────────────────────────────────────────────────────────────────────────

test.use({ serviceWorkers: 'block' });

let ctx: APIRequestContext;
let base = '';
let wsId = '';

test.beforeEach(async ({ page }, info) => {
  test.skip(info.project.name !== 'desktop-browser', 'desktop-browser project only');
  const c = await apiCtx();
  ctx = c.ctx;
  base = c.base;
  wsId = await seedWorkspace(ctx, base, `Journeys ${Date.now()}`);
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, wsId);
});

test.afterEach(async () => {
  await ctx?.dispose();
});

async function openShareFor(page: Page, title: string): Promise<void> {
  const r = await ctx.post(`${base}/api/v1/workspaces/${wsId}/sessions`, {
    data: { kind: 'agent', provider: 'shell', title, cwd: '/tmp', meta: { origin: 'e2e' } },
  });
  expect(r.ok()).toBeTruthy();
  const id = ((await r.json()) as { id: string }).id;
  await page.goto(`/#/agents/${id}`);
  const tab = page.locator('.tab', { hasText: title });
  await expect(tab).toBeVisible({ timeout: 15_000 });
  await tab.click({ button: 'right' });
  await page.locator('.ctx-item', { hasText: 'Share…' }).click();
  await expect(page.getByRole('button', { name: 'Generate link' })).toBeVisible({ timeout: 10_000 });
}

async function mintViewerLink(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Generate link' }).click();
  // Every link is outward-facing: the confirm names where it goes first.
  await page.getByRole('dialog').getByRole('button', { name: 'Create viewer link' }).click();
}

test('share: a loopback-only link warns instead of showing a phone QR', async ({ page }) => {
  await openShareFor(page, 'Journey share local');
  // The isolated daemon has no Public link domain and no bound LAN listener,
  // so the REAL mint answers reach "local".
  const minted = page.waitForResponse((r) => /\/sessions\/[^/]+\/share$/.test(r.url()) && r.request().method() === 'POST');
  await mintViewerLink(page);
  const resp = await minted;
  expect(resp.ok()).toBeTruthy();
  const body = (await resp.json()) as { reach?: string; reachable_remotely: boolean };
  expect(body.reachable_remotely).toBe(false);
  expect(body.reach).toBe('local');
  await expect(page.getByTestId('share-local-only')).toBeVisible();
  await expect(page.getByTestId('share-local-only')).toContainText('only works on this Mac');
  await expect(page.locator('canvas.sm-qr')).toHaveCount(0);
  await expect(page.getByTestId('share-lan-only')).toHaveCount(0);
});

test('share: a LAN link keeps the QR and says Wi-Fi only + self-signed certificate', async ({ page }) => {
  await openShareFor(page, 'Journey share lan');
  // A bound LAN listener can't be faked on the shared isolated daemon (it is
  // process-wide): answer the mint as the daemon would with one serving.
  await page.route('**/api/v1/sessions/*/share', async (route) => {
    if (route.request().method() !== 'POST') return route.continue();
    const real = await route.fetch();
    const json = (await real.json()) as Record<string, unknown>;
    const url = String(json.url).replace(/^https?:\/\/[^/]+/, 'https://192.168.1.20:7700');
    await route.fulfill({ json: { ...json, url, reachable_remotely: true, reach: 'lan' } });
  });
  await mintViewerLink(page);
  await expect(page.locator('canvas.sm-qr')).toBeVisible();
  await expect(page.getByTestId('share-lan-only')).toContainText('Works on your Wi-Fi only');
  await expect(page.getByTestId('share-lan-only')).toContainText('self-signed certificate');
  await expect(page.getByTestId('share-local-only')).toHaveCount(0);
});

test('run approval gate: evidence links open THIS run’s review and branch (S20-301)', async ({ page }) => {
  const { repoId, dir } = await seedGitRepo(ctx, base, wsId);
  // The run's branch, one commit ahead, NOT checked out (as a run worktree leaves it).
  const git = (...args: string[]) => execFileSync('git', ['-C', dir, ...args], { stdio: 'ignore' });
  git('checkout', '-q', '-b', 'otto/run-e2e');
  writeFileSync(join(dir, 'run-change.txt'), 'changed by the run\n');
  git('add', '-A');
  git('commit', '-q', '-m', 'E2E run commit');
  git('checkout', '-q', '-');

  const now = new Date().toISOString();
  const runId = 'run-e2e-journey';
  const reviewId = 'rv-e2e-journey';
  const run = {
    id: runId, workspace_id: wsId, title: 'Fix the login bug', source_kind: 'text', source_ref: '', goal: 'fix it',
    mode: 'standard', provider: 'claude', model: '', repo_id: repoId, base_branch: 'main', branch: 'otto/run-e2e',
    status: 'awaiting_approval', origin_kind: 'ui', review_id: reviewId, findings_total: 2, findings_blocking: 1,
    auto_open_pr: false, created_by: 'e2e', created_at: now, updated_at: now,
  };
  const finding = (id: string, severity: string, title: string) => ({
    id, review_id: reviewId, workspace_id: wsId, repo_id: repoId, pr_number: null, fingerprint: id,
    severity, category: 'correctness', path: 'file_01.txt', line: 1, line_end: 1, title, body: `${title} — details`,
    evidence: '', agent_reasoning_summary: '', suggested_fix: null, status: 'open', linked_commit: null,
    linked_test: null, reviewer: 'claude', state: 'open', regressed: false, requires_human_approval: false,
    approval_decision: null, approved_by: null, approved_at: null, jira_key: null, jira_url: null,
    produced_by_agent: 'claude', repo_rule_id: null, fix_session_id: null, occurrence_count: 1,
    created_at: now, updated_at: now,
  });
  // A run row can't be put into awaiting_approval without a real agent turn:
  // serve the run + its review; the repo, refs and graph are the daemon's.
  await page.route(`**/api/v1/workspaces/${wsId}/runs`, (r) =>
    r.request().method() === 'GET' ? r.fulfill({ json: [run] }) : r.continue());
  await page.route(`**/api/v1/runs/${runId}`, (r) => r.fulfill({ json: run }));
  await page.route(`**/api/v1/runs/${runId}/events`, (r) => r.fulfill({ json: [] }));
  await page.route(`**/api/v1/reviews/${reviewId}`, (r) => r.fulfill({ json: {
    id: reviewId, repo_id: repoId, pr_number: 0, status: 'done', comments: [], agents: [],
    created_at: now, updated_at: now, summary_fallback: false,
  } }));
  await page.route(`**/api/v1/reviews/${reviewId}/findings*`, (r) => r.fulfill({ json: [
    finding('f-block', 'critical', 'Token is logged in plain text'),
    finding('f-minor', 'low', 'Rename the helper'),
  ] }));

  await page.goto(`/#/run-with-otto/${runId}`);
  const gate = page.getByTestId('run-gate-evidence');
  await expect(gate).toBeVisible({ timeout: 20_000 });

  await gate.getByRole('button', { name: 'Open findings' }).click();
  await expect(page).toHaveURL(new RegExp(`#/git/${repoId}/review/${reviewId}$`));
  const linked = page.getByTestId('linked-review');
  await expect(linked).toBeVisible({ timeout: 20_000 });
  await expect(linked).toContainText('From run');
  await expect(linked).toContainText('Fix the login bug');
  await expect(linked).toContainText('read-only');
  // The run's findings are on screen — not hidden in a collapsed "Past reviews".
  await expect(linked).toContainText('Token is logged in plain text', { timeout: 15_000 });
  await expect(linked).toContainText('Rename the helper');
  await expect(page.getByRole('button', { name: /Past reviews/ })).toHaveCount(0);
  // "From run" links back to the gate.
  await linked.getByRole('button', { name: /Fix the login bug/ }).click();
  await expect(gate).toBeVisible({ timeout: 15_000 });

  await gate.getByRole('button', { name: 'View branch diff' }).click();
  await expect(page).toHaveURL(new RegExp(`#/git/${repoId}/graph/otto%2Frun-e2e$`));
  // The graph selects the run branch's tip.
  await expect(page.locator('.graph-row-selected')).toHaveAttribute('title', /E2E run commit/, { timeout: 20_000 });
});

test('scheduled task form: "Next fires" comes from the daemon before Save', async ({ page }) => {
  await page.goto('/#/scheduled-tasks');
  await page.getByRole('button', { name: 'New task' }).first().click();
  await page.getByLabel('Cadence').selectOption('cron');
  await page.getByLabel('Cron expression (5 fields)').fill('0 9 * * 0');
  await page.getByLabel('Timezone').fill('UTC');
  const preview = page.getByTestId('sched-next-fires');
  await expect(preview).toContainText('Next fires (UTC)', { timeout: 10_000 });
  // `0 9 * * 0` is Sundays — every listed fire says so.
  const items = preview.locator('li');
  await expect(items.first()).toBeVisible();
  for (const text of await items.allTextContents()) expect(text).toMatch(/Sun/);
  // An invalid edit drops the stale preview instead of showing the old times.
  await page.getByLabel('Cron expression (5 fields)').fill('0 9 *');
  await expect(preview).toHaveCount(0);
});
