import { test, expect, type Page } from '@playwright/test';
import { apiCtx, seedWorkspace } from './seed';

// Kafka figures that are not known yet must never read as 0 (phones + tablets).
// The cluster can't connect in CI, so overview / metrics / groups are stubbed
// at the network layer:
// - Overview throughput: the first metrics answer has no rate yet (one sweep
//   is only a baseline) → "measuring…"; the next poll carries a rate → it shows.
//   The owner saw "0" here until the Topics tab had been opened once.
// - Consumer groups list: per-group lag, "—" for unknown, "…" while computing.

let workspaceId = '';
let clusterId = '';

test.beforeAll(async () => {
  const { ctx, base } = await apiCtx();
  workspaceId = await seedWorkspace(ctx, base);
  const r = await ctx.post(`${base}/api/v1/workspaces/${workspaceId}/brokers/clusters`, {
    data: { name: 'figures-kafka', bootstrap_servers: 'broker-1.example.com:9092', security_protocol: 'plaintext' },
  });
  if (!r.ok()) throw new Error(`seed cluster → ${r.status()} ${await r.text()}`);
  clusterId = (await r.json()).id;
  await ctx.dispose();
});

const overview = {
  cluster_id: 'c',
  controller_id: 1,
  brokers: [{ id: 1, host: 'broker-1.example.com', port: 9092, rack: null, is_controller: true, partition_leaders: 3 }],
  topic_count: 2,
  internal_topic_count: 0,
  partition_count: 6,
  consumer_group_count: 3,
};

function metrics(rate: number | null, total: number | null) {
  return {
    throughput: rate == null ? [] : [{ ts_ms: Date.now(), total_messages: total, messages_per_sec: rate }],
    messages_per_sec: rate,
    total_messages: total,
    brokers: [],
    prometheus_available: false,
    sampled_at: new Date().toISOString(),
  };
}

async function openCluster(page: Page): Promise<void> {
  await page.addInitScript((wsId) => {
    localStorage.setItem('otto_workspace', wsId as string);
    localStorage.setItem('otto_rail_expanded', '0');
  }, workspaceId);
  await page.goto('/#/brokers');
  await expect(page.locator('.brokers-page')).toBeVisible({ timeout: 30_000 });
  await page.locator('.cluster .cn', { hasText: 'figures-kafka' }).first().click();
  await expect(page.locator('.cluster-head .ph-title')).toBeVisible({ timeout: 15_000 });
}

test('overview throughput reads "measuring…" until a rate exists, never 0', async ({ page }) => {
  let calls = 0;
  await page.route(new RegExp(`/brokers/clusters/${clusterId}/overview`), (route) => route.fulfill({ json: overview }));
  await page.route(new RegExp(`/brokers/clusters/${clusterId}/metrics`), (route) => {
    calls += 1;
    return route.fulfill({ json: calls === 1 ? metrics(null, 1000) : metrics(42, 1200) });
  });
  await openCluster(page);
  await page.locator('.tabs button', { hasText: 'Overview' }).first().click();

  const card = page.locator('.card', { hasText: 'Throughput' });
  await expect(card.locator('.v')).toHaveText('measuring…', { timeout: 15_000 });
  await expect(card.locator('.v')).not.toHaveText(/^0/);
  await expect(card.locator('.sub')).toHaveText(`${(1000).toLocaleString()} total`);
  // The next poll (≈4 s) carries a rate — no Topics-tab visit needed.
  await expect(card.locator('.v')).toHaveText(/^42\s*\/s$/, { timeout: 15_000 });
});

test('consumer group list shows lag, "—" when unknown and "…" while computing', async ({ page }) => {
  await page.route(new RegExp(`/brokers/clusters/${clusterId}/groups$`), (route) =>
    route.fulfill({
      json: [
        { group_id: 'billing', state: 'Stable', protocol_type: 'consumer', members: 2, total_lag: 1500, lag_pending: false },
        { group_id: 'connect-sink', state: 'Stable', protocol_type: 'connect', members: 1, total_lag: null, lag_pending: false },
        { group_id: 'orders', state: 'Empty', protocol_type: 'consumer', members: 0, total_lag: null, lag_pending: true },
      ],
    }),
  );
  await page.route(new RegExp(`/brokers/clusters/${clusterId}/groups/`), (route) =>
    route.fulfill({ status: 502, json: { error: 'upstream', message: 'no broker' } }),
  );
  await openCluster(page);
  await page.locator('.tabs button', { hasText: 'Consumer groups' }).first().click();

  const row = (id: string) => page.locator('.grow-row', { hasText: id }).locator('.lag');
  await expect(row('billing')).toHaveText(`lag ${(1500).toLocaleString()}`, { timeout: 15_000 });
  await expect(row('connect-sink')).toHaveText('lag —');
  await expect(row('connect-sink')).toHaveAttribute('title', /Not a consumer group/);
  await expect(row('orders')).toHaveText('lag …');
  // The lag badge must stay inside the phone-width list.
  const fits = await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth + 1);
  expect(fits).toBe(true);
});
