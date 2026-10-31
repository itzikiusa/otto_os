// "Today" on the Home desktop: the glance strip above the widgets — what needs
// you, what is running, what is next, what you touched recently. Built from
// data Otto already has (no new endpoints):
//
//   Needs you  pending MCP approvals · sessions waiting on input · unread warnings
//   Running    working agent sessions · in-flight workflow runs
//   Up next    scheduled-task runs due in the next 24 h — forward-looking only.
//              Past notices live in the bell (warnings already surface under Needs you).
//   Recent     Design Hall artifacts
//
// The live parts (ws, notifications) are derived; the fetched parts poll on a
// quiet cadence only while Home is mounted (start/stop are ref-counted).

import { api } from '../../lib/api/client';
import { listArtifacts } from '../../lib/api/design';
import { scheduledTasksApi } from '../../lib/api/scheduledTasks';
import type { IconName } from '../../lib/components/Icon.svelte';
import type { DesignArtifact, McpApproval, Notice, ScheduledTask } from '../../lib/api/types';
import { untrack } from 'svelte';
import { router } from '../../lib/router.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { notifications } from '../../lib/stores/notifications.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { livePoll, type Poller } from './boxes/poll';

/** One glance row. `open` runs on click; `live` pulses its dot. */
export interface TodayRow {
  id: string;
  title: string;
  detail: string;
  icon: IconName;
  tone: 'warning' | 'working' | 'idle' | 'accent';
  /** ISO time the row refers to (sorting + "2m ago"). */
  at?: string;
  open: () => void;
}

const POLL_MS = 60_000;
/** Scheduled runs due within this window show under Up next. */
const UP_NEXT_MS = 24 * 60 * 60 * 1000;

