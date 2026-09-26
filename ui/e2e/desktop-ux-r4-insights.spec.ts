import { test, expect, type Page } from '@playwright/test';
import { assistantState, mockAssistant } from './assistant-fixture';
import { expectNoHorizontalOverflow } from './helpers';
import { apiCtx, seedWorkspace } from './seed';
test.use({ serviceWorkers: 'block', actionTimeout: 10_000 });
test.describe.configure({ timeout: 120_000 });
const report = { kind: 'daily', period_start: '2026-09-24', period_end: '2026-09-24', summary: '# Original report', html_path: '', created_at: '2026-09-25T08:00:00Z' };
async function usageFixture(page: Page) {
  await page.route('**/api/v1/usage/**', r => {
    const path = new URL(r.request().url()).pathname;
    if (path.endsWith('/status')) return r.fulfill({ json: { available: true, enabled: true, retention_days: 180, metrics_interval_secs: 60, usage_rows: 0, metric_rows: 0, disk_bytes: 0 } });
    if (path.endsWith('/summary')) return r.fulfill({ json: { days: 30, total_events: 0, total_input_tokens: 0, total_output_tokens: 0, total_cache_read_tokens: 0, total_cache_write_tokens: 0, total_tokens: 0, total_cost_usd: 0, providers: [], daily: [], sessions: [], by_kind: [] } });
    if (path.endsWith('/budgets')) return r.fulfill({ json: { config: { enforce: false, block_on_exceed: false, window_days: 30, providers: [], workspaces: [] }, window_days: 30, rows: [] } });
    return r.fulfill({ json: [] });
  });
}
test('Insights waits for initial report identity before allowing generation', async ({ page }) => {
  let release!: () => void;
  await mockAssistant(page, assistantState());
  await page.goto('/#/assistant/memory');
  await expect(page.getByLabel('Profile (markdown)')).toBeVisible();
  await page.route('**/api/v1/insights/reports', async r => { await new Promise<void>(resolve => { release = resolve; }); return r.fulfill({ json: [report] }); });
  await page.evaluate(() => { location.hash = '#/insights'; });
  await expect.poll(() => !!release).toBe(true);
  await expect(page.getByRole('button', { name: 'Run now', exact: true })).toBeDisabled();
  release();
  await expect(page.getByRole('button', { name: 'Run now', exact: true })).toBeEnabled();
});
test('Usage duplicate scope caps are explained before saving', async ({ page }) => {
  await usageFixture(page); await page.goto('/#/usage');
  await page.getByRole('button', { name: 'Edit budgets', exact: true }).click();
  for (let i = 0; i < 2; i++) {
    await page.getByRole('button', { name: 'Add provider cap' }).click();
    await page.getByRole('combobox', { name: 'Provider', exact: true }).nth(i).selectOption('codex');
    await page.getByLabel('Cap in US dollars').nth(i).fill(String(25 + i));
    await page.getByLabel('Cap in US dollars').nth(i).blur();
  }
  await expect(page.getByRole('button', { name: 'Save budgets', exact: true })).toBeDisabled();
  await expect(page.getByRole('alert')).toContainText('one cap per provider');
  await page.getByRole('button', { name: 'Remove this provider cap' }).last().click();
  await expect(page.getByRole('button', { name: 'Save budgets', exact: true })).toBeEnabled();
});
test('Assistant attachment-only failed send restores files and partial upload names', async ({ page }) => {
  const state = assistantState(); await mockAssistant(page, state);
  await page.route('**/api/v1/assistant/threads/th-personal/attachments', async r => {
    const body = r.request().postDataJSON();
    if (body.name.startsWith('bad-')) return r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic upload failure' } });
    return r.fallback();
  });
  await page.goto('/#/assistant/th-personal');
  await page.locator('input[type=file]').setInputFiles(['bad-first.txt', 'good.txt', 'bad-last.txt'].map(name => ({ name, mimeType: 'text/plain', buffer: Buffer.from('synthetic') })));
  await expect(page.locator('.composer .fname')).toHaveText('good.txt');
  await expect(page.locator('.composer').getByRole('alert')).toContainText('bad-first.txt');
  await expect(page.locator('.composer').getByRole('alert')).toContainText('bad-last.txt');
  state.fail['/assistant/threads/th-personal/turns'] = 503;
  await page.locator('.composer').getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.locator('.composer').getByRole('alert')).toContainText('Couldn’t send');
  await expect(page.locator('.composer .fname')).toHaveText('good.txt');
  await expect(page.getByRole('textbox', { name: 'Message Otto' })).toHaveValue('');
  delete state.fail['/assistant/threads/th-personal/turns'];
  await page.locator('.composer').getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.locator('.composer .fname')).toHaveCount(0);
  await expect(page.locator('.composer').getByRole('alert')).toHaveCount(0);
  await expectNoHorizontalOverflow(page);
});
test('Health support bundle failure leaves download available for retry', async ({ page }) => {
  let fail = true;
  await page.route('**/api/v1/support-bundle', r => r.fulfill(fail ? { status: 503, json: { code: 'upstream', message: 'Synthetic bundle unavailable' } } : { json: { redaction_hits: 0, generated_at: '2026-09-26T00:00:00Z', synthetic: true } }));
  await page.goto('/#/insights/health');
  const download = page.getByRole('button', { name: 'Download support bundle', exact: true });
  await download.click();
  await expect(page.getByText("Couldn't download the support bundle", { exact: true })).toBeVisible();
  await expect(download).toBeEnabled(); fail = false;
  const pending = page.waitForEvent('download'); await download.click();
  expect((await pending).suggestedFilename()).toMatch(/^otto-support-bundle-/);
});

