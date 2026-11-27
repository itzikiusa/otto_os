import { test, expect } from '@playwright/test';
import { expectFullyInViewport, expectNoHorizontalOverflow } from './helpers';
import { agentStep, cancelStarted, chain, createWorkflow, fork, reviewStep, reviewTarget, settled, startRun, waitRun, wfApi, type WfApi } from './workflowsFixture';
import { openRun, runRow, runsTab, stepPane, stepRow } from './workflowsUi';

// The run page (ux.md §4, §5, §16; test-plan.md §7, gate candidate) on the
// isolated daemon with fake agents: a 12-reviewer fan-out as named rows and a
// ring with one tick each, a parallel group as one block, the header's one
// primary per state, Retry step, Stop run's confirm, the old link format.

let api: WfApi;
test.beforeAll(async ({}, info) => {
  if (!(info.project.name === 'desktop-browser')) return;
  api = await wfApi();
});
test.afterEach(async () => {
  await cancelStarted(api);
});
test.afterAll(async () => {
  await api?.ctx.dispose();
});

/** The open step's ⋯ menu items. */
async function stepMenu(page: import('@playwright/test').Page): Promise<string[]> {
  await stepPane(page).getByRole('button', { name: 'Step actions' }).click();
  await expect(page.locator('.ctx-menu')).toBeVisible();
  const items = await page.locator('.ctx-menu').getByRole('menuitem').allTextContents();
  await page.keyboard.press('Escape');
  return items.map((t) => t.trim());
}
test.beforeEach(async ({ page }, info) => {
  test.skip(!(info.project.name === 'desktop-browser'), 'not this project');
  await page.addInitScript((ws) => {
    localStorage.setItem('otto_workspace', ws as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, api.ws);
});

test('a wide fan-out: named reviewer rows, one ring tick each, no "Open session" wall', async ({ page }) => {
  // The scope plan keeps at most 10 of the 11 declared lenses; with the
  // summarizer the ring counts every child the step list shows (ux.md §4.3
  // children), one tick each up to 12 (DR-020).
  const wf = await createWorkflow(api, 'E2E Fan-out', chain(agentStep('prep', 'Prepare'), reviewStep('review', 11)));
  const run = await startRun(api, wf, await reviewTarget(api));
  const children = (await waitRun(api, run, settled)).nodes.find((n) => n.node_id === 'review')?.children?.length ?? 0;
  expect(children).toBeGreaterThanOrEqual(11);
  expect(children).toBeLessThanOrEqual(12);
  await openRun(page, wf, run, 'review');
  await expect(page.getByTestId('wf-run-orbit')).toHaveAttribute('data-children', String(children));
  await expect(page.getByTestId('wf-run-orbit')).toHaveAttribute('data-grouped', 'false');
  // Reviewers are named rows (done ones may sit under their "N reviewers done" fold); the summarizer trails.
  const kids = page.getByTestId('wf-step-child');
  expect(await kids.count()).toBeGreaterThan(0);
  for (const k of await kids.all()) await expect(k).toContainText(/lens-\d+|Summarizer/);
  await expect(page.getByRole('button', { name: 'Open session' })).toHaveCount(0);
});

test('a parallel group renders as one block and the steps follow execution order', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Fork 3', fork([agentStep('a', 'Alpha'), agentStep('b', 'Beta'), agentStep('c', 'Gamma')], agentStep('join', 'Join')));
  const run = await startRun(api, wf);
  await waitRun(api, run, settled);
  await openRun(page, wf, run);
  const group = page.getByTestId('wf-step-group');
  await expect(group).toHaveCount(1);
  await expect(group.getByTestId('wf-step')).toHaveCount(3);
  const order = await page.getByTestId('wf-step').evaluateAll((els) => els.map((e) => e.getAttribute('data-node-id')));
  expect(order.indexOf('join')).toBeGreaterThan(Math.max(order.indexOf('a'), order.indexOf('b'), order.indexOf('c')));
});

test('one primary per state: Retry step on a failure (and it retries), Run again… when done', async ({ page }) => {
  // No automatic retry (agent steps retry twice by default, engine.md E3.5):
  // the first attempt's failure fails the run; Retry step's attempt succeeds.
  const failing = await createWorkflow(api, 'E2E Fails once', chain(agentStep('flaky', 'Flaky step', ['fail-attempts 1 provider'], { retry: { max_attempts: 0 } })));
  const run = await startRun(api, failing);
  await waitRun(api, run, settled);
  await openRun(page, failing, run);
  const primary = page.getByTestId('wf-run-primary');
  await expect(primary).toHaveAttribute('data-action', 'retry');
  await expect(primary).toHaveText(/Retry step/);
  // Only a failed step re-runs alone (CR1-044).
  await expect(stepPane(page)).toHaveAttribute('data-node-id', 'flaky');
  expect(await stepMenu(page)).toContain('Retry only this step');
  await primary.click();
  await expect(page.getByTestId('wf-run-page')).toHaveAttribute('data-state', 'succeeded', { timeout: 30_000 });
  // The step itself ran again (not skipped past): its own status is success.
  const retried = (await waitRun(api, run, settled)).nodes.find((n) => n.node_id === 'flaky');
  expect(retried?.status, JSON.stringify(retried?.logs)).toBe('success');
  await expect(page.getByTestId('wf-run-primary')).toHaveAttribute('data-action', 'run-again');
  await stepRow(page, 'flaky').click();
  await expect(stepPane(page).getByTestId('wf-step-error')).toHaveCount(0);
  const items = await stepMenu(page);
  expect(items).toContain('Retry from here');
  expect(items).not.toContain('Retry only this step');
});

test('a second failed step says its own failure in the pane; the card keeps the first (CR1-047)', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Two failures', fork([agentStep('p', 'Provider down', ['fail provider']), agentStep('a', 'Signed out', ['fail auth'])], agentStep('join', 'Join')));
  const run = await startRun(api, wf);
  const done = await waitRun(api, run, settled);
  const first = done.overview?.failed_step?.node_id;
  const other = done.nodes.find((n) => n.status === 'error' && n.node_id !== first);
  expect(first && other, JSON.stringify(done.nodes.map((n) => [n.node_id, n.status]))).toBeTruthy();
  await openRun(page, wf, run);
  await expect(page.getByTestId('wf-run-card')).toHaveAttribute('data-kind', 'error');
  // The card's own step: its failure is the card's, not repeated in the pane.
  await stepRow(page, first!).click();
  await expect(stepPane(page).getByTestId('wf-step-error')).toHaveCount(0);
  await stepRow(page, other!.node_id).click();
  await expect(stepPane(page).getByTestId('wf-step-error')).toContainText(other!.node_id === 'a' ? 'Not logged in' : 'API Error: 500');
});

