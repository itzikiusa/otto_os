// "Today" on the Home desktop: the glance strip above the widgets — what needs
// you, what is running, what is next, what you touched recently. Built from
// data Otto already has (no new endpoints):
//
//   Needs you  assistant tasks waiting on you · pending MCP approvals ·
//              sessions waiting on input · work items awaiting approval ·
//              unread warnings
//   Running    assistant tasks running · working agent sessions · in-flight
//              workflow runs
//   Up next    the assistant's reminders due in the next 24 h · today's notices
//   Recent     Design Hall artifacts · pull requests from the work graph
//
// The live parts (ws, notifications) are derived; the fetched parts poll on a
// quiet cadence only while Home is mounted (start/stop are ref-counted).

import { api } from '../../lib/api/client';
import { listArtifacts } from '../../lib/api/design';
import { missionControlApi } from '../../lib/api/missionControl';
import type { IconName } from '../../lib/components/Icon.svelte';
import type { AssistantTask, DesignArtifact, McpApproval, Notice, WorkItem } from '../../lib/api/types';
import { router } from '../../lib/router.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { assistant } from '../../lib/stores/assistant.svelte';
import { notifications } from '../../lib/stores/notifications.svelte';
import { isForeground, ws } from '../../lib/stores/workspace.svelte';
import { poll, type Poller } from './boxes/poll';

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

/** An upcoming reminder (the Up next slot) — today, the assistant's
 *  `reminder` tasks; any other source can map into the same shape. */
export interface Reminder {
  id: string;
  title: string;
  /** ISO due time. */
  due: string;
  /** Where it is delivered ("here", "here + phone", …). */
  channel?: string;
  open?: () => void;
}

const POLL_MS = 60_000;
/** Reminders due within this window show under Up next. */
const UP_NEXT_MS = 24 * 60 * 60 * 1000;

function openAssistant(t: AssistantTask): void {
  router.go(t.thread_id ? `assistant/${t.thread_id}` : 'assistant');
}

