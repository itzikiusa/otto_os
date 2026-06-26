// Scheduled Tasks store: list + per-task run history, REST loaders, and live-event
// application. Like the loops/swarm stores it does NOT import events.svelte.ts —
// the event dispatcher calls `scheduledTasks.applyEvent(...)` on this singleton.

import { scheduledTasksApi, type ScheduledTaskInput } from '../api/scheduledTasks';
import type { OttoEvent, ScheduledTask, ScheduledTaskPreset, ScheduledTaskRun } from '../api/types';

class ScheduledTasksStore {
  list: ScheduledTask[] = $state([]);
  loadingList = $state(false);
  presets: ScheduledTaskPreset[] = $state([]);
  /** task_id → its recent runs (loaded on demand when a task is expanded). */
  runsByTask: Record<string, ScheduledTaskRun[]> = $state({});
  private wsId = '';

  async loadList(workspaceId: string): Promise<void> {
    this.wsId = workspaceId;
    this.loadingList = true;
    try {
      this.list = await scheduledTasksApi.list(workspaceId);
    } catch {
      this.list = [];
    } finally {
      this.loadingList = false;
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
    } catch {
      this.runsByTask = { ...this.runsByTask, [taskId]: [] };
    }
  }

  async create(workspaceId: string, body: ScheduledTaskInput): Promise<ScheduledTask> {
    const t = await scheduledTasksApi.create(workspaceId, body);
    await this.loadList(workspaceId);
    return t;
  }

  async update(id: string, body: Partial<ScheduledTaskInput>): Promise<void> {
    await scheduledTasksApi.update(id, body);
    if (this.wsId) await this.loadList(this.wsId);
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

  /** Live WS tick: refresh the affected task's runs + the list status. */
  applyEvent(ev: Extract<OttoEvent, { type: 'scheduled_task_run_updated' }>): void {
    if (this.wsId && ev.workspace_id !== this.wsId) return;
    void this.loadRuns(ev.task_id);
    if (this.wsId) void this.loadList(this.wsId);
  }
}

export const scheduledTasks = new ScheduledTasksStore();