test('a failed step under another card says its failure in the pane (CR1-047)', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Held failure', chain(agentStep('p', 'Provider down', ['fail provider'], { retry: { max_attempts: 0 } })));
  const run = await startRun(api, wf);
  await waitRun(api, run, settled);
  // A held run (a goal out of checks, DR-101): its card is the hold, so the error card is not shown.
  await page.route(`**/api/v1/workflow-runs/${run}`, async (route) => {
    const res = await route.fetch();
    const real = await res.json();
    const hold = { kind: 'blocked', reason: 'Check failed twice', since: real.finished_at, node_ids: [] };
    await route.fulfill({ response: res, json: { ...real, overview: { ...real.overview, state: 'blocked', hold } } });
  });
  await openRun(page, wf, run, 'p');
  await expect(page.getByTestId('wf-run-card')).toHaveAttribute('data-kind', 'hold');
  await expect(stepPane(page).getByTestId('wf-step-error')).toContainText('API Error: 500');
});

test('a live step thread is read-only and offers neither an agent restart nor Open in Threads (CR2-009)', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Live step thread', chain(agentStep('slow', 'Slow step', ['sleep 60000'])));
  const run = await startRun(api, wf);
  const live = await waitRun(api, run, (r) => !!r.nodes.find((n) => n.node_id === 'slow')?.thread?.thread_id);
  const thread = live.nodes.find((n) => n.node_id === 'slow')!.thread!.thread_id;
  // The workspace's servers now differ from the ones the step's agent started with…
  await page.route(`**/api/v1/workspaces/${api.ws}/mcp-servers`, (route) => route.fulfill({ json: [{ name: 'late-probe', enabled: true }] }));
  // …and the step's agent opened a thread of its own.
  await api.post(`/runtime-sessions/${thread}/children`, { prompt: 'hi', targets: [{ provider: 'claude' }], title: 'Step subagent' });
  await openRun(page, wf, run, 'slow');
  const conv = page.getByTestId('wf-step-conversation');
  await expect(conv.getByTestId('embedded-thread-header')).toBeVisible({ timeout: 20_000 });
  await expect(conv.getByTestId('runtime-composer')).toHaveCount(0);
  await expect(conv.getByTestId('embedded-thread-open')).toHaveCount(0);
  await conv.getByTestId('runtime-linked').click({ timeout: 20_000 });
  const list = page.getByRole('list', { name: 'Subagents' });
  await expect(list).toContainText('Step subagent');
  await expect(list.getByRole('button', { name: /in Threads/i })).toHaveCount(0);
  await page.keyboard.press('Escape');
  await expect(conv.getByTestId('runtime-mcp-changed')).toHaveCount(0);
});

