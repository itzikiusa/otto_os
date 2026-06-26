// Usage analytics over the persistent embedded ClickHouse server.
//
// The old engine spawned a fresh `clickhouse local` per query (no background
// merges → 25k-part bloat → ~30s cold load). This verifies the rework: the
// daemon brings up ONE long-lived `clickhouse server` (loopback HTTP), so the
// status flips `available`, every dashboard query path answers over HTTP, and a
// warm second call is fast. Data-correctness is covered by the crate's
// `tests/e2e.rs` against the real binary; here we prove the daemon wiring.
//
// CI-safe: if no clickhouse binary is installed on the host (engine never
// becomes available), the data assertions skip rather than fail.

import { test, expect, type APIRequestContext } from '@playwright/test';
import { apiCtx } from './seed';

const V1 = '/api/v1';

async function waitAvailable(ctx: APIRequestContext, base: string, ms = 45_000) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    const r = await ctx.get(`${base}${V1}/usage/status`);
    if (r.ok()) {
      const s = await r.json();
      if (s.available) return s;
    }
    await new Promise((res) => setTimeout(res, 500));
  }
  return null;
}

test.describe('usage (persistent clickhouse server)', () => {
  test.beforeEach(({}, testInfo) => {
    test.skip(testInfo.project.name !== 'desktop-browser', 'desktop-only suite');
  });

  test('status available + all query paths answer + warm call is fast', async () => {
    const { ctx, base } = await apiCtx();
    const status = await waitAvailable(ctx, base);
    if (!status) {
      await ctx.dispose();
      test.skip(true, 'no clickhouse binary on this host — usage engine stays disabled');
      return;
    }
    // The persistent server is up and the schema is live.
    expect(status.available).toBeTruthy();
    expect(String(status.version || '')).toContain('ClickHouse');
    expect(typeof status.disk_bytes === 'number' || status.disk_bytes === undefined).toBeTruthy();

    // summary (cold) returns a valid shape over the HTTP server path.
    const t0 = Date.now();
    let r = await ctx.get(`${base}${V1}/usage/summary?days=30`);
    expect(r.ok(), await r.text()).toBeTruthy();
    const summary = await r.json();
    for (const k of ['days', 'total_events', 'total_tokens', 'providers', 'daily', 'sessions']) {
      expect(summary, `summary missing ${k}`).toHaveProperty(k);
    }
    expect(Array.isArray(summary.providers)).toBeTruthy();
    const coldMs = Date.now() - t0;

    // warm: a second identical call hits the always-on server → fast.
    const t1 = Date.now();
    r = await ctx.get(`${base}${V1}/usage/summary?days=30`);
    expect(r.ok()).toBeTruthy();
    const warmMs = Date.now() - t1;
    // Generous bound: warm is normally tens of ms. The whole point of the rework
    // is that this is NOT seconds (the old per-query spawn re-attached the data).
    expect(warmMs, `warm summary was ${warmMs}ms`).toBeLessThan(3_000);
    // eslint-disable-next-line no-console
    console.log(`[usage-e2e] summary cold=${coldMs}ms warm=${warmMs}ms`);

    // every other dashboard query path answers 200 over the server.
    for (const path of [
      '/usage/by-kind?days=30',
      '/usage/metrics?minutes=60',
      '/usage/attribution?by=origin&days=30',
    ]) {
      const rr = await ctx.get(`${base}${V1}${path}`);
      expect(rr.ok(), `${path} → ${rr.status()} ${await rr.text()}`).toBeTruthy();
    }

    // forecast (POST) prices an explicit estimate without needing history.
    const f = await ctx.post(`${base}${V1}/usage/forecast`, {
      data: { feature: 'agent', provider: 'claude', est_tokens: 2_000 },
    });
    expect(f.ok(), await f.text()).toBeTruthy();
    expect((await f.json()).projected_cost_usd).toBeGreaterThan(0);

    await ctx.dispose();
  });

  test('Usage page renders against the live daemon', async ({ page }) => {
    const { ctx, base } = await apiCtx();
    await waitAvailable(ctx, base, 20_000);
    await ctx.dispose();
    await page.goto('/#/usage');
    // The page mounts + shows its header regardless of how much data exists.
    await expect(page.getByRole('heading', { name: /Usage/i }).first()).toBeVisible({
      timeout: 30_000,
    });
  });
});
