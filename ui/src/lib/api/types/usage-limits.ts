// `GET /usage/limits` — remaining subscription allowance per provider login.
// Mirrors `otto_usage::limits` (docs/contracts/api.md, Usage).

export type LimitStatus = 'ok' | 'stale' | 'unknown';

export interface LimitCredits {
  has_credits: boolean;
  unlimited: boolean;
  /** As the CLI wrote it (a decimal string, e.g. "62500.0"). */
  balance: string | null;
}

export interface WindowLimit {
  /** `primary` / `secondary`; `<limit_id>:<slot>` for a model-specific bucket. */
  id: string;
  /** `5-hour`, `Weekly`, `Monthly`, … */
  name: string;
  window_minutes: number | null;
  /** null once the window reset after the snapshot. */
  used_percent: number | null;
  left_percent: number | null;
  resets_at: string | null;
  /** `resets_at` passed after the snapshot was taken. */
  reset_since_snapshot: boolean;
  /** false: listed by the provider but not enforced right now (shown dimmed). Absent on older daemons. */
  active?: boolean;
  /** The provider's own level (`normal`, …) when it sent one. */
  severity?: string | null;
}

export interface AccountLimits {
  key: string;
  provider: 'codex' | 'claude' | 'agy' | 'grok' | string;
  account_id: string | null;
  label: string;
  /** `default` (the CLI's own home) | `config_dir` (~/.claude-*) | `account` (an Otto profile). */
  source: 'default' | 'config_dir' | 'account' | string;
  status: LimitStatus;
  /** Why there are no numbers (status `unknown`). */
  reason: string | null;
  plan: string | null;
  /** When the CLI saw these numbers. */
  taken_at: string | null;
  windows: WindowLimit[];
  credits: LimitCredits | null;
  /** Set while a window was exhausted (as of `taken_at`). */
  limit_reached: string | null;
  /** Claude rows only: the opt-in "Check with Anthropic" state (absent on older daemons). */
  check?: ClaudeCheck | null;
}

/** The opt-in active check of one Claude login (off by default). */
export interface ClaudeCheck {
  enabled: boolean;
  /** A check is running now. */
  checking: boolean;
  /** When the last check finished. */
  checked_at: string | null;
  /** Why the last check failed (while enabled). */
  error: string | null;
}

/** `PUT /usage/limits/claude-check` → LimitsResp. */
export interface ClaudeCheckReq {
  key: string;
  enabled: boolean;
}

/** `POST /usage/limits/claude-check/run` → LimitsResp. */
export interface ClaudeRunReq {
  key: string;
}

export interface LimitsResp {
  accounts: AccountLimits[];
  generated_at: string;
  /** A snapshot older than this is `stale`. */
  stale_after_secs: number;
}
