// Canned Usage page data (desktop-usage-page.spec.ts, usage-page-mobile.spec.ts):
// a 30-day window ending today with four providers, models, kinds of work and
// sessions — what one isolated daemon with no transcripts cannot show. Every
// /usage/* request is counted so specs can assert what opening a view costs.
import type { Page } from '@playwright/test';
import { cannedLimits, routeLimits } from './usage-remaining-fixture';

const DAY = 86_400_000;
const p2 = (n: number) => String(n).padStart(2, '0');
const localDay = (t: number) => {
  const d = new Date(t);
  return `${d.getFullYear()}-${p2(d.getMonth() + 1)}-${p2(d.getDate())}`;
};
const tok = (total: number, cost: number) => ({
  events: Math.max(1, Math.round(total / 50_000)),
  input_tokens: Math.round(total * 0.1),
  output_tokens: Math.round(total * 0.02),
  cache_read_tokens: Math.round(total * 0.82),
  cache_write_tokens: Math.round(total * 0.06),
  total_tokens: total,
  cost_usd: cost,
});

export function usageSummary(days = 30) {
  const now = Date.now();
  const daily = Array.from({ length: days }, (_, i) => ({ day: localDay(now - (days - 1 - i) * DAY), ...tok(40_000_000 + (i % 7) * 6_000_000, 22 + (i % 7) * 3.5) }));
  const providers = [
    { provider: 'claude', ...tok(853_200_000, 529.62) },
    { provider: 'codex', ...tok(371_600_000, 230.64) },
    { provider: 'agy', ...tok(110_100_000, 68.34) },
    { provider: 'grok', ...tok(41_300_000, 25.63) },
  ];
  const models = [
    { provider: 'claude', model: 'claude-opus-5-5', ...tok(700_000_000, 470.1) },
    { provider: 'claude', model: 'claude-fable-5', ...tok(153_200_000, 59.52) },
    { provider: 'codex', model: 'gpt-5.5-codex-with-a-very-long-model-name', ...tok(371_600_000, 230.64) },
    { provider: 'agy', model: 'gemini-3-pro', ...tok(110_100_000, 68.34) },
  ];
  const sessions = Array.from({ length: 40 }, (_, i) => ({
    session_id: `sess-${String(i).padStart(4, '0')}-abcdef`,
    workspace_id: i % 5 === 0 ? 'external' : 'ws1',
    provider: i % 2 ? 'codex' : 'claude',
    model: i % 2 ? 'gpt-5.5-codex' : 'claude-opus-5-5',
    ...tok(30_000_000 - i * 600_000, 40 - i * 0.9),
    last_active: `${localDay(now - i * 3_600_000)} 14:32:05.000`,
    title: i % 5 === 0 ? null : `Fix flaky clipboard tests ${i}`,
    kind: i % 5 === 0 ? null : i % 3 ? 'agent' : 'review',
    workspace_name: i % 5 === 0 ? null : 'Harbor',
    fallback_priced: i === 3,
  }));
  const total = providers.reduce((s, p) => s + p.total_tokens, 0);
  const cost = providers.reduce((s, p) => s + p.cost_usd, 0);
  return {
    days,
    total_events: 27_600,
    total_input_tokens: Math.round(total * 0.1),
    total_output_tokens: Math.round(total * 0.02),
    total_cache_read_tokens: Math.round(total * 0.82),
    total_cache_write_tokens: Math.round(total * 0.06),
    total_tokens: total,
    total_cost_usd: cost,
    providers,
    daily,
    sessions,
    by_kind: [
      { feature: 'agent', sessions: 30, ...tok(1_000_000_000, 620.4) },
      { feature: 'review', sessions: 6, ...tok(300_000_000, 180.2) },
      { feature: 'goal_loop', sessions: 4, ...tok(76_200_000, 53.6) },
    ],
    models,
    daily_models: daily.map((d) => ({ ...d, provider: 'claude', model: 'claude-opus-5-5' })),
    scope: 'all',
  };
}

export const USAGE_STATUS = { available: true, enabled: true, binary: '/fixture/clickhouse', version: '25.1', data_dir: '/fixture/usage', retention_days: 180, metrics_interval_secs: 60, usage_rows: 12_345, metric_rows: 900, disk_bytes: 48_000_000, priced_as_of: '2026-11-01' };

export interface UsageRoutes {
  /** Every /api/v1/usage/* path requested, in order (method + path, no query). */
  calls: string[];
}

/** Route every /usage/* call the page makes to canned data; `budgetRows` adds caps. */
export async function routeUsage(page: Page, opts: { budgetRows?: unknown[]; empty?: boolean } = {}): Promise<UsageRoutes> {
  const calls: string[] = [];
  await routeLimits(page, cannedLimits());
  await page.route('**/api/v1/usage/**', async (route) => {
    const req = route.request();
    const url = new URL(req.url());
    const path = url.pathname.replace('/api/v1', '');
    calls.push(`${req.method()} ${path}`);
    if (path.startsWith('/usage/limits')) return route.fallback();
    const json = (body: unknown) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) });
    if (path === '/usage/status') return json(USAGE_STATUS);
    if (path === '/usage/summary') {
      const s = usageSummary(Number(url.searchParams.get('days') ?? 30));
      return json(opts.empty ? { ...s, total_events: 0, total_tokens: 0, total_cost_usd: 0, providers: [], daily: [], sessions: [], by_kind: [], models: [], daily_models: [] } : s);
    }
    if (path === '/usage/budgets') return json({ config: { enforce: false, block_on_exceed: false, window_days: 30, workspaces: [], providers: [] }, window_days: 30, rows: opts.budgetRows ?? [] });
    if (path === '/usage/metrics') return json(Array.from({ length: 30 }, (_, i) => ({ ts: new Date(Date.now() - (30 - i) * 60_000).toISOString(), cpu_pct: 10 + (i % 5) * 4, mem_used_mb: 9000, mem_total_mb: 16384, mem_pct: 55, load_avg_1: 2, process_rss_mb: 210, process_cpu_pct: 3, active_sessions: 4 })));
    if (path === '/usage/forecast') return json({ projected_cost_usd: 0.42, basis: 'fixture' });
    if (path === '/usage/attribution') return json([{ key: 'otto', cost_usd: 120.5, tokens: 90_000_000, sessions: 12 }, { key: 'koala-pragmatic', cost_usd: 40.25, tokens: 30_000_000, sessions: 4 }]);
    if (path === '/usage/report') return json({ days: 30, generated_at: new Date().toISOString(), priced_as_of: '2026-11-01', scope: 'all', otto_only: true, totals: tok(0, 0), daily: [], monthly: [], models: [], daily_models: [], sessions: [] });
    return route.fallback();
  });
  return { calls };
}
