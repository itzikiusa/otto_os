import { test, expect } from '@playwright/test';
import { assistantState, mockAssistant } from './assistant-fixture';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { apiCtx, seedWorkspace } from './seed';
import { expectNoHorizontalOverflow } from './helpers';
test.use({ serviceWorkers: 'block', actionTimeout: 10_000 });
test.describe.configure({ timeout: 60_000 });
const report = { kind: 'daily', period_start: '2026-09-24', period_end: '2026-09-24', summary: '# Daily review\n\nOriginal headline. 64 sessions, 142 user turns, 31 tool errors and 78% achievement.\n\n## Action Plan\n\n1. **Reuse the review context** — repeated setup: 12 → 3 — effort S — new\n2. **Verify retry errors** — failures: 31 → 10 — effort M — improved\n\n## Notes\n\nThe synthetic review data covers two providers and a week of focused development. Review the measurements before changing your workflow.', html_path: '', created_at: '2026-09-25T08:00:00Z' };
test('phone approval exposes complete outbound payload before Send', async ({ page }) => {
  const s = assistantState();
  const payload = 'Hotel shortlist\n' + 'Review this hotel with a quiet room.\n'.repeat(12) + 'Final line: ask Dana to confirm by Friday.';
  s.tasks.find(t => t.id === 'task-appr')!.needs_you!.approval!.what = payload;
  await mockAssistant(page, s);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#/assistant/tasks');
  const card = page.getByTestId('assistant-tasks').getByTestId('card-needs-approval');
  await card.getByText('Review exactly what is sent', { exact: true }).click();
  const preview = card.getByRole('region', { name: 'Exactly what is sent' });
  await expect(preview).toHaveText(payload);
  await preview.focus();
  await preview.press('End');
  await expectNoHorizontalOverflow(page);
  await page.screenshot({ path: '/tmp/otto-ux-screenshots/insights-r2-approval-phone.png' });
  await card.getByRole('button', { name: 'Send', exact: true }).click();
  expect(s.calls.some(c => c.path.endsWith('/task-appr/approve'))).toBeTruthy();
});
test('Memory guards unsaved tab navigation and saves after cancelling', async ({ page }) => {
  await mockAssistant(page, assistantState());
  await page.goto('/#/assistant/memory');
  const profile = page.getByLabel('Profile (markdown)');
  await expect(profile).toHaveValue(/Travels/);
  await profile.fill('- Keep this draft');
  await page.getByRole('tab', { name: /^Tasks/ }).click();
  await expect(page.getByRole('dialog')).toContainText('Discard unsaved changes?');
  await page.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(profile).toHaveValue('- Keep this draft');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText('Saved', { exact: true })).toBeVisible();
  await page.getByRole('tab', { name: /^Tasks/ }).click();
  await expect(page.getByTestId('assistant-tasks')).toBeVisible();
});
test('Assistant tabs follow physical arrow direction in RTL', async ({ page }) => {
  await mockAssistant(page, assistantState());
  await page.goto('/#/assistant');
  await page.evaluate(() => { document.documentElement.dir = 'rtl'; });
  const chat = page.getByRole('tab', { name: 'Chat', exact: true });
  await chat.focus(); await chat.press('ArrowLeft');
  await expect(page.getByRole('tab', { name: /^Tasks/ })).toBeFocused();
  await expect(page.getByRole('tab', { name: /^Tasks/ })).toHaveAttribute('aria-selected', 'true');
});
test('Insights phone report deep link opens detail and Back reveals list', async ({ page }) => {
  await page.route('**/api/v1/insights/reports', r => r.fulfill({ json: [report] }));
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#/insights/r/daily/2026-09-24/2026-09-24');
  await expect(page.getByTestId('insight-report')).toBeVisible();
  await page.getByRole('button', { name: 'Back to reports' }).click();
  await expect(page.getByTestId('report-list')).toBeVisible();
});
test('Insights regenerated period completes without increasing report count', async ({ page }) => {
  let replaced = false;
  await page.route('**/api/v1/insights/reports', r => r.fulfill({ json: [{ ...report, ...(replaced ? { summary: '# Daily review\n\nReplacement headline.', created_at: '2026-09-25T09:00:00Z' } : {}) }] }));
  await page.route('**/api/v1/insights/run', r => { replaced = true; return r.fulfill({ json: { started: true, run_id: 'synthetic-run', report_key: 'daily:20260924_20260924' } }); });
  await page.goto('/#/insights');
  await page.getByRole('button', { name: 'Run now', exact: true }).click();
  await expect(page.getByText('Insights report ready', { exact: true })).toBeVisible({ timeout: 10_000 });
  await expect(page.getByRole('button', { name: 'Run now', exact: true })).toBeEnabled();
  await expect(page.getByTestId('insight-report')).toContainText('Replacement headline.');
});
test('Insights Health keyboard navigation, recovery, dependencies and bundle', async ({ page }) => {
  let fail = true;
  await page.route('**/api/v1/insights/reports', r => r.fulfill({ json: [report] }));
  const caps = [{ feature: 'sessions', status: 'degraded', reasons: ['One provider needs configuration'], fixes: ['Choose an installed provider in Settings'], deps: [{ kind: 'provider', name: 'Synthetic Codex', ok: true, detail: '/tmp/fake-codex' }] }];
  await page.route('**/api/v1/capabilities', r => r.fulfill(fail ? { status: 503, json: { code: 'upstream', message: 'Synthetic health unavailable' } } : { json: caps }));
  await page.route('**/api/v1/support-bundle', r => r.fulfill({ json: { version: 'synthetic', settings: {}, capabilities: caps, recent_audit: [], migration_level: 1, redaction_hits: 0 } }));
  await page.goto('/#/insights');
  const reports = page.getByRole('tab', { name: 'Reports', exact: true });
  await reports.focus(); await reports.press('ArrowRight');
  await expect(page.getByRole('tab', { name: 'Health', exact: true })).toBeFocused();
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  fail = false;
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await page.getByRole('button', { name: /Agent Sessions Degraded/ }).click();
  await expect(page.getByText('/tmp/fake-codex', { exact: true })).toBeVisible();
  const dl = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Download support bundle', exact: true }).click();
  expect((await dl).suggestedFilename()).toMatch(/^otto-support-bundle-.*\.json$/);
  await page.setViewportSize({ width: 390, height: 844 });
  await expectNoHorizontalOverflow(page);
  const label = page.locator('.feature-label');
  await expect.poll(() => label.evaluate(e => e.scrollWidth <= e.clientWidth)).toBe(true);
  const toggle = page.getByRole('button', { name: /Agent Sessions Degraded/ });
  if (await toggle.getAttribute('aria-expanded') !== 'true') await toggle.click();
  await page.screenshot({ path: '/tmp/otto-ux-screenshots/insights-r2-health-phone.png' });
});
test('History date filter replaces an excluded open conversation', async ({ page }) => {
  const row = { session_id: null, provider: 'claude', title: 'Current conversation', first_prompt: 'Review the design', cwd: '/tmp/synthetic-history', repo_name: 'Synthetic', started_at: new Date().toISOString(), last_active_at: new Date().toISOString(), turns: 2, status: 'on_disk', transcript_path: '/tmp/current.jsonl', resumable: true };
  const old = { ...row, title: 'Older conversation', last_active_at: '2020-01-01T00:00:00Z', transcript_path: '/tmp/old.jsonl' };
  await page.route('**/api/v1/workspaces/*/history/page?*', r => r.fulfill({ json: { entries: [row, old], next_cursor: null } }));
  await page.route('**/api/v1/workspaces/*/history/transcript?*', r => r.fulfill({ json: { session_id: null, provider: 'claude', title: 'Synthetic conversation', cwd: '/tmp/synthetic-history', model: null, cursor: '', has_earlier: false, turns: [], stats: { turns: 0, tool_calls: 0, cost_usd: null, input_tokens: null, output_tokens: null, duration_ms: null, reasoning_steps: 0, thinking_steps: 0, unknown_records: 0 }, subagents: [], unavailable_reason: null } }));
  await page.goto('/#/history');
  await page.getByTestId('history-row').filter({ hasText: 'Older conversation' }).click();
  await page.getByRole('combobox', { name: 'Date', exact: true }).selectOption('today');
  await expect(page.locator('.dtitle')).toHaveText('Current conversation');
});

