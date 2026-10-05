import { test, expect } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';
import type { TelemetryConfig } from '../src/lib/api/types';

test.use({ serviceWorkers: 'block', viewport: { width: 1440, height: 900 } });
test.describe.configure({ mode: 'serial' });
let workspace = '';
let original: TelemetryConfig;
test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspace = await seedWorkspace(ctx, base);
  const response = await ctx.get(`${base}/api/v1/telemetry/config`);
  expect(response.ok()).toBeTruthy();
  original = await response.json();
  await ctx.dispose();
});
test.afterAll(async () => {
  if (!original) return;
  const { ctx, base } = await apiCtx();
  await ctx.put(`${base}/api/v1/telemetry/config`, { data: original });
  await ctx.dispose();
});
test.beforeEach(async ({ page }) => {
  await page.addInitScript((id) => {
    localStorage.setItem('otto_workspace', id);
    localStorage.setItem('otto_firstrun_dismissed', '1');
  }, workspace);
});

test('opt-in settings, defaults, error recovery and responsive light/dark views', async ({ page }, info) => {
  await page.goto('/#/usage');
  await page.getByRole('tab', { name: 'Otto usage', exact: true }).click();
  await expect(page.getByLabel('Collect application telemetry on this Mac')).not.toBeChecked();
  await page.getByText('Retention, analysis and profiling', { exact: true }).click();
  await expect(page.getByLabel('Trace retention (days)')).toHaveValue('1');
  await expect(page.getByLabel('Log retention (days)')).toHaveValue('2');
  await expect(page.getByLabel('Metric retention (days)')).toHaveValue('7');
  await expect(page.getByLabel('Analyze every (hours)')).toHaveValue('24');
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await expect(page.locator('html')).toHaveAttribute('data-scheme', scheme);
    await page.screenshot({ path: info.outputPath(`telemetry-${scheme}.png`), fullPage: true, animations: 'disabled' });
    await info.attach(`telemetry-${scheme}`, { path: info.outputPath(`telemetry-${scheme}.png`), contentType: 'image/png' });
  }
  await page.getByLabel('Trace retention (days)').fill('2');
  await page.getByRole('tab', { name: 'Report', exact: true }).click();
  await expect(page.getByRole('dialog', { name: 'Discard unsaved changes?' })).toBeVisible();
  await page.getByRole('button', { name: 'Keep editing', exact: true }).click();
  await expect(page.getByLabel('Trace retention (days)')).toHaveValue('2');
  await page.getByLabel('Trace retention (days)').fill('1');
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.locator('.shell')).toHaveClass(/mobile/);
  await page.screenshot({ path: info.outputPath('telemetry-phone.png'), animations: 'disabled' });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1)).toBeTruthy();
  await page.route('**/api/v1/telemetry/config', (route) => route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ code: 'upstream', message: 'Fixture collector unavailable' }) }));
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByTestId('load-stale')).toContainText('Fixture collector unavailable');
  await expect(page.getByRole('button', { name: 'Retry', exact: true })).toHaveCount(1);
  await page.unroute('**/api/v1/telemetry/config');
  await page.getByRole('button', { name: 'Retry', exact: true }).click();
  await expect(page.getByTestId('load-stale')).toHaveCount(0);
});

test('authenticated configuration, bounded ingestion and opt-out reject unsafe input', async () => {
  const { ctx, base } = await apiCtx();
  try {
    expect((await ctx.get(`${base}/api/v1/telemetry/status`, { headers: { Authorization: '' } })).status()).toBe(401);
    expect((await ctx.put(`${base}/api/v1/telemetry/config`, { data: { ...original, traces_days: 0 } })).status()).toBe(400);
    expect(await (await ctx.get(`${base}/api/v1/telemetry/config`)).json()).toEqual(original);
    const span = { trace_id: '1'.repeat(32), span_id: '2'.repeat(16), parent_span_id: null, name: 'ui.render', component: 'agents', kind: 'internal', start_unix_nano: String(BigInt(Date.now()) * 1_000_000n), duration_ms: 1, status: 'ok', attributes: {} };
    expect(await (await ctx.post(`${base}/api/v1/telemetry/ingest`, { data: { spans: [span] } })).json()).toEqual({ accepted: 0 });
    expect((await ctx.post(`${base}/api/v1/telemetry/ingest`, { data: { spans: [span, { ...span, name: 'private-project-name' }] } })).status()).toBe(400);
    expect((await ctx.post(`${base}/api/v1/telemetry/ingest`, { data: { spans: Array.from({ length: 101 }, () => span) } })).status()).toBe(400);
    expect((await ctx.post(`${base}/api/v1/telemetry/ingest`, { data: { spans: [{ ...span, name: 'x'.repeat(140_000) }] } })).status()).toBe(413);
  } finally { await ctx.dispose(); }
});

