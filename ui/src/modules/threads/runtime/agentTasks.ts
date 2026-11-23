// Every subagent, linked child thread, and background task of a runtime thread,
// for the strip above the composer (RuntimeQueueBar) and Thread details'
// "Agents & tasks" (ThreadAgentsTasks.svelte). Built from the projection:
// running background tasks merged with their TurnItem (subagent cards, command
// executions, scheduled timers), settled subagent/notice items, and linked
// child threads (`parent_id === sessionId`). Pure (unit/threadsAgentTasks.test.ts).
import type {
  BackgroundTask,
  RuntimeSessionStatus,
  RuntimeTurnOutcome,
  TurnItem,
  TurnItemStatus,
} from '../../../lib/api/types';
import { plural } from '../../../lib/plural.ts';
import { subagentChildren, workLabel } from './timeline.ts';
import { perfCount } from '../../../lib/perfCount.ts';

/** `needs-you`: a linked child thread waits on a person (an approval, a
 *  question, a secret) — still active, listed with the running ones. */
export type AgentTaskStatus = 'running' | 'needs-you' | 'done' | 'failed' | 'stopped';

/** Still going (running or waiting on a person): the strip's and the running list's rows. */
export const isActiveTask = (t: Pick<AgentTask, 'status'>): boolean => t.status === 'running' || t.status === 'needs-you';

export interface LinkedChildThreadSummary {
  id: string;
  title: string;
  provider: string;
  model: string | null;
  status: RuntimeSessionStatus;
  createdAt: string;
  updatedAt: string;
  /** Latest activity summary from the child thread's projection when loaded. */
  stepSummary?: string | null;
  /** How its latest turn ended (`RuntimeSession.last_turn_status`, or the
   *  loaded projection's latest turn); absent = unknown. */
  lastTurnStatus?: RuntimeTurnOutcome | null;
}

export interface AgentTask {
  id: string;
  /** `agent`, `command`, `timer`, `thread`, or the provider's own task type. */
  kind: string;
  label: string;
  status: AgentTaskStatus;
  background: boolean;
  startedAt: string | null;
  endedAt: string | null;
  /** Linked TurnItem id (subagent card, command_execution, or dynamic_tool). */
  itemId: string | null;
  /** Linked child RuntimeSession id (`otto.thread_open` / `Ask another agent`). */
  childSessionId: string | null;
  /** The provider's task id (a Claude background command's output file). */
  taskId: string | null;
  /** Live 1-line summary of what the agent or task is currently doing / produced. */
  stepSummary: string | null;
  /** Subagent type or provider/model badge text. */
  sublabel: string | null;
  /** Prompt, command, or timer instruction when available. */
  prompt: string | null;
  /** Live stdout/stderr tail or final output when available. */
  output: string | null;
  /** Error text when failed. */
  error: string | null;
  /** Number of tool steps executed by a subagent. */
  toolUses: number;
  /** Whether full TurnItem details were omitted and can be loaded on expand. */
  detailOmitted: boolean;
  /** The daemon's background task id when the row is one of the thread's
   *  running background tasks: what `POST …/tasks/{task_id}/stop` stops.
   *  Absent = no per-task stop (a turn's own subagent, a swarm member). */
  stopId?: string | null;
  /** The daemon is stopping it / its last stop failed (newer daemons). */
  stopping?: boolean;
  stopError?: string | null;
}

export interface AgentTasksOptions {
  /** Live `command_output.delta` buffers keyed by TurnItem id. */
  output?: Readonly<Record<string, string>>;
  /** Linked child runtime threads (`parent_id === sessionId`). */
  childThreads?: readonly LinkedChildThreadSummary[];
}

export interface AgentTasks {
  /** Oldest first. */
  running: AgentTask[];
  /** Newest first. */
  finished: AgentTask[];
}

