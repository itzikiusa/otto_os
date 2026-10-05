// Scheduled Tasks store: list + per-task run history, REST loaders, and live-event
// application. Like the loops/swarm stores it does NOT import events.svelte.ts —
// the event dispatcher calls `scheduledTasks.applyEvent(...)` on this singleton.

import { scheduledTasksApi, type ScheduledTaskInput } from '../api/scheduledTasks';
import type { OttoEvent, ScheduledTask, ScheduledTaskPreset, ScheduledTaskRun } from '../api/types';
import { loadErrorText } from '../loadError';
import { announceModule } from '../lazyModule';

class ScheduledTasksStore {
  list: ScheduledTask[] = $state([]);
  loadingList = $state(false);
  /** Last list-load failure (human text) — rendered inline with Retry, never as "no tasks". */
  listError = $state<string | null>(null);
  presets: ScheduledTaskPreset[] = $state([]);
  /** task_id → its recent runs (loaded on demand when a task is expanded). */
  runsByTask: Record<string, ScheduledTaskRun[]> = $state({});
  /** task_id → why its run history failed to load (absent = ok). */
  runsError: Record<string, string> = $state({});
  private wsId = '';
  private listGeneration = 0;
  private workspaceGeneration = 0;

  async loadList(workspaceId: string): Promise<void> {
    // Another workspace's tasks are not "stale data" for this one.
    if (this.wsId !== workspaceId) { this.list = []; this.listError = null; this.workspaceGeneration++; }
    const generation = ++this.listGeneration;
    this.wsId = workspaceId;
    this.loadingList = true;
    try {
      const list = await scheduledTasksApi.list(workspaceId);
      // A slower load for a workspace we've since left must not land here.
      if (this.wsId !== workspaceId || generation !== this.listGeneration) return;
      this.list = list;
      this.listError = null;
    } catch (e) {
      if (this.wsId === workspaceId && generation === this.listGeneration) this.listError = loadErrorText(e);
    } finally {
      if (this.wsId === workspaceId && generation === this.listGeneration) this.loadingList = false;
    }
  }

  async loadPresets(): Promise<void> {
    if (this.presets.length) return;
    try {
      this.presets = await scheduledTasksApi.presets();
    } catch {
      this.presets = [];
    }
  }

  async loadRuns(taskId: string): Promise<void> {
    try {
      this.runsByTask = { ...this.runsByTask, [taskId]: await scheduledTasksApi.runs(taskId) };
      const { [taskId]: _drop, ...rest } = this.runsError;
      this.runsError = rest;
    } catch (e) {
      this.runsError = { ...this.runsError, [taskId]: loadErrorText(e) };
    }
  }

  async create(workspaceId: string, body: ScheduledTaskInput): Promise<ScheduledTask> {
    const generation = this.workspaceGeneration;
    const t = await scheduledTasksApi.create(workspaceId, body);
    if (this.wsId === workspaceId && generation === this.workspaceGeneration) await this.loadList(workspaceId);
    return t;
  }

  async update(id: string, body: Partial<ScheduledTaskInput>): Promise<void> {
    const workspaceId = this.wsId, generation = this.workspaceGeneration;
    await scheduledTasksApi.update(id, body);
    if (workspaceId && this.wsId === workspaceId && generation === this.workspaceGeneration) await this.loadList(workspaceId);
  }

  async setEnabled(id: string, enabled: boolean): Promise<void> {
    await this.update(id, { enabled });
  }

  async remove(id: string): Promise<void> {
    await scheduledTasksApi.remove(id);
    if (this.wsId) await this.loadList(this.wsId);
    const next = { ...this.runsByTask };
    delete next[id];
    this.runsByTask = next;
  }

  async runNow(id: string): Promise<void> {
    await scheduledTasksApi.run(id);
    await this.loadRuns(id);
    if (this.wsId) await this.loadList(this.wsId);
  }

  private listReload: ReturnType<typeof setTimeout> | null = null;

  /** Live WS tick: refresh the affected task's runs — only when they were
   *  loaded (the task was expanded; expanding reloads them anyway) — and the
   *  list status, coalesced (each run fires start + finish events; backlog
   *  B6 / SE-23). */
  applyEvent(ev: Extract<OttoEvent, { type: 'scheduled_task_run_updated' }>): void {
    if (this.wsId && ev.workspace_id !== this.wsId) return;
    if (ev.task_id in this.runsByTask) void this.loadRuns(ev.task_id);
    if (!this.wsId || this.listReload) return;
    this.listReload = setTimeout(() => {
      this.listReload = null;
      if (this.wsId) void this.loadList(this.wsId);
    }, 300);
  }
}

export const scheduledTasks = new ScheduledTasksStore();
// Routed by `peek()` in lib/events.svelte.ts (perf H1): let it see this store
// however it was first imported.
announceModule('scheduledTasks', scheduledTasks);
