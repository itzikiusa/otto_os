// Agent Swarm store: swarms, the open swarm's detail (agents + projects), tasks
// per project, runs, and the shared board. Fed by REST loads and the events WS
// (swarm_run_updated / swarm_task_updated / swarm_message_posted / swarm_status).

import { api } from '../api/client';
import type { OttoEvent } from '../api/types';
import type {
  CreateAgentReq,
  RecruitedAgent,
  RunFilters,
  Swarm,
  SwarmAgent,
  SwarmDetail,
  SwarmGraph,
  SwarmMessage,
  SwarmPreset,
  SwarmProject,
  SwarmRun,
  SwarmTask,
  UpdateAgentReq,
} from '../../modules/swarm/types';

type Lifecycle = 'start' | 'pause' | 'abort' | 'resume';

class SwarmStore {
  swarms: Swarm[] = $state([]);
  detail: SwarmDetail | null = $state(null);
  tasksByProject: Record<string, SwarmTask[]> = $state({});
  runs: SwarmRun[] = $state([]);
  board: SwarmMessage[] = $state([]);
  presets: SwarmPreset[] = $state([]);
  graph: SwarmGraph | null = $state(null);
  selectedProjectId: string | null = $state(null);
  selectedSessionId: string | null = $state(null);
  loading = $state(false);
  /** Set when a deep-link (e.g. Product → Swarm) wants the Kanban view shown
   *  for the just-opened project. SwarmPage reads + clears this on mount. */
  pendingKanban = $state(false);

  private wsId: string | null = null;

  get openId(): string | null {
    return this.detail?.id ?? null;
  }

  agentById(id: string | null | undefined): SwarmAgent | null {
    if (!id || !this.detail) return null;
    return this.detail.agents.find((a) => a.id === id) ?? null;
  }

  // -- Swarms ---------------------------------------------------------------

  async loadSwarms(workspaceId: string): Promise<void> {
    this.wsId = workspaceId;
    try {
      this.swarms = await api.get<Swarm[]>(`/workspaces/${workspaceId}/swarm/swarms`);
    } catch {
      this.swarms = [];
    }
  }

  async openSwarm(sid: string): Promise<void> {
    this.loading = true;
    try {
      this.detail = await api.get<SwarmDetail>(`/swarm/swarms/${sid}`);
      this.selectedProjectId = this.detail.projects[0]?.id ?? null;
      this.selectedSessionId = null;
      await Promise.all([
        this.loadAllTasks(),
        this.loadRuns({ swarm_id: sid }),
        this.loadBoard(),
        this.loadGraph(sid),
      ]);
    } finally {
      this.loading = false;
    }
  }

  /**
   * Deep-link helper: ensure swarms are loaded for `workspaceId`, open `sid`,
   * select project `pid`, and flag the Kanban view. Used by the Product → Swarm
   * hand-off so the user lands directly on the new project's board.
   */
  async openProject(workspaceId: string, sid: string, pid: string): Promise<void> {
    if (this.wsId !== workspaceId || this.swarms.length === 0) {
      await this.loadSwarms(workspaceId);
    }
    if (this.detail?.id !== sid) {
      await this.openSwarm(sid);
    }
    this.selectedProjectId = pid;
    if (!this.tasksByProject[pid]) await this.loadTasks(pid);
    this.pendingKanban = true;
  }

  closeSwarm(): void {
    this.detail = null;
    this.tasksByProject = {};
    this.runs = [];
    this.board = [];
    this.graph = null;
    this.selectedProjectId = null;
    this.selectedSessionId = null;
  }

  async createSwarm(name: string, presetSlug?: string): Promise<SwarmDetail | null> {
    if (!this.wsId) return null;
    const created = await api.post<SwarmDetail>(`/workspaces/${this.wsId}/swarm/swarms`, {
      name,
      preset_slug: presetSlug ?? null,
    });
    await this.loadSwarms(this.wsId);
    this.detail = created;
    this.selectedProjectId = created.projects[0]?.id ?? null;
    await this.openSwarm(created.id);
    return created;
  }

  async updateSwarm(sid: string, patch: Partial<Swarm>): Promise<void> {
    const updated = await api.patch<Swarm>(`/swarm/swarms/${sid}`, patch);
    if (this.detail?.id === sid) this.detail = { ...this.detail, ...updated };
    this.swarms = this.swarms.map((s) => (s.id === sid ? updated : s));
  }

