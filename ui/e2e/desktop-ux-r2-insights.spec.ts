import { test, expect } from '@playwright/test';
import { expectNoHorizontalOverflow } from './helpers';
test.use({ serviceWorkers: 'block', actionTimeout: 10_000 });
test.describe.configure({ timeout: 60_000 });
const report = { kind: 'daily', period_start: '2026-09-24', period_end: '2026-09-24', summary: '# Daily review\n\nOriginal headline. 64 sessions, 142 user turns, 31 tool errors and 78% achievement.\n\n## Action Plan\n\n1. **Reuse the review context** — repeated setup: 12 → 3 — effort S — new\n2. **Verify retry errors** — failures: 31 → 10 — effort M — improved\n\n## Notes\n\nThe synthetic review data covers two providers and a week of focused development. Review the measurements before changing your workflow.', html_path: '', created_at: '2026-09-25T08:00:00Z' };

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
  await page.route('**/api/v1/insights/report-status?*', r => r.fulfill({ json: { report: { ...report, summary: new URL(r.request().url()).searchParams.get('summary') === 'true' ? (replaced ? '# Daily review\n\nReplacement headline.' : report.summary) : '', html_path: replaced ? '/tmp/synthetic-report.html' : null }, html_revision: replaced ? 'new' : 'old' } }));
  await page.route('**/api/v1/insights/run', r => { replaced = true; return r.fulfill({ json: { started: true, run_id: 'synthetic-run', report_revision: 'old', report_key: 'daily:20260924_20260924' } }); });
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
  await page.getByRole('button', { name: /Agent sessions Degraded/ }).click();
  await expect(page.getByText('/tmp/fake-codex', { exact: true })).toBeVisible();
  const dl = page.waitForEvent('download');
  await page.getByRole('button', { name: 'Download support bundle', exact: true }).click();
  expect((await dl).suggestedFilename()).toMatch(/^otto-support-bundle-.*\.json$/);
  await page.setViewportSize({ width: 390, height: 844 });
  await expectNoHorizontalOverflow(page);
  const label = page.locator('.feature-label');
  await expect.poll(() => label.evaluate(e => e.scrollWidth <= e.clientWidth)).toBe(true);
  const toggle = page.getByRole('button', { name: /Agent sessions Degraded/ });
  if (await toggle.getAttribute('aria-expanded') !== 'true') await toggle.click();
  await page.screenshot({ path: '/tmp/otto-ux-screenshots/insights-r2-health-phone.png' });
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
  await expect(page.getByText('Couldn’t save budgets', { exact: true })).toBeVisible();
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
    await page.route('**/api/v1/insights/reports', r => r.fulfill({ json: [report] }));
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/#/insights/r/daily/2026-09-24/2026-09-24');
    await page.evaluate(dir => { document.documentElement.dir = dir; }, dir);
    await expect(page.getByTestId('insight-report')).toBeVisible();
    await expectNoHorizontalOverflow(page);
    await page.screenshot({ path: `/tmp/otto-ux-screenshots/insights-r2-reports-${theme}-${scheme}-${width}.png` });
  });
}