test('live agent-opened sub-thread row and opening its card in the pane stay read-only', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Live sub-thread', chain(agentStep('work', 'Work step', ['child codex sleep 60000', 'sleep 60000'])));
  const run = await startRun(api, wf);
  await waitRun(api, run, (r) => r.nodes.some((n) => n.node_id === 'work' && n.status === 'running' && !!n.thread?.thread_id));
  await openRun(page, wf, run, 'work');

  const conv = page.getByTestId('wf-step-conversation');
  await expect(conv).toBeVisible({ timeout: 20_000 });
  // Workflow conversations stay strictly read-only with no Open in Threads button.
  await expect(conv.getByTestId('runtime-composer')).toHaveCount(0);
  await expect(conv.getByTestId('embedded-thread-open')).toHaveCount(0);
  // The sub-thread appears as a live row under the step while both are still running.
  const childRow = page.getByTestId('wf-step-child').first();
  await expect(childRow).toBeVisible({ timeout: 25_000 });
  await expect(childRow).toHaveAttribute('data-status', 'running');
  await expect(page.getByTestId('wf-run-page')).toHaveAttribute('data-state', 'running');

  // Clicking the sub-thread card inside the conversation opens it in the same pane and selects its row.
  const childCardOpen = conv.locator('[data-testid^="runtime-child-"] [data-testid="runtime-item-open"]').first();
  await expect(childCardOpen).toBeVisible({ timeout: 20_000 });
  const childThreadId = await childRow.getAttribute('data-child-id');
  expect(childThreadId).toBeTruthy();
  await childCardOpen.click();
  await expect(stepPane(page)).toHaveAttribute('data-child-id', childThreadId!, { timeout: 15_000 });
  await expect(page.locator(`[data-testid="wf-step-child"][data-child-id="${childThreadId}"]`)).toHaveAttribute('aria-current', 'true');
  await expect(page.getByTestId('wf-crumb-parent')).toBeVisible();

  // Back crumb returns to the parent step.
  await page.getByTestId('wf-crumb-parent').click();
  await expect(stepPane(page)).not.toHaveAttribute('data-child-id', /.+/);
  await expect(stepRow(page, 'work')).toHaveAttribute('aria-current', 'true');
});

test('Stop run… asks first; Stop is never beside a primary', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Long step', chain(agentStep('slow', 'Slow step', ['sleep 60000'])));
  const run = await startRun(api, wf);
  await waitRun(api, run, (r) => r.status === 'running');
  await openRun(page, wf, run);
  await expect(page.getByTestId('wf-run-primary')).toHaveCount(0);
  await page.getByTestId('wf-run-stop').click();
  const dialog = page.getByRole('dialog', { name: 'Stop run?' });
  await expect(dialog).toBeVisible();
  await expectFullyInViewport(page, dialog);
  await dialog.getByRole('button', { name: 'Stop run' }).click();
  await expect(page.getByTestId('wf-run-page')).toHaveAttribute('data-state', 'canceled', { timeout: 30_000 });
});