test('Usage auto-refresh cannot restore an old budget after save completes', async ({ page }) => {
  await usageFixture(page);
  let config = { enforce: false, block_on_exceed: false, window_days: 30, providers: [], workspaces: [] };
  let releasePut!: () => void, releaseGet!: () => void, getCount = 0;
  await page.route('**/api/v1/usage/budgets', async r => {
    if (r.request().method() === 'PUT') {
      const next = r.request().postDataJSON();
      await new Promise<void>(resolve => { releasePut = resolve; }); config = next;
      return r.fulfill({ json: { config, window_days: config.window_days, rows: [] } });
    }
    const snapshot = structuredClone(config);
    if (++getCount > 1) await new Promise<void>(resolve => { releaseGet = resolve; });
    return r.fulfill({ json: { config: snapshot, window_days: snapshot.window_days, rows: [] } });
  });
  await page.goto('/#/usage');
  await page.getByRole('button', { name: 'Edit budgets', exact: true }).click();
  await page.clock.install();
  await page.getByRole('button', { name: 'Auto-refresh', exact: true }).click();
  const days = page.getByLabel('Compare spend over the last'); await days.fill('7');
  await page.getByRole('button', { name: 'Save budgets', exact: true }).click();
  await expect.poll(() => !!releasePut).toBe(true);
  await page.clock.runFor(60_050);
  await expect.poll(() => !!releaseGet).toBe(true);
  releasePut(); await expect(page.getByText('Budgets saved', { exact: true })).toBeVisible();
  releaseGet(); await page.waitForTimeout(150);
  await expect(days).toHaveValue('7');
});

