// Live turn-activity and background-task waiting summaries for RuntimeTurnStatus.svelte.
// Pure so every state transition is covered by unit tests (unit/runtimeLiveSyncAndScroll.test.ts).

import type { TurnItem } from '../../../lib/api/types';
import type { AgentTask } from './agentTasks';
import type { RuntimeProjection } from './projection';
import { latestTurn, toolWords } from './timeline';

export type LiveFooterState =
  | {
      mode: 'working';
      prefix: string;
      activity: string;
      elapsed: string;
    }
  | {
      mode: 'waiting_bg';
      /** "Waiting on 2 subagents and 1 command". */
      text: string;
      /** Their titles: "Share link API, Share link docs +1". */
      detail: string;
    };

/** Format elapsed seconds as `0s`, `14s`, `2m 08s`, or `1h 04m`. */
export function formatLiveElapsed(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  if (s < 60) return `${s}s`;
  if (s < 3600) {
    const m = Math.floor(s / 60);
    const rem = String(s % 60).padStart(2, '0');
    return `${m}m ${rem}s`;
  }
  const h = Math.floor(s / 3600);
  const m = String(Math.floor((s % 3600) / 60)).padStart(2, '0');
  return `${h}h ${m}m`;
}

function itemActivity(item: TurnItem): string | null {
  switch (item.type) {
    case 'command_execution': {
      const cmd = item.input.trim().split('\n')[0]?.trim() ?? '';
      return cmd ? `Running ${cmd.slice(0, 48)}` : 'Running command';
    }
    case 'dynamic_tool':
      return item.title?.trim() || toolWords(item.tool_name);
    case 'file_change':
      return item.title?.trim() || 'Editing files';
    case 'web_search': {
      const q = item.title?.trim() || item.patterns?.[0]?.trim() || '';
      return q ? `Searching "${q.slice(0, 40)}"` : 'Searching the web';
    }
    case 'subagent':
      return `Subagent: ${(item.description ?? item.subagent_type ?? item.title ?? 'working').slice(0, 48)}`;
    case 'assistant_message':
      return item.streaming ? 'Streaming reply…' : null;
    case 'reasoning':
      return item.streaming ? 'Thinking…' : null;
    default:
      return null;
  }
}

/** Human summary of the active step inside a running turn. */
export function activeActivityLabel(p: RuntimeProjection | null): string {
  if (!p) return 'Working…';
  const turn = latestTurn(p);
  const scope = turn ? p.items.filter((i) => i.turn_id === turn.id && !i.parent_item_id) : p.items;
  for (let idx = scope.length - 1; idx >= 0; idx -= 1) {
    const item = scope[idx];
    const open =
      item.status === 'running' ||
      item.status === 'pending' ||
      (item.type === 'assistant_message' && item.streaming) ||
      (item.type === 'reasoning' && item.streaming);
    if (!open) continue;
    const label = itemActivity(item);
    if (label) return label;
  }
  return 'Working…';
}

/** Derive the live footer shown at the bottom of a runtime thread timeline. */
export function deriveLiveFooter(
  p: RuntimeProjection | null,
  workingFor: string | null,
  ownTurn: boolean,
  background: readonly Pick<AgentTask, 'label' | 'kind' | 'childSessionId'>[],
): LiveFooterState | null {
  if (workingFor) {
    return {
      mode: 'working',
      prefix: ownTurn ? 'Background wakeup' : 'Working',
      activity: activeActivityLabel(p),
      elapsed: workingFor,
    };
  }
  if (background.length > 0) {
    const names = background
      .slice(0, 3)
      .map((b) => b.label.trim())
      .filter(Boolean)
      .join(', ');
    const more = background.length > 3 ? ` +${background.length - 3}` : '';
    return { mode: 'waiting_bg', text: `Waiting on ${waitingGroups(background)}`, detail: names ? `${names}${more}` : '' };
  }
  return null;
}

/** A subagent (a thread delegated to, the provider's own subagent, a swarm
 *  member), as opposed to a command or a timer. */
export const isSubagentTask = (t: Pick<AgentTask, 'kind' | 'childSessionId'>): boolean => t.kind === 'agent' || !!t.childSessionId;

/** "2 subagents and 1 command" (T3 Code's `backgroundWorkNowItem` groups). */
export function waitingGroups(tasks: readonly Pick<AgentTask, 'kind' | 'childSessionId'>[]): string {
  const n = (f: (t: Pick<AgentTask, 'kind' | 'childSessionId'>) => boolean): number => tasks.filter(f).length;
  const groups: [number, string, string][] = [
    [n(isSubagentTask), 'subagent', 'subagents'],
    [n((t) => !isSubagentTask(t) && t.kind !== 'timer'), 'command', 'commands'],
    [n((t) => !isSubagentTask(t) && t.kind === 'timer'), 'timer', 'timers'],
  ];
  const parts = groups.filter(([c]) => c > 0).map(([c, one, many]) => `${c} ${c === 1 ? one : many}`);
  return parts.length > 1 ? `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}` : (parts[0] ?? '');
}
