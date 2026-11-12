// Shared by desktop-usage-remaining.spec.ts and usage-remaining-mobile.spec.ts:
// seed one REAL Codex rollout into the isolated daemon's transcript fixture dir
// (`<data_dir>/e2e-transcripts/codex`, the Codex root an OTTO_E2E daemon reads —
// otto-transcript roots.rs) so GET /usage/limits answers from a file, exactly as
// it does from ~/.codex/sessions; plus canned responses for the states a single
// daemon root cannot show at once (stale, reset, error, empty).
import { expect, type APIRequestContext, type Page } from '@playwright/test';
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

const SLOT = process.env.OTTO_E2E_SLOT ?? '0';
const MIN = 60_000;
const HOUR = 60 * MIN;

function dataDir(): string {
  const p = join(process.cwd(), 'e2e', `.auth-${SLOT}`, 'daemon.json');
  return (JSON.parse(readFileSync(p, 'utf8')) as { dataDir: string }).dataDir;
}

/** The `token_count` line Codex writes after every turn. `info: null` keeps it
 *  out of the token totals the usage tailer records from the same dir. */
function tokenCountLine(now: number): string {
  return JSON.stringify({
    timestamp: new Date(now - 2 * MIN).toISOString(),
    type: 'event_msg',
    payload: {
      type: 'token_count',
      info: null,
      rate_limits: {
        limit_id: 'codex',
        limit_name: null,
        primary: { used_percent: 13.0, window_minutes: 300, resets_at: Math.floor((now + 2 * HOUR + 20 * MIN) / 1000) },
        secondary: { used_percent: 41.0, window_minutes: 10080, resets_at: Math.floor((now + 76 * HOUR) / 1000) },
        credits: { has_credits: true, unlimited: false, balance: '62500.0' },
        plan_type: 'pro',
        rate_limit_reached_type: null,
      },
    },
  });
}

/** Write the rollout and wait until the daemon serves it (its per-root cache
 *  holds an earlier answer for up to 15 s). */
export async function seedCodexLimits(ctx: APIRequestContext, base: string): Promise<void> {
  const now = Date.now();
  const d = new Date(now);
  const day = join(
    dataDir(),
    'e2e-transcripts',
    'codex',
    String(d.getFullYear()),
    String(d.getMonth() + 1).padStart(2, '0'),
    String(d.getDate()).padStart(2, '0'),
  );
  mkdirSync(day, { recursive: true });
  writeFileSync(join(day, 'rollout-e2e-usage-remaining.jsonl'), tokenCountLine(now) + '\n');
  await expect
    .poll(
      async () => {
        const r = await ctx.get(`${base}/api/v1/usage/limits`);
        if (!r.ok()) return `HTTP ${r.status()}`;
        const body = (await r.json()) as { accounts: { key: string; status: string }[] };
        return body.accounts.find((a) => a.key === 'codex:default')?.status ?? 'missing';
      },
      { timeout: 30_000, intervals: [500, 1000, 2000] },
    )
    .toBe('ok');
}

/** A canned answer with every row state: a stale profile with one window
 *  reset since its reading and one nearly spent, and an unknown Claude login. */
export function cannedLimits(now = Date.now()): unknown {
  const iso = (t: number) => new Date(t).toISOString();
  return {
    generated_at: iso(now),
    stale_after_secs: 1800,
    accounts: [
      {
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
          { id: 'primary', name: '5-hour', window_minutes: 300, used_percent: null, left_percent: null, resets_at: iso(now - HOUR), reset_since_snapshot: true },
          { id: 'secondary', name: 'Weekly', window_minutes: 10080, used_percent: 92, left_percent: 8, resets_at: iso(now + 30 * HOUR), reset_since_snapshot: false },
        ],
        credits: null,
        limit_reached: null,
      },
      {
        key: 'claude:account:acc-max',
        provider: 'claude',
        account_id: 'acc-max',
        label: 'Max',
        source: 'account',
        status: 'unknown',
        reason: 'Claude Code does not write its plan limits to disk (they reach only its status line and /usage), so Otto has nothing local to read.',
        plan: null,
        taken_at: null,
        windows: [],
        credits: null,
        limit_reached: null,
      },
    ],
  };
}

/** Answer GET /usage/limits from the page with `body` (or a 500). */
export async function routeLimits(page: Page, body: unknown | 'error'): Promise<void> {
  await page.route('**/api/v1/usage/limits', (route) =>
    body === 'error'
      ? route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ code: 'internal', message: 'read provider limits: e2e failure' }) })
      : route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify(body) }),
  );
}

export const remaining = (page: Page) => page.getByTestId('usage-remaining');

/** One Claude login in a given check state: `off` (never checked, switch off),
 *  `on` (switched on, not yet checked), `ok` (checked just now), `stale`
 *  (checked 2 h ago) or `failed` (the CLI could not answer). */
export function claudeLimits(state: 'off' | 'on' | 'ok' | 'stale' | 'failed', now = Date.now()): unknown {
  const iso = (t: number) => new Date(t).toISOString();
  const at = state === 'stale' ? now - 2 * HOUR : now - MIN;
  const numbers = state === 'ok' || state === 'stale';
  return {
    generated_at: iso(now),
    stale_after_secs: 1800,
    accounts: [
      {
        key: 'claude:account:acc-max',
        provider: 'claude',
        account_id: 'acc-max',
        label: 'Max',
        source: 'account',
        status: numbers ? (state === 'stale' ? 'stale' : 'ok') : 'unknown',
        reason: numbers
          ? null
          : state === 'failed'
            ? 'Couldn’t check with Anthropic: The Claude CLI didn’t answer within 10 s.'
            : state === 'on'
              ? 'Not checked with Anthropic yet.'
              : 'Claude Code does not write its plan limits to disk. Otto shows them once a structured Claude session reports them, or when Check with Anthropic is on for this login.',
        plan: numbers ? 'max' : null,
        taken_at: numbers ? iso(at) : null,
        windows: numbers
          ? [
              // The shape the real CLI 2.1.322 answered on a Max plan: no
              // 5-hour window, the weekly one, and an inactive Fable bucket.
              { id: 'seven_day', name: 'Weekly', window_minutes: 10080, used_percent: 61, left_percent: 39, resets_at: iso(now + 60 * HOUR), reset_since_snapshot: false, active: true, severity: 'normal' },
              { id: 'seven_day_fable', name: 'Weekly · Fable', window_minutes: 10080, used_percent: 23, left_percent: 77, resets_at: iso(now + 60 * HOUR), reset_since_snapshot: false, active: false, severity: 'normal' },
            ]
          : [],
        credits: null,
        limit_reached: null,
        check: {
          enabled: state !== 'off',
          checking: false,
          checked_at: state === 'off' || state === 'on' ? null : iso(at),
          error: state === 'failed' ? 'The Claude CLI didn’t answer within 10 s.' : null,
        },
      },
    ],
  };
}
