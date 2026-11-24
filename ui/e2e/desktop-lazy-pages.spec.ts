import { test, expect } from '@playwright/test';
import { LAZY_CASES, expectLazyOnFirstUse, mockMode } from './lazyPages';

// Page diets (effort 2): each heavy view loads on first use behind a calm
// placeholder, never with its page (desktop; the phone twin is
// lazy-pages-mobile.spec.ts).

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
