import { test, expect } from '@playwright/test';
import { assistantState, mockAssistant } from './assistant-fixture';
import { apiCtx, seedWorkspace } from './seed';
type BudgetFixture = { enforce: boolean; block_on_exceed: boolean; window_days: number; providers: { provider: string; monthly_usd: number }[]; workspaces: { workspace_id: string; monthly_usd: number }[] };
test.use({ serviceWorkers: 'block', actionTimeout: 10_000 });
test.describe.configure({ timeout: 60_000 });
const report = { kind: 'daily', period_start: '2026-09-24', period_end: '2026-09-24', summary: '# Daily review\n\nOriginal headline.', html_path: '', created_at: '2026-09-25T08:00:00Z' };

test('Insights waits for the requested period when another scheduled report arrives', async ({ page }) => {
  let phase = 0, polls = 0;
  const unrelated = { ...report, kind: 'weekly', period_start: '2026-09-14', period_end: '2026-09-20', summary: '# Scheduled weekly review' };
  await page.route('**/api/v1/insights/reports', r => { polls++; return r.fulfill({ json: phase === 0 ? [report] : [unrelated, { ...report, ...(phase === 2 ? { summary: '# Requested daily result', created_at: '2026-09-25T09:00:00Z' } : {}) }] }); });
  await page.route('**/api/v1/insights/run', r => { phase = 1; return r.fulfill({ json: { started: true, run_id: 'requested-run', report_key: 'daily:20260924_20260924' } }); });
  await page.goto('/#/insights');
  await expect(page.getByTestId('insight-report')).toContainText('Original headline');
  const before = polls;
  await page.getByRole('button', { name: 'Run now', exact: true }).click();
  await expect.poll(() => polls).toBeGreaterThan(before);
  await expect(page.getByRole('button', { name: 'Running…', exact: true })).toBeDisabled();
  await expect(page.getByText('Insights report ready', { exact: true })).toHaveCount(0);
  phase = 2;
  await expect(page.getByTestId('insight-report')).toContainText('Requested daily result', { timeout: 10_000 });
  await expect(page.getByRole('button', { name: 'Run now', exact: true })).toBeEnabled();
});

test('Assistant failed send preserves a newer draft and protects thread navigation', async ({ page }) => {
  await mockAssistant(page, assistantState());
  let release!: () => void;
  await page.route('**/api/v1/assistant/threads/th-personal/turns', async r => {
    if (r.request().method() !== 'POST') return r.fallback();
    await new Promise<void>(resolve => { release = resolve; });
    return r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic send failure' } });
  });
  await page.goto('/#/assistant');
  const input = page.getByRole('textbox', { name: 'Message Otto' });
  await input.fill('Original message'); await input.press('Enter');
  await expect.poll(() => !!release).toBe(true);
  await input.fill('Newer draft while sending'); release();
  await expect(page.getByRole('alert')).toContainText('Couldn’t send');
  await expect(input).toHaveValue('Original message\n\nNewer draft while sending');
  await page.getByRole('tab', { name: /^Tasks/ }).click();
  await expect(page.getByRole('dialog')).toContainText('Discard unsaved changes?');
  await page.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(input).toHaveValue(/Newer draft/);
  await page.screenshot({ path: '/tmp/otto-ux-screenshots/insights-r3-chat-recovery.png' });
});

test('Usage rejects an invalid window and guards a newer budget draft on navigation', async ({ page }) => {
  let puts = 0;
  await page.route('**/api/v1/usage/**', r => {
    const path = new URL(r.request().url()).pathname;
    if (path.endsWith('/status')) return r.fulfill({ json: { available: true, enabled: true, retention_days: 180, metrics_interval_secs: 60, usage_rows: 0, metric_rows: 0, disk_bytes: 0 } });
    if (path.endsWith('/summary')) return r.fulfill({ json: { days: 30, total_events: 0, total_input_tokens: 0, total_output_tokens: 0, total_cache_read_tokens: 0, total_cache_write_tokens: 0, total_tokens: 0, total_cost_usd: 0, providers: [], daily: [], sessions: [], by_kind: [] } });
    if (path.endsWith('/budgets')) {
      if (r.request().method() === 'PUT') puts++;
      return r.fulfill({ json: { config: { enforce: false, block_on_exceed: false, window_days: 30, providers: [], workspaces: [] }, window_days: 30, rows: [] } });
    }
    return r.fulfill({ json: [] });
  });
  await page.goto('/#/usage');
  await page.getByRole('button', { name: 'Edit budgets', exact: true }).click();
  const days = page.getByLabel('Compare spend over the last');
  await days.fill('-1');
  await days.blur();
  await expect(page.getByRole('button', { name: 'Save budgets', exact: true })).toBeDisabled();
  await expect(page.getByRole('alert')).toContainText('whole number');
  expect(puts).toBe(0);
  await days.fill('7');
  await page.evaluate(() => { window.location.hash = '#/insights'; });
  await expect(page.getByRole('dialog')).toContainText('Discard unsaved changes?');
  await page.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(days).toHaveValue('7');
});

