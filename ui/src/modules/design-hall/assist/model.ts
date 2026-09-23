// Design assist — pure helpers (no Svelte, no fetch) behind the Otto panel,
// the variants tray, the lobby hand-off and the learning page: the turn-status
// reducer fed by `design_assist_updated`, provenance-chip mapping (verified vs
// unverified citations), quick action → request mapping, variant cards, the
// chat thread and the learned-rule vocabulary. Unit-tested in
// ui/unit/designAssist.test.ts, so the components stay thin.
//
// Contract: docs/contracts/api.md § "Design assist"; ws.md
// `design_assist_updated` / `design_variants_ready`.

import type {
  DesignAssistMode,
  DesignAssistReq,
  DesignAssistStatus,
  DesignAssistTurn,
  DesignLearnedEdit,
  DesignLearnedResp,
  DesignRuleCandidate,
  DesignSignalReq,
  DesignVariantRun,
  DesignVariantsReq,
  DesignVersion,
} from '../../../lib/api/types';

export type Tone = 'neutral' | 'info' | 'warn' | 'ok' | 'bad';

// ── Turn status ─────────────────────────────────────────────────────────────

const TERMINAL: ReadonlySet<DesignAssistStatus> = new Set(['done', 'unchanged', 'conflict', 'failed']);

export function isTerminal(s: DesignAssistStatus | string): boolean {
  return TERMINAL.has(s as DesignAssistStatus);
}

export interface StatusInfo {
  label: string;
  tone: Tone;
  /** Still running — show the working dot + Stop/View live session. */
  working: boolean;
}

/** Words for a turn's state (never colour alone). */
export function statusInfo(status: DesignAssistStatus | string, mode: string = 'refine'): StatusInfo {
  switch (status) {
    case 'starting':
      return { label: 'Starting…', tone: 'info', working: true };
    case 'running':
      return { label: 'Working', tone: 'info', working: true };
    case 'done':
      return { label: mode === 'variant' ? 'Variant ready' : 'New version', tone: 'ok', working: false };
    case 'unchanged':
      return { label: mode === 'critique' ? 'Review ready' : 'No changes', tone: 'neutral', working: false };
    case 'conflict':
      return { label: 'Kept as a draft', tone: 'warn', working: false };
    case 'failed':
      return { label: 'Failed', tone: 'bad', working: false };
    default:
      return { label: String(status), tone: 'neutral', working: false };
  }
}

/** The fields of `design_assist_updated` the reducer reads. */
export interface AssistEventLike {
  artifact_id: string;
  workspace_id: string;
  turn_id: string;
  status: DesignAssistStatus;
  mode: DesignAssistMode | 'variant';
  branch: string;
  session_id: string | null;
  version_id: string | null;
  error: string | null;
}

/** A turn known only from its event (the full turn comes from `GET …/assist`). */
export function placeholderTurn(ev: AssistEventLike, now: string): DesignAssistTurn {
  return {
    turn_id: ev.turn_id,
    artifact_id: ev.artifact_id,
    workspace_id: ev.workspace_id,
    mode: ev.mode,
    status: ev.status,
    branch: ev.branch,
    direction: null,
    provider: '',
    session_id: ev.session_id,
    base_version_id: null,
    version_id: ev.version_id,
    references: [],
    cited: [],
    unverified_citations: [],
    team_rules: [],
    findings: [],
    message: null,
    error: ev.error,
    started_at: now,
    finished_at: isTerminal(ev.status) ? now : null,
  };
}

/**
 * Fold one `design_assist_updated` into the known turns (newest first).
 * Events can arrive out of order across a reconnect: a terminal state never
 * regresses to `running`. Ids that travel as `null` never erase a known one.
 * `refetch` = the turn just finished → fetch the full turn (summary,
 * citations, findings) from `GET …/assist`.
 */
