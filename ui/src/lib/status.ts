// One status vocabulary for the whole app (patterns.md §1). Pure mapping —
// domain state in, `{ key, label, tone }` out — so a session reads the same in
// the sidebar, the tab strip, a tile, the pane header and the composer, and a
// run reads the same in Workflows, Swarm, Mission Control, Scheduled Tasks and
// Reviews. `StatusBadge` / `StatusDot` / `EnvBadge` render these; tones map to
// the SEMANTIC tokens (--success/--warning/--danger/--info + -soft), never to
// the --status-* indicator colours (those are for dots only).
//
// Kept free of Svelte/DOM imports so node:test can cover it (unit/status.test.ts).

/** Semantic tone of a status. `neutral` = --text-dim on a --surface-2 tint. */
export type Tone = 'neutral' | 'info' | 'success' | 'warning' | 'danger';

export interface StatusInfo {
  /** Stable, normalised key — safe for `data-status` / class names / tests. */
  key: string;
  /** Sentence-case human label ("Needs you", "Succeeded"). */
  label: string;
  tone: Tone;
  /** Live and moving right now — the dot pulses (unless reduced motion). */
  live?: boolean;
  /** Longer explanation for a tooltip / status line. */
  hint?: string;
}

// ---------------------------------------------------------------------------
// Sessions
// ---------------------------------------------------------------------------

/** Mirrors `SessionStatus` in api/types.ts (kept structural so this module has
 *  no app imports). */
export type RawSessionStatus = 'running' | 'working' | 'idle' | 'exited' | 'reconnectable';

export type SessionStateKey =
  | 'working'
  | 'needs-you'
  | 'running'
  | 'idle'
  | 'suspended'
  | 'ended'
  | 'failed'
  | 'stale';

/** What `StatusDot` draws for a session state. */
export type DotKind = SessionStateKey;

export interface SessionStateInfo extends StatusInfo {
  key: SessionStateKey;
  /** Parked but resumable: opening (or Resume) brings it back via `--resume`. */
  resumable: boolean;
  /** No live process: show Resume/Reconnect instead of an input. */
  inactive: boolean;
}

export interface SessionLike {
  kind?: string | null;
  provider_session_id?: string | null;
  status?: RawSessionStatus | string | null;
}

export interface SessionStateOpts {
  /** The events stream is not connected — live claims can't be trusted. */
  stale?: boolean;
  /** Process exit code when known (a terminal `exit` frame). */
  exitCode?: number | null;
}

export const SUSPENDED_HINT = 'Suspended — resumes on open';

/** A session is "suspended / resumable" when it is `reconnectable`, or an
 *  exited AGENT session that still carries its provider session id (opening it
 *  runs `--resume`). A plain exited shell is genuinely ended. */
export function isResumable(session: SessionLike | null | undefined, status: string | null | undefined): boolean {
  if (status === 'reconnectable') return true;
  return status === 'exited' && session?.kind === 'agent' && session?.provider_session_id != null;
}

/** A session with no live process: failed (non-zero exit), suspended
 *  (resumable) or ended. Used by `sessionState` and the terminal's exit
 *  overlay, which knows the exit code. */
export function exitState(exitCode: number | null | undefined, resumable: boolean): SessionStateInfo {
  const base = { resumable, inactive: true };
  if (exitCode != null && exitCode !== 0) {
    return {
      ...base,
      key: 'failed',
      label: `Failed (exit ${exitCode})`,
      tone: 'danger',
      hint: resumable ? `Exited with code ${exitCode} — resume to continue` : `Exited with code ${exitCode}`,
    };
  }
  if (resumable) return { ...base, key: 'suspended', label: 'Suspended', tone: 'neutral', hint: SUSPENDED_HINT };
  return { ...base, key: 'ended', label: 'Ended', tone: 'neutral', hint: 'This session has ended' };
}

/**
 * The one session state every surface shows.
 *
 * - `liveStatus` is the events-fed `ws.statusMap[id]`; falls back to the row's
 *   own `status`, then `idle`.
 * - `needsYou` (the sticky operator flag) wins over every LIVE state, but a
 *   session with no process left reads as suspended/ended, not "needs you".
 * - `stale`: while the events socket is down a "working"/"running" claim may
 *   be minutes old — say "Reconnecting…" and stop pulsing (patterns.md §1).
 */
export function sessionState(
  session: SessionLike | null | undefined,
  liveStatus?: RawSessionStatus | string | null,
  needsYou = false,
  opts: SessionStateOpts = {},
): SessionStateInfo {
  const status = liveStatus ?? session?.status ?? 'idle';
  const resumable = isResumable(session, status);
  const base = { resumable, inactive: false };

  if (status === 'exited' || status === 'reconnectable') return exitState(opts.exitCode, resumable);

  if (needsYou) {
    return {
      ...base,
      key: 'needs-you',
      label: 'Needs you',
      tone: 'warning',
      live: !opts.stale,
      hint: 'Waiting on you — input or a permission',
    };
  }

  if (opts.stale && (status === 'working' || status === 'running')) {
    return {
      ...base,
      key: 'stale',
      label: 'Reconnecting…',
      tone: 'neutral',
      hint: 'Live updates paused — last known state may be out of date',
    };
  }

  switch (status) {
    case 'working':
      return { ...base, key: 'working', label: 'Working', tone: 'success', live: true, hint: 'Working' };
    case 'running':
      return { ...base, key: 'running', label: 'Running', tone: 'info', hint: 'Running — alive, not producing output' };
    default:
      return { ...base, key: 'idle', label: 'Idle', tone: 'neutral', hint: 'Idle' };
  }
}

