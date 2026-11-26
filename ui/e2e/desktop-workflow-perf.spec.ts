import { test, expect } from '@playwright/test';
import { chain, createWorkflow, reviewStep, wfApi, type WfApi } from './workflowsFixture';

// Run page performance (test-plan.md §7, DR-027): first paint of the run
// page on a large run (29 reviewer sessions, full logs) replayed through route
// mocks ≤ 300 ms after the response; ≤ 2 requests in 5 idle seconds per
// screen. The real-daemon timing of a ≈400 KB row needs the test-only seed
// helper (not built yet) — marked fixme below.

let api: WfApi;
test.beforeAll(async ({}, info) => {
  if (!(info.project.name === 'desktop-browser' || info.project.name === 'desktop-webkit')) return;
  api = await wfApi();
});
test.afterAll(async () => {
  await api?.ctx.dispose();
});
test.beforeEach(async ({ page }, info) => {
  test.skip(!(info.project.name === 'desktop-browser' || info.project.name === 'desktop-webkit'), 'not this project');
  await page.addInitScript((ws) => {
    localStorage.setItem('otto_workspace', ws as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, api.ws);
});

function bigRun(wf: string, run: string) {
  const now = Date.now();
  const at = (m: number) => new Date(now - m * 60_000).toISOString();
  const logs = Array.from({ length: 200 }, (_, i) => `10:${String(i % 60).padStart(2, '0')}  reviewer progress line ${i}`);
  const children = Array.from({ length: 29 }, (_, i) => ({ id: `c${i}`, thread_id: `th${i}`, label: `Lens ${i + 1} · ${i % 2 ? 'Codex' : 'Claude'}`, provider: i % 2 ? 'codex' : 'claude', status: i < 20 ? 'success' : 'running', note: i < 20 ? `${i} findings` : undefined, started_at: at(10) }));
  return {
    id: run, workflow_id: wf, status: 'running', started_at: at(25), input: { msg: 'big' }, rev: 9,
    nodes: [
      { node_id: 'start', status: 'success', started_at: at(25), duration_ms: 1, logs: [] },
      { node_id: 'review', status: 'running', started_at: at(20), logs, children, review_ids: [] },
    ],
  };
}

test('the run page paints within 300 ms of a large run arriving', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Perf', chain(reviewStep('review', 29)));
  const run = 'perf-run-1';
  let served = 0;
  await page.route(`**/api/v1/workflow-runs/${run}`, async (route) => { served = Date.now(); await route.fulfill({ json: bigRun(wf, run) }); });
  await page.route(`**/api/v1/workflow-runs/${run}/progress**`, (route) => route.fulfill({ json: { changed: false, rev: 9 } }));
  await page.goto(`/#/workflows/${wf}/runs/${run}`);
  await expect(page.getByTestId('wf-step-child').first()).toBeVisible({ timeout: 20_000 });
  const painted = Date.now();
  expect(painted - served, 'first paint after the response').toBeLessThanOrEqual(300);
  await expect(page.getByTestId('wf-run-orbit')).toHaveAttribute('data-grouped', 'true');
});

for (const screen of ['runs', 'all'] as const) test(`≤ 2 requests in 5 idle seconds on ${screen}`, async ({ page }) => {
  await page.goto(screen === 'runs' ? '/#/workflows' : '/#/workflows/all');
  await expect(page.getByTestId(screen === 'runs' ? 'wf-tab-runs' : 'wf-tab-workflows')).toBeVisible({ timeout: 20_000 });
  await page.waitForTimeout(2_000); // ui-guards: allow (let the first loads settle)
  const seen: string[] = [];
  page.on('request', (r) => { if (r.url().includes('/api/v1/')) seen.push(r.url()); });
  await page.waitForTimeout(5_000); // ui-guards: allow (counting idle requests over a fixed window)
  expect(seen.length, seen.join('\n')).toBeLessThanOrEqual(2);
});

test.fixme('the real daemon answers a ≈400 KB run in ≤ 150 ms and paints it in ≤ 300 ms', async () => {
  // Needs the test-only seed helper that writes a frozen large row (test-plan.md §7).
});