export function applyAssistEvent(
  turns: DesignAssistTurn[],
  ev: AssistEventLike,
  now: string,
): { turns: DesignAssistTurn[]; refetch: boolean } {
  const i = turns.findIndex((t) => t.turn_id === ev.turn_id);
  if (i < 0) return { turns: [placeholderTurn(ev, now), ...turns], refetch: isTerminal(ev.status) };
  const cur = turns[i];
  if (isTerminal(cur.status) && !isTerminal(ev.status)) return { turns, refetch: false };
  const next: DesignAssistTurn = {
    ...cur,
    status: ev.status,
    branch: ev.branch || cur.branch,
    session_id: ev.session_id ?? cur.session_id,
    version_id: ev.version_id ?? cur.version_id,
    error: ev.error ?? (isTerminal(ev.status) ? cur.error : null),
    finished_at: isTerminal(ev.status) ? (cur.finished_at ?? now) : null,
  };
  const out = turns.slice();
  out[i] = next;
  return { turns: out, refetch: isTerminal(ev.status) && !isTerminal(cur.status) };
}

/**
 * Merge a fresh `GET …/assist` answer with what the panel already knows. The
 * server's copy wins (it has the summary + citations), except that a turn the
 * server no longer lists (it keeps ≤ 20, none after a restart) is kept, and a
 * locally-terminal turn is never overwritten by a stale running copy.
 */
export function mergeTurns(local: DesignAssistTurn[], fetched: DesignAssistTurn[]): DesignAssistTurn[] {
  const byId = new Map(local.map((t) => [t.turn_id, t]));
  const out: DesignAssistTurn[] = [];
  for (const f of fetched) {
    const l = byId.get(f.turn_id);
    out.push(l && isTerminal(l.status) && !isTerminal(f.status) ? l : f);
    byId.delete(f.turn_id);
  }
  for (const l of byId.values()) out.push(l);
  return out.sort((a, b) => b.started_at.localeCompare(a.started_at));
}

/** Whether any turn (or variants run) still holds the artifact. */
export function busyTurn(turns: DesignAssistTurn[]): DesignAssistTurn | null {
  return turns.find((t) => !isTerminal(t.status)) ?? null;
}

// ── Provenance chips ────────────────────────────────────────────────────────

export interface ProvChip {
  key: string;
  label: string;
  /** Tooltip / accessible description. */
  title: string;
  kind: 'ref' | 'brand' | 'unverified';
  /** Opens this artifact (null for unverified citations). */
  artifactId: string | null;
  versionId: string | null;
  verified: boolean;
}

function obj(v: unknown): Record<string, unknown> | null {
  return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
}

/**
 * The chips under an agent message: every VERIFIED citation (`turn.cited` —
 * the server dropped anything it couldn't check) named after the offered
 * reference, the brand kit the version was drawn with (from the committed
 * version's provenance, when known), then each unverified citation, flagged —
 * never clickable, because it is not in the provenance.
 */
export function provenanceChips(turn: DesignAssistTurn, provenance?: Record<string, unknown> | null): ProvChip[] {
  const chips: ProvChip[] = [];
  const seen = new Set<string>();
  for (const c of turn.cited) {
    if (seen.has(c.label)) continue;
    seen.add(c.label);
    const offered = turn.references.find((r) => r.label === c.label);
    const title = offered?.title ?? 'Reference';
    const seq = c.seq ?? offered?.seq ?? null;
    chips.push({
      key: `ref:${c.label}`,
      label: `${c.label} ${title}`,
      title: `${c.label}: ${title}${seq != null ? ` v${seq}` : ''} — verified citation. Opens the reference.`,
      kind: 'ref',
      artifactId: c.artifact_id,
      versionId: c.version_id,
      verified: true,
    });
  }
  const bk = obj(provenance?.brand_kit);
  if (bk && typeof bk.artifact_id === 'string') {
    const seq = typeof bk.seq === 'number' ? bk.seq : null;
    chips.push({
      key: 'brand',
      label: seq != null ? `Brand Kit v${seq}` : 'Brand Kit',
      title: 'The brand kit this version was drawn with. Opens the kit.',
      kind: 'brand',
      artifactId: bk.artifact_id,
      versionId: typeof bk.version_id === 'string' ? bk.version_id : null,
      verified: true,
    });
  }
  for (const u of turn.unverified_citations) {
    if (!u.trim()) continue;
    chips.push({
      key: `unverified:${u}`,
      label: u,
      title: 'Otto cited this, but it wasn’t one of the references offered — it is not recorded in the provenance.',
      kind: 'unverified',
      artifactId: null,
      versionId: null,
      verified: false,
    });
  }
  return chips;
}