// ---------------------------------------------------------------------------
// Runs (workflows, swarm, mission control, scheduled tasks, reviews, evals)
// ---------------------------------------------------------------------------

export type RunStatusKey =
  | 'succeeded'
  | 'failed'
  | 'queued'
  | 'waiting'
  | 'running'
  | 'cancelled'
  | 'skipped';

const RUN_ALIASES: Record<string, RunStatusKey> = {
  success: 'succeeded',
  succeeded: 'succeeded',
  successful: 'succeeded',
  ok: 'succeeded',
  done: 'succeeded',
  completed: 'succeeded',
  complete: 'succeeded',
  passed: 'succeeded',
  pass: 'succeeded',
  finished: 'succeeded',
  error: 'failed',
  errored: 'failed',
  failed: 'failed',
  failure: 'failed',
  fail: 'failed',
  timeout: 'failed',
  timed_out: 'failed',
  pending: 'queued',
  queued: 'queued',
  scheduled: 'queued',
  waiting: 'waiting',
  waiting_approval: 'waiting',
  awaiting_approval: 'waiting',
  blocked: 'waiting',
  paused: 'waiting',
  running: 'running',
  in_progress: 'running',
  started: 'running',
  starting: 'running',
  active: 'running',
  working: 'running',
  cancelled: 'cancelled',
  canceled: 'cancelled',
  stopped: 'cancelled',
  aborted: 'cancelled',
  killed: 'cancelled',
  skipped: 'skipped',
};

const RUN_INFO: Record<RunStatusKey, Omit<StatusInfo, 'key'>> = {
  succeeded: { label: 'Succeeded', tone: 'success' },
  failed: { label: 'Failed', tone: 'danger' },
  queued: { label: 'Queued', tone: 'neutral' },
  waiting: { label: 'Waiting', tone: 'warning' },
  running: { label: 'Running', tone: 'info', live: true },
  cancelled: { label: 'Cancelled', tone: 'neutral' },
  skipped: { label: 'Skipped', tone: 'neutral' },
};

/** "waiting_on_user" → "Waiting on user". */
export function sentenceCase(raw: string): string {
  const s = raw.replace(/[_-]+/g, ' ').trim().toLowerCase();
  return s ? s[0].toUpperCase() + s.slice(1) : s;
}

/** Normalise any run/step/job status word into the shared vocabulary. Unknown
 *  words keep their own (sentence-cased) label with a neutral tone. */
export function runStatus(raw: string | null | undefined): StatusInfo {
  const k = (raw ?? '').trim().toLowerCase().replace(/[\s-]+/g, '_');
  const key = RUN_ALIASES[k];
  if (key) return { key, ...RUN_INFO[key] };
  return { key: k || 'unknown', label: k ? sentenceCase(k) : 'Unknown', tone: 'neutral' };
}

// ---------------------------------------------------------------------------
// Environments
// ---------------------------------------------------------------------------

export type EnvKey = 'prod' | 'staging' | 'dev';

/** prod → danger, staging → warning, dev (and anything unknown) → neutral. */
export function envTone(env: string | null | undefined): StatusInfo & { key: EnvKey | string } {
  const e = (env ?? '').trim().toLowerCase();
  if (e === 'prod' || e === 'production' || e === 'prd' || e === 'live') {
    return { key: 'prod', label: 'prod', tone: 'danger', hint: 'Production' };
  }
  if (e === 'staging' || e === 'stage' || e === 'stg' || e === 'preprod' || e === 'uat') {
    return { key: 'staging', label: 'staging', tone: 'warning', hint: 'Staging' };
  }
  if (e === '' || e === 'dev' || e === 'development' || e === 'local' || e === 'test') {
    return { key: 'dev', label: 'dev', tone: 'neutral', hint: 'Development' };
  }
  return { key: e, label: e, tone: 'neutral' };
}

// ---------------------------------------------------------------------------
// Product story stages
// ---------------------------------------------------------------------------

/** The lifecycle stages an operator sets by hand (the Overview stage picker).
 *  The daemon also writes intermediate stages as agents work (`imported`,
 *  `analyzed`, `refined`, `tests_drafted`, `planned`). */
export const STORY_STAGES = ['draft', 'review', 'approved', 'done'] as const;

const STAGE_INFO: Record<string, Omit<StatusInfo, 'key'>> = {
  draft: { label: 'Draft', tone: 'neutral', hint: 'Draft — not reviewed yet' },
  imported: { label: 'Imported', tone: 'neutral', hint: 'Imported from the source, not analysed yet' },
  analyzed: { label: 'Analyzed', tone: 'info', hint: 'Agents analysed the story' },
  refined: { label: 'Refined', tone: 'info', hint: 'An agent suggested a rewrite' },
  tests_drafted: { label: 'Tests drafted', tone: 'info', hint: 'Test cases were generated' },
  planned: { label: 'Planned', tone: 'info', hint: 'An implementation plan exists' },
  review: { label: 'Review', tone: 'warning', hint: 'Waiting on a review' },
  approved: { label: 'Approved', tone: 'success', hint: 'Approved — ready to send to a swarm' },
  done: { label: 'Done', tone: 'success', hint: 'Delivered' },
};

/** One mapping for a Product story's `stage` — the story list rows, the
 *  Overview stage picker and the epic Children board all read it. Unknown
 *  stages keep their own sentence-cased label with a neutral tone. Done is
 *  success, never the accent (accent means selected). */
export function storyStage(raw: string | null | undefined): StatusInfo {
  const k = (raw ?? '').trim().toLowerCase().replace(/[\s-]+/g, '_');
  const info = STAGE_INFO[k];
  if (info) return { key: k, ...info };
  return { key: k || 'unknown', label: k ? sentenceCase(k) : 'Unknown', tone: 'neutral' };
}