test('Usage budget failed save preserves draft and delayed save preserves newer edits', async ({ page }) => {
  let fail = true;
  let release: (() => void) | null = null;
  let config = { enforce: false, block_on_exceed: false, window_days: 30, providers: [], workspaces: [] };
  await page.route('**/api/v1/usage/**', async r => {
    const path = new URL(r.request().url()).pathname;
    if (path.endsWith('/status')) return r.fulfill({ json: { available: true, enabled: true, retention_days: 180, metrics_interval_secs: 60, usage_rows: 0, metric_rows: 0, disk_bytes: 0 } });
    if (path.endsWith('/summary')) return r.fulfill({ json: { days: 30, total_events: 0, total_input_tokens: 0, total_output_tokens: 0, total_cache_read_tokens: 0, total_cache_write_tokens: 0, total_tokens: 0, total_cost_usd: 0, providers: [], daily: [], sessions: [], by_kind: [] } });
    if (path.endsWith('/budgets')) {
      if (r.request().method() === 'PUT') {
        if (fail) return r.fulfill({ status: 503, json: { code: 'upstream', message: 'Synthetic save failed' } });
        config = r.request().postDataJSON();
        await new Promise<void>(resolve => { release = resolve; });
      }
      return r.fulfill({ json: { config, window_days: config.window_days, rows: [] } });
    }
    return r.fulfill({ json: [] });
  });
  await page.goto('/#/usage');
  await page.getByRole('button', { name: 'Edit budgets', exact: true }).click();
  const days = page.getByLabel('Compare spend over the last');
  await days.fill('7');
  await page.getByRole('button', { name: 'Save budgets', exact: true }).click();
  await expect(page.getByText('Could not save budgets', { exact: true })).toBeVisible();
  await expect(days).toHaveValue('7');
  fail = false;
  await page.getByRole('button', { name: 'Save budgets', exact: true }).click();
  await expect.poll(() => release !== null).toBe(true);
  await days.fill('90');
  await days.blur();
  release!();
  await expect(page.getByText('Budgets saved', { exact: true })).toBeVisible();
  await expect(days).toHaveValue('90');
  await expect(page.getByRole('button', { name: 'Save budgets', exact: true })).toBeEnabled();
});

