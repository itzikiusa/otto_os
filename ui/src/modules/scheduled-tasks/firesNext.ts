// Pure helpers for the unified "Fires next" radar and emergency "Pause all
// automations" summary across Scheduled Tasks and Personal Agent routines.
import type { PersonalAgent, PersonalAgentSchedule, ScheduledTask } from '../../lib/api/types';

export interface UpcomingFire {
  id: string;
  kind: 'task' | 'routine';
  title: string;
  subtitle: string;
  nextAt: string;
  nextMs: number;
  route: string;
}

export interface ActiveAutomationsCount {
  tasks: number;
  agents: number;
  routines: number;
  total: number;
}

function routineTitle(directive: string): string {
  const first = directive.trim().split('\n')[0]?.trim() ?? '';
  return first.length > 64 ? `${first.slice(0, 63)}…` : first || 'Routine';
}

export function buildFiresNext(
  tasks: readonly ScheduledTask[],
  agents: readonly PersonalAgent[],
  schedulesByAgent: Record<string, PersonalAgentSchedule[]>,
  limit = 6,
): UpcomingFire[] {
  const items: UpcomingFire[] = [];
  for (const t of tasks) {
    if (!t.enabled || !t.next_run_at) continue;
    const ms = Date.parse(t.next_run_at);
    if (!Number.isFinite(ms)) continue;
    items.push({
      id: `task:${t.id}`,
      kind: 'task',
      title: t.name,
      subtitle: t.kind === 'workflow' ? 'Workflow task' : 'Scheduled task',
      nextAt: t.next_run_at,
      nextMs: ms,
      route: `scheduled-tasks/${t.id}`,
    });
  }
  for (const a of agents) {
    if (!a.enabled) continue;
    for (const s of schedulesByAgent[a.id] ?? []) {
      if (!s.enabled || !s.next_run_at) continue;
      const ms = Date.parse(s.next_run_at);
      if (!Number.isFinite(ms)) continue;
      items.push({
        id: `routine:${s.id}`,
        kind: 'routine',
        title: routineTitle(s.directive),
        subtitle: a.name,
        nextAt: s.next_run_at,
        nextMs: ms,
        route: `personal-agents/${a.id}/routines`,
      });
    }
  }
  items.sort((a, b) => a.nextMs - b.nextMs);
  return items.slice(0, Math.max(1, limit));
}

export function countActiveAutomations(
  tasks: readonly ScheduledTask[],
  agents: readonly PersonalAgent[],
  schedulesByAgent: Record<string, PersonalAgentSchedule[]>,
): ActiveAutomationsCount {
  const activeTasks = tasks.filter((t) => t.enabled).length;
  const activeAgents = agents.filter((a) => a.enabled);
  const activeRoutines = activeAgents.reduce(
    (sum, a) => sum + (schedulesByAgent[a.id] ?? []).filter((s) => s.enabled).length,
    0,
  );
  return {
    tasks: activeTasks,
    agents: activeAgents.length,
    routines: activeRoutines,
    total: activeTasks + activeAgents.length,
  };
}
