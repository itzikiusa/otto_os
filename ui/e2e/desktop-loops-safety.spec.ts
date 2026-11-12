import { test, expect, type Page } from '@playwright/test';

// Goal loops run unattended: a restart pause must say so and offer Resume,
// Otto's 4 h run cap must be visible, and an iteration must show the evidence
// (including a verify command's output) behind each criterion verdict.

const RESTART = 'Interrupted by a daemon restart — Resume to continue with preserved work.';
const MARKER = '\n--- verify output ---\n';

function fixtureLoop(over: Record<string, unknown> = {}) {
  return {
    id: 'goal', name: 'Nightly import fix', workspace_id: 'ws', status: 'paused', phase: 'done',
    current_iteration: 1, iterations_started: 1, progress_pct: 50, elapsed_secs: 600, run_started_at: null,
    worktree_path: '/isolated/retained-work', error: RESTART, summary: null,
    definition: {
      title: 'Fix import', summary: 'Reject malformed CSV rows',
      acceptance_criteria: [
        { id: 'tests', text: 'Import tests pass', verify: 'cargo test import', verify_kind: 'command', verify_cmd: 'cargo test import' },
        { id: 'msg', text: 'Error message names the row', verify: 'Read the handler', verify_kind: 'agent', verify_cmd: null },
      ],
    },
    config: { executors: [{ name: 'Executor', provider: 'codex', model: '' }], mode: 'build', allow_commits: false },
    limits: { max_iterations: 5, max_runtime_secs: 3600, per_phase_timeout_secs: 600, max_attempts_per_executor: 3 },
    ledger: { verifications: [], questions: [], next_action: '', last_failure_signature: '', repeated_failures: 0, review_summary: '', review_passed: false },
    ...over,
  };
}

const iterations = [{
  id: 'it1', loop_id: 'goal', workspace_id: 'ws', idx: 1, status: 'done', plan: 'Validate each row', context_in: '', context_out: '',
  agents: [{ name: 'Executor', provider: 'codex', model: '', status: 'done', note: 'Added validation', session_id: null }],
  evaluation: {
    progress_pct: 50, verdict: 'continue', feedback: 'Fix the failing test',
    rationale: 'One criterion is still unmet',
    criteria: [
      { id: 'tests', met: false, evidence: `verify command failed (exit 101)${MARKER}test import::rejects_blank_row ... FAILED` },
      { id: 'msg', met: false, evidence: 'Marked met without evidence, so it counts as not met. The evaluator must cite what it checked.' },
    ],
  },
  tokens_input: 0, tokens_output: 0, cost_usd: 0, started_at: new Date().toISOString(), finished_at: null,
}];

async function mount(page: Page, loop: ReturnType<typeof fixtureLoop>, posts: string[] = []) {
  await page.addInitScript(() => {
    localStorage.setItem('otto_base', location.origin); localStorage.setItem('otto_token', 'fixture');
  });
  await page.route('**/api/v1/**', async (route) => {
    const request = route.request(); const path = new URL(request.url()).pathname;
    if (request.method() === 'POST') posts.push(path);
    if (path === '/api/v1/goal-loops/goal/resume') {
      Object.assign(loop, { status: 'running', error: null, run_started_at: new Date().toISOString() });
      return route.fulfill({ json: loop });
    }
    if (path === '/api/v1/goal-loops/goal') return route.fulfill({ json: { loop, iterations } });
    return route.fulfill({ json: [] });
  });
  await page.goto('/e2e/fixtures/goal-loop-verification.html');
}

test('a loop paused by a restart says so and resumes from the banner', async ({ page }) => {
  const posts: string[] = [];
  await mount(page, fixtureLoop(), posts);
  await expect(page.getByText('Paused by restart', { exact: true })).toBeVisible();
  const banner = page.locator('.restart');
  await expect(banner).toContainText('Otto restarted while this loop was running');
  // The banner replaces the raw error line; the error text is not repeated as a failure.
  await expect(page.locator('.errline')).toHaveCount(0);
  await banner.getByRole('button', { name: 'Resume' }).click();
  await expect.poll(() => posts).toContain('/api/v1/goal-loops/goal/resume');
  await expect(page.getByText('Paused by restart', { exact: true })).toHaveCount(0);
});

test('a person’s pause is not reported as a restart', async ({ page }) => {
  await mount(page, fixtureLoop({ error: null }));
  await expect(page.locator('.cap-note')).toBeVisible();
  await expect(page.locator('.restart')).toHaveCount(0);
  await expect(page.getByText('Paused by restart', { exact: true })).toHaveCount(0);
});

test('the 4 h run cap is visible, and called out when it is the binding limit', async ({ page }) => {
  await mount(page, fixtureLoop({ error: null, limits: { max_iterations: 5, max_runtime_secs: 8 * 3600, per_phase_timeout_secs: 600, max_attempts_per_executor: 3 } }));
  const note = page.locator('.cap-note');
  await expect(note).toContainText('a run stops after 4 h of active time');
  await expect(note).toHaveClass(/binds/);
});

test('an iteration shows the evidence behind each verdict, including verify output', async ({ page }) => {
  await mount(page, fixtureLoop({ error: null }));
  const list = page.getByRole('list', { name: 'Criteria and evidence' });
  await expect(list).toContainText('verify command failed (exit 101)');
  await expect(list).toContainText('Marked met without evidence');
  await expect(page.getByText('One criterion is still unmet')).toBeVisible();
  const output = list.locator('details', { hasText: 'Command output' });
  await expect(output.locator('pre')).toBeHidden();
  await output.locator('summary').click();
  await expect(output.locator('pre')).toContainText('rejects_blank_row ... FAILED');

  // Phone width: the banner, note and output fit without a sideways scroll.
  await page.setViewportSize({ width: 390, height: 844 });
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});