test('Insights regeneration cannot be overwritten by an earlier summary response', async ({ page }) => {
  let replaced = false;
  let release!: () => void;
  let reads = 0;
  const html_path = '/tmp/synthetic-insights/daily/report-daily-20260924_20260924.html';
  await page.route('**/api/v1/insights/reports', r => r.fulfill({ json: [{ ...report, html_path, ...(replaced ? { summary: '# Fresh summary\n\nCurrent result.', created_at: '2026-09-25T09:00:00Z' } : {}) }] }));
  await page.route('**/api/v1/insights/report?*', async r => {
    if (!new URL(r.request().url()).searchParams.get('path')?.includes('summary-')) return r.fulfill({ json: { series: [], ledger: [] } });
    if (++reads === 1) { await new Promise<void>(resolve => { release = resolve; }); return r.fulfill({ body: '# Stale summary\n\nOld generation body.' }); }
    return r.fulfill({ body: '# Fresh summary\n\nCurrent result.' });
  });
  await page.route('**/api/v1/insights/run', r => { replaced = true; return r.fulfill({ json: { started: true, run_id: 'requested-run', report_key: 'daily:20260924_20260924' } }); });
  await page.goto('/#/insights');
  await expect.poll(() => !!release).toBe(true);
  await page.getByRole('button', { name: 'Run now', exact: true }).click();
  await expect(page.getByTestId('insight-report')).toContainText('Current result.', { timeout: 10_000 });
  release();
  await expect.poll(() => reads).toBeGreaterThan(1);
  await page.waitForTimeout(300);
  await expect(page.getByTestId('insight-report')).toContainText('Current result.');
  await expect(page.getByTestId('insight-report')).not.toContainText('Old generation body.');
});

test('Assistant phone attachment recovery shows the full identity and model pin retries', async ({ page }) => {
  const state = assistantState();
  await mockAssistant(page, state);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#/assistant/th-personal');
  await expect(page.getByRole('textbox', { name: 'Message Otto' })).toBeVisible();
  const longName = 'synthetic-review-for-september-with-complete-customer-identity-final-approved-version.txt';
  state.fail['/assistant/threads/th-personal/attachments'] = 503;
  await page.locator('input[type=file]').setInputFiles({ name: longName, mimeType: 'text/plain', buffer: Buffer.from('synthetic') });
  await expect(page.getByRole('alert')).toContainText('Couldn’t attach');
  delete state.fail['/assistant/threads/th-personal/attachments'];
  await page.locator('input[type=file]').setInputFiles({ name: longName, mimeType: 'text/plain', buffer: Buffer.from('synthetic') });
  await expect(page.locator('.fname')).toHaveText(longName);
  await expect(page.locator('.composer').getByRole('alert')).toHaveCount(0);
  await expect.poll(() => page.locator('.fname').evaluate(e => e.scrollWidth <= e.clientWidth)).toBe(true);
  await page.screenshot({ path: '/tmp/otto-ux-screenshots/insights-r3-attachment-phone.png' });
  await page.getByRole('button', { name: `Remove ${longName}`, exact: true }).click();
  await expect(page.locator('.fname')).toHaveCount(0);
  await page.getByTestId('model-chip').click();
  const sheet = page.getByRole('dialog', { name: 'Model for this thread' });
  state.fail['/assistant/threads/th-personal/route'] = 503;
  await sheet.getByRole('button', { name: 'Codex', exact: true }).click();
  await sheet.getByRole('button', { name: 'Pin to thread' }).click();
  await expect(sheet.getByRole('alert')).toContainText('Couldn’t pin');
  delete state.fail['/assistant/threads/th-personal/route'];
  await sheet.getByRole('button', { name: 'Pin to thread' }).click();
  await expect(sheet).toHaveCount(0);
  await expect(page.getByTestId('model-chip')).toContainText('Codex');
});