function timeOfDay(iso: string): string {
  return new Date(iso).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function sessionIdOf(n: Notice): string | null {
  if (n.action?.type === 'open_session') return n.action.session_id;
  const key = n.source_key ?? '';
  return key.startsWith('session:') ? (key.split(':')[1] ?? null) : null;
}

class TodayStore {
  approvals: McpApproval[] = $state([]);
  designs: DesignArtifact[] = $state([]);
  /** Enabled scheduled tasks (their `next_run_at` feeds Up next). */
  scheduled: ScheduledTask[] = $state([]);
  /** First fetch settled (success or not) — the fetched rows stop skeletoning. */
  loaded = $state(false);
  /** The last fetch had a failing source — the cards say so (with Retry)
   *  instead of passing a broken backend off as "nothing here". */
  failed = $state(false);

  private users = 0;
  private poller: Poller | null = null;
  private seq = 0;
  private workspace: string | null | undefined;

  private syncWorkspace(): void {
    if (this.workspace === ws.currentId) return;
    this.workspace = ws.currentId;
    this.seq++;
    this.approvals = [];
    this.designs = [];
    this.scheduled = [];
    this.loaded = false;
    this.failed = false;
  }

  /** Sessions blocked on the operator (foreground only, like the sidebar). */
  waiting = $derived(ws.foregroundActive.filter((s) => ws.needsYou[s.id] === true));

  needs: TodayRow[] = $derived.by(() => {
    const rows: TodayRow[] = [];
    for (const a of this.approvals) {
      rows.push({
        id: `approval:${a.id}`,
        title: a.title,
        detail: a.server_name ? `Approval · ${a.server_name}` : 'Approval',
        icon: 'shield',
        tone: 'warning',
        at: a.created_at,
        open: () => router.go('mcp/activity'),
      });
    }
    for (const s of this.waiting) {
      rows.push({
        id: `session:${s.id}`,
        title: s.title,
        detail: 'Waiting for your input',
        icon: 'terminal',
        tone: 'warning',
        at: s.last_active_at,
        open: () => ws.navigateToSession(s.id),
      });
    }
    const waitingIds = new Set(this.waiting.map((s) => s.id));
    for (const n of notifications.notices) {
      if (n.read || n.severity === 'info') continue;
      const sid = sessionIdOf(n);
      if (sid && waitingIds.has(sid)) continue;
      rows.push({
        id: `notice:${n.id}`,
        title: n.title,
        detail: n.body,
        icon: 'warning',
        tone: 'warning',
        at: n.created_at,
        open: () => void notifications.runAction(n),
      });
    }
    return rows;
  });

  running: TodayRow[] = $derived.by(() => {
    const rows: TodayRow[] = ws.foregroundActive
      .filter((s) => ws.statusMap[s.id] === 'working')
      .map((s) => ({
        id: `session:${s.id}`,
        title: s.title,
        detail: s.provider === 'shell' ? 'Terminal' : `Agent · ${s.provider}`,
        icon: 'terminal' as IconName,
        tone: 'working' as const,
        at: s.last_active_at,
        open: () => ws.navigateToSession(s.id),
      }));
    for (const r of ws.activeWorkflowRuns) {
      rows.push({
        id: `run:${r.run_id}`,
        title: r.workflow_name,
        detail: r.nodes_total ? `Workflow · step ${Math.min(r.nodes_done + 1, r.nodes_total)} of ${r.nodes_total}` : 'Workflow',
        icon: 'split',
        tone: 'working',
        at: r.started_at,
        open: () => router.go('workflows'),
      });
    }
    return rows;
  });

  /** What happens next, soonest first: scheduled-task runs due within UP_NEXT_MS. */
  upNext: TodayRow[] = $derived.by(() => {
    const now = Date.now();
    const runs: TodayRow[] = this.scheduled
      .filter((t) => {
        if (!t.enabled || !t.next_run_at) return false;
        const d = Date.parse(t.next_run_at) - now;
        return Number.isFinite(d) && d < UP_NEXT_MS;
      })
      .map((t) => ({
        id: `scheduled:${t.id}`,
        title: t.name,
        detail: `Scheduled task · ${timeOfDay(t.next_run_at ?? '')}`,
        icon: 'calendar' as IconName,
        tone: 'idle' as const,
        at: t.next_run_at ?? undefined,
        open: () => router.go('scheduled-tasks'),
      }));
    return runs.sort((a, b) => Date.parse(a.at ?? '') - Date.parse(b.at ?? ''));
  });

  recent: TodayRow[] = $derived.by(() => {
    const rows: TodayRow[] = this.designs.map((a) => ({
      id: `design:${a.id}`,
      title: a.title,
      detail: `Design Hall · ${a.studio}${a.head_seq ? ` · v${a.head_seq}` : ''}`,
      icon: 'designHall' as IconName,
      tone: 'accent' as const,
      at: a.updated_at,
      open: () => router.go(`design/a/${encodeURIComponent(a.id)}`),
    }));
    return rows.sort((a, b) => Date.parse(b.at ?? '') - Date.parse(a.at ?? '')).slice(0, 5);
  });

  private async load(): Promise<boolean> {
    this.syncWorkspace();
    const mine = ++this.seq;
    const wsId = ws.currentId;
    const can = (f: Parameters<typeof auth.can>[0]) => auth.can(f, 'view');
    const settle = <T>(p: Promise<T>, fallback: T): Promise<{ ok: boolean; v: T }> =>
      p.then((v) => ({ ok: true, v })).catch(() => ({ ok: false, v: fallback }));
    const [appr, designs, sched] = await Promise.all([
      can('mcp') ? settle(api.get<McpApproval[]>('/mcp/approvals?status=pending'), [] as McpApproval[]) : { ok: true, v: [] as McpApproval[] },
      can('design') ? settle(listArtifacts({ workspace_id: wsId ?? undefined, limit: 4 }), [] as DesignArtifact[]) : { ok: true, v: [] as DesignArtifact[] },
      wsId && can('scheduled_tasks') ? settle(scheduledTasksApi.list(wsId), [] as ScheduledTask[]) : { ok: true, v: [] as ScheduledTask[] },
    ]);
    if (mine !== this.seq || ws.currentId !== wsId) return true;
    this.approvals = appr.v.filter((a) => !a.workspace_id || !wsId || a.workspace_id === wsId);
    this.designs = designs.v;
    this.scheduled = sched.v;
    this.loaded = true;
    this.failed = !(appr.ok && designs.ok && sched.ok);
    return !this.failed;
  }

  /** Home mounted: start polling (and load notices if nothing has yet). */
  start(): void {
    this.users += 1;
    if (this.users > 1) return;
    void notifications.ensureLoaded();
    // Event-fed: approvals, designs and scheduled runs each announce changes; a
    // burst (an agent editing a design) is coalesced to at most one reload per
    // ~15 s. POLL_MS only while the event socket is down.
    this.poller = livePoll(
      () => this.load(),
      POLL_MS,
      ['mcp_approval_changed', 'design_artifact_updated', 'scheduled_task_run_updated'],
      {
        match: (ev) => typeof ev.workspace_id !== 'string' || !ws.currentId || ev.workspace_id === ws.currentId,
        debounceMs: 3000,
        maxWaitMs: 15_000,
        minIntervalMs: 15_000,
        // Cold boot: wait for the workspace list (HomeToday refreshes when it
        // settles) — an unscoped first load was repeated scoped right after.
        immediate: untrack(() => ws.listSettled),
      },
    );
  }

  stop(): void {
    this.users = Math.max(0, this.users - 1);
    if (this.users > 0) return;
    this.seq++;
    this.poller?.stop();
    this.poller = null;
  }

  /** Workspace switched / manual refresh. */
  refresh(): void {
    this.syncWorkspace();
    this.poller?.now();
  }
}

export const today = new TodayStore();