function statusOf(s: TurnItemStatus): AgentTaskStatus {
  if (s === 'completed') return 'done';
  if (s === 'failed') return 'failed';
  if (s === 'cancelled' || s === 'interrupted') return 'stopped';
  return 'running';
}

function noticeStatusOf(verb: string, fallback: TurnItemStatus): AgentTaskStatus {
  if (verb === 'finished') return 'done';
  if (verb === 'failed') return 'failed';
  if (verb === 'stopped') return 'stopped';
  return statusOf(fallback);
}

/** A linked child thread's row: a waiting request needs you, a turn runs,
 *  else how its last turn ended — a closed child that did not fail stopped. */
export function childThreadStatusOf(s: RuntimeSessionStatus, lastTurn?: RuntimeTurnOutcome | null): AgentTaskStatus {
  if (s === 'awaiting_permission') return 'needs-you';
  if (s === 'working' || s === 'starting') return 'running';
  if (lastTurn === 'failed') return 'failed';
  if (lastTurn === 'interrupted' || s === 'closed') return 'stopped';
  return 'done';
}

const firstLine = (raw: string | null | undefined): string | null => {
  if (!raw) return null;
  const line = raw
    .trim()
    .split('\n')
    .find((l) => l.trim() !== '');
  return line ? line.trim() : null;
};

const lastLine = (raw: string | null | undefined): string | null => {
  if (!raw) return null;
  const lines = raw.trim().split('\n');
  for (let i = lines.length - 1; i >= 0; i--) {
    const t = lines[i].trim();
    if (t) return t.length > 140 ? `${t.slice(0, 139)}…` : t;
  }
  return null;
};

/** Matches `Background (command|agent|timer) (finished|failed|stopped): <label>`. */
/** Claude's launch result: "Command running in background with ID: <task>. …". */
const LAUNCHED = /background with ID: ([A-Za-z0-9_-]+)/;
const BG_NOTICE = /^Background (command|agent|timer) (finished|failed|stopped): (.+)$/s;
/** Matches one-shot `Timer (<seconds>s): ...` labels emitted by ACP `schedule`. */
const ONESHOT_TIMER = /^Timer \((\d+)s\):/;
/** Top-level `dynamic_tool` calls that spawn a subagent when no `subagent` card exists. */
const SUBAGENT_DYNAMIC_TOOLS = new Set([
  'invoke_subagent',
  'delegate_to_agent',
  'otto_personal_agent_delegate',
  'mcp__otto__otto_personal_agent_delegate',
  'mcp_otto_otto_personal_agent_delegate',
]);

function subagentFields(
  item: Extract<TurnItem, { type: 'subagent' }>,
  children: readonly TurnItem[],
  status: AgentTaskStatus,
): Pick<
  AgentTask,
  'sublabel' | 'prompt' | 'output' | 'error' | 'toolUses' | 'stepSummary' | 'detailOmitted'
> {
  const name = item.description ?? item.title ?? item.subagent_type ?? 'Subagent';
  const sublabel = item.subagent_type && item.subagent_type !== name ? item.subagent_type : null;
  const childWorkCount = children.filter(
    (c) => c.type !== 'assistant_message' && c.type !== 'reasoning',
  ).length;
  const toolUses = Math.max(item.tool_uses ?? 0, childWorkCount);
  const steps = toolUses > 0 ? plural(toolUses, 'step') : null;
  const lastChild = children.findLast((c) => c.type !== 'assistant_message');
  const lastStep = item.last_tool ?? (lastChild ? workLabel(lastChild) : null);
  const stepSummary =
    status === 'running'
      ? [steps, lastStep ? `last: ${lastStep}` : null].filter(Boolean).join(' · ') ||
        (children.length > 0 ? workLabel(children[children.length - 1]) : null)
      : firstLine(item.error ?? item.output) ?? steps;
  return {
    sublabel,
    prompt: item.prompt ?? null,
    output: item.output ?? null,
    error: item.error ?? null,
    toolUses,
    stepSummary,
    detailOmitted: Boolean(item.detail_omitted),
  };
}

