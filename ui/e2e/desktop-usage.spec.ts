// Usage analytics over the persistent embedded ClickHouse server.
//
// The old engine spawned a fresh `clickhouse local` per query (no background
// merges → 25k-part bloat → ~30s cold load). This verifies the rework: the
// daemon brings up ONE long-lived `clickhouse server` (loopback HTTP), so the
// status flips `available`, every dashboard query path answers over HTTP, and a
// warm second call is fast. Data-correctness is covered by the crate's
// `tests/e2e.rs` against the real binary; here we prove the daemon wiring.
//
// CI-safe: if no clickhouse binary is installed on the host, the suite skips
// up front rather than timing out on an engine that can never come up. CI
// installs the pinned binary, so there it runs.

import { test, expect, type APIRequestContext } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { existsSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';
import { apiCtx } from './seed';

const V1 = '/api/v1';

/** Whether the daemon can find a `clickhouse` binary — the same lookup as
 *  `ClickHouse::locate` (crates/otto-usage/src/clickhouse.rs): PATH, then the
 *  well-known install locations. The test daemon shares this host's PATH and
 *  HOME (ui/e2e/global-setup.ts). */
function hasClickhouseBinary(): boolean {
  try {
    if (execFileSync('which', ['clickhouse'], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim()) return true;
  } catch {
    // not on PATH — fall through to the well-known locations
  }
  const home = homedir();
  return [
    '/usr/local/bin/clickhouse',
    '/opt/homebrew/bin/clickhouse',
    join(home, 'clickhouse'),
    join(home, '.local/bin/clickhouse'),
    join(home, 'Library/Application Support/Otto/bin/clickhouse'),
  ].some((p) => existsSync(p));
}

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
    test.skip(!hasClickhouseBinary(), 'no clickhouse binary on this host — usage engine stays disabled');
  });

  test('status available + all query paths answer + warm call is fast', async () => {
    // The engine's cold start (server boot + schema) shares the budget with
    // the wait below; leave room for the queries after it.
    test.setTimeout(90_000);
    const { ctx, base } = await apiCtx();
    const status = await waitAvailable(ctx, base);
    // With a binary present the engine MUST come up — never a silent skip.
    expect(status, 'usage engine never became available').not.toBeNull();
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

    // Tokens-first rollups: the summary carries the model breakdown + scope,
    // and the ccusage-style report answers with every table.
    expect(Array.isArray(summary.models), 'summary.models').toBeTruthy();
    expect(Array.isArray(summary.daily_models), 'summary.daily_models').toBeTruthy();
    expect(summary.scope).toBe('all');
    const rep = await ctx.get(`${base}${V1}/usage/report?days=30&otto_only=false`);
    expect(rep.ok(), await rep.text()).toBeTruthy();
    const report = await rep.json();
    for (const k of ['totals', 'daily', 'monthly', 'models', 'daily_models', 'sessions', 'priced_as_of']) {
      expect(report, `report missing ${k}`).toHaveProperty(k);
    }
    expect(report.otto_only).toBe(false);
    // Slim by default (the page): no day×model table; the export opts in.
    expect(report.daily_models).toEqual([]);
    expect(report.sessions.length).toBeLessThanOrEqual(100);
    const fullRep = await ctx.get(`${base}${V1}/usage/report?days=30&otto_only=false&sessions_limit=1000&include=daily_models`);
    expect(fullRep.ok(), await fullRep.text()).toBeTruthy();
    const full = await fullRep.json();
    expect(Array.isArray(full.daily_models), 'full.daily_models').toBeTruthy();
    expect(full.totals).toEqual(report.totals);

    // forecast (POST) prices an explicit estimate without needing history.
    const f = await ctx.post(`${base}${V1}/usage/forecast`, {
      data: { feature: 'agent', provider: 'claude', est_tokens: 2_000 },
    });
    expect(f.ok(), await f.text()).toBeTruthy();
    expect((await f.json()).projected_cost_usd).toBeGreaterThan(0);

    await ctx.dispose();
  });

  test('Usage page renders against the live daemon', async ({ page }) => {
    test.setTimeout(90_000);
    const { ctx, base } = await apiCtx();
    expect(await waitAvailable(ctx, base), 'usage engine never became available').not.toBeNull();
    await ctx.dispose();
    await page.goto('/#/usage');
    // The page mounts + shows its header regardless of how much data exists.
    await expect(page.getByRole('heading', { name: /Usage/i }).first()).toBeVisible({
      timeout: 30_000,
    });
    // With the engine up, the Report view opens and offers the HTML download.
    const reportBtn = page.getByTestId('usage-view-report');
    if (await reportBtn.isVisible().catch(() => false)) {
      await reportBtn.click();
      await expect(page.getByTestId('usage-report')).toBeVisible({ timeout: 15_000 });
      await expect(page.getByRole('button', { name: /Download HTML/ })).toBeVisible();
    }
  });
});