test('Usage ignores stale initial summary errors after changing window and scope', async ({ page }) => {
  let release!: () => void;
  let initial = true;
  const tokens = { events: 2, input_tokens: 100, output_tokens: 100, cache_read_tokens: 0, cache_write_tokens: 0, total_tokens: 200, cost_usd: 7 };
  await page.route('**/api/v1/usage/**', async r => {
    const url = new URL(r.request().url());
    if (url.pathname.endsWith('/status')) return r.fulfill({ json: { available: true, enabled: true, retention_days: 180, metrics_interval_secs: 60, usage_rows: 2, metric_rows: 0, disk_bytes: 0 } });
    if (url.pathname.endsWith('/summary')) {
      if (initial) { initial = false; await new Promise<void>(resolve => { release = resolve; }); return r.fulfill({ status: 503, json: { code: 'upstream', message: 'Stale thirty-day failure' } }); }
      return r.fulfill({ json: { days: 7, total_events: 2, total_input_tokens: 100, total_output_tokens: 100, total_cache_read_tokens: 0, total_cache_write_tokens: 0, total_tokens: 200, total_cost_usd: 7, providers: [{ provider: url.searchParams.get('otto_only') === 'false' ? 'codex' : 'claude', ...tokens }], daily: [], sessions: [], by_kind: [] } });
    }
    if (url.pathname.endsWith('/budgets')) return r.fulfill({ json: { config: { enforce: false, block_on_exceed: false, window_days: 30, providers: [], workspaces: [] }, window_days: 30, rows: [] } });
    return r.fulfill({ json: [] });
  });
  await page.goto('/#/usage');
  await expect.poll(() => !!release).toBe(true);
  await page.getByRole('group', { name: 'Time window' }).getByRole('button', { name: '7d', exact: true }).click();
  await page.getByRole('group', { name: 'Sessions to count' }).getByRole('button', { name: 'All', exact: true }).click();
  release();
  await expect(page.getByText('Stale thirty-day failure', { exact: false })).toHaveCount(0);
  await expect(page.locator('.bar-name').filter({ hasText: 'codex' })).toBeVisible();
  await page.screenshot({ path: '/tmp/otto-ux-screenshots/insights-r2-usage.png' });
});