function toolFields(
  item: TurnItem | undefined,
  liveOut: string | undefined,
): Pick<
  AgentTask,
  'sublabel' | 'prompt' | 'output' | 'error' | 'toolUses' | 'stepSummary' | 'detailOmitted'
> {
  if (!item) {
    return {
      sublabel: null,
      prompt: null,
      output: liveOut ?? null,
      error: null,
      toolUses: 0,
      stepSummary: lastLine(liveOut),
      detailOmitted: false,
    };
  }
  if (item.type === 'command_execution') {
    const out = liveOut || item.output || null;
    return {
      sublabel: item.exit_code != null ? `exit ${item.exit_code}` : null,
      prompt: item.input || null,
      output: out,
      error: item.error ?? null,
      toolUses: 0,
      stepSummary: lastLine(item.error ?? out),
      detailOmitted: Boolean(item.detail_omitted),
    };
  }
  if (item.type === 'dynamic_tool') {
    const inp = (item.input && typeof item.input === 'object' ? item.input : {}) as Record<
      string,
      unknown
    >;
    const prompt =
      [inp.Prompt, inp.prompt, inp.CommandLine, inp.command, inp.description].find(
        (v): v is string => typeof v === 'string' && v.trim() !== '',
      ) ?? null;
    const out = liveOut || item.output || null;
    return {
      sublabel: item.tool_name || null,
      prompt,
      output: out,
      error: item.error ?? null,
      toolUses: 0,
      stepSummary: lastLine(item.error ?? out),
      detailOmitted: Boolean(item.detail_omitted),
    };
  }
  const itemErr =
    'error' in item && typeof item.error === 'string'
      ? item.error
      : item.type === 'error'
        ? item.failure.message
        : null;
  return {
    sublabel: null,
    prompt: null,
    output: liveOut ?? null,
    error: itemErr,
    toolUses: 0,
    stepSummary: lastLine(itemErr ?? liveOut),
    detailOmitted: Boolean(item.detail_omitted),
  };
}

function stopFields(b: BackgroundTask): Pick<AgentTask, 'stopId' | 'stopping' | 'stopError'> {
  return { stopId: b.id, stopping: Boolean(b.stopping), stopError: b.stop_error ?? null };
}