// ── Quick actions → requests ────────────────────────────────────────────────

export type QuickActionId = 'engaging' | 'a11y_check' | 'brand_check' | 'mobile' | 'copy' | 'variants';
export type Intent = QuickActionId | 'prompt' | 'fix_a11y' | 'fix_findings' | 'generate';

export interface QuickAction {
  id: QuickActionId;
  label: string;
  /** An `IconName` (checked where it is rendered). */
  icon: string;
  hint: string;
}

export const QUICK_ACTIONS: readonly QuickAction[] = [
  { id: 'engaging', label: 'More engaging', icon: 'sparkle', hint: 'Stronger hierarchy, social proof and motion — within the brand' },
  { id: 'a11y_check', label: 'Check accessibility', icon: 'eye', hint: 'Contrast, text alternatives, tap targets, heading order. Otto reports first; you choose what to fix' },
  { id: 'brand_check', label: 'On-brand check', icon: 'palette', hint: 'Colours, type and spacing against the brand kit. Reports first' },
  { id: 'mobile', label: 'Fix mobile', icon: 'layout', hint: 'Make it work at phone width' },
  { id: 'copy', label: 'Real copy from story', icon: 'note', hint: 'Replace placeholder copy with text from the linked story' },
  { id: 'variants', label: '3 variants', icon: 'columns', hint: 'Three directions side by side. Nothing changes until you apply one' },
];

/** A selected node/section the turn focuses on (≤ 4 KB JSON on the wire). */
export interface AssistSelection {
  node_id: string;
  label?: string;
}

export interface AssistContext {
  /** The person's own words (composer text), if any. */
  text?: string;
  selection?: AssistSelection | null;
  references?: string[];
  provider?: string;
  model?: string;
}

export type AssistRequest =
  | { kind: 'assist'; intent: Intent; prompt: string; body: DesignAssistReq }
  | { kind: 'variants'; intent: Intent; prompt: string; body: DesignVariantsReq };

const QUICK_PROMPTS: Record<Exclude<QuickActionId, 'variants'>, { mode: DesignAssistMode; prompt: string }> = {
  engaging: {
    mode: 'refine',
    prompt:
      'Make this more engaging: a stronger headline hierarchy, social proof and a subtle motion or micro-interaction — ' +
      'within the brand tokens, with reduced-motion defaults. Keep everything else.',
  },
  a11y_check: {
    mode: 'critique',
    prompt:
      'Check accessibility: text contrast (WCAG AA 4.5:1, 3:1 for large text), text alternatives for images and embeds, ' +
      'tap targets (≥ 44 px), heading order and focus visibility. Report each problem as a finding with its rule, ' +
      'the element and the fix. Do not edit.',
  },
  brand_check: {
    mode: 'critique',
    prompt:
      'Check this against the brand kit: colours, typography, spacing and radius that are not brand tokens, and copy ' +
      'tone. Report each problem as a finding with the token it should use. Do not edit.',
  },
  mobile: {
    mode: 'refine',
    prompt:
      'Fix the mobile layout: it must work at 390 px wide with no horizontal scroll, readable type, ≥ 44 px tap targets ' +
      'and the primary action visible without scrolling. Keep the desktop layout.',
  },
  copy: {
    mode: 'refine',
    prompt:
      'Replace placeholder copy with real copy from the linked story and its acceptance criteria. Keep the layout; ' +
      'keep headlines short and calls to action as verbs.',
  },
};

function focusSuffix(sel: AssistSelection | null | undefined): string {
  return sel ? ` Focus on ${sel.label ? `“${sel.label}”` : `node ${sel.node_id}`}; leave the rest as it is.` : '';
}

function common(ctx: AssistContext): Pick<DesignAssistReq, 'selection' | 'references' | 'provider' | 'model'> {
  const out: Pick<DesignAssistReq, 'selection' | 'references' | 'provider' | 'model'> = {};
  if (ctx.selection?.node_id) {
    out.selection = ctx.selection.label
      ? { node_id: ctx.selection.node_id, label: ctx.selection.label }
      : { node_id: ctx.selection.node_id };
  }
  const refs = (ctx.references ?? []).filter((r) => r.trim()).slice(0, 8);
  if (refs.length) out.references = refs;
  if (ctx.provider?.trim()) out.provider = ctx.provider.trim();
  if (ctx.model?.trim()) out.model = ctx.model.trim();
  return out;
}

