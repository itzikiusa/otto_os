import { test, expect } from '@playwright/test';
import { assistantState, mockAssistant } from './assistant-fixture';
import { expectNoHorizontalOverflow } from './helpers';

test.use({ serviceWorkers: 'block' });
test.describe.configure({ timeout: 120_000 });

test('Assistant tabs move keyboard focus with selection', async ({ page }) => {
  await mockAssistant(page, assistantState());
  await page.goto('/#/assistant');
  const chat = page.getByRole('tab', { name: 'Chat', exact: true });
  await chat.focus();
  await chat.press('ArrowRight');
  const tasks = page.getByRole('tab', { name: /^Tasks/ });
  await expect(tasks).toHaveAttribute('aria-selected', 'true');
  await expect(tasks).toBeFocused();
  await tasks.press('ArrowRight');
  await expect(page.getByRole('tab', { name: 'Memory', exact: true })).toBeFocused();
});

test('Memory preserves edits typed while profile save is pending', async ({ page }) => {
  await mockAssistant(page, assistantState());
  let release!: () => void;
  const pending = new Promise<void>((resolve) => { release = resolve; });
  await page.route('**/api/v1/assistant/memory', async (route) => {
    if (route.request().method() === 'PUT') await pending;
    await route.fallback();
  });
  await page.goto('/#/assistant/memory');
  const profile = page.getByLabel('Profile (markdown)');
  await expect(profile).toHaveValue(/Travels with Dana/);
  await profile.fill('- First edit');
  await page.getByRole('button', { name: 'Save', exact: true }).click();
  await expect(page.getByText('Saving…', { exact: true })).toBeVisible();
  await profile.fill('- A newer edit during save');
  release();
  await expect(page.getByText('Saving…', { exact: true })).toBeHidden();
  await expect(profile).toHaveValue('- A newer edit during save');
  await expect(page.getByRole('button', { name: 'Save', exact: true })).toBeEnabled();
});

const reports = [
  { kind: 'daily', period_start: '2026-09-24', period_end: '2026-09-24', summary: '# Daily review\n\nDaily headline.', html_path: '', created_at: '2026-09-25T08:00:00Z' },
  { kind: 'weekly', period_start: '2026-09-14', period_end: '2026-09-20', summary: '# Weekly review\n\nWeekly headline.', html_path: '', created_at: '2026-09-21T08:00:00Z' },
];

test('Insights period filter keeps the open report in the visible list', async ({ page }) => {
  let failed = true;
  await page.route('**/api/v1/insights/reports', (route) => route.fulfill(failed ? { status: 503, json: { message: 'Reports temporarily unavailable' } } : { json: reports }));
  await page.goto('/#/insights');
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  failed = false;
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.locator('[data-testid="report-list"] .rep-row')).toHaveCount(2);
  await page.getByRole('button', { name: 'Weekly 1', exact: true }).click();
  await expect(page.locator('[data-testid="report-list"] .rep-row[aria-current="true"]')).toHaveCount(1);
  await expect(page.locator('[data-testid="insight-report"]')).toContainText('Weekly headline.');
  await page.screenshot({ path: '/tmp/otto-ux-screenshots/insights-r1-reports-loaded.png' });
});

test('Assistant long memory metadata fits phone and RTL tablet', async ({ page }) => {
  const s = assistantState();
  s.memory.memories[0].tags = ['unbroken-tag-'.repeat(30)];
  await mockAssistant(page, s);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#/assistant/memory');
  await expect(page.locator('[data-testid="assistant-memory"]')).toBeVisible();
  await expectNoHorizontalOverflow(page);
  const tag = page.locator('.memory .tag').first();
  const box = await tag.boundingBox();
  expect(box!.x + box!.width).toBeLessThanOrEqual(page.viewportSize()!.width);
  await page.setViewportSize({ width: 1024, height: 768 });
  await page.evaluate(() => { document.documentElement.dir = 'rtl'; });
  await expectNoHorizontalOverflow(page);
});

test('History phone exposes a failed filtered search with Retry', async ({ page }) => {
  let fail = false;
  const row = { session_id: null, provider: 'claude', title: 'Earlier design conversation', first_prompt: 'Review the design', cwd: '/tmp/ux-history', repo_name: 'Design', started_at: new Date().toISOString(), last_active_at: new Date().toISOString(), turns: 2, status: 'on_disk', transcript_path: '/tmp/ux-history.jsonl', resumable: true };
  await page.route('**/api/v1/workspaces/*/history/page?*', (route) => route.fulfill(fail ? { status: 503, json: { message: 'History temporarily unavailable' } } : { json: { entries: new URL(route.request().url()).searchParams.has('q') ? [] : [row], next_cursor: null } }));
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#/history');
  await expect(page.locator('[data-testid="history-row"]')).toHaveCount(1);
  await page.getByRole('textbox', { name: 'Search history' }).fill('no match');
  await expect(page.locator('[data-testid="history-row"]')).toHaveCount(0);
  fail = true;
  await page.getByRole('textbox', { name: 'Search history' }).fill('failed search');
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  fail = false;
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByText('Nothing matches these filters.', { exact: false }).first()).toBeVisible();
  await expectNoHorizontalOverflow(page);
});