export function agentTasks(
  items: readonly TurnItem[],
  background: readonly BackgroundTask[],
  opts: AgentTasksOptions = {},
): AgentTasks {
  perfCount('agentTasks');
  const outputMap = opts.output ?? {};
  const itemsById = new Map<string, TurnItem>();
  let latestActivityMs = 0;
  const settledNotices: {
    id: string;
    taskId: string | null;
    kind: string;
    label: string;
    status: AgentTaskStatus;
    endedAt: string | null;
  }[] = [];

  for (let idx = 0; idx < items.length; idx++) {
    const i = items[idx];
    itemsById.set(i.id, i);
    const ts = Date.parse(i.started_at ?? i.updated_at ?? '');
    if (Number.isFinite(ts) && ts > latestActivityMs) latestActivityMs = ts;
    if (i.type === 'system_notice') {
      if (i.task_settlement) {
        const tsSettle = i.task_settlement;
        const settleStatus: AgentTaskStatus =
          tsSettle.status === 'completed' || tsSettle.status === 'finished'
            ? 'done'
            : tsSettle.status === 'failed'
              ? 'failed'
              : tsSettle.status === 'stopped' || tsSettle.status === 'cancelled'
                ? 'stopped'
                : statusOf(i.status);
        settledNotices.push({
          id: i.id,
          taskId: tsSettle.task_id || null,
          kind: tsSettle.kind || 'command',
          label: tsSettle.label || i.message,
          status: settleStatus,
          endedAt: i.updated_at,
        });
        continue;
      }
      const m = BG_NOTICE.exec(i.message);
      if (m) {
        const noticeKind = m[1] === 'agent' ? 'agent' : m[1] === 'timer' ? 'timer' : 'command';
        settledNotices.push({
          id: i.id,
          taskId: null,
          kind: noticeKind,
          label: m[3],
          status: noticeStatusOf(m[2], i.status),
          endedAt: i.updated_at,
        });
      }
    }
  }

  const childrenByParent = subagentChildren(items);
  /** A background command's launch call, by the task id its result names. */
  const launches = new Map<string, TurnItem>();
  for (let idx = 0; idx < items.length; idx++) {
    const i = items[idx];
    const m = i.type === 'command_execution' && i.output ? LAUNCHED.exec(i.output) : null;
    if (m) launches.set(m[1], i);
  }
  const rows: AgentTask[] = [];
  const listedSubagentIds = new Set<string>();
  const listedDynamicToolIds = new Set<string>();
  const consumedNoticeIds = new Set<string>();

  for (let idx = 0; idx < background.length; idx++) {
    const b = background[idx];
    let linked = (b.item_id ? itemsById.get(b.item_id) : undefined) ?? launches.get(b.id);
    if (!linked && b.kind === 'agent') {
      linked = items.find(
        (i) =>
          i.type === 'subagent' &&
          (i.id === b.id ||
            (i.description ?? i.subagent_type ?? i.title) === b.label) &&
          !listedSubagentIds.has(i.id),
      );
    }

    const timerMatch = ONESHOT_TIMER.exec(b.label);
    const kind = b.due_at || timerMatch ? 'timer' : b.kind;
    let status: AgentTaskStatus = 'running';
    let endedAt: string | null = null;

    // Reconcile against explicit completion notices or expired one-shot timers
    // followed by newer conversation activity (protects resumed sessions).
    const matchingNotice = settledNotices.find(
      (n) =>
        !consumedNoticeIds.has(n.id) &&
        (n.taskId === b.id ||
          n.id === `bg:${b.id}` ||
          (!n.taskId &&
            n.label === b.label &&
            (!b.started_at || !n.endedAt || Date.parse(n.endedAt) >= Date.parse(b.started_at)))),
    );
    if (matchingNotice) {
      consumedNoticeIds.add(matchingNotice.id);
      status = matchingNotice.status;
      endedAt = matchingNotice.endedAt;
    } else if (b.due_at) {
      const rawDueMs = Date.parse(b.due_at);
      if (Number.isFinite(rawDueMs) && latestActivityMs > rawDueMs + 5_000) {
        status = 'done';
        endedAt = new Date(rawDueMs).toISOString();
      }
    } else if (timerMatch && b.started_at) {
      const durSec = Number(timerMatch[1]);
      const dueMs = Date.parse(b.started_at) + (durSec + 5) * 1000;
      if (Number.isFinite(dueMs) && latestActivityMs > dueMs) {
        status = 'done';
        endedAt = new Date(Date.parse(b.started_at) + durSec * 1000).toISOString();
      }
    }

    if (linked?.type === 'subagent') {
      listedSubagentIds.add(linked.id);
      const itemStatus = statusOf(linked.status);
      if (itemStatus !== 'running') {
        status = itemStatus;
        endedAt = linked.completed_at ?? linked.updated_at;
      }
      const children = childrenByParent.get(linked.id) ?? [];
      rows.push({
        id: b.id,
        kind: 'agent',
        label: linked.description ?? linked.subagent_type ?? linked.title ?? b.label,
        status,
        background: true,
        startedAt: linked.started_at ?? b.started_at,
        endedAt,
        itemId: linked.id,
        childSessionId: null,
        taskId: null,
        ...subagentFields(linked, children, status),
        ...stopFields(b),
      });
      continue;
    }

    if (linked?.type === 'dynamic_tool') listedDynamicToolIds.add(linked.id);
    const liveOut = b.item_id ? outputMap[b.item_id] : undefined;
    rows.push({
      id: b.id,
      kind,
      label: b.label,
      status,
      background: true,
      startedAt: b.started_at,
      endedAt,
      itemId: linked?.id ?? b.item_id ?? null,
      childSessionId: null,
      taskId: kind === 'command' ? b.id : null,
      ...toolFields(linked, liveOut),
      ...stopFields(b),
    });
  }

  for (let idx = 0; idx < items.length; idx++) {
    const i = items[idx];
    if (i.type === 'subagent' && !listedSubagentIds.has(i.id)) {
      const status = statusOf(i.status);
      const children = childrenByParent.get(i.id) ?? [];
      rows.push({
        id: i.id,
        kind: 'agent',
        label: i.description ?? i.subagent_type ?? i.title ?? 'Subagent',
        status,
        background: i.background,
        startedAt: i.started_at,
        endedAt: i.completed_at ?? (status === 'running' ? null : i.updated_at),
        itemId: i.id,
        childSessionId: null,
        taskId: null,
        ...subagentFields(i, children, status),
      });
      continue;
    }
    if (
      i.type === 'dynamic_tool' &&
      !i.parent_item_id &&
      !listedDynamicToolIds.has(i.id) &&
      SUBAGENT_DYNAMIC_TOOLS.has(i.tool_name)
    ) {
      const inp = (i.input && typeof i.input === 'object' ? i.input : {}) as Record<
        string,
        unknown
      >;
      const dynLabel =
        [inp.Role, inp.role, inp.description, inp.agent, inp.TypeName, inp.subagent_type, i.title]
          .find((v): v is string => typeof v === 'string' && v.trim() !== '')
          ?.trim() ?? 'Subagent';
      const alreadyCovered =
        rows.some((r) => r.itemId === i.id || (r.kind === 'agent' && r.label === dynLabel)) ||
        Boolean(opts.childThreads?.some((c) => c.title === dynLabel));
      if (!alreadyCovered) {
        const status = statusOf(i.status);
        const liveOut = outputMap[i.id];
        rows.push({
          id: i.id,
          kind: 'agent',
          label: dynLabel,
          status,
          background: false,
          startedAt: i.started_at,
          endedAt: status === 'running' ? null : i.updated_at,
          itemId: i.id,
          childSessionId: null,
          taskId: null,
          ...toolFields(i, liveOut),
        });
      }
    }
  }

  for (let idx = 0; idx < settledNotices.length; idx++) {
    const n = settledNotices[idx];
    if (consumedNoticeIds.has(n.id)) continue;
    // Skip duplicate agent notice when a subagent card with the same label already settled.
    if (
      n.kind === 'agent' &&
      rows.some((r) => r.kind === 'agent' && r.status !== 'running' && r.label === n.label)
    ) {
      continue;
    }
    // claude_tasks.rs ids the notice `bg:<task id>`; its launch call is the row's work.
    const taskId = n.kind === 'command' ? (n.taskId ?? (n.id.startsWith('bg:') ? n.id.slice(3) : null)) : null;
    const launch = taskId ? launches.get(taskId) : undefined;
    rows.push({
      id: n.id,
      kind: n.kind,
      label: n.label,
      status: n.status,
      background: true,
      startedAt: launch?.started_at ?? null,
      endedAt: n.endedAt,
      itemId: launch?.id ?? n.id,
      childSessionId: null,
      taskId,
      ...toolFields(launch, undefined),
    });
  }

  if (opts.childThreads?.length) {
    for (let idx = 0; idx < opts.childThreads.length; idx++) {
      const c = opts.childThreads[idx];
      if (rows.some((r) => r.childSessionId === c.id)) continue;
      const status = childThreadStatusOf(c.status, c.lastTurnStatus);
      const sublabel = [c.provider, c.model].filter(Boolean).join(' · ') || null;
      rows.push({
        id: `thread:${c.id}`,
        kind: 'agent',
        label: c.title || `${c.provider} thread`,
        status,
        background: true,
        startedAt: c.createdAt,
        endedAt: status === 'running' || status === 'needs-you' ? null : c.updatedAt,
        itemId: null,
        childSessionId: c.id,
        taskId: null,
        stepSummary:
          c.stepSummary ??
          (c.status === 'awaiting_permission' ? 'Waiting for approval' : null),
        sublabel,
        prompt: null,
        output: null,
        error: null,
        toolUses: 0,
        detailOmitted: false,
      });
    }
  }

  const at = (iso: string | null): number => (iso ? Date.parse(iso) : 0);
  return {
    running: rows
      .filter(isActiveTask)
      .sort((a, b) => at(a.startedAt) - at(b.startedAt)),
    finished: rows
      .filter((r) => !isActiveTask(r))
      .sort((a, b) => at(b.endedAt ?? b.startedAt) - at(a.endedAt ?? a.startedAt)),
  };
}