/** A quick-action chip → the request it sends (composer text, if any, is appended). */
export function quickActionRequest(id: QuickActionId, ctx: AssistContext = {}): AssistRequest {
  const extra = ctx.text?.trim() ? ` ${ctx.text.trim()}` : '';
  if (id === 'variants') {
    const what = ctx.selection ? (ctx.selection.label ? `“${ctx.selection.label}”` : 'the selected section') : 'this design';
    const prompt = `Three distinct directions for ${what}.${extra}`;
    const { selection, references, provider, model } = common(ctx);
    const body: DesignVariantsReq = { prompt, n: 3 };
    if (selection) body.selection = selection;
    if (references) body.references = references;
    if (provider) body.provider = provider;
    if (model) body.model = model;
    return { kind: 'variants', intent: 'variants', prompt, body };
  }
  const q = QUICK_PROMPTS[id];
  const prompt = `${q.prompt}${focusSuffix(ctx.selection)}${extra}`;
  return { kind: 'assist', intent: id, prompt, body: { prompt, mode: q.mode, ...common(ctx) } };
}

/** Free text from the composer → one refine turn (or `n` variants). */
export function promptRequest(text: string, ctx: AssistContext = {}, n = 1, mode: DesignAssistMode = 'refine'): AssistRequest {
  const prompt = text.trim();
  if (n > 1) {
    const { selection, references, provider, model } = common(ctx);
    const body: DesignVariantsReq = { prompt, n: Math.min(4, Math.max(1, n)) };
    if (selection) body.selection = selection;
    if (references) body.references = references;
    if (provider) body.provider = provider;
    if (model) body.model = model;
    return { kind: 'variants', intent: mode === 'generate' ? 'generate' : 'prompt', prompt, body };
  }
  return { kind: 'assist', intent: mode === 'generate' ? 'generate' : 'prompt', prompt, body: { prompt, mode, ...common(ctx) } };
}

export interface Finding {
  index: number;
  severity: string;
  rule: string | null;
  message: string;
  nodeId: string | null;
  fix: string | null;
  fixed: boolean;
}

/** The agent's findings (`{severity?, rule?, message?, node_id?, fix?, fixed?}`),
 *  normalised; an entry with no text at all is dropped (it can't be acted on). */
export function findingsOf(turn: DesignAssistTurn): Finding[] {
  const s = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v.trim() : null);
  return turn.findings
    .map((f, index) => ({
      index,
      severity: s(f.severity) ?? 'info',
      rule: s(f.rule),
      message: s(f.message) ?? s(f.fix) ?? s(f.rule) ?? '',
      nodeId: s(f.node_id),
      fix: s(f.fix),
      fixed: f.fixed === true,
    }))
    .filter((f) => f.message);
}

/**
 * "Fix these" under a review → the turn that applies them: an accessibility
 * check becomes an `a11y` turn (fixes in place and lists what it fixed); any
 * other review becomes a `refine` turn quoting the findings.
 */
export function fixRequest(findings: Finding[], a11y: boolean, ctx: AssistContext = {}): AssistRequest {
  const list = findings.map((f) => `- ${f.rule ? `[${f.rule}] ` : ''}${f.message}${f.fix ? ` → ${f.fix}` : ''}`).join('\n');
  const prompt = `${a11y ? 'Fix these accessibility problems' : 'Fix these review findings'}; keep everything else:\n${list}`;
  return {
    kind: 'assist',
    intent: a11y ? 'fix_a11y' : 'fix_findings',
    prompt,
    body: { prompt, mode: a11y ? 'a11y' : 'refine', ...common(ctx) },
  };
}

// ── Variants ────────────────────────────────────────────────────────────────

/** `variant/<run>/<k>` → its run id and k (1-based). */
export function parseBranch(branch: string): { run: string; k: number } | null {
  const m = /^variant\/([^/]+)\/(\d+)$/.exec(branch);
  return m ? { run: m[1], k: Number(m[2]) } : null;
}