  async setParallelCap(sid: string, cap: number): Promise<void> {
    const cfg = { ...(this.detail?.config ?? {}), max_parallel_sessions: cap };
    await this.updateSwarm(sid, { config: cfg } as Partial<Swarm>);
  }

  async deleteSwarm(sid: string): Promise<void> {
    await api.del(`/swarm/swarms/${sid}`);
    if (this.detail?.id === sid) this.closeSwarm();
    this.swarms = this.swarms.filter((s) => s.id !== sid);
  }

  async lifecycle(action: Lifecycle, sid: string): Promise<void> {
    if (!this.wsId) return;
    const updated = await api.post<Swarm>(
      `/workspaces/${this.wsId}/swarm/swarms/${sid}/${action}`,
    );
    if (this.detail?.id === sid) this.detail = { ...this.detail, ...updated };
    this.swarms = this.swarms.map((s) => (s.id === sid ? updated : s));
  }

  // -- Agents ---------------------------------------------------------------

  async createAgent(sid: string, req: CreateAgentReq): Promise<void> {
    await api.post<SwarmAgent>(`/swarm/swarms/${sid}/agents`, req);
    await this.refreshDetail();
  }

  async updateAgent(aid: string, req: UpdateAgentReq): Promise<void> {
    await api.patch<SwarmAgent>(`/swarm/agents/${aid}`, req);
    await this.refreshDetail();
  }

  async deleteAgent(aid: string): Promise<void> {
    await api.del(`/swarm/agents/${aid}`);
    await this.refreshDetail();
  }

  async recruit(role: string, context?: string): Promise<RecruitedAgent> {
    if (!this.wsId) throw new Error('no workspace');
    return api.post<RecruitedAgent>(`/workspaces/${this.wsId}/swarm/recruit`, {
      swarm_id: this.detail?.id ?? null,
      role,
      context: context ?? null,
    });
  }

  // -- Projects -------------------------------------------------------------

  async createProject(
    sid: string,
    body: { name: string; description?: string; repo_path?: string; goal_md?: string },
  ): Promise<void> {
    const p = await api.post<SwarmProject>(`/swarm/swarms/${sid}/projects`, body);
    await this.refreshDetail();
    this.selectedProjectId = p.id;
    await this.loadTasks(p.id);
  }

  async updateProject(pid: string, patch: Partial<SwarmProject>): Promise<void> {
    await api.patch<SwarmProject>(`/swarm/projects/${pid}`, patch);
    await this.refreshDetail();
  }

  async deleteProject(pid: string): Promise<void> {
    await api.del(`/swarm/projects/${pid}`);
    delete this.tasksByProject[pid];
    await this.refreshDetail();
    if (this.selectedProjectId === pid) {
      this.selectedProjectId = this.detail?.projects[0]?.id ?? null;
    }
  }

  async plan(pid: string): Promise<void> {
    if (!this.wsId) return;
    const tasks = await api.post<SwarmTask[]>(
      `/workspaces/${this.wsId}/swarm/projects/${pid}/plan`,
      {},
    );
    this.tasksByProject[pid] = tasks;
    this.tasksByProject = { ...this.tasksByProject };
  }

  // -- Tasks ----------------------------------------------------------------

  async loadAllTasks(): Promise<void> {
    if (!this.detail) return;
    await Promise.all(this.detail.projects.map((p) => this.loadTasks(p.id)));
  }

  async loadTasks(pid: string): Promise<void> {
    try {
      this.tasksByProject[pid] = await api.get<SwarmTask[]>(`/swarm/projects/${pid}/tasks`);
      this.tasksByProject = { ...this.tasksByProject };
    } catch {
      /* best-effort */
    }
  }

  tasks(pid: string | null): SwarmTask[] {
    return pid ? (this.tasksByProject[pid] ?? []) : [];
  }

  async createTask(
    pid: string,
    body: { title: string; description?: string; assignee_agent_id?: string; priority?: string },
  ): Promise<void> {
    await api.post<SwarmTask>(`/swarm/projects/${pid}/tasks`, body);
    await this.loadTasks(pid);
  }

  async updateTask(task: SwarmTask, patch: Partial<SwarmTask>): Promise<void> {
    await api.patch<SwarmTask>(`/swarm/tasks/${task.id}`, patch);
    await this.loadTasks(task.project_id);
  }

  async deleteTask(task: SwarmTask): Promise<void> {
    await api.del(`/swarm/tasks/${task.id}`);
    await this.loadTasks(task.project_id);
  }

