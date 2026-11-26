// The film's Workflows stills and clip (scripts/capture-workflows.mjs runs this
// spec with the UI's Playwright against the ISOLATED e2e daemon of
// ui/e2e/global-setup.ts: temp data dir, its own port, the fake agent CLIs on
// its PATH). Every run is real: workflows built with ui/e2e/workflowsFixture.ts,
// their agent steps acted out by the fakes' OTTO_FAKE: scripts, the review's
// reviewers by the fake review persona. Invented names only.
import { test, expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { agentStep, cancelStarted, chain, createWorkflow, node, startRun, waitRun, settled, needsYou, wfApi, type WfApi } from 'UI_E2E/workflowsFixture';
import { openRun, runRow, stepRow, selectStep } from 'UI_E2E/workflowsUi';
// @ts-expect-error plain ESM helper of the tour pipeline
import { record, CURSOR_SCRIPT } from 'TOUR_LIB/recorder.mjs';

const FILM = process.env.OTTO_TOUR_FILM_DIR!;
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const snap = async (page: Page, name: string) => {
  await page.mouse.move(-20, -20).catch(() => {});
  await sleep(600);
  await page.screenshot({ path: join(FILM, name + '.jpg'), type: 'jpeg', quality: 90 });
  console.log('[film] ' + name);
};
async function glide(page: Page, loc: ReturnType<Page['locator']>, click = true) {
  const b = (await loc.boundingBox())!;
  await page.mouse.move(b.x + b.width / 2, b.y + b.height / 2, { steps: 24 });
  await sleep(250);
  if (click) await page.mouse.click(b.x + b.width / 2, b.y + b.height / 2);
}

let api: WfApi;
const ids: Record<string, string> = {};

/** acme-storefront: a small JS shop with a float bug in its totals. */
function seedRepo(): string {
  const dir = join(process.env.TMPDIR!, 'acme-film', 'acme-storefront');
  mkdirSync(join(dir, 'src/checkout'), { recursive: true });
  const w = (f: string, t: string) => writeFileSync(join(dir, f), t);
  w('README.md', '# Acme Storefront\n\nThe shop front: catalog, cart and checkout.\n');
  w('CHANGELOG.md', '# Changelog\n\n## 2.14.0\n- Gift cards at checkout.\n');
  w('src/checkout/totals.mjs', 'export function total(items) {\n  return items.reduce((s, i) => s + i.price * i.qty, 0);\n}\n');
  w('src/checkout/cart.mjs', "export const parse = (row) => ({ sku: row.sku, price: parseFloat(row.price), qty: Number(row.qty) });\n");
  w('src/checkout/totals.test.mjs', "import test from 'node:test';\nimport assert from 'node:assert';\nimport { total } from './totals.mjs';\n\ntest('rounds the total to the cent', () => {\n  assert.strictEqual(total([{ price: 0.1, qty: 3 }]), 0.3);\n});\n");
  const git = (...a: string[]) => execFileSync('git', ['-C', dir, ...a], { stdio: 'ignore' });
  git('init', '-q', '-b', 'main');
  git('config', 'user.email', 'maya@example.com');
  git('config', 'user.name', 'Maya Chen');
  git('add', '-A');
  git('commit', '-q', '-m', 'Checkout: gift cards');
  return dir;
}

/** The PR under review: its branch in a linked worktree with the change. */
function prWorktree(dir: string): Record<string, unknown> {
  const path = join(process.env.TMPDIR!, 'acme-film', 'pr-482');
  execFileSync('git', ['-C', dir, 'worktree', 'add', '-q', '-b', 'fix/checkout-rounding', path], { stdio: 'ignore' });
  writeFileSync(join(path, 'src/checkout/totals.mjs'), 'const cents = (n) => Math.round(n * 100) / 100;\n\nexport function total(items) {\n  return cents(items.reduce((s, i) => s + i.price * i.qty, 0));\n}\n');
  return { msg: 'Review PR #482 · Round checkout totals to the cent', repos: [{ repo: dir, type: 'worktree', name: path, source: 'main' }] };
}

const LENSES = ['correctness', 'security', 'performance', 'architecture', 'tests', 'devex', 'accessibility', 'concurrency', 'error-handling', 'api-contracts', 'data-migrations', 'docs'];

test('film: Workflows', async ({ page, browser }) => {
  test.setTimeout(900_000);
  api = await wfApi('Acme Storefront');
  // The person on screen.
  const me = await api.get<{ user: { id: string } }>('/auth/me');
  const renamed = await api.ctx.patch(api.base + '/api/v1/users/' + me.user.id, { data: { display_name: 'Maya Chen' } });
  expect(renamed.ok(), await renamed.text()).toBeTruthy();
  const repoDir = seedRepo();
  const repo = await api.post<{ id: string }>('/workspaces/' + api.ws + '/repos', { path: repoDir, name: 'acme-storefront' });

  await page.addInitScript((ws) => { localStorage.setItem('otto_workspace', ws as string); }, api.ws);
  await page.addInitScript(CURSOR_SCRIPT);

  // ── The workflows ──────────────────────────────────────────────────────
  const review = { reviewers: LENSES.map((lens, i) => ({ lens, provider: i % 2 ? 'codex' : 'claude' })), mode: 'fan_out', threshold: 0 };
  const pr = chain(
    node('checkout', 'agent_prompt', 'Check out the PR', { provider: 'claude', prompt: 'git fetch, --detach at the merge-base, write repos.json with base_branch.' }),
    // Each reads only the checkout's file: Optimize can start them side by side.
    node('context', 'agent_prompt', 'Gather context', { provider: 'claude', prompt: 'Read step2-check-out-the-pr.md, then gather the context the reviewers need.' }),
    node('gates', 'agent_prompt', 'Run the PR gates', { provider: 'claude', prompt: 'Read step2-check-out-the-pr.md, then run the PR gates and report each one.' }),
    node('cross', 'agent_prompt', 'Cross-repo check', { provider: 'claude', prompt: 'Read step2-check-out-the-pr.md, then check the repositories that use this one.' }),
    node('review', 'review_run', 'Multi-agent review', review),
    agentStep('summary', 'Write the summary'),
  );
  ids.pr = await createWorkflow(api, 'PR review', pr);
  ids.docs = await createWorkflow(api, 'Docs refresh', chain(agentStep('read', 'Read the changes'), agentStep('write', 'Update the guides')));
  ids.deps = await createWorkflow(api, 'Dependency check', chain(agentStep('scan', 'Scan the lockfiles'), agentStep('report', 'Write the report')));
  ids.cases = await createWorkflow(api, 'Jira test cases', chain(agentStep('ticket', 'Read the ticket'), agentStep('cases', 'Write the test cases', ['fail auth'], { retry: { max_attempts: 0 } })));
  ids.notes = await createWorkflow(api, 'Release notes', chain(agentStep('collect', 'Collect merged PRs'), agentStep('confirm', 'Confirm the scope', ['ask Release 2.15 or the 2.14.1 patch?'])));

  // ── Recent: two done, one failed; Needs you: one question ─────────────
  for (const wf of [ids.deps, ids.docs]) await waitRun(api, await startRun(api, wf, { msg: 'Nightly' }), settled);
  ids.failedRun = await startRun(api, ids.cases, { msg: 'ACME-142 · Gift card refunds' });
  await waitRun(api, ids.failedRun, settled);
  ids.askRun = await startRun(api, ids.notes, { msg: 'Draft the 2.15 notes' });
  await waitRun(api, ids.askRun, needsYou);

  // ── A goal: Work fixes the totals each iteration; the cart check stays unmet ──
  const fixed = 'const cents = (n) => Math.round(n * 100) / 100; export function total(items) { return cents(items.reduce((s, i) => s + i.price * i.qty, 0)); }';
  const goal = await api.post<{ workflow: { id: string }; run: { id: string } }>('/workspaces/' + api.ws + '/goals', {
    title: 'Round checkout totals to the cent',
    goal: 'Totals in the cart and at checkout are rounded to the cent, with no float math left in the checkout.',
    criteria: [
      { id: 'tests', text: 'The checkout tests pass', verify: 'command', command: 'node --test src/checkout' },
      { id: 'cart', text: 'No float parsing left in the cart', verify: 'command', command: '! grep -n parseFloat src/checkout/cart.mjs' },
      { id: 'changelog', text: 'The changelog mentions the fix', verify: 'command', command: 'grep -qi rounding CHANGELOG.md' },
    ],
    work: { kind: 'build', repo_id: repo.id },
    budget: { iterations: 6, active_s: 3600 },
    worker: { provider: 'claude', instructions: ['OTTO_FAKE: write src/checkout/totals.mjs ' + fixed, 'OTTO_FAKE: write CHANGELOG.md # Changelog — 2.15.0: rounding — checkout totals are rounded to the cent.', 'OTTO_FAKE: sleep 45000', 'OTTO_FAKE: write $HANDOFF Rounded the totals; the changelog says so.'].join('\n') },
  });
  ids.goalWf = goal.workflow.id;
  ids.goalRun = goal.run.id;

  // ── Two earlier PR reviews (Optimize estimates from three), then the filmed one,
  // whose reviewers take a while (the fake review persona acts out <role>.pre first).
  const prInput = prWorktree(repoDir);
  for (let i = 0; i < 2; i++) expect((await waitRun(api, await startRun(api, ids.pr, prInput), settled, 180_000)).status).toBe('success');
  const meta = JSON.parse(readFileSync(process.env.OTTO_TOUR_DAEMON_META!, 'utf8')) as { dataDir: string };
  writeFileSync(join(meta.dataDir, 'fake-review/reviewer.pre'), 'sleep 26000\n');
  writeFileSync(join(meta.dataDir, 'fake-review/summarizer.pre'), 'sleep 3000\n');

  // ── The 12-reviewer fan-out, on its run page ──────────────────────────
  await page.setViewportSize({ width: 1600, height: 900 });
  ids.prRun = await startRun(api, ids.pr, prInput);
  await waitRun(api, ids.prRun, (r) => r.nodes.some((n) => n.node_id === 'review' && n.status === 'running'), 120_000);
  await openRun(page, ids.pr, ids.prRun, 'review');
  await expect(page.getByTestId('wf-run-orbit')).toBeVisible({ timeout: 20_000 });
  await waitRun(api, ids.prRun, (r) => (r.nodes.find((n) => n.node_id === 'review')?.children?.length ?? 0) >= 12, 60_000);
  await sleep(2500);
  const rec = await record(page);
  await sleep(4000);
  await snap(page, 'wf-fanout');
  // A reviewer's conversation opens beside the list.
  const kid = page.getByTestId('wf-step-child').filter({ hasText: 'security' }).first();
  await glide(page, kid);
  await expect(page.getByTestId('wf-step-conversation')).toBeVisible({ timeout: 20_000 });
  await sleep(3500);
  await rec.stop(join(FILM, 'wf-fanout.mp4'));
  await snap(page, 'wf-reviewer');

  // ── The same run on a phone, then the inbox ───────────────────────────
  const phone = await browser.newContext({ baseURL: process.env.OTTO_TOUR_BASE_URL, storageState: process.env.OTTO_TOUR_STATE, viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true, colorScheme: 'dark' });
  const ph = await phone.newPage();
  await ph.addInitScript((ws) => { localStorage.setItem('otto_workspace', ws as string); }, api.ws);
  await openRun(ph, ids.pr, ids.prRun);
  await sleep(2500);
  await snap(ph, 'wf-phone-run');
  await ph.goto('/#/workflows');
  await expect(runRow(ph, ids.prRun)).toBeVisible({ timeout: 20_000 });
  await sleep(2000);
  await snap(ph, 'wf-phone-inbox');
  await phone.close();
  await page.goto('/#/workflows');
  await expect(page.getByTestId('wf-runs-running').locator('[data-run-id="' + ids.prRun + '"]')).toBeVisible({ timeout: 20_000 });
  await sleep(2500);
  await snap(page, 'wf-inbox');

  // ── A failed run leads with the step's own error ──────────────────────
  await openRun(page, ids.cases, ids.failedRun);
  await expect(page.getByTestId('wf-run-primary')).toHaveAttribute('data-action', 'retry');
  await sleep(1500);
  await snap(page, 'wf-failed');

  // ── The goal run with its criteria ────────────────────────────────────
  await waitRun(api, ids.goalRun, (r) => (r.overview?.goal?.iteration ?? 0) >= 2, 240_000);
  await openRun(page, ids.goalWf, ids.goalRun);
  await expect(page.getByTestId('wf-run-card')).toBeVisible({ timeout: 20_000 });
  await sleep(2500);
  await snap(page, 'wf-goal-run');

  // ── The editor: a step's model and effort ─────────────────────────────
  await page.goto('/#/workflows/' + ids.pr);
  await expect(page.getByTestId('wf-editor-canvas')).toBeVisible({ timeout: 20_000 });
  await sleep(1500);
  await selectStep(page, 'context');
  await page.getByTestId('wf-step-model').click();
  const picker = page.getByTestId('model-picker');
  await expect(picker).toBeVisible();
  await sleep(1200);
  await snap(page, 'wf-editor-model');
  await picker.locator('.mp-row:not([disabled])').filter({ hasNotText: 'Default model' }).first().click();
  await page.keyboard.press('Escape');
  await page.getByTestId('wf-step-effort').click();
  await picker.getByRole('group', { name: 'Reasoning effort' }).getByRole('button', { name: /^high$/i }).click();
  await page.keyboard.press('Escape');
  await page.keyboard.press('Meta+s');
  await sleep(1500);
  await snap(page, 'wf-editor');

  // ── Optimize: before / after from this workflow's own run ─────────────
  await waitRun(api, ids.prRun, settled, 180_000);
  await page.goto('/#/workflows/' + ids.pr + '/optimize');
  await expect(page.getByTestId('wf-opt-page')).toBeVisible({ timeout: 20_000 });
  await sleep(2500);
  await snap(page, 'wf-optimize');

  // ── The Goal form ─────────────────────────────────────────────────────
  await page.goto('/#/workflows');
  await sleep(1500);
  const input = page.getByRole('combobox', { name: /^(Ask Otto or search commands|Search commands)$/ });
  await expect(async () => {
    await page.keyboard.press('Meta+k');
    await expect(input).toBeFocused({ timeout: 1_000 });
  }).toPass({ timeout: 15_000 });
  await page.keyboard.type('New goal');
  await page.getByRole('option').filter({ hasText: 'New goal' }).filter({ hasNotText: 'Ask Otto' }).first().click();
  const form = page.getByTestId('wf-goal-form');
  await expect(form).toBeVisible();
  await page.getByTestId('wf-goal-field-goal').fill('Gift card refunds go back to the card, never to store credit');
  const row = page.getByTestId('wf-goal-criterion-row').first();
  await row.getByLabel(/what must be true/).fill('The refund tests pass');
  await row.getByLabel(/How criterion 1 is checked/).selectOption('command');
  await row.getByLabel(/Command for criterion 1/).fill('node --test src/refunds');
  await form.getByRole('button', { name: 'Add criterion' }).click();
  const second = page.getByTestId('wf-goal-criterion-row').nth(1);
  await second.getByLabel(/what must be true/).fill('The refund email reads well');
  await second.getByLabel(/How criterion 2 is checked/).selectOption('human');
  await second.getByPlaceholder('What you will look at').fill('Open a refund email in the preview');
  // Back to the top of the sheet: the goal, then its criteria.
  await page.getByTestId('wf-goal-field-goal').evaluate((el) => {
    for (let p = el.parentElement; p; p = p.parentElement) if (p.scrollTop) p.scrollTop = 0;
  });
  await sleep(1200);
  await snap(page, 'wf-goal-form');

  await cancelStarted(api);
  for (const id of [ids.goalRun, ids.askRun]) await api.ctx.post(api.base + '/api/v1/workflow-runs/' + id + '/cancel', { data: {} }).catch(() => undefined);
  writeFileSync(join(FILM, 'ids.json'), JSON.stringify(ids, null, 2));
});
