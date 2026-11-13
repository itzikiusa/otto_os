// Every subagent and background task of a runtime thread, for the strip
// above the composer (RuntimeQueueBar) and Thread details' "Agents & tasks"
// (ThreadAgentsTasks.svelte). Built from the projection: the session's
// running background list plus the turn items (subagent cards, a finished
// background command's notice), so a background subagent still shows while
// its card runs. Pure (unit/threadsAgentTasks.test.ts).
import type { BackgroundTask, TurnItem, TurnItemStatus } from '../../../lib/api/types';

export type AgentTaskStatus = 'running' | 'done' | 'failed' | 'stopped';

export interface AgentTask {
  id: string;
  /** `agent`, `command`, or the provider's own task type. */
  kind: string;
  label: string;
  status: AgentTaskStatus;
  background: boolean;
  startedAt: string | null;
  endedAt: string | null;
  /** The turn item to open: the subagent card, or the command's launch call. */
  itemId: string | null;
  /** The provider's task id (a Claude background command's output file). */
  taskId: string | null;
  /** The raw command, shown under its description. */
  command: string | null;
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

/** claude_tasks.rs `task_finished`: "Background command finished: npm test". */
const COMMAND_NOTICE = /^Background command (?:finished|failed|stopped): (.+)$/s;

/** Claude's launch result: "Command running in background with ID: <task>. …". */
const LAUNCHED = /background with ID: ([A-Za-z0-9_-]+)/;

function commandOf(i: TurnItem | undefined): string | null {
  return i?.type === 'command_execution' ? i.input || null : null;
}

export function agentTasks(items: readonly TurnItem[], background: readonly BackgroundTask[]): AgentTasks {
  const byId = new Map(items.map((i) => [i.id, i] as const));
  /** A background command's launch call, by the task id its result names. */
  const launches = new Map<string, TurnItem>();
  for (const i of items) {
    const m = i.type === 'command_execution' && i.output ? LAUNCHED.exec(i.output) : null;
    if (m) launches.set(m[1], i);
  }
  const rows: AgentTask[] = background.map((b) => {
    const launch = (b.item_id ? byId.get(b.item_id) : undefined) ?? launches.get(b.id);
    return {
      id: b.id,
      kind: b.kind,
      label: b.label,
      status: 'running',
      background: true,
      startedAt: b.started_at,
      endedAt: null,
      itemId: launch?.id ?? b.item_id ?? null,
      taskId: b.kind === 'agent' ? null : b.id,
      command: commandOf(launch),
    };
  });
  const listed = new Set(background.flatMap((b) => (b.item_id ? [b.item_id] : [])));
  for (const i of items) {
    if (i.type === 'subagent' && !listed.has(i.id)) {
      rows.push({
        id: i.id,
        kind: 'agent',
        label: i.description ?? i.subagent_type ?? i.title ?? 'Subagent',
        status: statusOf(i.status),
        background: i.background,
        startedAt: i.started_at,
        endedAt: i.completed_at ?? (statusOf(i.status) === 'running' ? null : i.updated_at),
        itemId: i.id,
        taskId: null,
        command: null,
      });
    } else if (i.type === 'system_notice') {
      const m = COMMAND_NOTICE.exec(i.message);
      if (!m) continue;
      // claude_tasks.rs ids the notice `bg:<task id>`.
      const taskId = i.id.startsWith('bg:') ? i.id.slice(3) : null;
      const launch = taskId ? launches.get(taskId) : undefined;
      rows.push({
        id: i.id,
        kind: 'command',
        label: m[1],
        status: statusOf(i.status),
        background: true,
        startedAt: launch?.started_at ?? null,
        endedAt: i.updated_at,
        itemId: launch?.id ?? null,
        taskId,
        command: commandOf(launch),
      });
    }
  }
  const at = (iso: string | null): number => (iso ? Date.parse(iso) : 0);
  return {
    running: rows.filter((r) => r.status === 'running').sort((a, b) => at(a.startedAt) - at(b.startedAt)),
    finished: rows.filter((r) => r.status !== 'running').sort((a, b) => at(b.endedAt ?? b.startedAt) - at(a.endedAt ?? a.startedAt)),
  };
}

/** The strip's words: the one task named, several counted (the list is a tap away). */
export function backgroundStripText(tasks: readonly AgentTask[]): string {
  return tasks.length === 1 ? `Running in the background: ${tasks[0].label}` : `${tasks.length} running in the background`;
}

/** "12s", "4m 5s", "1h 2m" from a start to an end (or `nowMs`); null without a start. */
export function taskElapsed(t: Pick<AgentTask, 'startedAt' | 'endedAt'>, nowMs: number): string | null {
  if (!t.startedAt) return null;
  const s = Math.max(0, Math.floor(((t.endedAt ? Date.parse(t.endedAt) : nowMs) - Date.parse(t.startedAt)) / 1000));
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
    taskId: null,
    command: null,
  });
  return {
    running: [...tasks.running, ...extra.filter((m) => !m.killed).map(row)],
    finished: [...tasks.finished, ...extra.filter((m) => m.killed).map(row)],
  };
}