test('History long paged transcript supports keyboard search across mounted windows', async ({ page }, info) => {
  const stamp = new Date().toISOString();
  const row = { session_id: null, provider: 'claude', title: 'Synthetic 360 turn release review', first_prompt: 'Review', cwd: '/tmp/synthetic-history', repo_name: 'Synthetic', started_at: stamp, last_active_at: stamp, turns: 360, status: 'on_disk', transcript_path: '/tmp/release.jsonl', resumable: true };
  await page.route('**/api/v1/workspaces/*/history/page?*', r => r.fulfill({ json: { entries: [row], next_cursor: null } }));
  await page.route('**/api/v1/workspaces/*/history/transcript?*', r => {
    const before = new URL(r.request().url()).searchParams.get('before');
    const start = before ? 0 : 300, end = before ? 300 : 360;
    return r.fulfill({ json: { session_id: null, provider: 'claude', title: row.title, cwd: row.cwd, model: null, cursor: String(start), has_earlier: !before, turns: Array.from({ length: end - start }, (_, i) => ({ id: `turn-${i + start}`, role: (i + start) % 2 ? 'assistant' : 'user', ts: stamp, blocks: [{ kind: 'text', md: `Checkpoint ${i + start}: synthetic release review notes. ${'Check dependencies and validate the recovery steps. '.repeat(4)}` }], duration_ms: null, model: null, system: [], reasoning_steps: 0 })), stats: { turns: 360, tool_calls: 0, cost_usd: null, input_tokens: null, output_tokens: null, duration_ms: null, reasoning_steps: 0, thinking_steps: 0, unknown_records: 0 }, subagents: [], unavailable_reason: null } });
  });
  await page.goto('/#/history');
  await page.getByTestId('history-row').click();
  const conv = page.getByTestId('history-conversation');
  await expect(conv).toContainText('Checkpoint 359:');
  const earlier = conv.getByRole('button', { name: 'Load earlier', exact: true });
  // Keyboard focus scrolls the earlier button into view, triggering the
  // conversation's documented automatic pagination at the top.
  await earlier.focus();
  await expect(conv).toContainText('Checkpoint 0:');
  await conv.getByRole('button', { name: 'Search this conversation', exact: true }).click();
  const search = conv.getByRole('textbox', { name: 'Search this conversation', exact: true });
  await expect(search).toBeFocused();
  await search.fill('Checkpoint 359:'); await search.press('Enter');
  await expect(conv.locator('[data-turn-id="turn-359"]')).toBeInViewport();
  await search.fill('Checkpoint 0:'); await search.press('Enter');
  await expect(conv.locator('[data-turn-id="turn-0"]')).toBeInViewport();
  await page.screenshot({ path: info.outputPath('history-long-keyboard.png') });
  await expectNoHorizontalOverflow(page);
});

test('Assistant incremental turns remain readable and running task cancellation recovers', async ({ page }, info) => {
  const state = assistantState(); const taskIndex = state.tasks.findIndex(t => t.id === 'task-hotels'); state.tasks[taskIndex] = { ...state.tasks[taskIndex], result: null, title: 'Synthetic long research', detail: 'Reviewing synthetic notes' };
  await mockAssistant(page, state);
  let send!: (data: string) => void;
  await page.routeWebSocket('**/ws/events**', socket => { const server = socket.connectToServer(); send = data => socket.send(data); socket.onMessage(data => server.send(data)); server.onMessage(data => socket.send(data)); });
  await page.goto('/#/assistant/th-personal');
  await expect.poll(() => !!send).toBe(true);
  const base = state.turns['th-personal'].find(t => t.role === 'assistant')!;
  const thread = { ...state.threads[0], updated_at: new Date().toISOString() };
  for (const text of ['Research started.', 'Research started. The dependency review is complete.']) {
    send(JSON.stringify({ type: 'assistant_turn', thread_id: thread.id, thread, turn: { ...base, id: 'incremental', text, created_at: new Date().toISOString() } }));
    await expect(page.getByTestId('assistant-thread')).toContainText(text);
  }
  const task = page.getByTestId('card-task').filter({ hasText: 'Synthetic long research' });
  await task.getByRole('button', { name: 'Stop', exact: true }).click();
  await page.getByRole('dialog', { name: 'Stop task' }).getByRole('button', { name: 'Cancel', exact: true }).click();
  await expect(task.getByRole('button', { name: 'Stop', exact: true })).toBeEnabled();
  await task.getByRole('button', { name: 'Stop', exact: true }).click();
  await page.getByRole('dialog', { name: 'Stop task' }).getByRole('button', { name: 'Stop', exact: true }).click();
  await expect(task).toContainText('Cancelled');
  expect(state.calls.some(c => c.path.endsWith('/task-hotels/cancel'))).toBe(true);
  await task.scrollIntoViewIfNeeded();
  await page.screenshot({ path: info.outputPath('assistant-cancelled.png') });
});

