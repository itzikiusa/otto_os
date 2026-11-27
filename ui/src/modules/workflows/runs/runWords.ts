// The words of a run and its steps (ux.md §2): the run's status word and tone,
// a step's word and glyph, a reviewer's word, a deliverable's line, the run
// page's sub line (it ticks, so it is the UI's — the headline is the engine's
// `overview.headline`) and the Runs inbox's folds and order. Pure; unit-tested
// in ui/unit/workflowRunWords.test.ts.
import type {
  ActiveWorkflowRun,
  AttentionKind,
  ChildThread,
  Deliverable,
  NodeRunState,
  RunListItem,
  RunOverview,
  RunState,
  RunSubject,
} from '../../../lib/api/types';
import { elapsed, type StatusWord } from '../../../lib/statusWords.ts';

/** A word and its tone (the tones of lib/statusWords.ts). */
export type Word = StatusWord;

const calm = (word: string): Word => ({ word, tone: '' });

/** "1st", "2nd", "3rd", "11th". */
export function ordinal(n: number): string {
  const teen = n % 100 >= 11 && n % 100 <= 13;
  const suffix = teen ? 'th' : ['th', 'st', 'nd', 'rd'][n % 10] ?? 'th';
  return `${n}${suffix}`;
}

/** A step's duration: "0.4s", "38s", "3m 12s", "1h 5m". */
export function stepDuration(ms: number): string {
  if (ms < 1000) return `${Math.round(ms / 100) / 10}s`;
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${String(s % 60).padStart(2, '0')}s`.replace(/ 00s$/, '');
  return elapsed(ms);
}

/** The word a person-wait shows (several can wait: the oldest decides). */
export function attentionWord(kind: AttentionKind | string | undefined): string {
  switch (kind) {
    case 'approval': return 'Approval';
    case 'question': return 'Question';
    case 'verify': return 'Verify';
    case 'usage_limit': return 'Limited';
    default: return 'Needs you';
  }
}

/** A list row or run the engine did not summarize (a row from an older daemon):
 *  its state from the stored status. */
export function fallbackState(status: string, waitingApproval = false): RunState {
  switch (status) {
    case 'pending': return 'queued';
    case 'running': return waitingApproval ? 'needs_you' : 'running';
    case 'success': return 'succeeded';
    case 'error': return 'failed';
    case 'canceled': return 'canceled';
    default: return 'unknown';
  }
}

/** The fields of an overview the words read. */
export type OverviewLike = Pick<RunOverview, 'state' | 'attention' | 'queue'> & Partial<Pick<RunOverview, 'hold'>>;

/** The ring centre's accessible/hover time label and the small word under the
 *  number, so elapsed time ("2m") never reads like a countdown or timeout.
 *  `typical`: formatted usual duration (e.g. "25m") when known on a live run. */
export function orbitTime(state: RunState, span: string, typical?: string): { label: string; sub: string } {
  switch (state) {
    case 'running':
      return {
        label: typical ? `Running for ${span} (usually about ${typical})` : `Running for ${span}`,
        sub: typical ? `of ~${typical}` : 'elapsed',
      };
    case 'waiting':
      return {
        label: typical ? `Waiting ${span} (usually about ${typical})` : `Waiting ${span}`,
        sub: typical ? `of ~${typical}` : 'waiting',
      };
    case 'needs_you':
    case 'blocked':
    case 'exhausted':
      return { label: `Waiting ${span}`, sub: 'waiting' };
    case 'queued':
      return { label: `Queued ${span}`, sub: 'queued' };
    case 'paused':
      return { label: `Paused after ${span}`, sub: 'paused' };
    case 'succeeded':
    case 'failed':
    case 'canceled':
      return { label: `Took ${span}`, sub: 'total' };
    default:
      return { label: `Elapsed ${span}`, sub: 'elapsed' };
  }
}

/** The run's word (ux.md §2.1). `startedAt`/`now` tick "Working 12m". `raw`:
 *  the state as sent, for a state this build does not know. */
export function runWord(ov: OverviewLike, opts: { startedAt?: string | null; now?: number; raw?: string } = {}): Word {
  switch (ov.state) {
    case 'needs_you': return { word: attentionWord(ov.attention[0]?.kind), tone: 'warning' };
    case 'failed': return { word: 'Failed', tone: 'danger' };
    case 'queued': return calm(ov.queue?.position ? `Queued · ${ordinal(ov.queue.position)}` : 'Queued');
    case 'waiting': return calm('Waiting');
    case 'running': {
      const t = opts.startedAt ? Date.parse(opts.startedAt) : NaN;
      return calm(opts.now && Number.isFinite(t) ? `Working ${elapsed(opts.now - t)}` : 'Working');
    }
    case 'canceled': return calm('Canceled');
    case 'succeeded': return { word: 'Done', tone: 'success' };
    case 'paused': return calm('Paused');
    case 'blocked': return { word: 'Blocked', tone: 'warning' };
    case 'exhausted': return { word: 'Out of budget', tone: 'warning' };
    default: {
      const raw = (opts.raw ?? ov.state ?? '').replace(/_/g, ' ');
      return calm(raw ? raw.charAt(0).toUpperCase() + raw.slice(1) : 'Unknown');
    }
  }
}

/** Where a run sits in the inbox. */
export type InboxFoldKey = 'needs' | 'running' | 'recent';

export function inboxFold(state: RunState): InboxFoldKey {
  if (state === 'needs_you' || state === 'blocked' || state === 'exhausted') return 'needs';
  if (state === 'running' || state === 'waiting' || state === 'queued' || state === 'paused') return 'running';
  return 'recent';
}

/** One inbox row, from the active list or the history. */
export interface InboxRun {
  id: string;
  workflowId: string;
  workflowName: string;
  status: string;
  startedAt: string;
  finishedAt: string | null;
  durationMs: number | null;
  overview: RunOverview | null;
  /** A history row's own subject, for a row without an overview. */
  subject?: RunSubject | null;
  state: RunState;
  /** The first line of the ask (history rows). */
  message: string | null;
  repo: string | null;
  branch: string | null;
  pr: string | null;
  currentStep: string | null;
  error: string | null;
}

export function fromActive(r: ActiveWorkflowRun): InboxRun {
  return {
    id: r.run_id, workflowId: r.workflow_id, workflowName: r.workflow_name, status: r.status,
    startedAt: r.started_at, finishedAt: null, durationMs: null, overview: r.overview ?? null,
    state: r.overview?.state ?? fallbackState(r.status, r.waiting_approval),
    message: null, repo: null, branch: null, pr: null, currentStep: r.current_step ?? null, error: null,
  };
}

export function fromHistory(r: RunListItem): InboxRun {
  return {
    id: r.id, workflowId: r.workflow_id, workflowName: r.workflow_name, status: r.status,
    startedAt: r.started_at, finishedAt: r.finished_at ?? null, durationMs: r.duration_ms ?? null,
    overview: r.overview ?? null, subject: r.subject ?? null, state: r.overview?.state ?? fallbackState(r.status),
    message: r.message ?? null, repo: r.repo ?? null, branch: r.branch ?? null, pr: r.pr ?? null,
    currentStep: r.current_step ?? null, error: r.error ?? null,
  };
}

const t = (iso: string | null | undefined): number => (iso ? Date.parse(iso) || 0 : 0);

/** The inbox's three folds in order (patterns.md §10): Needs you, oldest wait
 *  first; Running, queued last by position; Recent, unread failures first,
 *  then newest first. `unread(run)`: finished since the person last looked. */
export function inboxFolds(runs: readonly InboxRun[], unread: (r: InboxRun) => boolean): Record<InboxFoldKey, InboxRun[]> {
  const out: Record<InboxFoldKey, InboxRun[]> = { needs: [], running: [], recent: [] };
  for (const r of runs) out[inboxFold(r.state)].push(r);
  const since = (r: InboxRun) => t(r.overview?.attention[0]?.since ?? r.overview?.hold?.since ?? r.startedAt);
  out.needs.sort((a, b) => since(a) - since(b));
  const queued = (r: InboxRun) => (r.state === 'queued' ? r.overview?.queue?.position ?? Number.MAX_SAFE_INTEGER : 0);
  out.running.sort((a, b) => queued(a) - queued(b) || t(b.startedAt) - t(a.startedAt));
  const urgent = (r: InboxRun) => (r.state === 'failed' && unread(r) ? 0 : 1);
  out.recent.sort((a, b) => urgent(a) - urgent(b) || t(b.finishedAt ?? b.startedAt) - t(a.finishedAt ?? a.startedAt));
  return out;
}

/** Line 2 of a row: the PR or ticket, else the branch or repository, else the ask, else "{workflow} run". */
export function runTitle(r: Pick<InboxRun, 'overview' | 'subject' | 'message' | 'branch' | 'repo' | 'pr' | 'workflowName'>): string {
  const s = r.overview?.subject ?? r.subject;
  if (s?.pr) return s.pr.title ? `PR #${s.pr.number} · ${s.pr.title}` : `${s.pr.repo || 'PR'} #${s.pr.number}`;
  if (s?.ticket) return s.ticket.title ? `${s.ticket.key} · ${s.ticket.title}` : s.ticket.key;
  if (s?.branch || s?.repo) return (s.branch ?? s.repo)!;
  if (r.pr) return r.repo ? `${r.repo} #${r.pr.replace(/^#/, '')}` : `PR ${r.pr}`;
  if (r.branch || r.repo) return (r.branch ?? r.repo)!;
  const first = r.message?.split('\n').find((l) => l.trim());
  return first?.trim() || `${r.workflowName} run`;
}

