// Agent UI control — Personal Agents: the PURE half of the `pa_*` handlers
// (argument checks and the text every confirm shows). No stores, no Svelte,
// so unit/uiCommandsPersonalAgents.test.ts imports it directly; the handlers
// in ./personalAgents.ts do the navigation, the confirms and the writes.

import type { OutwardConfirm } from '../confirmOutward';

/** A bad argument; the handler reports it as `invalid_args`. */
export class PaArgError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'PaArgError';
  }
}

export type PaPermission = 'read_only' | 'directed';

/** A schedule as a command passes it (the daemon's `schedule_json` + extras). */
export interface PaScheduleArg {
  schedule: Record<string, unknown>;
  timezone?: string;
  directive?: string;
  permission?: PaPermission;
  enabled?: boolean;
}

/** The fields `pa_create` fills into the New-agent sheet. */
export interface PaCreateFill {
  template?: string;
  name?: string;
  avatar?: string;
  soul_md?: string;
  provider?: string;
  model?: string;
  schedule?: PaScheduleArg;
}

/** The editable agent fields of `pa_edit`. */
export interface PaAgentPatch {
  name?: string;
  avatar?: string;
  soul_md?: string;
  provider?: string;
  model?: string;
  enabled?: boolean;
}

const CADENCES = ['interval', 'daily', 'weekly', 'cron', 'once'];

/** Validate a cadence object; throws `invalid_args` naming the problem. */
export function checkCadence(spec: unknown): Record<string, unknown> {
  if (!spec || typeof spec !== 'object' || Array.isArray(spec)) {
    throw new PaArgError('schedule must be an object like {cadence:"cron", expr:"0 7 * * 1-5"}.');
  }
  const s = spec as Record<string, unknown>;
  const cad = s.cadence;
  if (typeof cad !== 'string' || !CADENCES.includes(cad)) {
    throw new PaArgError(`schedule.cadence must be one of ${CADENCES.join(' | ')}.`);
  }
  if (cad === 'cron' && (typeof s.expr !== 'string' || s.expr.trim().split(/\s+/).length !== 5)) {
    throw new PaArgError('A cron schedule needs expr: a 5-field cron expression.');
  }
  if (cad === 'once' && typeof s.run_at !== 'string') {
    throw new PaArgError('A once schedule needs run_at (YYYY-MM-DDTHH:MM in the timezone).');
  }
  return s;
}

export function checkPermission(p: unknown): PaPermission | undefined {
  if (p === undefined || p === null) return undefined;
  if (p === 'read_only' || p === 'directed') return p;
  throw new PaArgError('permission must be read_only or directed.');
}

/** "cron 0 7 * * 0-5 (Asia/Jerusalem)" — the schedule in one line. */
export function cadenceText(spec: Record<string, unknown>, timezone?: string): string {
  const tz = timezone?.trim() ? ` (${timezone.trim()})` : '';
  switch (spec.cadence) {
    case 'interval':
      return `every ${Number(spec.every_min ?? 60)} min`;
    case 'daily':
      return `daily at ${String(spec.at ?? '09:00')}${tz}`;
    case 'weekly':
      return `weekly (day ${Number(spec.weekday ?? 0)}) at ${String(spec.at ?? '09:00')}${tz}`;
    case 'cron':
      return `cron ${String(spec.expr ?? '')}${tz}`;
    case 'once':
      return `once at ${String(spec.run_at ?? '?')}${tz}`;
    default:
      return 'an unknown cadence';
  }
}