for (const [theme, scheme, width, dir] of [
  ['native', 'light', 1440, 'ltr'], ['native', 'dark', 1440, 'ltr'], ['warm', 'light', 390, 'ltr'], ['warm', 'dark', 1024, 'rtl'], ['pro-dark', 'dark', 1440, 'ltr'],
] as const) {
  test(`loaded review ${theme} ${scheme} ${width} ${dir}`, async ({ page }) => {
    await page.addInitScript(({ theme, scheme, dir }) => {
      localStorage.setItem('otto_theme', theme); localStorage.setItem('otto_scheme', scheme); localStorage.setItem('otto_direction', dir);
    }, { theme, scheme, dir });
    await mockAssistant(page, assistantState());
    await page.route('**/api/v1/insights/reports', r => r.fulfill({ json: [report] }));
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/#/assistant/memory');
    await expect(page.getByLabel('Profile (markdown)')).toHaveValue(/Travels/);
    await page.evaluate(dir => { document.documentElement.dir = dir; }, dir);
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: `/tmp/otto-ux-screenshots/insights-r2-memory-${theme}-${scheme}-${width}.png` });
    await page.goto('/#/insights/r/daily/2026-09-24/2026-09-24');
    await expect(page.getByTestId('insight-report')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: `/tmp/otto-ux-screenshots/insights-r2-reports-${theme}-${scheme}-${width}.png` });
  });
}


test('History real isolated transcript loads and synthetic import resumes after failure', async ({ page }) => {
  const a = await apiCtx();
  const wid = await seedWorkspace(a.ctx, a.base);
  const dir = mkdtempSync(join(tmpdir(), 'otto-ux-history-'));
  const path = join(dir, 'synthetic.jsonl');
  const sid = '00000000-0000-4000-8000-000000000001';
  writeFileSync(path, [
    { type: 'user', uuid: 'ux-user', sessionId: sid, timestamp: new Date().toISOString(), cwd: dir, message: { role: 'user', content: 'Compare two synthetic launch options.' } },
    { type: 'assistant', uuid: 'ux-assistant', sessionId: sid, timestamp: new Date().toISOString(), message: { id: 'ux-answer', role: 'assistant', model: 'synthetic', content: [{ type: 'text', text: 'Option A is ready. Option B needs a review.' }], usage: { input_tokens: 10, output_tokens: 10 } } },
  ].map(x => JSON.stringify(x)).join('\n') + '\n');
  const response = await a.ctx.post(`${a.base}/api/v1/workspaces/${wid}/sessions`, { data: { kind: 'agent', provider: 'claude', title: 'Synthetic launch review', cwd: dir, meta: { origin: 'e2e', e2e_transcript_path: path } } });
  expect(response.ok()).toBe(true);
  const session = await response.json();
  await a.ctx.dispose();
  await page.addInitScript(wid => localStorage.setItem('otto_workspace', wid), wid);
  await page.goto('/#/history');
  const row = page.getByTestId('history-row').filter({ hasText: 'Synthetic launch review' });
  await row.click();
  const conv = page.getByTestId('history-conversation');
  await expect(conv).toContainText('Compare two synthetic launch options.');
  await expect(conv).toContainText('Option A is ready.');
  await expect(conv.locator('textarea')).toHaveCount(0);
  await page.screenshot({ path: '/tmp/otto-ux-screenshots/insights-r2-history-real-synthetic.png' });
  // Exercise the import/retry UI with a synthetic disk row and a real isolated
  // fake-CLI session. No external provider is launched.
  let fail = true;
  await page.route('**/api/v1/workspaces/*/history/page?*', r => r.fulfill({ json: { entries: [{ session_id: null, provider: 'claude', title: 'Synthetic import review', first_prompt: 'Resume review', cwd: dir, repo_name: 'Synthetic', started_at: new Date().toISOString(), last_active_at: new Date().toISOString(), turns: 2, status: 'on_disk', transcript_path: path, resumable: true }], next_cursor: null } }));
  await page.route('**/api/v1/workspaces/*/history/import', r => r.fulfill(fail ? { status: 503, json: { code: 'upstream', message: 'Synthetic import temporarily unavailable' } } : { json: session }));
  await page.reload();
  await page.getByTestId('history-row').filter({ hasText: 'Synthetic import review' }).click();
  await page.getByRole('button', { name: 'Resume in Otto', exact: true }).click();
  await expect(page.getByText('Could not resume', { exact: true })).toBeVisible();
  fail = false;
  await page.getByRole('button', { name: 'Resume in Otto', exact: true }).click();
  await expect(page).toHaveURL(new RegExp(`#/agents/${session.id}$`));
});