  async runTask(task: SwarmTask): Promise<void> {
    await api.post<SwarmRun>(`/swarm/tasks/${task.id}/run`);
    await this.loadTasks(task.project_id);
    if (this.detail) await this.loadRuns({ swarm_id: this.detail.id });
  }

  // -- Runs -----------------------------------------------------------------

  async loadRuns(filters: RunFilters): Promise<void> {
    if (!this.wsId) return;
    const q = new URLSearchParams();
    if (filters.swarm_id) q.set('swarm_id', filters.swarm_id);
    if (filters.project_id) q.set('project_id', filters.project_id);
    if (filters.agent_id) q.set('agent_id', filters.agent_id);
    if (filters.status) q.set('status', filters.status);
    try {
      this.runs = await api.get<SwarmRun[]>(
        `/workspaces/${this.wsId}/swarm/runs?${q.toString()}`,
      );
    } catch {
      this.runs = [];
    }
  }

  async stopRun(rid: string): Promise<void> {
    const updated = await api.post<SwarmRun>(`/swarm/runs/${rid}/stop`);
    this.runs = this.runs.map((r) => (r.id === rid ? updated : r));
  }

  // -- Board ----------------------------------------------------------------

  async loadBoard(projectId?: string, taskId?: string): Promise<void> {
    if (!this.detail) return;
    const q = new URLSearchParams();
    if (projectId) q.set('project_id', projectId);
    if (taskId) q.set('task_id', taskId);
    try {
      this.board = await api.get<SwarmMessage[]>(
        `/swarm/swarms/${this.detail.id}/board?${q.toString()}`,
      );
    } catch {
      this.board = [];
    }
  }

  async postBoard(body: {
    body: string;
    kind?: string;
    project_id?: string;
    task_id?: string;
    to_agent_id?: string;
  }): Promise<void> {
    if (!this.detail) return;
    await api.post<SwarmMessage>(`/swarm/swarms/${this.detail.id}/board`, body);
    // The WS echoes it back; no optimistic insert.
  }

  // -- Graph ----------------------------------------------------------------

  async loadGraph(sid: string): Promise<void> {
    try {
      this.graph = await api.get<SwarmGraph>(`/swarm/swarms/${sid}/graph`);
    } catch {
      this.graph = null;
    }
  }

  // -- Presets --------------------------------------------------------------

  async loadPresets(): Promise<void> {
    if (this.presets.length) return;
    try {
      this.presets = await api.get<SwarmPreset[]>(`/swarm/presets`);
    } catch {
      this.presets = [];
    }
  }

  // -- Live events ----------------------------------------------------------

  applyEvent(ev: OttoEvent): boolean {
    switch (ev.type) {
      case 'swarm_status': {
        if (this.detail?.id === ev.swarm_id) {
          this.detail = { ...this.detail, status: ev.status as Swarm['status'] };
        }
        this.swarms = this.swarms.map((s) =>
          s.id === ev.swarm_id ? { ...s, status: ev.status as Swarm['status'] } : s,
        );
        return true;
      }
      case 'swarm_task_updated': {
        if (this.detail?.id !== ev.swarm_id) return true;
        const task = ev.task as unknown as SwarmTask;
        const list = this.tasksByProject[ev.project_id] ?? [];
        const idx = list.findIndex((t) => t.id === task.id);
        const next = idx >= 0 ? list.map((t) => (t.id === task.id ? task : t)) : [...list, task];
        this.tasksByProject[ev.project_id] = next;
        this.tasksByProject = { ...this.tasksByProject };
        return true;
      }
      case 'swarm_run_updated': {
        if (this.detail?.id !== ev.swarm_id) return true;
        const run = ev.run as unknown as SwarmRun;
        const idx = this.runs.findIndex((r) => r.id === run.id);
        this.runs = idx >= 0 ? this.runs.map((r) => (r.id === run.id ? run : r)) : [run, ...this.runs];
        return true;
      }
      case 'swarm_message_posted': {
        if (this.detail?.id !== ev.swarm_id) return true;
        const msg = ev.message as unknown as SwarmMessage;
        if (this.board.some((m) => m.id === msg.id)) return true;
        this.board = [msg, ...this.board];
        return true;
      }
      default:
        return false;
    }
  }

  private async refreshDetail(): Promise<void> {
    if (this.detail) {
      const sid = this.detail.id;
      this.detail = await api.get<SwarmDetail>(`/swarm/swarms/${sid}`);
    }
  }
}

export const swarm = new SwarmStore();