const DIRECTIONS: Record<string, { name: string; blurb: string }> = {
  defaults: { name: 'Team defaults', blurb: 'Follows your team rules' },
  explore: { name: 'Explore', blurb: 'Ignores soft preferences on purpose' },
  calm: { name: 'Calm', blurb: 'Quieter, with more space' },
  story: { name: 'Story', blurb: 'Leads with the story' },
};

export function directionName(d: string | null | undefined): string {
  if (!d) return 'Variant';
  return DIRECTIONS[d]?.name ?? d.charAt(0).toUpperCase() + d.slice(1);
}

export function directionBlurb(d: string | null | undefined): string {
  return (d && DIRECTIONS[d]?.blurb) || 'A custom direction';
}

export interface VariantCard {
  k: number;
  branch: string;
  direction: string | null;
  /** The committed variant version (null while running / when it failed). */
  version: DesignVersion | null;
  turn: DesignAssistTurn | null;
  state: 'running' | 'ready' | 'failed' | 'accepted' | 'passed';
  summary: string | null;
}

/** One card per variant of a run, k ascending (turns + committed versions). */
export function variantCards(run: DesignVariantRun): VariantCard[] {
  const byK = new Map<number, VariantCard>();
  const card = (k: number, branch: string): VariantCard => {
    let c = byK.get(k);
    if (!c) {
      c = { k, branch, direction: null, version: null, turn: null, state: 'running', summary: null };
      byK.set(k, c);
    }
    return c;
  };
  for (const t of run.turns) {
    const p = parseBranch(t.branch);
    if (!p || p.run !== run.run_id) continue;
    const c = card(p.k, t.branch);
    c.turn = t;
    c.direction = t.direction ?? c.direction;
    c.summary = t.message ?? c.summary;
  }
  for (const v of run.versions) {
    const p = parseBranch(v.branch);
    if (!p) continue;
    const c = card(p.k, v.branch);
    c.version = v;
    if (!c.summary && v.message) c.summary = v.message;
    const dir = obj(obj(v.provenance)?.assist)?.direction;
    if (!c.direction && typeof dir === 'string') c.direction = dir;
  }
  for (const c of byK.values()) {
    if (c.version) {
      if (run.accepted_version_id) c.state = c.version.id === run.accepted_version_id ? 'accepted' : 'passed';
      else c.state = 'ready';
    } else if (c.turn && isTerminal(c.turn.status)) c.state = 'failed';
    else c.state = run.status === 'running' || (c.turn && !isTerminal(c.turn.status)) ? 'running' : 'failed';
  }
  return [...byK.values()].sort((a, b) => a.k - b.k);
}

export const REJECT_REASONS: readonly { id: string; label: string }[] = [
  { id: 'too_busy', label: 'Too busy' },
  { id: 'off_brand', label: 'Off-brand' },
  { id: 'not_accessible', label: 'Not accessible' },
  { id: 'boring', label: 'Boring' },
  { id: 'other', label: 'Other' },
];

/**
 * The `variant_rejected` signal a 👎 posts. `reason` feeds the learning
 * extractor (`reject_reason:<slug>`, `other` ignored) and `direction` its
 * per-direction counts. Accepting a variant is recorded by the daemon — the
 * UI never posts `variant_accepted` itself.
 */
export function rejectSignal(a: {
  artifactId: string;
  card: VariantCard;
  runId: string;
  reason: string;
}): DesignSignalReq {
  const payload: Record<string, unknown> = {
    source: 'variant_tray',
    reason: a.reason,
    run_id: a.runId,
    k: a.card.k,
  };
  if (a.card.direction) payload.direction = a.card.direction;
  if (a.card.version) {
    payload.rejected_version_id = a.card.version.id;
    payload.rejected_seq = a.card.version.seq;
  }
  const out: DesignSignalReq = { artifact_id: a.artifactId, kind: 'variant_rejected', payload };
  if (a.card.version) out.version_id = a.card.version.id;
  return out;
}

// ── The thread ──────────────────────────────────────────────────────────────

export type ThreadItem =
  | { kind: 'turn'; at: string; turn: DesignAssistTurn }
  | { kind: 'run'; at: string; run: DesignVariantRun };