test('picking Files or Output on a live run stays on that tab without a step in the address', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Live tab stay', chain(agentStep('slow', 'Slow step', ['sleep 60000'])));
  const run = await startRun(api, wf);
  await waitRun(api, run, (r) => r.status === 'running' && (r.overview?.state ?? 'running') === 'running' && r.nodes.some((n) => n.node_id === 'slow' && n.status === 'running'));
  await openRun(page, wf, run);
  await expect(stepPane(page)).toHaveAttribute('data-node-id', 'slow', { timeout: 20_000 });

  const orbitTime = page.getByTestId('wf-run-orbit-time');
  await expect(orbitTime).toHaveAttribute('aria-label', /^Running for /);
  await expect(orbitTime).toHaveAttribute('title', /^Running for /);
  await expect(orbitTime).toContainText('elapsed');

  const filesTab = page.getByTestId('wf-step-tab-files');
  await filesTab.click();
  await expect(filesTab).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#wf-step-panel-files')).toBeVisible();
  await page.waitForTimeout(3000); // ui-guards: allow — absence: proves the Files tab does not reset across live ticks
  await expect(page.getByTestId('wf-run-page')).toHaveAttribute('data-state', 'running');
  await expect(filesTab).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#wf-step-panel-files')).toBeVisible();

  const outputTab = page.getByTestId('wf-step-tab-output');
  await outputTab.click();
  await expect(outputTab).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#wf-step-panel-output')).toBeVisible();
  await page.waitForTimeout(3000); // ui-guards: allow — absence: proves the Output tab does not reset across live ticks
  await expect(page.getByTestId('wf-run-page')).toHaveAttribute('data-state', 'running');
  await expect(outputTab).toHaveAttribute('aria-selected', 'true');
  await expect(page.locator('#wf-step-panel-output')).toBeVisible();
});

test('the old link format and the inbox row both land on the run page', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Link', chain(agentStep('one', 'One step')));
  const run = await startRun(api, wf);
  await waitRun(api, run, settled);
  await page.goto(`/#/workflows/${wf}/runs/${run}`);
  await expect(page.locator(`[data-testid="wf-run-page"][data-run-id="${run}"]`)).toBeVisible({ timeout: 20_000 });
  await runsTab(page);
  await runRow(page, run).getByRole('button').first().click();
  await expect(page).toHaveURL(new RegExp(`#/workflows/${wf}/runs/${run}$`));
  await expectNoHorizontalOverflow(page);
});

test('a settled step\'s conversation is read-only: no Revive, Stop or limit card (CR1-038)', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Read-only conversation', chain(agentStep('one', 'One step')));
  const run = await startRun(api, wf);
  await waitRun(api, run, settled);
  await openRun(page, wf, run, 'one');
  await expect(page.getByTestId('wf-step-conversation')).toBeVisible({ timeout: 20_000 });
  // The step persona's answer (fake_script.rs): the real thread, read-only once settled.
  await expect(page.getByTestId('wf-step-conversation').getByText('step done').first()).toBeVisible();
  await expect(page.getByTestId('wf-step-conversation').getByTestId('runtime-composer')).toHaveCount(0);
  for (const id of ['runtime-closed', 'runtime-reopen', 'runtime-background', 'runtime-background-stop-all']) await expect(page.getByTestId(id)).toHaveCount(0);
});

test('switching from a step whose thread is gone to a live one shows the live conversation (CR1-039)', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Gone then live', chain(agentStep('gone', 'Gone step'), agentStep('kept', 'Kept step')));
  const run = await startRun(api, wf);
  const done = await waitRun(api, run, settled);
  const goneThread = done.nodes.find((n) => n.node_id === 'gone')?.thread?.thread_id;
  expect(goneThread, 'the step ran on a thread').toBeTruthy();
  await page.route(`**/api/v1/runtime-sessions/${goneThread}**`, (r) => r.fulfill({ status: 404, json: { error: 'not found' } }));
  await openRun(page, wf, run, 'gone');
  await expect(page.getByTestId('wf-step-digest')).toBeVisible({ timeout: 20_000 });
  await stepRow(page, 'kept').click();
  await expect(page.getByTestId('wf-step-conversation')).toBeVisible({ timeout: 20_000 });
  await expect(page.getByTestId('wf-step-digest')).toHaveCount(0);
});

