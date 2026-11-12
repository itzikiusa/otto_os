// Mock-mode `GET /usage/limits` (ui/src/lib/api/mock.ts routes it). Default:
// a fresh Codex login, a stale Codex profile (one window reset since its
// reading) and two Claude logins Otto has no numbers for.
// `localStorage.otto_mock_limits = 'empty'` answers with no logins, `'error'`
// fails the request — read per request, so an e2e can flip it.
import type { AccountLimits, LimitsResp } from './types/usage-limits';

type Out = { status?: number; json?: unknown } | undefined;
type MockRoute = { method: string; re: RegExp; handle: (m: RegExpMatchArray, body: unknown, q: URLSearchParams) => Out };

const MIN = 60_000;
const HOUR = 60 * MIN;

function mode(): string {
  try {
    return localStorage.getItem('otto_mock_limits') ?? 'default';
  } catch {
    return 'default';
  }
}

const iso = (t: number): string => new Date(t).toISOString();

const CLAUDE_REASON =
  'Claude Code does not write its plan limits to disk. Otto shows them once a structured Claude session reports them, or when Check with Anthropic is on for this login.';

/** Claude rows whose "Check with Anthropic" switch is on, and when each last ran. */
const checked = new Map<string, number | null>();

export function mockUsageLimits(now: number = Date.now()): LimitsResp {
  const fresh: AccountLimits = {
    key: 'codex:default',
    provider: 'codex',
    account_id: null,
    label: 'Default Codex login',
    source: 'default',
    status: 'ok',
    reason: null,
    plan: 'pro',
    taken_at: iso(now - 4 * MIN),
    windows: [
      { id: 'primary', name: '5-hour', window_minutes: 300, used_percent: 13, left_percent: 87, resets_at: iso(now + 2 * HOUR + 10 * MIN), reset_since_snapshot: false },
      { id: 'secondary', name: 'Weekly', window_minutes: 10080, used_percent: 41, left_percent: 59, resets_at: iso(now + 76 * HOUR), reset_since_snapshot: false },
    ],
    credits: { has_credits: true, unlimited: false, balance: '62500.0' },
    limit_reached: null,
  };
  const stale: AccountLimits = {
    key: 'codex:account:acc-work',
    provider: 'codex',
    account_id: 'acc-work',
    label: 'Work',
    source: 'account',
    status: 'stale',
    reason: null,
    plan: 'plus',
    taken_at: iso(now - 5 * HOUR),
    windows: [
      { id: 'primary', name: '5-hour', window_minutes: 300, used_percent: null, left_percent: null, resets_at: iso(now - 1 * HOUR), reset_since_snapshot: true },
      { id: 'secondary', name: 'Weekly', window_minutes: 10080, used_percent: 92, left_percent: 8, resets_at: iso(now + 30 * HOUR), reset_since_snapshot: false },
    ],
    credits: null,
    limit_reached: null,
  };
  const unknown = (key: string, label: string, source: string, account_id: string | null): AccountLimits => ({
    key,
    provider: 'claude',
    account_id,
    label,
    source,
    status: 'unknown',
    reason: CLAUDE_REASON,
    plan: null,
    taken_at: null,
    windows: [],
    credits: null,
    limit_reached: null,
    check: { enabled: checked.has(key), checking: false, checked_at: null, error: null },
  });
  const claude = (key: string, label: string, source: string, account_id: string | null): AccountLimits => {
    const at = checked.get(key);
    if (at === undefined) return unknown(key, label, source, account_id);
    if (at === null) return { ...unknown(key, label, source, account_id), reason: 'Not checked with Anthropic yet.' };
    return {
      ...unknown(key, label, source, account_id),
      status: now - at > 30 * MIN ? 'stale' : 'ok',
      reason: null,
      plan: 'max',
      taken_at: iso(at),
      windows: [
        { id: 'five_hour', name: '5-hour', window_minutes: 300, used_percent: 23.5, left_percent: 76.5, resets_at: iso(at + 3 * HOUR), reset_since_snapshot: false },
        { id: 'seven_day', name: 'Weekly', window_minutes: 10080, used_percent: 41.2, left_percent: 58.8, resets_at: iso(at + 60 * HOUR), reset_since_snapshot: false },
      ],
      check: { enabled: true, checking: false, checked_at: iso(at), error: null },
    };
  };
  return {
    accounts: [fresh, stale, claude('claude:~/.claude', 'Default Claude login', 'default', null), claude('claude:account:acc-max', 'Max', 'account', 'acc-max')],
    generated_at: iso(now),
    stale_after_secs: 1800,
  };
}

export const usageLimitsMockRoutes: MockRoute[] = [
  {
    method: 'GET',
    re: /^\/usage\/limits$/,
    handle: () => {
      const m = mode();
      if (m === 'error') return { status: 500, json: { code: 'internal', message: 'read provider limits: mock failure' } };
      if (m === 'empty') return { json: { accounts: [], generated_at: new Date().toISOString(), stale_after_secs: 1800 } satisfies LimitsResp };
      return { json: mockUsageLimits() };
    },
  },
  {
    method: 'PUT',
    re: /^\/usage\/limits\/claude-check$/,
    handle: (_m, body) => {
      const { key, enabled } = body as { key: string; enabled: boolean };
      if (enabled) checked.set(key, null);
      else checked.delete(key);
      return { json: mockUsageLimits() };
    },
  },
  {
    method: 'POST',
    re: /^\/usage\/limits\/claude-check\/run$/,
    handle: (_m, body) => {
      const { key } = body as { key: string };
      if (!checked.has(key)) return { status: 400, json: { code: 'invalid', message: 'Turn on Check with Anthropic for this login first.' } };
      checked.set(key, Date.now());
      return { json: mockUsageLimits() };
    },
  },
];