/**
 * The chat thread, oldest first: every main turn (incl. a conflict's side
 * draft) and every variants run as one message. Variant turns fold into their
 * run. A conflict's side version is listed by `GET …/variants` as a one-draft
 * "run" named after the turn — while the turn is known, the turn message owns
 * it (Apply / Compare there), so that run is not repeated.
 */
export function buildThread(turns: DesignAssistTurn[], runs: DesignVariantRun[]): ThreadItem[] {
  const runIds = new Set(runs.map((r) => r.run_id));
  const mainIds = new Set<string>();
  const items: ThreadItem[] = [];
  for (const t of turns) {
    const p = parseBranch(t.branch);
    if (t.mode === 'variant' || (p && runIds.has(p.run) && p.run !== t.turn_id)) continue;
    mainIds.add(t.turn_id);
    items.push({ kind: 'turn', at: t.started_at, turn: t });
  }
  for (const r of runs) {
    if (mainIds.has(r.run_id)) continue;
    const starts = [...r.turns.map((t) => t.started_at), ...r.versions.map((v) => v.created_at)].sort();
    items.push({ kind: 'run', at: starts[0] ?? '', run: r });
  }
  return items.sort((a, b) => a.at.localeCompare(b.at));
}

// ── Learning ────────────────────────────────────────────────────────────────

export const RULE_MIN_SIGNALS = 3;
export const RULE_MIN_ARTIFACTS = 2;

/** "2 of 3 signals · 1 of 2 designs" — how far a candidate is from being proposed. */
export function candidateProgress(c: DesignRuleCandidate): { text: string; pct: number } {
  const s = Math.min(c.signal_count, RULE_MIN_SIGNALS);
  const a = Math.min(c.artifact_count, RULE_MIN_ARTIFACTS);
  const pct = Math.round(((s / RULE_MIN_SIGNALS + a / RULE_MIN_ARTIFACTS) / 2) * 100);
  if (c.ready) return { text: `Ready · ${c.signal_count} signals across ${c.artifact_count} designs`, pct: 100 };
  return {
    text: `${c.signal_count} of ${RULE_MIN_SIGNALS} signals · ${c.artifact_count} of ${RULE_MIN_ARTIFACTS} designs`,
    pct,
  };
}

export function candidateKindLabel(kind: DesignRuleCandidate['kind'] | string): string {
  switch (kind) {
    case 'variant_preference':
      return 'Variant picks';
    case 'reject_reason':
      return 'Reject reasons';
    case 'edit_after_draft':
      return 'Edits after drafts';
    case 'a11y':
      return 'Accessibility fixes';
    default:
      return String(kind);
  }
}

export function editStatusLabel(s: DesignLearnedEdit['status'] | string): { label: string; tone: Tone } {
  switch (s) {
    case 'pending':
      return { label: 'Pending', tone: 'warn' };
    case 'applied':
      return { label: 'Approved', tone: 'ok' };
    case 'rejected':
      return { label: 'Rejected', tone: 'neutral' };
    case 'rolled_back':
      return { label: 'Rolled back', tone: 'neutral' };
    case 'conflict':
      return { label: 'Conflict — re-proposed', tone: 'bad' };
    default:
      return { label: String(s), tone: 'neutral' };
  }
}

/** One readable line for a pending edit: its rule(s). */
export function editHeadline(e: DesignLearnedEdit): string {
  if (e.rules.length === 0) return e.rationale || 'A change to the team rules';
  return e.rules.map((r) => r.rule).join(' · ');
}

/** Labels for the signal kinds the learning page filters and shows. */
export function signalKindLabel(kind: string): string {
  const m: Record<string, string> = {
    variant_chosen: 'Variant chosen',
    variant_accepted: 'Variant accepted',
    variant_rejected: 'Variant rejected',
    agent_draft: 'Agent draft',
    edit_after_draft: 'Edit after draft',
    review_comment: 'Comment',
    critique_finding: 'Critique',
    a11y_fix: 'A11y fix',
    brand_correction: 'Brand correction',
    rule_feedback: 'Rule decision',
    status_change: 'Status change',
    shipped: 'Shipped',
    restored: 'Restored',
    reference_added: 'Reference added',
    forked: 'Started from',
  };
  return m[kind] ?? kind;
}

