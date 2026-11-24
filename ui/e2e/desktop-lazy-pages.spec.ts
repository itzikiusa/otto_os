import { test, expect } from '@playwright/test';
import { LAZY_CASES, expectLazyOnFirstUse, mockMode, productCase, seedStory, seedVault, skillsCase, vaultCase } from './lazyPages';

// Page diets (effort 2): each heavy view loads on first use behind a calm
// placeholder, never with its page (desktop; the phone twin is
// lazy-pages-mobile.spec.ts).

// The app's service worker would serve module fetches page.route never sees.
test.use({ serviceWorkers: 'block' });

test.beforeEach(async ({ page }, ti) => {
  test.skip(ti.project.name !== 'desktop-browser', 'desktop-browser only');
  await page.setViewportSize({ width: 1440, height: 900 });
});

for (const [name, make] of Object.entries(LAZY_CASES)) {
  test(`${name}: the heavy view loads on first use behind a calm placeholder`, async ({ page }) => {
    await mockMode(page);
    await expectLazyOnFirstUse(page, make((l) => l.click()));
  });
}

// Goal Loops: the embedded agent session (terminal + xterm) loads when an
// agent's session is opened, not with the loop (phone opens it in Agents).
// LoopDetail on its fixture page, the API stubbed per route.
test('loops: the agent session loads on first open; an unmet iteration reads Continuing', async ({ page }) => {
  const loop = {
    id: 'goal', name: 'Nightly import fix', workspace_id: 'ws', status: 'running', phase: 'execute',
    current_iteration: 2, iterations_started: 2, progress_pct: 55, elapsed_secs: 600, run_started_at: new Date().toISOString(),
    worktree_path: null, error: null, summary: null,
    definition: { title: 'Fix import', summary: 'Reject malformed CSV rows', acceptance_criteria: [{ id: 'tests', text: 'Import tests pass', verify: 'cargo test import', verify_kind: 'command', verify_cmd: 'cargo test import' }] },
    config: { executors: [{ name: 'Executor', provider: 'codex', model: '' }], mode: 'build', allow_commits: false },
    limits: { max_iterations: 5, max_runtime_secs: 3600, per_phase_timeout_secs: 600, max_attempts_per_executor: 3 },
    ledger: { verifications: [], questions: [], next_action: '', last_failure_signature: '', repeated_failures: 0, review_summary: '', review_passed: false },
  };
  const iterations = [{
    id: 'it1', loop_id: 'goal', workspace_id: 'ws', idx: 1, status: 'succeeded', plan: 'Validate each row', context_in: '', context_out: '',
    agents: [{ name: 'Executor', provider: 'codex', model: '', status: 'done', note: 'Added validation', session_id: 'ses_exec' }],
    evaluation: { progress_pct: 55, verdict: 'continue', feedback: '', rationale: '', criteria: [{ id: 'tests', met: false, evidence: 'exit 101' }] },
    tokens_input: 0, tokens_output: 0, cost_usd: 0, started_at: new Date().toISOString(), finished_at: new Date().toISOString(),
  }];
  await page.addInitScript(() => {
    localStorage.setItem('otto_base', location.origin);
    localStorage.setItem('otto_token', 'fixture');
  });
  await page.route('**/api/v1/**', (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === '/api/v1/goal-loops/goal') return route.fulfill({ json: { loop, iterations } });
    return route.fulfill({ json: [] });
  });
  await expectLazyOnFirstUse(page, {
    overview: 'e2e/fixtures/goal-loop-verification.html',
    painted: (p) => p.getByRole('button', { name: /Iteration 1/ }),
    module: '/src/modules/agents/SessionView.svelte',
    open: async (p) => {
      const head = p.getByRole('button', { name: /Iteration 1/ });
      await expect(head).toContainText('Continuing');
      await expect(head).not.toContainText('Succeeded');
      if ((await head.getAttribute('aria-expanded')) !== 'true') await head.click();
      await p.getByRole('button', { name: 'Open session' }).click();
    },
    placeholder: 'Loading the agent session',
    ready: (p) => p.locator('.sess-pane > *').first(),
  });
});

test('vault: the note editor loads on the first Edit', async ({ page }) => {
  await seedVault(page);
  await expectLazyOnFirstUse(page, vaultCase((l) => l.click()));
});

// Vault used to poll status AND the docs-agent runs every 5 s while open.
// Idle (no run active, nothing written), it is one status request a tick.
test('vault: an idle vault polls one request every 5 s', async ({ page }) => {
  await seedVault(page);
  const seen: string[] = [];
  page.on('request', (r) => {
    const p = new URL(r.url()).pathname;
    if (/\/vault\/vaults\/\d+\/status$/.test(p) || p.endsWith('/docs-agents/runs')) seen.push(p);
  });
  await page.goto('/#/vault');
  await expect(page.locator('.tree').getByText('services', { exact: true })).toBeVisible({ timeout: 20_000 });
  await page.waitForLoadState('networkidle').catch(() => {});
  seen.length = 0;
  await page.waitForTimeout(21_000); // ui-guards: allow (counting the poll over a fixed window)
  const runs = seen.filter((p) => p.endsWith('/docs-agents/runs')).length;
  const status = seen.length - runs;
  expect(runs, 'no runs poll while none is active').toBe(0);
  expect(status).toBeGreaterThanOrEqual(3);
  expect(status).toBeLessThanOrEqual(5);
});

test('product: a story tab loads when it is opened', async ({ page }) => {
  const title = await seedStory(page);
  await expectLazyOnFirstUse(page, productCase((l) => l.click(), title));
});

test('skills lab: the editor loads on Files; copies compare by hash, not one request each', async ({ page }) => {
  const perCopy: string[] = [];
  page.on('request', (r) => {
    const p = new URL(r.url()).pathname;
    if (/\/library\/provider-skills\/[^/]+\/[^/]+$/.test(p)) perCopy.push(p);
  });
  await expectLazyOnFirstUse(page, skillsCase((l) => l.click()));
  expect(perCopy, 'no per-copy body fetch to compare copies').toEqual([]);
});

// Workflows: the canvas is its own chunk; selecting a node (a click, no move)
// must not mark the workflow Unsaved.
test('workflows: the canvas loads behind a placeholder; a node click is not an edit', async ({ page }) => {
  await mockMode(page);
  let release!: () => void;
  const gate = new Promise<void>((r) => (release = r));
  await page.route((u) => u.pathname.endsWith('/src/modules/workflows/WorkflowCanvas.svelte'), async (route) => {
    await gate;
    await route.continue();
  });
  await page.goto('/#/workflows');
  await expect(page.getByRole('status', { name: 'Loading the canvas' })).toBeVisible({ timeout: 20_000 });
  const header = page.getByTestId('page-header').first();
  const before = await header.boundingBox();
  release();
  const node = page.locator('.node').first();
  await expect(node).toBeVisible({ timeout: 20_000 });
  expect(await header.boundingBox()).toEqual(before);
  await node.click();
  await page.waitForTimeout(300); // ui-guards: allow (proving the badge does not appear)
  await expect(page.getByText('Unsaved', { exact: true })).toHaveCount(0);
});