/** A persona in a sentence: its first heading / line and its size. */
export function personaSummary(soul: string): string {
  const text = soul.trim();
  if (!text) return 'no persona';
  const first = text
    .split('\n')
    .map((l) => l.replace(/<!--.*?-->/g, '').replace(/^#+\s*/, '').trim())
    .find((l) => l.length > 0) ?? '';
  const head = first.length > 80 ? `${first.slice(0, 79)}…` : first;
  return `“${head}” (${text.length} characters)`;
}

/** One schedule line for a confirm. */
export function scheduleText(s: PaScheduleArg, fallbackDirective = ''): string {
  const directive = (s.directive ?? '').trim() || fallbackDirective;
  const perm = s.permission === 'read_only' ? 'read-only' : 'directed (your normal approvals)';
  const parts = [cadenceText(s.schedule, s.timezone), `permission ${perm}`];
  if (s.enabled === false) parts.push('paused');
  if (directive) parts.push(`directive “${directive.length > 120 ? `${directive.slice(0, 119)}…` : directive}”`);
  return parts.join('; ');
}

/** The `pa_create` confirm text (the template, as filled, after overrides). */
export function createConfirmText(
  f: PaCreateFill,
  t: { title: string; name: string; soul_md: string; directive: string; schedule: Record<string, unknown> } | undefined,
  defaultTz: string,
): string {
  const name = (f.name ?? t?.name ?? '').trim() || '(unnamed)';
  const lines = [`Create the personal agent “${name}”${t ? ` from the template “${t.title}”` : ''}.`];
  const soul = f.soul_md ?? t?.soul_md ?? '';
  lines.push(`Persona: ${personaSummary(soul)}${f.soul_md !== undefined && t ? ' — edited from the template' : ''}.`);
  lines.push(`Provider: ${f.provider?.trim() || 'the default'}${f.model?.trim() ? ` · ${f.model.trim()}` : ''}.`);
  const sched = f.schedule ?? (t ? { schedule: t.schedule, timezone: defaultTz } : undefined);
  lines.push(sched ? `Schedule: ${scheduleText(sched, t?.directive ?? '')}.` : 'No schedule (it runs only when asked).');
  return lines.join('\n');
}

/** The fields of `patch` that differ from `agent` (persona summarized). */
export function agentChanges(
  agent: Required<Omit<PaAgentPatch, 'enabled'>> & { enabled: boolean },
  patch: PaAgentPatch,
): { body: PaAgentPatch; lines: string[] } {
  const body: PaAgentPatch = {};
  const lines: string[] = [];
  for (const k of ['name', 'avatar', 'provider', 'model'] as const) {
    const v = patch[k];
    if (v !== undefined && v !== agent[k]) {
      body[k] = v;
      lines.push(`${k}: “${agent[k] || '(none)'}” → “${v || '(none)'}”`);
    }
  }
  if (patch.soul_md !== undefined && patch.soul_md !== agent.soul_md) {
    body.soul_md = patch.soul_md;
    lines.push(`persona: ${personaSummary(agent.soul_md)} → ${personaSummary(patch.soul_md)}`);
  }
  if (patch.enabled !== undefined && patch.enabled !== agent.enabled) {
    body.enabled = patch.enabled;
    lines.push(patch.enabled ? 'enable it (its schedules fire again)' : 'pause it (its schedules stop firing)');
  }
  return { body, lines };
}

/** The ALWAYS-shown confirm for lending the user's Slack identity. */
export function slackAsMeConfirm(
  agentName: string,
  agentLabelText: string,
  teams: { team: string; user: string }[],
): OutwardConfirm {
  if (teams.length === 0) {
    return {
      verb: 'Turn off',
      title: `${agentLabelText} wants to stop “${agentName}” acting as you in Slack`,
      where: `Personal agent “${agentName}” → no Slack workspace`,
      what: 'The agent can no longer read, search or post in Slack as you.',
      who: 'Only you — nothing is sent.',
    };
  }
  const list = teams.map((t) => `${t.team} (as ${t.user})`).join(', ');
  return {
    verb: 'Let it act as me',
    title: `${agentLabelText} wants “${agentName}” to act as you in Slack`,
    where: `Slack: ${list}`,
    what: `“${agentName}” may read your channels and DMs, search, mark messages unread and post AS YOU there (every post still asks you first).`,
    who: 'Everyone in those Slack workspaces sees what it posts under your name.',
    danger: true,
  };
}