/** Line 1 of a row: "{workflow} · {repository or ticket}". */
export function runContext(r: Pick<InboxRun, 'overview' | 'subject' | 'workflowName' | 'repo'>): string {
  const s = r.overview?.subject ?? r.subject;
  const where = s?.pr?.repo ?? s?.repo ?? s?.ticket?.key ?? r.repo;
  return where ? `${r.workflowName} · ${where}` : r.workflowName;
}

/** "3m ago", "just now". */
export function ago(ms: number): string {
  if (ms < 60_000) return 'just now';
  return ms < 86_400_000 ? `${elapsed(ms)} ago` : `${Math.floor(ms / 86_400_000)}d ago`;
}

/** The engine's headline, or the UI's fallback for a row without an overview. */
export function headline(r: Pick<InboxRun, 'overview' | 'state' | 'currentStep' | 'error' | 'durationMs'>): string {
  if (r.overview?.headline) return r.overview.headline;
  switch (r.state) {
    case 'failed': return r.error ? `Failed: ${r.error.split('\n')[0]}` : 'Failed';
    case 'succeeded': return r.durationMs != null ? `Done in ${elapsed(r.durationMs)}` : 'Done';
    case 'needs_you': return 'Needs you — an approval is waiting';
    case 'queued': return 'Queued';
    default: return r.currentStep ?? '';
  }
}