test('files sent with the message show under Result → Input: a text one previews, any downloads (CR1-043)', async ({ page }) => {
  const wf = await createWorkflow(api, 'E2E Attachments', chain(agentStep('one', 'One step')));
  const run = await startRun(api, wf, { msg: 'see the files' });
  await waitRun(api, run, settled);
  await page.route(`**/api/v1/workflow-runs/${run}`, async (route) => {
    const res = await route.fetch();
    const real = await res.json();
    // An outside link only when it is http(s) (CR1-077).
    const overview = { ...real.overview, deliverables: [{ label: 'Posted the summary', state: 'done', url: 'https://example.invalid/pr/1' }, { label: 'Notified the thread', state: 'done', url: 'javascript:alert(1)' }] };
    const body = { ...real, overview, attachments: [
      { name: 'trace.log', mime: 'text/plain', size_bytes: 2048, path: 'attachments/trace.log', source: 'start' },
      { name: 'screen.png', mime: 'image/png', source: 'steer', error: 'too large' },
      // Two files of one name: the daemon keeps the name and gives each its own path (CR2-012).
      { name: 'shot.png', mime: 'image/png', size_bytes: 1024, path: 'attachments/shot.png', source: 'start' },
      { name: 'shot.png', mime: 'image/png', size_bytes: 1024, path: 'attachments/shot-2.png', source: 'steer' },
    ] };
    await route.fulfill({ response: res, json: body });
  });
  // The run's context read (api.md: `…/context/read?path=…[&download=1]`).
  const reads: string[] = [];
  await page.route(`**/api/v1/workflow-runs/${run}/context/read?*`, async (route) => {
    reads.push(route.request().url());
    if (route.request().url().includes('download=1')) await route.fulfill({ status: 200, contentType: 'text/plain', body: 'line one\nline two' });
    else await route.fulfill({ json: { path: 'attachments/trace.log', content: 'line one\nline two', language: 'text', truncated: false } });
  });
  await openRun(page, wf, run);
  const card = page.getByTestId('wf-run-card');
  await expect(card).toHaveAttribute('data-kind', 'delivered');
  await expect(card.locator('a')).toHaveCount(1);
  await expect(card.locator('a')).toHaveAttribute('href', 'https://example.invalid/pr/1');
  await page.getByTestId('wf-step-tab-input').click();
  await expect(page.getByTestId('wf-run-attachment')).toHaveCount(4);
  const failed = page.locator('[data-testid="wf-run-attachment"][data-error="true"]');
  await expect(failed).toContainText('not downloaded');
  await expect(failed.getByRole('button')).toHaveCount(0);
  const trace = page.locator('[data-testid="wf-run-attachment"]').filter({ hasText: 'trace.log' });
  await trace.getByRole('button', { name: 'Preview' }).click();
  await expect(page.getByTestId('wf-run-attachment-preview')).toContainText('line two');
  await trace.getByRole('button', { name: 'Download' }).click();
  await expect.poll(() => reads.some((u) => u.includes('path=attachments%2Ftrace.log') && u.includes('download=1'))).toBe(true);
});

