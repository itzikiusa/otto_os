// `open_route` notice actions (automation notices, review 08 · N1): the daemon
// names an in-app route; this parses it into the page + item to open. Pure, so
// the store's dispatcher stays thin and this is unit-testable.

export type NoticeTarget =
  | { kind: 'workflow_run'; workflowId: string; runId: string }
  | { kind: 'scheduled_task'; taskId: string; runId: string | null }
  | { kind: 'goal_loop'; loopId: string }
  /** Any other route (e.g. `personal-agents/<id>/runs`): navigate to it as-is. */
  | { kind: 'route'; route: string };

export function parseNoticeRoute(route: string): NoticeTarget {
  const clean = route.replace(/^#?\/?/, '');
  const p = clean.split('/').filter(Boolean).map(decodeURIComponent);
  if (p[0] === 'workflows' && p[1] && p[2] === 'runs' && p[3]) {
    return { kind: 'workflow_run', workflowId: p[1], runId: p[3] };
  }
  if (p[0] === 'scheduled-tasks' && p[1]) {
    return { kind: 'scheduled_task', taskId: p[1], runId: p[2] === 'runs' && p[3] ? p[3] : null };
  }
  if (p[0] === 'loops' && p[1]) return { kind: 'goal_loop', loopId: p[1] };
  return { kind: 'route', route: clean };
}