/** The strip's words for background work that is not a subagent (commands,
 *  timers — subagents are on the waiting line): the one task named, several
 *  counted (the list is a tap away). */
export function backgroundStripText(tasks: readonly AgentTask[]): string {
  if (tasks.length === 1) {
    const step = tasks[0].stepSummary ? ` (${tasks[0].stepSummary})` : '';
    return `Background task: ${tasks[0].label}${step}`;
  }
  return `${tasks.length} background tasks`;
}

/** "12s", "4m 5s", "1h 2m" from a start to an end (or `nowMs`); null without a start. */
export function taskElapsed(
  t: Pick<AgentTask, 'startedAt' | 'endedAt'>,
  nowMs: number,
): string | null {
  if (!t.startedAt) return null;
  const s = Math.max(
    0,
    Math.floor(((t.endedAt ? Date.parse(t.endedAt) : nowMs) - Date.parse(t.startedAt)) / 1000),
  );
  if (!Number.isFinite(s)) return null;
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
  return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
}

/** An Antigravity swarm member as the daemon reads it from agy's files. */
export interface SwarmMemberLite {
  name: string;
  killed: boolean;
  phase?: string;
}

/** `swarm:<name>`: a swarm member row with no card of its own. */
export const SWARM_ROW = 'swarm:';