test('live conversation opens at bottom, follows tail, and holds scrollTop steady for 3s when scrolled up', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const script: string[] = [
    ...Array.from({ length: 28 }, (_, i) => `reply Initial item ${i + 1}: ${'verifying workflow conversation tail anchoring '.repeat(6)}`),
    ...Array.from({ length: 30 }, (_, i) => ['sleep 350', `reply Live arrival ${i + 1}: ${'streaming live workflow step output '.repeat(5)}`]).flat(),
    'sleep 60000',
  ];
  const wf = await createWorkflow(api, 'E2E Conversation tail', chain(agentStep('stream', 'Streaming step', script)));
  const run = await startRun(api, wf);
  await waitRun(api, run, (r) => r.nodes.some((n) => n.node_id === 'stream' && n.status === 'running' && !!n.thread?.thread_id));
  await openRun(page, wf, run, 'stream');

  const conv = page.getByTestId('wf-step-conversation');
  const scroller = conv.locator('.rt-scroll');
  await expect(conv.getByText('Initial item 28:', { exact: false })).toBeAttached({ timeout: 25_000 });

  // Single scroll container: `.pane-col` never overflows vertically while showing a live thread.
  const outerOverflow = await page.locator('.pane-col').evaluate((el) => el.scrollHeight - el.clientHeight);
  expect(outerOverflow, 'outer .pane-col should not scroll when showing the live conversation').toBeLessThanOrEqual(1);

  // Opens at the bottom and stays pinned to the bottom as live items arrive.
  await expect.poll(() => scroller.evaluate((el) => el.scrollHeight - el.clientHeight)).toBeGreaterThan(200);
  await expect.poll(() => scroller.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(40);
  const heightWhileFollowing = await scroller.evaluate((el) => el.scrollHeight);
  await expect.poll(() => scroller.evaluate((el) => el.scrollHeight), { timeout: 10_000 }).toBeGreaterThan(heightWhileFollowing);
  await expect.poll(() => scroller.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(40);

  // User scrolls up with the wheel: scrollTop must not move for 3 seconds while new items keep arriving.
  const box = (await scroller.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, -600);
  await page.mouse.wheel(0, -600);
  await expect.poll(() => scroller.getAttribute('data-at-bottom')).toBe('0');
  const pinnedTop = await scroller.evaluate((el) => el.scrollTop);
  const beforeHeight = await scroller.evaluate((el) => el.scrollHeight);
  expect(pinnedTop).toBeGreaterThan(0);

  await page.waitForTimeout(3000); // ui-guards: allow — absence: proves scrollTop stays put for 3s while live items arrive
  const after = await scroller.evaluate((el) => ({ top: el.scrollTop, height: el.scrollHeight, atBottom: el.dataset.atBottom }));
  expect(after.height, 'live items arrived during the 3s hold').toBeGreaterThan(beforeHeight);
  expect(after.atBottom).toBe('0');
  expect(Math.abs(after.top - pinnedTop), 'scrollTop did not jump while scrolled up').toBeLessThanOrEqual(1);

  // The follow pill returns smoothly to the latest line.
  await conv.getByTestId('runtime-new-pill').click();
  await expect.poll(() => scroller.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(40);
});

test('long conversation folds like Threads, loads newest page first, and anchors scroll when loading older rows', async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  const wf = await createWorkflow(api, 'E2E Paged conversation', chain(agentStep('one', 'One step')));
  const run = await startRun(api, wf);
  const done = await waitRun(api, run, settled);
  const threadId = done.nodes.find((n) => n.node_id === 'one')?.thread?.thread_id;
  expect(threadId, 'step ran on a runtime thread').toBeTruthy();

  const TOTAL_TURNS = 60;
  const T0 = Date.parse('2026-11-26T10:00:00Z');
  const at = (s: number) => new Date(T0 + s * 1000).toISOString();
  const buildPage = (fromOrd: number, toOrd: number) => {
    const turns = [];
    const turn_items = [];
    let ord = (fromOrd - 1) * 5 + 1;
    for (let t = fromOrd; t <= toOrd; t++) {
      const tid = `turn-${t}`;
      turns.push({ id: tid, ordinal: t, status: 'completed', started_at: at(t * 20), completed_at: at(t * 20 + 15), origin: 'user' });
      turn_items.push(
        { id: `u-${t}`, turn_id: tid, ordinal: ord++, status: 'completed', title: null, type: 'user_message', text: `Question number ${t}`, started_at: at(t * 20), completed_at: at(t * 20), updated_at: at(t * 20) },
        { id: `c1-${t}`, turn_id: tid, ordinal: ord++, status: 'completed', title: `rg turn_${t}`, type: 'command_execution', input: `rg turn_${t}`, exit_code: 0, detail_omitted: true, started_at: at(t * 20 + 1), completed_at: at(t * 20 + 4), updated_at: at(t * 20 + 4) },
        { id: `c2-${t}`, turn_id: tid, ordinal: ord++, status: 'completed', title: `cargo check ${t}`, type: 'command_execution', input: `cargo check ${t}`, exit_code: 0, detail_omitted: true, started_at: at(t * 20 + 5), completed_at: at(t * 20 + 9), updated_at: at(t * 20 + 9) },
        { id: `a-${t}`, turn_id: tid, ordinal: ord++, status: 'completed', title: null, type: 'assistant_message', message_id: `m-${t}`, streaming: false, text: `Answer for turn ${t}: ${'detailed workflow verification prose '.repeat(5)}`, started_at: at(t * 20 + 10), completed_at: at(t * 20 + 15), updated_at: at(t * 20 + 15) },
      );
    }
    return {
      provider_session: { status: 'ready', cwd: '/tmp', model: null, updated_at: at(TOTAL_TURNS * 20 + 20), last_error: null },
      provider_thread: null,
      turns,
      turn_items,
      runtime_requests: [],
      plans: [],
      has_older: fromOrd > 1,
      oldest_turn_ordinal: fromOrd,
      older_turns: fromOrd - 1,
    };
  };

  const eventUrls: string[] = [];
  const itemFetches: string[] = [];
  await page.route(`**/api/v1/runtime-sessions/${threadId}/events**`, (route) => {
    const u = new URL(route.request().url());
    eventUrls.push(u.pathname + u.search);
    const before = Number(u.searchParams.get('before_turn_ordinal') ?? '0');
    const snapshot = before > 1 ? buildPage(Math.max(1, before - 40), before - 1) : buildPage(TOTAL_TURNS - 39, TOTAL_TURNS);
    return route.fulfill({ json: { kind: 'snapshot', snapshot_sequence: 1000, snapshot } });
  });
  await page.route(`**/api/v1/runtime-sessions/${threadId}/items/**`, (route) => {
    itemFetches.push(route.request().url());
    return route.fulfill({ status: 200, json: { item: { id: 'noop' } } });
  });

  await openRun(page, wf, run, 'one');
  const conv = page.getByTestId('wf-step-conversation');
  const scroller = conv.locator('.rt-scroll');
  await expect(conv.getByText('Answer for turn 60:', { exact: false })).toBeVisible({ timeout: 20_000 });

  // Uses Threads' exact tail budget (`tail_turns=40&max_lines=10000&compact=true`), folds finished work, and fires 0 unsolicited item hydrations.
  expect(eventUrls.some((u) => u.includes('tail_turns=40&max_lines=10000&compact=true'))).toBe(true);
  expect(itemFetches).toHaveLength(0);
  await expect(conv.locator('details.rf').first()).toBeVisible();
  await expect(conv.locator('.rf-body')).toHaveCount(0);
  await expect(conv.getByText('Answer for turn 15:', { exact: false })).toHaveCount(0);

  // Opens at the bottom with no outer scrollbar.
  await expect.poll(() => scroller.evaluate((el) => el.scrollHeight - el.scrollTop - el.clientHeight)).toBeLessThan(40);
  const outerOverflow = await page.locator('.pane-col').evaluate((el) => el.scrollHeight - el.clientHeight);
  expect(outerOverflow).toBeLessThanOrEqual(1);

  // Scrolling to the top triggers loadOlder and keeps the first visible row anchored in place.
  const beforeTop21 = await scroller.evaluate((el) => {
    el.scrollTop = 40;
    const q21 = Array.from(el.querySelectorAll('*')).find((n) => n.childElementCount === 0 && n.textContent?.trim() === 'Question number 21');
    return q21 ? q21.getBoundingClientRect().top : NaN;
  });
  expect(Number.isFinite(beforeTop21)).toBe(true);
  await expect(conv.getByText('Answer for turn 15:', { exact: false })).toBeAttached({ timeout: 10_000 });
  expect(eventUrls.some((u) => u.includes('before_turn_ordinal=21&limit_turns=40&max_lines=10000&compact=true'))).toBe(true);
  const scrolledDown = await scroller.evaluate((el) => el.scrollTop);
  expect(scrolledDown, 'scrollTop shifted downward when older turns were prepended').toBeGreaterThan(240);
  const afterTop21 = await conv.getByText('Question number 21', { exact: true }).evaluate((el) => el.getBoundingClientRect().top);
  expect(Math.abs(afterTop21 - beforeTop21), 'first visible row stayed anchored within 4px across prepend').toBeLessThanOrEqual(4);
});