/** Line 3 of a row: the headline without the prefix the word already says,
 *  "usually ~{typical}" while running, who started it and where, and how long
 *  ago a finished run ended. */
export function runLine3(r: InboxRun, now: number): string {
  const head = headline(r).replace(/^(Needs you|Queued) — /, '');
  const parts = [head.charAt(0).toUpperCase() + head.slice(1)];
  const ov = r.overview;
  const live = r.state === 'running' || r.state === 'waiting';
  if (live && ov?.typical_ms) parts.push(`usually ~${elapsed(ov.typical_ms)}`);
  const o = ov?.origin;
  if (o?.user_name && o.channel_name) parts.push(`${o.user_name.split(' ')[0]} in ${o.channel_name}`);
  else if (o?.user_name || o?.channel_name) parts.push((o.user_name ?? o.channel_name)!);
  const waitSince = ov?.attention[0]?.since;
  if (r.state === 'needs_you' && waitSince) parts.push(ago(now - t(waitSince)));
  else if (inboxFold(r.state) === 'recent' && r.finishedAt) parts.push(ago(now - t(r.finishedAt)));
  return parts.filter(Boolean).join(' · ');
}

/** "14:02" in the person's locale. */
export function clockTime(iso: string): string {
  return new Date(iso).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

/** The run page's sub line (ux.md §2.3). `waitingSteps`: steps waiting for
 *  the machine right now. */
export function subLine(
  ov: Pick<RunOverview, 'state' | 'attention' | 'queue' | 'time' | 'typical_ms'> & Partial<Pick<RunOverview, 'hold'>>,
  run: { startedAt: string; finishedAt?: string | null; now: number; waitingSteps?: number },
): string {
  const typical = ov.typical_ms ? elapsed(ov.typical_ms) : null;
  const total = (run.finishedAt ? t(run.finishedAt) : run.now) - t(run.startedAt);
  const parts: string[] = [];
  switch (ov.state) {
    case 'running':
    case 'waiting':
      if (typical) parts.push(`Usually about ${typical}`);
      if (run.waitingSteps) parts.push(`${run.waitingSteps} waiting`);
      break;
    case 'needs_you': {
      const a = ov.attention[0];
      if (a?.kind === 'usage_limit' && a.resets_at) return `Otto continues it at ${clockTime(a.resets_at)}`;
      const verb = a?.kind === 'approval' ? 'decide' : 'answer';
      parts.push(a ? `Waiting ${elapsed(run.now - t(a.since))}` : 'Waiting', `the run continues when you ${verb}`);
      break;
    }
    case 'queued': {
      const cap = ov.queue?.cap ?? 0;
      return cap === 2 ? 'Both run slots are busy' : cap > 0 ? `All ${cap} run slots are busy` : 'Every run slot is busy';
    }
    case 'failed':
      parts.push(`After ${elapsed(total)}`);
      if (run.finishedAt) parts.push(ago(run.now - t(run.finishedAt)));
      if (typical) parts.push(`usually about ${typical}`);
      break;
    case 'succeeded': {
      if (run.finishedAt) parts.push(`Finished at ${clockTime(run.finishedAt)}`);
      if (ov.typical_ms) {
        const diff = total - ov.typical_ms;
        if (Math.abs(diff) >= ov.typical_ms * 0.1) parts.push(`${elapsed(Math.abs(diff))} ${diff < 0 ? 'faster' : 'slower'} than usual`);
      }
      break;
    }
    case 'canceled':
      parts.push(`After ${elapsed(total)}`);
      if (run.finishedAt) parts.push(ago(run.now - t(run.finishedAt)));
      break;
    case 'paused':
      if (ov.hold?.reason) parts.push(ov.hold.reason);
      break;
    case 'blocked':
    case 'exhausted':
      if (ov.hold?.since) parts.push(`Waiting for you since ${clockTime(ov.hold.since)}`);
      break;
  }
  const finished = ov.state === 'succeeded' || ov.state === 'failed' || ov.state === 'canceled';
  if (finished && total > 0 && ov.time.waiting_ms >= total * 0.25) parts.push(`${elapsed(ov.time.waiting_ms)} of it waiting for the machine`);
  return parts.join(' · ');
}

/** A step's mark beside its name. */
export type StepGlyph = 'pending' | 'clock' | 'working' | 'needs' | 'done' | 'failed' | 'dash';

export interface StepWord extends Word {
  glyph: StepGlyph;
}

/** A step's word and glyph (ux.md §2.2). `run`: the run's state — once it
 *  ended, never-reached steps say "Not run". `kind`: a condition shows its outcome. */
export function stepWord(n: NodeRunState, opts: { now: number; run?: RunState; kind?: string }): StepWord {
  const runEnded = !!opts.run && !['queued', 'running', 'waiting', 'needs_you'].includes(opts.run);
  const since = (iso?: string | null) => (iso ? elapsed(opts.now - t(iso)) : '');
  switch (n.status) {
    case 'running':
      switch (n.phase) {
        case 'queued': return { word: 'Queued', tone: '', glyph: 'pending' };
        case 'waiting': return { word: `Waiting ${since(n.wait?.since ?? n.started_at)}`.trim(), tone: '', glyph: 'clock' };
        case 'starting': return { word: 'Starting', tone: '', glyph: 'working' };
        case 'finishing': return { word: 'Finishing', tone: '', glyph: 'working' };
        case 'needs_you': return { word: attentionWord(n.attention?.kind), tone: 'warning', glyph: 'needs' };
        default: return { word: `Working ${since(n.started_at)}`.trim(), tone: '', glyph: 'working' };
      }
    case 'success': {
      if (opts.kind === 'condition') {
        const r = (n.output as { result?: unknown } | null | undefined)?.result;
        if (typeof r === 'boolean') return { word: r ? 'Yes' : 'No', tone: '', glyph: 'done' };
      }
      return { word: n.duration_ms != null ? stepDuration(n.duration_ms) : 'Done', tone: '', glyph: 'done' };
    }
    case 'error': {
      const word = n.error_class === 'timeout' ? 'Timed out' : n.error_class === 'auth' ? 'Signed out' : 'Failed';
      return { word: n.duration_ms != null ? `${word} · ${elapsed(n.duration_ms)}` : word, tone: 'danger', glyph: 'failed' };
    }
    case 'skipped':
      if (n.error_class === 'canceled') return { word: 'Stopped', tone: '', glyph: 'dash' };
      return { word: opts.run === 'canceled' && !n.started_at && !n.brief ? 'Not run' : 'Skipped', tone: '', glyph: 'dash' };
    default:
      return { word: runEnded ? 'Not run' : '', tone: '', glyph: 'pending' };
  }
}

/** A step row's second line: what it waits on or does while live, its brief
 *  once done, the reason when skipped. */
export function stepLine2(n: NodeRunState): string {
  if (n.status === 'running') {
    if (n.phase === 'waiting') return n.wait?.text ?? '';
    if (n.phase === 'needs_you') return n.attention?.text ?? '';
    return n.activity?.phase ?? '';
  }
  if (n.status === 'skipped' || n.status === 'success') return n.brief ?? '';
  return '';
}

/** A reviewer's (child thread's) word. An earlier failed attempt followed by a
 *  retry reads muted ("Attempt 1 failed · retried") instead of bare danger "Failed". */
export function childWord(c: ChildThread & { retried_attempt?: number | null }, now: number): Word {
  if (c.status === 'error' && c.retried_attempt != null) {
    return calm(`Attempt ${c.retried_attempt} failed · retried`);
  }
  switch (c.status) {
    case 'running': return calm(c.started_at ? `Working ${elapsed(now - t(c.started_at))}` : 'Working');
    case 'success': return { word: c.note ? `Done · ${c.note}` : 'Done', tone: '' };
    case 'error': return { word: 'Failed', tone: 'danger' };
    case 'skipped': return calm('Skipped');
    case 'needs_you': return { word: 'Needs you', tone: 'warning' };
    default: return calm('Queued');
  }
}

/** One deliverable's words. */
export function deliverableWord(d: Deliverable): Word {
  switch (d.state) {
    case 'done': return { word: d.detail ?? 'Posted', tone: 'success' };
    case 'partial': return { word: d.detail ?? 'Partly posted', tone: 'warning' };
    case 'failed': return { word: d.detail ? `Not posted: ${d.detail}` : 'Not posted', tone: 'danger' };
    case 'pending_approval': return { word: 'Waiting for approval', tone: 'warning' };
    default: return calm('Not confirmed');
  }
}

/** Failed and pending deliverables first. */
export function deliverablesInOrder(list: readonly Deliverable[]): Deliverable[] {
  const rank = (d: Deliverable) => (d.state === 'failed' ? 0 : d.state === 'pending_approval' || d.state === 'partial' ? 1 : 2);
  return [...list].sort((a, b) => rank(a) - rank(b));
}

/** A step's own error as a card: the leading "<kind>:" dropped, the first
 *  sentence the title, the rest the body. */
export function errorParts(error: string): { title: string; body: string } {
  const text = error.trim().replace(/^[a-z][a-z0-9_]*:\s+/, '');
  const m = /^(.+?[.!?])(\s+)([\s\S]+)$/.exec(text);
  if (!m) return { title: text, body: '' };
  return { title: m[1], body: m[3].trim() };
}

/** "24 hours", "7 days": a retention setting in words. */
function keptFor(h: number): string {
  if (h % 24 === 0 && h >= 48) return `${h / 24} days`;
  return h === 1 ? '1 hour' : `${h} hours`;
}

/** The read-only note under a step's conversation, from the two retention
 *  settings (`workflow_thread_retention_success_hours` / `_failed_hours`). */
export function retentionNote(successHours: number, failedHours: number): string {
  const lead = 'Read-only: this conversation belongs to the run.';
  if (successHours === 0) return `${lead} It is removed when the step succeeds; a summary stays (${keptFor(failedHours)} if the run fails).`;
  return `${lead} It is kept for ${keptFor(successHours)} after the run ends (${keptFor(failedHours)} if the run fails).`;
}