test('History changing scope ignores a delayed first page', async ({ page }) => {
  const a = await apiCtx(); const wid = await seedWorkspace(a.ctx, a.base); await a.ctx.dispose();
  await page.addInitScript(wid => localStorage.setItem('otto_workspace', wid), wid);
  await mockAssistant(page, assistantState());
  await page.goto('/#/assistant/memory');
  let release!: () => void, calls = 0;
  const stamp = new Date().toISOString();
  const row = { session_id: null, provider: 'claude', title: 'Current synthetic scope', first_prompt: 'Review', cwd: '/tmp/synthetic-history', repo_name: 'Synthetic', started_at: stamp, last_active_at: stamp, turns: 0, status: 'on_disk', transcript_path: '/tmp/current.jsonl', resumable: true };
  await page.route('**/api/v1/workspaces/*/history/page?*', async r => {
    const first = ++calls === 1;
    if (first) await new Promise<void>(resolve => { release = resolve; });
    return r.fulfill({ json: { entries: [{ ...row, title: first ? 'Stale prior scope' : row.title }], next_cursor: null } });
  });
  await page.evaluate(() => { location.hash = '#/history'; });
  await expect.poll(() => !!release).toBe(true);
  await page.getByRole('button', { name: 'No workspace', exact: true }).click();
  await expect(page.getByTestId('history-row')).toContainText(row.title);
  release(); await page.waitForTimeout(200);
  await expect(page.getByTestId('history-row')).not.toContainText('Stale prior scope');
});

test('Insights report timeout releases generation and retry detects the new result', async ({ page }) => {
  let polls = 0, ready = false;
  await page.route('**/api/v1/insights/reports', r => { polls++; return r.fulfill({ json: [{ ...report, ...(ready ? { summary: '# Ready after retry', created_at: '2026-09-26T09:00:00Z' } : {}) }] }); });
  await page.route('**/api/v1/insights/run', r => r.fulfill({ json: { started: true, run_id: 'synthetic-run', report_key: 'daily:20260924_20260924' } }));
  await page.goto('/#/insights');
  await expect(page.getByTestId('insight-report')).toContainText('Original report');
  await page.clock.install();
  await page.getByRole('button', { name: 'Run now', exact: true }).click();
  await expect(page.getByRole('button', { name: 'Running…', exact: true })).toBeDisabled();
  for (let i = 0; i < 100; i++) {
    const before = polls; await page.clock.runFor(3100);
    await expect.poll(() => polls).toBeGreaterThan(before);
    // Flush the fulfilled response before advancing the next recursive timer.
    await page.waitForTimeout(20);
  }
  await expect(page.getByText('Still generating the insights report', { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Run now', exact: true })).toBeEnabled();
  await page.getByRole('button', { name: 'Run now', exact: true }).click(); ready = true;
  await expect(page.getByRole('button', { name: 'Running…', exact: true })).toBeDisabled();
  await page.clock.runFor(3100);
  await expect(page.getByTestId('insight-report')).toContainText('Ready after retry');
});

for (const [theme, scheme, width, dir] of [['native', 'light', 1440, 'ltr'], ['native', 'dark', 1440, 'ltr'], ['warm', 'dark', 834, 'rtl']] as const) {
  test(`Insights findings fill available width coherently ${theme} ${scheme} ${width}`, async ({ page }, info) => {
    await page.addInitScript(({ theme, scheme, dir }) => { localStorage.setItem('otto_theme', theme); localStorage.setItem('otto_scheme', scheme); localStorage.setItem('otto_direction', dir); }, { theme, scheme, dir });
    await page.setViewportSize({ width, height: 1000 });
    await page.route('**/api/v1/insights/reports', r => r.fulfill({ json: [{ ...report, summary: '# Synthetic review\n\n64 sessions, 142 user turns, 31 tool errors and 78% achievement.\n\n## Action Plan\n\n1. **Verify recovery** — failures: 31 → 10 — effort M — improved\n\n## Notes\n\nUse the report to plan the next review.' }] }));
    await page.goto('/#/insights/r/daily/2026-09-24/2026-09-24');
    const kpis = page.locator('.kpis'); await expect(kpis.locator('.kpi')).toHaveCount(4);
    const rects = await kpis.locator('.kpi').evaluateAll(nodes => nodes.map(n => { const r = n.getBoundingClientRect(); return { top: r.top, width: r.width }; }));
    if (width === 834) { expect(rects[0].top).toBe(rects[1].top); expect(rects[2].top).toBe(rects[3].top); expect(rects[2].top).toBeGreaterThan(rects[0].top); }
    else { expect(new Set(rects.map(r => r.top)).size).toBe(1); }
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: info.outputPath(`report-${theme}-${scheme}-${width}.png`) });
  });
}