test('actual browser → server → collector → ClickHouse trace and opt-out', async ({ page }, info) => {
  test.setTimeout(180_000);
  const { ctx, base } = await apiCtx();
  const saved = await ctx.put(`${base}/api/v1/telemetry/config`, { data: { ...original, enabled: true } });
  expect(saved.ok()).toBeTruthy();
  await expect.poll(async () => (await (await ctx.get(`${base}/api/v1/telemetry/status`)).json()).collector_ready, { timeout: 100_000 }).toBe(true);
  await page.goto('/#/agents');
  await expect.poll(async () => page.evaluate(async () => {
    const module = await import(/* @vite-ignore */ String('/src/lib/telemetry.ts'));
    return module.telemetryState().enabled;
  })).toBe(true);
  const traceIds = new Set<string>();
  page.on('request', (request) => {
    const header = request.headers()['traceparent'];
    if (header && !request.url().includes('/telemetry/')) traceIds.add(header.split('-')[1]);
  });
  await page.evaluate(async () => {
    const { router } = await import(/* @vite-ignore */ String('/src/lib/router.svelte.ts'));
    router.go('git');
  });
  await expect.poll(() => page.evaluate(async () => (await import(/* @vite-ignore */ String('/src/lib/router.svelte.ts'))).router.module)).toBe('git');
  await expect.poll(async () => {
    await page.evaluate(async () => (await import(/* @vite-ignore */ String('/src/lib/telemetry.ts'))).flushTelemetry());
    for (const id of traceIds) {
      const spans: { name: string; kind: string; span_id: string; parent_span_id: string | null }[] = await (await ctx.get(`${base}/api/v1/telemetry/traces/${id}`)).json();
      const navigation = spans.find((span) => span.name === 'ui.navigation');
      const render = spans.find((span) => span.name === 'ui.render');
      const client = spans.find((span) => span.kind === 'client' && span.parent_span_id === navigation?.span_id);
      const server = spans.find((span) => span.kind === 'server' && span.parent_span_id === client?.span_id);
      if (navigation && render?.parent_span_id === navigation.span_id && client && server) return true;
    }
    return false;
  }, { timeout: 30_000 }).toBe(true);
  await page.goto('/#/usage');
  await page.getByRole('tab', { name: 'Otto usage', exact: true }).click();
  await expect(page.getByText('Collecting locally', { exact: true })).toBeVisible();
  await expect.poll(async () => {
    const overview = await (await ctx.get(`${base}/api/v1/telemetry/overview?hours=1`)).json();
    return overview.resources.some((row: { cpu_percent: number | null }) => row.cpu_percent !== null);
  }, { timeout: 20_000 }).toBe(true);
  await page.getByRole('button', { name: 'Refresh', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'daemon', exact: true })).toBeVisible();
  await expect.poll(() => page.locator('.otto-usage .mc-svg').first().evaluate((node) => {
    const svg = node as SVGSVGElement;
    return Math.abs(svg.getBoundingClientRect().width / svg.viewBox.baseVal.width - 1);
  })).toBeLessThan(0.02);
  expect(await page.locator('.otto-usage .mc-svg').evaluateAll((charts) => charts.every((chart) => {
    const ticks = [...chart.querySelectorAll('.tick.x')].map((tick) => tick.getBoundingClientRect());
    return ticks.every((tick, index) => index === 0 || ticks[index - 1].right + 4 <= tick.left);
  }))).toBe(true);
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await expect(page.locator('html')).toHaveAttribute('data-scheme', scheme);
    await page.screenshot({ path: info.outputPath(`telemetry-active-${scheme}.png`), fullPage: true, animations: 'disabled' });
  }
  await ctx.put(`${base}/api/v1/telemetry/config`, { data: { ...original, enabled: false } });
  await expect.poll(async () => (await (await ctx.get(`${base}/api/v1/telemetry/status`)).json()).collector_ready).toBe(false);
  await ctx.dispose();
});