/**
 * A readable line for the signals the co-design flow records (the graph's own
 * `signalSummary` covers the rest — return null to fall back to it).
 */
export function assistSignalSummary(kind: string, payload: Record<string, unknown> | null | undefined): string | null {
  const p = payload ?? {};
  const str = (k: string): string | null => (typeof p[k] === 'string' && (p[k] as string).trim() ? (p[k] as string).trim() : null);
  switch (kind) {
    case 'variant_accepted': {
      const d = str('direction');
      return d ? `Applied the “${directionName(d)}” variant` : 'Applied a variant';
    }
    case 'variant_rejected': {
      if (p.source !== 'variant_tray') return null;
      const d = str('direction');
      const reason = REJECT_REASONS.find((r) => r.id === p.reason)?.label ?? null;
      return `Rejected ${d ? `the “${directionName(d)}” variant` : 'a variant'}${reason ? ` · ${reason}` : ''}`;
    }
    case 'agent_draft': {
      const mode = str('mode');
      const d = str('direction');
      const cited = Array.isArray(p.cited) ? p.cited.length : 0;
      const what = mode === 'variant' ? `a “${directionName(d)}” variant` : mode === 'a11y' ? 'an accessibility fix' : 'a new version';
      return `Otto drafted ${what}${cited ? ` · cited ${cited} reference${cited === 1 ? '' : 's'}` : ''}`;
    }
    case 'a11y_fix': {
      const rule = str('rule');
      return rule ? `Accepted accessibility fix: ${rule}` : null;
    }
    case 'critique_finding': {
      if (p.disposition !== 'dismissed') return null;
      const rule = str('rule');
      return `Dismissed a finding${rule ? `: ${rule}` : ''}`;
    }
    case 'restored': {
      const from = typeof p.from_seq === 'number' ? p.from_seq : null;
      const to = typeof p.new_seq === 'number' ? p.new_seq : null;
      return from != null ? `Restored v${from}${to != null ? ` as v${to}` : ''}` : 'Restored an older version';
    }
    case 'reference_added':
      return 'Added a design as a reference';
    case 'forked':
      return p.source === 'copy' ? 'Made an editable copy' : 'Started a new design from a reference';
    default:
      return null;
  }
}

/** The active rules as a Markdown file (Settings → Export rules). */
export function exportRulesMarkdown(r: DesignLearnedResp, workspaceName: string): string {
  const lines = [
    `# Design team rules — ${workspaceName}`,
    '',
    `Skill: \`${r.skill}\` (${r.skill_path})`,
    `Learning mode: ${r.mode === 'off' ? 'off' : 'suggest only'}`,
    '',
  ];
  if (!r.active.length) lines.push('_No approved rules yet._');
  for (const a of r.active) lines.push(`- [rule:${a.key}] ${a.rule}`);
  lines.push('');
  return lines.join('\n');
}

// ── Lobby hand-off ──────────────────────────────────────────────────────────

const STOP = new Set(
  (
    'a an and are as at be but by for from has have in into is it its of on or our that the their this to with ' +
    'we you your make made create design page screen new some more very like want need using use one two three ' +
    'about over under e.g eg etc'
  ).split(' '),
);

/**
 * The prompt's most salient terms for the "Use references" preview: words of
 * 3+ letters that aren't stop words, longest first (ties keep prompt order),
 * de-duplicated. Library search ANDs its terms, so the lobby searches each
 * term on its own and merges the hits.
 */
export function salientTerms(prompt: string, n = 3): string[] {
  const words = prompt
    .toLowerCase()
    .split(/[^\p{L}\p{N}+-]+/u)
    .map((w) => w.replace(/^[-+]+|[-+]+$/g, ''))
    .filter((w) => w.length >= 3 && !STOP.has(w));
  const seen = new Set<string>();
  const uniq = words.filter((w) => (seen.has(w) ? false : (seen.add(w), true)));
  return uniq
    .map((w, i) => ({ w, i }))
    .sort((a, b) => b.w.length - a.w.length || a.i - b.i)
    .slice(0, n)
    .map((x) => x.w);
}
