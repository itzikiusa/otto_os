// Contract mirror: insights-share. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id, WorkspaceRole,
} from './core';

// ---------------------------------------------------------------------------
// Insights — scheduled (opt-in, catch-up) HTML reports.
// ---------------------------------------------------------------------------

/** Which scheduled insight reports are turned on. All default to false (opt-in). */
export interface InsightsConfig {
  /** Previous day, generated the next morning the app is open. */
  daily: boolean;
  /** Previous week (Sunday), catch-up if the app was closed. */
  weekly: boolean;
  /** Previous month (1st), catch-up if the app was closed. */
  monthly: boolean;
  /** Agent provider to generate reports on (built-in or custom, e.g. grok).
   *  Empty = the global default agent. */
  provider?: string;
  /** Optional model alias (empty = provider default). */
  model?: string;
}

export type InsightKind = 'daily' | 'weekly' | 'monthly' | 'adhoc';

/** One generated insight report. `html_path` points at the rendered HTML. */
export interface InsightReport {
  kind: InsightKind;
  /** ISO start of the period the report covers. */
  period_start: string;
  /** ISO end of the period the report covers. */
  period_end: string;
  /** On-disk path to the rendered HTML report. */
  html_path: string | null;
  /** Optional preview (80 lines / 64 KiB); empty for metadata-only pages. */
  summary: string;
  /** ISO timestamp the report was created. */
  created_at: string;
}

/** The period to run an ad-hoc insights report for. */
export type InsightRunPeriod = 'day' | 'week' | 'month';

export interface RunInsightsReq {
  period: InsightRunPeriod;
  /** How many calendar periods back (0 = current, 1 = previous complete period). */
  offset?: number;
}

export interface RunInsightsResp {
  started: boolean;
  /** Requested daemon-local collector period, e.g. daily:20260924_20260924. */
  report_key?: string | null;
  /** HTML revision before the run; a changed summary alone is not completion. */
  report_revision?: string | null;
  /** Session id of the spawned insights run (when started === true). */
  run_id?: string | null;
  /** Human-readable explanation when started === false (e.g. skill not installed). */
  reason?: string | null;
  /** True when a run for this period was already generating and this request
   *  attached to it (`run_id` is that run) instead of starting a second one. */
  attached?: boolean;
}

/** `GET /insights/runs/active` row — an insights run still generating. */
export interface ActiveInsightsRun {
  run_id: string;
  report_key: string;
  report_revision: string | null;
  started_at: string;
}

export interface InsightReportStatus {
  report: InsightReport | null;
  html_revision: string | null;
}

// ---------------------------------------------------------------------------
// Share-link tokens (Phase 3 — remote/mobile access)
// Mirrors crates/otto-core/src/api.rs ShareInfo / CreateShareReq / CreateShareResp.
// ---------------------------------------------------------------------------

/** Metadata for one scoped share-link token. The raw token is never present
 *  after the initial mint response (only the first 12 chars are stored). */
export interface ShareInfo {
  id: Id;
  /** The single session this token may reach. */
  session_id: Id;
  /** Capped role on that session: 'viewer' (read-only) or 'editor' (read + input). */
  role: WorkspaceRole;
  /** First 12 chars of the raw token, for display/identification. */
  token_prefix: string;
  label: string | null;
  created_at: string;
  /** FIXED expiry (created_at + ttl); never slid for share tokens. */
  expires_at: string;
  /** Lapsed email-OTP share the link holder can still revive via
   *  `POST /share/extend` (until its 7-day absolute lifetime) — listed so the
   *  owner can revoke it. */
  dormant?: boolean;
}

/** `GET /auth/shares` row — one of the caller's live links, any session. */
export interface MyShare extends ShareInfo {
  /** The shared session's title; null when the session no longer exists. */
  session_title: string | null;
}

/** `POST /api/v1/sessions/{id}/share` request body. */
export interface CreateShareReq {
  /** 'viewer' (read-only) or 'editor' (read + input). Never 'admin'. */
  role: string;
  /** Fixed TTL in seconds (plain share only). Absent → 3600. Clamped to [60, 86400]. */
  ttl_secs?: number;
  /** Human-friendly label (e.g. "for Alice"). Optional. */
  label?: string;
  /** Email-OTP gate (mobile plan Task 7.2). When set, the recipient must redeem a
   *  6-digit code emailed to THIS address (POST /share/verify) before attaching —
   *  a leaked link alone is useless. The address is LOCKED for the share's life.
   *  Requires a verified email sender; absent → a plain share with no OTP gate. */
  recipient_email?: string;
  /** Session window (seconds) for an OTP-gated share — how long the guest may stay
   *  attached once verified. Clamped server-side to (0, 43200] (≤12h). Only
   *  meaningful with `recipient_email`; absent → default 1h. */
  duration_secs?: number;
}

/** `POST /api/v1/share/verify` — redeem an emailed OTP for a share token (mobile
 *  plan Task 7.3). Public/Exempt: the `token` (the share link) is the auth. */
export interface VerifyShareReq {
  /** The raw share token (from the `#/s/<session>/<token>` link). */
  token: string;
  /** The 6-digit one-time code the recipient received by email. */
  otp: string;
}

/** Response for `POST /api/v1/share/verify`. */
export interface VerifyShareResp {
  /** `true` once the code matched and the share is verified for attach. */
  verified: boolean;
}

/** `POST /api/v1/share/extend` — re-issue a FRESH OTP for an existing OTP share,
 *  emailed to the LOCKED original recipient ONLY (mobile plan Task 7.4).
 *  Public/Exempt: the `token` (the share link) is the auth. The request carries
 *  NO email field by design — the destination is read from the share row, never
 *  the request — so access can never be redirected to a different mailbox. The
 *  fresh code re-pends the share and opens a fresh ≤12h window; the guest then
 *  re-verifies via `POST /api/v1/share/verify`. Returns `{ ok: true }`. */
export interface ExtendShareReq {
  /** The raw share token (from the `#/s/<session>/<token>` link). */
  token: string;
}

/** Response for `POST /api/v1/sessions/{id}/share`. The raw token is returned
 *  exactly once — store or copy it immediately. */
export interface CreateShareResp {
  /** The raw share token (shown exactly once). */
  token: string;
  /** Ready-to-use share URL (`<origin>/#/s/<session_id>/<token>`). */
  url: string;
  /** Metadata for the newly-minted share. */
  info: ShareInfo;
  /** Whether another device can open `url`. False when the origin is loopback
   *  or empty (no Public link domain and no network listener) — show the
   *  "only works on this Mac" warning instead of the phone QR hint. */
  reachable_remotely: boolean;
  /** Who can open `url` (S20-303): `remote` (routable/public), `lan` (the
   *  LAN listener's private address — same Wi-Fi only, self-signed
   *  certificate) or `local` (this Mac only). Absent on older daemons. */
  reach?: ShareReach;
}

/** See {@link CreateShareResp.reach}. */
export type ShareReach = 'remote' | 'lan' | 'local';