test('Memory discard branch and delayed thread load preserve the newly selected thread', async ({ page }) => {
  await mockAssistant(page, assistantState());
  let release!: () => void;
  await page.route('**/api/v1/assistant/threads/th-personal/turns*', async r => { await new Promise<void>(resolve => { release = resolve; }); return r.fulfill({ json: assistantState().turns['th-personal'] }); });
  await page.goto('/#/assistant/memory');
  await page.getByLabel('Profile (markdown)').fill('Discard this synthetic edit');
  await page.getByRole('tab', { name: 'Chat', exact: true }).click();
  await page.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect.poll(() => !!release).toBe(true);
  await page.getByRole('button', { name: /02 Work/ }).click();
  await expect(page.getByTestId('assistant-thread')).toContainText('Two PRs');
  release();
  await expect(page.getByTestId('assistant-thread')).not.toContainText('Three good fits');
  await page.getByRole('tab', { name: 'Memory', exact: true }).click();
  await expect(page.getByLabel('Profile (markdown)')).not.toHaveValue('Discard this synthetic edit');
});

test('History provider change ignores older pagination and exposes the full selected title on phone', async ({ page }) => {
  const row = { session_id: null, provider: 'claude', title: 'Synthetic history initial', first_prompt: 'Review the design', cwd: '/tmp/synthetic-history', repo_name: 'Synthetic', started_at: new Date().toISOString(), last_active_at: new Date().toISOString(), turns: 2, status: 'on_disk', transcript_path: '/tmp/current.jsonl', resumable: true };
  const title = 'Synthetic final design review covering the complete September release and remaining accessibility decisions';
  let release!: () => void;
  await page.route('**/api/v1/workspaces/*/history/page?*', async r => {
    const query = new URL(r.request().url()).searchParams;
    if (query.has('cursor')) { await new Promise<void>(resolve => { release = resolve; }); return r.fulfill({ json: { entries: [{ ...row, title: 'Old paginated Claude result', transcript_path: '/tmp/older.jsonl' }], next_cursor: null } }); }
    return r.fulfill({ json: query.get('provider') === 'codex' ? { entries: [{ ...row, title, provider: 'codex', transcript_path: '/tmp/codex.jsonl' }], next_cursor: null } : { entries: [row], next_cursor: 'older' } });
  });
  await page.route('**/api/v1/workspaces/*/history/transcript?*', r => r.fulfill({ json: { session_id: null, provider: 'codex', title, cwd: '/tmp/synthetic-history', model: null, cursor: '', has_earlier: false, turns: [], stats: { turns: 0, tool_calls: 0, cost_usd: null, input_tokens: null, output_tokens: null, duration_ms: null, reasoning_steps: 0, thinking_steps: 0, unknown_records: 0 }, subagents: [], unavailable_reason: null } }));
  await page.goto('/#/history');
  await page.getByRole('button', { name: 'Load older conversations' }).click();
  await expect.poll(() => !!release).toBe(true);
  await page.getByRole('combobox', { name: 'Provider', exact: true }).selectOption('codex');
  await expect(page.getByTestId('history-row')).toHaveCount(1);
  await expect(page.getByTestId('history-row')).toContainText(title);
  release();
  await expect(page.getByText('Old paginated Claude result', { exact: true })).toHaveCount(0);
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.locator('.dtitle')).toHaveText(title);
  await expect.poll(() => page.locator('.dtitle').evaluate(e => e.scrollWidth <= e.clientWidth)).toBe(true);
  await page.screenshot({ path: '/tmp/otto-ux-screenshots/insights-r3-history-phone.png' });
});