function timeOfDay(iso: string): string {
  return new Date(iso).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

function isToday(iso: string): boolean {
  const d = new Date(iso);
  const n = new Date();
  return d.getFullYear() === n.getFullYear() && d.getMonth() === n.getMonth() && d.getDate() === n.getDate();
}

function sessionIdOf(n: Notice): string | null {
  if (n.action?.type === 'open_session') return n.action.session_id;
  const key = n.source_key ?? '';
  return key.startsWith('session:') ? (key.split(':')[1] ?? null) : null;
}

class TodayStore {
  approvals: McpApproval[] = $state([]);
  workApprovals = $state(0);
  designs: DesignArtifact[] = $state([]);
  prs: WorkItem[] = $state([]);
  /** First fetch settled (success or not) — the fetched rows stop skeletoning. */
  loaded = $state(false);
  /** The assistant's reminders due soon, earliest first — see {@link Reminder}. */
  reminders: Reminder[] = $derived.by(() => {
    const now = Date.now();
    return assistant.tasks.data
      .filter((t) => t.kind === 'reminder' && t.state === 'queued' && t.run_at && Date.parse(t.run_at) - now < UP_NEXT_MS)
      .sort((a, b) => Date.parse(a.run_at ?? '') - Date.parse(b.run_at ?? ''))
      .map((t) => ({
        id: t.id,
        title: t.title,
        due: t.run_at ?? t.created_at,
        channel: timeOfDay(t.run_at ?? t.created_at),
        open: () => openAssistant(t),
      }));
  });

  private users = 0;
  private poller: Poller | null = null;
  private seq = 0;

  /** Sessions blocked on the operator (foreground only, like the sidebar). */
  waiting = $derived(ws.sessions.filter((s) => !s.archived && ws.needsYou[s.id] === true && isForeground(s)));

  needs: TodayRow[] = $derived.by(() => {
    const rows: TodayRow[] = [];
    for (const t of assistant.needs.items) {
      rows.push({
        id: `assistant:${t.id}`,
        title: t.title,
        detail: t.needs_you?.prompt ? `Assistant · ${t.needs_you.prompt}` : 'Assistant',
        icon: 'assistant',
        tone: 'warning',
        at: t.updated_at,
        open: () => openAssistant(t),
      });
    }
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
    if (this.workApprovals > 0) {
      rows.push({
        id: 'work-approvals',
        title: `${this.workApprovals} work item${this.workApprovals === 1 ? '' : 's'} awaiting approval`,
        detail: 'Mission Control',
        icon: 'radar',
        tone: 'warning',
        open: () => router.go('mission-control'),
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
    const tasks: TodayRow[] = assistant.tasks.data
      .filter((t) => t.state === 'running')
      .map((t) => ({
        id: `assistant:${t.id}`,
        title: t.title,
        detail: t.detail ? `Assistant · ${t.detail}` : 'Assistant',
        icon: 'assistant' as IconName,
        tone: 'working' as const,
        at: t.updated_at,
        open: () => openAssistant(t),
      }));
    const rows: TodayRow[] = ws.sessions
      .filter((s) => !s.archived && ws.statusMap[s.id] === 'working' && isForeground(s))
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
    return [...tasks, ...rows];
  });

  /** Today's notices not already under Needs you, newest first. */
  upNext: TodayRow[] = $derived.by(() => {
    const needIds = new Set(this.needs.map((r) => r.id));
    const reminders: TodayRow[] = this.reminders.map((r) => ({
      id: `reminder:${r.id}`,
      title: r.title,
      detail: r.channel ? `Reminder · ${r.channel}` : 'Reminder',
      icon: 'bell',
      tone: 'accent',
      at: r.due,
      open: r.open ?? (() => {}),
    }));
    const notices: TodayRow[] = notifications.notices
      .filter((n) => isToday(n.created_at) && !needIds.has(`notice:${n.id}`))
      .slice(0, 6)
      .map((n) => ({
        id: `notice:${n.id}`,
        title: n.title,
        detail: n.body,
        icon: 'bell',
        tone: 'idle',
        at: n.created_at,
        open: () => void notifications.runAction(n),
      }));
    return [...reminders, ...notices];
  });

  recent: TodayRow[] = $derived.by(() => {
    const rows: TodayRow[] = [
      ...this.designs.map((a) => ({
        id: `design:${a.id}`,
        title: a.title,
        detail: `Design Hall · ${a.studio}${a.head_seq ? ` · v${a.head_seq}` : ''}`,
        icon: 'designHall' as IconName,
        tone: 'accent' as const,
        at: a.updated_at,
        open: () => router.go(`design/a/${encodeURIComponent(a.id)}`),
      })),
      ...this.prs.map((p) => ({
        id: `pr:${p.id}`,
        title: p.title,
        detail: p.branch ? `Pull request · ${p.branch}` : 'Pull request',
        icon: 'pr' as IconName,
        tone: 'idle' as const,
        at: p.updated_at,
        open: () => router.go('mission-control'),
      })),
    ];
    return rows.sort((a, b) => Date.parse(b.at ?? '') - Date.parse(a.at ?? '')).slice(0, 5);
  });

  private async load(): Promise<boolean> {
    const mine = ++this.seq;
    // The assistant store stays live over the WS once loaded; this re-syncs it
    // on the same quiet cadence (its loaders guard their own stale results).
    void assistant.loadNeedsYou();
    void assistant.loadTasks();
    const wsId = ws.currentId;
    const can = (f: Parameters<typeof auth.can>[0]) => auth.can(f, 'view');
    const settle = <T>(p: Promise<T>, fallback: T): Promise<{ ok: boolean; v: T }> =>
      p.then((v) => ({ ok: true, v })).catch(() => ({ ok: false, v: fallback }));
    const [appr, summary, designs, prs] = await Promise.all([
      can('mcp') ? settle(api.get<McpApproval[]>('/mcp/approvals?status=pending'), [] as McpApproval[]) : { ok: true, v: [] as McpApproval[] },
      wsId && can('mission_control') ? settle(missionControlApi.summary(wsId), null) : { ok: true, v: null },
      can('design') ? settle(listArtifacts({ workspace_id: wsId ?? undefined, limit: 4 }), [] as DesignArtifact[]) : { ok: true, v: [] as DesignArtifact[] },
      wsId && can('mission_control') ? settle(missionControlApi.items(wsId, { kind: 'pr', limit: 3 }), [] as WorkItem[]) : { ok: true, v: [] as WorkItem[] },
    ]);
    if (mine !== this.seq) return true;
    this.approvals = appr.v.filter((a) => !a.workspace_id || !wsId || a.workspace_id === wsId);
    this.workApprovals = summary.v?.needs_approval ?? 0;
    this.designs = designs.v;
    this.prs = prs.v;
    this.loaded = true;
    return appr.ok && summary.ok && designs.ok && prs.ok;
  }

  /** Home mounted: start polling (and load notices if nothing has yet). */
  start(): void {
    this.users += 1;
    if (this.users > 1) return;
    if (!notifications.loaded) void notifications.load();
    this.poller = poll(() => this.load(), POLL_MS);
  }

  stop(): void {
    this.users = Math.max(0, this.users - 1);
    if (this.users > 0) return;
    this.poller?.stop();
    this.poller = null;
  }

  /** Workspace switched / manual refresh. */
  refresh(): void {
    this.poller?.now();
  }
}

export const today = new TodayStore();