/** The agy swarm member a row shows (its board and steps come from agy's own
 *  files): a swarm row's name, or the member a `swarm` subagent card is for. */
export function swarmMemberOf(t: AgentTask, items: readonly TurnItem[]): string | null {
  if (t.id.startsWith(SWARM_ROW)) return t.label;
  const card = t.itemId ? items.find((i) => i.id === t.itemId) : undefined;
  return card?.type === 'subagent' && card.tool_name === 'swarm' && card.description ? card.description : null;
}

/** Swarm members not already shown as a card (a thread whose `swarm add`
 *  calls predate the mapping, or whose agent restarted) as rows. */
export function withSwarm(tasks: AgentTasks, members: readonly SwarmMemberLite[], items: readonly TurnItem[]): AgentTasks {
  const carded = new Set(items.flatMap((i) => (i.type === 'subagent' && i.tool_name === 'swarm' && i.description ? [i.description] : [])));
  const extra = members.filter((m) => !carded.has(m.name));
  if (!extra.length) return tasks;
  const row = (m: SwarmMemberLite): AgentTask => ({
    id: `${SWARM_ROW}${m.name}`,
    kind: 'agent',
    label: m.name,
    status: m.killed ? 'stopped' : 'running',
    background: true,
    startedAt: null,
    endedAt: null,
    itemId: null,
    childSessionId: null,
    taskId: null,
    ...toolFields(undefined, undefined),
  });
  return {
    running: [...tasks.running, ...extra.filter((m) => !m.killed).map(row)],
    finished: [...tasks.finished, ...extra.filter((m) => m.killed).map(row)],
  };
}