test('Insights regeneration retains current metrics when an old index arrives late', async ({ page }) => {
  let replaced = false, indexReads = 0;
  let release!: () => void;
  const html_path = '/tmp/synthetic-insights/daily/report-daily-20260924_20260924.html';
  await page.route('**/api/v1/insights/reports', r => r.fulfill({ json: [{ ...report, html_path, ...(replaced ? { summary: '# Current result', created_at: '2026-09-25T09:00:00Z' } : {}) }] }));
  await page.route('**/api/v1/insights/report?*', async r => {
    if (!new URL(r.request().url()).searchParams.get('path')?.endsWith('index.json')) return r.fulfill({ body: '# Synthetic report\n\nReview the measurements.' });
    const first = ++indexReads === 1;
    if (first) await new Promise<void>(resolve => { release = resolve; });
    return r.fulfill({ json: { series: [{ period_key: 'daily:20260924_20260924', kind: 'daily', start: '2026-09-24', end: '2026-09-24', headline: { total_sessions: first ? 12 : 88 } }] } });
  });
  await page.route('**/api/v1/insights/run', r => { replaced = true; return r.fulfill({ json: { started: true, run_id: 'requested-run', report_key: 'daily:20260924_20260924' } }); });
  await page.goto('/#/insights');
  await expect.poll(() => !!release).toBe(true);
  await page.getByRole('button', { name: 'Run now', exact: true }).click();
  await expect(page.getByTestId('kpi-sessions')).toContainText('88', { timeout: 10_000 });
  release();
  await page.waitForTimeout(300);
  await expect(page.getByTestId('kpi-sessions')).toContainText('88');
});

test('Usage scope caps add remove and save with enforcement on RTL tablet', async ({ page }) => {
  const a = await apiCtx(); const wid = await seedWorkspace(a.ctx, a.base); await a.ctx.dispose();
  await page.addInitScript(() => { localStorage.setItem('otto_theme', 'warm'); localStorage.setItem('otto_scheme', 'dark'); localStorage.setItem('otto_direction', 'rtl'); });
  await page.setViewportSize({ width: 834, height: 1112 });
  let config: BudgetFixture = { enforce: false, block_on_exceed: false, window_days: 30, providers: [], workspaces: [] };
  let submitted: BudgetFixture | null = null;
  await page.route('**/api/v1/usage/**', r => {
    const path = new URL(r.request().url()).pathname;
    if (path.endsWith('/status')) return r.fulfill({ json: { available: true, enabled: true, retention_days: 180, metrics_interval_secs: 60, usage_rows: 0, metric_rows: 0, disk_bytes: 0 } });
    if (path.endsWith('/summary')) return r.fulfill({ json: { days: 30, total_events: 0, total_input_tokens: 0, total_output_tokens: 0, total_cache_read_tokens: 0, total_cache_write_tokens: 0, total_tokens: 0, total_cost_usd: 0, providers: [], daily: [], sessions: [], by_kind: [] } });
    if (path.endsWith('/budgets')) {
      if (r.request().method() === 'PUT') { submitted = r.request().postDataJSON(); config = submitted!; }
      return r.fulfill({ json: { config, window_days: config.window_days, rows: [] } });
    }
    return r.fulfill({ json: [] });
  });
  await page.goto('/#/usage');
  await page.getByRole('button', { name: 'Edit budgets', exact: true }).click();
  await page.getByRole('button', { name: 'Add provider cap' }).click();
  await page.getByRole('combobox', { name: 'Provider', exact: true }).selectOption('codex');
  await page.getByLabel('Cap in US dollars').fill('25');
  await page.getByRole('button', { name: 'Add workspace cap' }).click();
  await page.getByRole('combobox', { name: 'Workspace', exact: true }).selectOption(wid);
  await page.getByLabel('Cap in US dollars').first().fill('50');
  await page.getByRole('button', { name: 'Add provider cap' }).click();
  await page.getByRole('button', { name: 'Remove this provider cap' }).last().click();
  await page.getByLabel('Enforce budgets: warn prominently when a cap is exceeded').check();
  await page.getByLabel('Block new work in a scope that is over its cap (otherwise warn only)').check();
  await page.getByRole('button', { name: 'Save budgets', exact: true }).click();
  await expect(page.getByText('Budgets saved', { exact: true })).toBeVisible();
  expect(submitted).toMatchObject({ enforce: true, block_on_exceed: true, providers: [{ provider: 'codex', monthly_usd: 25 }], workspaces: [{ workspace_id: wid, monthly_usd: 50 }] });
  await expect(page.getByRole('button', { name: 'Save budgets', exact: true })).toBeDisabled();
  await page.screenshot({ path: '/tmp/otto-ux-screenshots/insights-r3-usage-rtl-tablet.png' });
});