test('Usage attribution recovers after failure and fits long keys on phone', async ({ page }) => {
  const tokens = { events: 2, input_tokens: 3000, output_tokens: 1000, cache_read_tokens: 0, cache_write_tokens: 0, total_tokens: 4000, cost_usd: 2.5 };
  let fail = true;
  await page.route('**/api/v1/usage/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path.endsWith('/status')) return route.fulfill({ json: { available: true, enabled: true, binary: '/tmp/clickhouse', version: 'test', data_dir: '/tmp/usage', retention_days: 180, metrics_interval_secs: 60, usage_rows: 2, metric_rows: 0, disk_bytes: 1024 } });
    if (path.endsWith('/summary')) return route.fulfill({ json: { days: 30, total_events: 2, total_input_tokens: 3000, total_output_tokens: 1000, total_cache_read_tokens: 0, total_cache_write_tokens: 0, total_tokens: 4000, total_cost_usd: 2.5, providers: [{ provider: 'claude', ...tokens }], daily: [{ day: '2026-09-24', ...tokens }], sessions: [], by_kind: [] } });
    if (path.endsWith('/attribution')) return route.fulfill(fail ? { status: 503, json: { message: 'Attribution unavailable' } } : { json: [{ key: 'long-repository-name-'.repeat(15), cost_usd: 2.5, tokens: 4000, sessions: 2 }] });
    if (path.endsWith('/metrics')) return route.fulfill({ json: [] });
    return route.fallback();
  });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#/usage');
  const panel = page.locator('.attribution-panel');
  await expect(panel.getByRole('button', { name: 'Retry', exact: true })).toBeVisible();
  fail = false;
  await panel.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(panel.locator('.attr-row')).toHaveCount(1);
  await expectNoHorizontalOverflow(page);
  const download = page.waitForEvent('download');
  await panel.getByRole('button', { name: 'Download attribution as CSV' }).click();
  expect((await download).suggestedFilename()).toBe('otto-attribution-origin-30d.csv');
  await page.screenshot({ path: '/tmp/otto-ux-screenshots/insights-r1-usage-phone-loaded.png' });
});

test('Assistant tasks expose a failed approvals queue with Retry on phone', async ({ page }) => {
  const s = assistantState();
  s.fail['/assistant/needs-you'] = 503;
  await mockAssistant(page, s);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/#/assistant/tasks');
  const tasks = page.locator('[data-testid="assistant-tasks"]');
  await expect(tasks.getByRole('heading', { name: /Running/ })).toBeVisible();
  await expect(tasks.getByText('Couldn’t load requests waiting on you.')).toBeVisible();
  delete s.fail['/assistant/needs-you'];
  await tasks.getByRole('button', { name: 'Retry requests' }).click();
  await expect(tasks.getByRole('heading', { name: /Needs you/ })).toBeVisible();
});

for (const scheme of ['light', 'dark'] as const) {
  test(`Assistant loaded chat, tasks and memory visual review ${scheme}`, async ({ page }) => {
    await page.addInitScript((s) => localStorage.setItem('otto_scheme', s), scheme);
    const s = await mockAssistant(page, assistantState());
    await page.goto('/#/assistant');
    await expect(page.locator('[data-testid="assistant-thread"]')).toBeVisible();
    await page.screenshot({ path: `/tmp/otto-ux-screenshots/insights-r1-assistant-${scheme}-loaded.png` });
    await page.getByLabel('Message Otto').fill('Summarize the hotel options');
    await page.getByLabel('Message Otto').press('Enter');
    await expect(page.locator('[data-testid="assistant-thread"]')).toContainText('Summarize the hotel options');
    await page.getByRole('tab', { name: /^Tasks/ }).click();
    const tasks = page.locator('[data-testid="assistant-tasks"]');
    await expect(tasks.getByRole('heading', { name: /Needs you/ })).toBeVisible();
    await page.screenshot({ path: `/tmp/otto-ux-screenshots/insights-r1-tasks-${scheme}-loaded.png` });
    await tasks.getByRole('button', { name: /Continue on Codex/ }).click();
    await expect(tasks.locator('[data-testid="card-needs-limit"]')).toHaveCount(0);
    expect(s.calls.some((c) => c.path === '/assistant/tasks/task-limit/approve')).toBeTruthy();
    await page.getByRole('tab', { name: 'Memory', exact: true }).click();
    await expect(page.getByLabel('Profile (markdown)')).toHaveValue(/Travels with Dana/);
    await page.screenshot({ path: `/tmp/otto-ux-screenshots/insights-r1-memory-${scheme}-loaded.png` });
    await expectNoHorizontalOverflow(page);
  });
}
