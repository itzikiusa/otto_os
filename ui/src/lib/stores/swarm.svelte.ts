// Agent Swarm store: swarms, the open swarm's detail (agents + projects), tasks
// per project, runs, and the shared board. Fed by REST loads and the events WS
// (swarm_run_updated / swarm_task_updated / swarm_message_posted / swarm_status).

import { api } from '../api/client';
import type { OttoEvent } from '../api/types';
import type {
  CreateAgentReq,
  CreateGoalReq,
  CreateTriggerReq,
  LibrarySkillMeta,
  RecruitedAgent,
  RunFilters,
  Swarm,
  SwarmAgent,
  SwarmChannelTrigger,
  SwarmDetail,
  SwarmGoal,
  SwarmGraph,
  SwarmMessage,
  SwarmPreset,
  SwarmProject,
  SwarmRun,
  SwarmTask,
  TaskVerification,
  UpdateAgentReq,
  UpdateGoalReq,
  UpdateTriggerReq,
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

  // -- Goals / verification / triggers / library skills --------------------
  /** Goals keyed by task id (per-task explicit + applied standing goals). */
  goalsByTask: Record<string, SwarmGoal[]> = $state({});
  /** Goals keyed by project id. */
  goalsByProject: Record<string, SwarmGoal[]> = $state({});
  /** The open swarm's standing goals (its quality bar, applied to every task). */
  standingGoals: SwarmGoal[] = $state([]);
  /** Live verification state per task id (running + last task_status). */
  verifyByTask: Record<string, { running: boolean; task_status: string }> = $state({});
  /** The open swarm's channel triggers. */
  triggers: SwarmChannelTrigger[] = $state([]);
  /** Cached library skills for the skill pickers (loaded once). */
  librarySkills: LibrarySkillMeta[] = $state([]);
  /** Set when a deep-link (e.g. Product → Swarm) wants the Kanban view shown
   *  for the just-opened project. SwarmPage reads + clears this on mount. */
  pendingKanban = $state(false);

  private wsId: string | null = null;
  private graphDebounce: ReturnType<typeof setTimeout> | null = null;

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
    this.goalsByTask = {};
    this.goalsByProject = {};
    this.standingGoals = [];
    this.verifyByTask = {};
    this.triggers = [];
  }

  goalsForTask(tid: string | null | undefined): SwarmGoal[] {
    return tid ? (this.goalsByTask[tid] ?? []) : [];
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

  async recruit(
    role: string,
    context?: string,
    namingTheme?: string,
    signal?: AbortSignal,
  ): Promise<RecruitedAgent> {
    if (!this.wsId) throw new Error('no workspace');
    return api.post<RecruitedAgent>(
      `/workspaces/${this.wsId}/swarm/recruit`,
      {
        swarm_id: this.detail?.id ?? null,
        role,
        context: context ?? null,
        naming_theme: namingTheme || null,
      },
      signal,
    );
  }

  /** Recruit run ids already hired from — hides their Runs-list "Hire" button. */
  recruitHired = $state<Set<string>>(new Set());
  markRecruitHired(runId: string): void {
    const n = new Set(this.recruitHired);
    n.add(runId);
    this.recruitHired = n;
  }

  /** Persist the swarm's recruit naming theme (remembered across recruits). */
  async setNamingTheme(sid: string, theme: string): Promise<void> {
    const cfg = { ...(this.detail?.config ?? {}), naming_theme: theme };
    await this.updateSwarm(sid, { config: cfg } as Partial<Swarm>);
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

  /** Stop an in-flight plan/recruit for `sid`: kills the live agent session(s)
   *  server-side and prevents retries. Best-effort. */
  async stopAgentRun(sid: string): Promise<void> {
    if (!this.wsId) return;
    try {
      await api.post(`/workspaces/${this.wsId}/swarm/swarms/${sid}/agent-stop`);
    } catch {
      /* best-effort */
    }
  }

  async plan(pid: string, signal?: AbortSignal): Promise<void> {
    if (!this.wsId) return;
    const tasks = await api.post<SwarmTask[]>(
      `/workspaces/${this.wsId}/swarm/projects/${pid}/plan`,
      {},
      signal,
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
  ): Promise<SwarmTask> {
    const created = await api.post<SwarmTask>(`/swarm/projects/${pid}/tasks`, body);
    await this.loadTasks(pid);
    await this.refreshGraph();
    return created;
  }

  async updateTask(task: SwarmTask, patch: Partial<SwarmTask>): Promise<void> {
    await api.patch<SwarmTask>(`/swarm/tasks/${task.id}`, patch);
    await this.loadTasks(task.project_id);
    await this.refreshGraph();
  }

  async deleteTask(task: SwarmTask): Promise<void> {
    await api.del(`/swarm/tasks/${task.id}`);
    await this.loadTasks(task.project_id);
    await this.refreshGraph();
  }

  /** Apply the same patch to many tasks (bulk move/assign), reloading once. */
  async bulkUpdateTasks(tasks: SwarmTask[], patch: Partial<SwarmTask>): Promise<void> {
    if (!tasks.length) return;
    await Promise.all(
      tasks.map((t) => api.patch<SwarmTask>(`/swarm/tasks/${t.id}`, patch).catch(() => null)),
    );
    await this.loadTasks(tasks[0].project_id);
    await this.refreshGraph();
  }

  /** Delete many tasks (bulk delete / clear board), reloading once. */
  async bulkDeleteTasks(tasks: SwarmTask[]): Promise<void> {
    if (!tasks.length) return;
    await Promise.all(tasks.map((t) => api.del(`/swarm/tasks/${t.id}`).catch(() => null)));
    await this.loadTasks(tasks[0].project_id);
    await this.refreshGraph();
  }

  /** Reload the dependency graph for the open swarm (the Graph tab reads it, so
   *  task mutations must refresh it or deleted/moved tasks linger in the graph). */
  private async refreshGraph(): Promise<void> {
    if (this.detail) await this.loadGraph(this.detail.id);
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

  // -- Goals ----------------------------------------------------------------

  async loadTaskGoals(tid: string): Promise<void> {
    try {
      this.goalsByTask[tid] = await api.get<SwarmGoal[]>(`/swarm/tasks/${tid}/goals`);
      this.goalsByTask = { ...this.goalsByTask };
    } catch {
      /* best-effort */
    }
  }

  async loadProjectGoals(pid: string): Promise<void> {
    try {
      this.goalsByProject[pid] = await api.get<SwarmGoal[]>(`/swarm/projects/${pid}/goals`);
      this.goalsByProject = { ...this.goalsByProject };
    } catch {
      /* best-effort */
    }
  }

  /** Create a goal on a task OR a project (exactly one scope must be set). */
  async createGoal(
    scope: { task?: string; project?: string },
    req: CreateGoalReq,
  ): Promise<SwarmGoal> {
    if (scope.task) {
      const g = await api.post<SwarmGoal>(`/swarm/tasks/${scope.task}/goals`, req);
      await this.loadTaskGoals(scope.task);
      return g;
    }
    if (scope.project) {
      const g = await api.post<SwarmGoal>(`/swarm/projects/${scope.project}/goals`, req);
      await this.loadProjectGoals(scope.project);
      return g;
    }
    throw new Error('createGoal needs a task or project scope');
  }

  async updateGoal(gid: string, patch: UpdateGoalReq): Promise<SwarmGoal> {
    const updated = await api.patch<SwarmGoal>(`/swarm/goals/${gid}`, patch);
    this.mergeGoal(updated);
    return updated;
  }

  async deleteGoal(gid: string): Promise<void> {
    await api.del(`/swarm/goals/${gid}`);
    // Drop it from whichever scope held it.
    for (const tid of Object.keys(this.goalsByTask)) {
      this.goalsByTask[tid] = this.goalsByTask[tid].filter((g) => g.id !== gid);
    }
    for (const pid of Object.keys(this.goalsByProject)) {
      this.goalsByProject[pid] = this.goalsByProject[pid].filter((g) => g.id !== gid);
    }
    this.standingGoals = this.standingGoals.filter((g) => g.id !== gid);
    this.goalsByTask = { ...this.goalsByTask };
    this.goalsByProject = { ...this.goalsByProject };
  }

  /** Splice an updated goal into every cache it appears in (or its task scope). */
  private mergeGoal(g: SwarmGoal): void {
    if (g.task_id) {
      const list = this.goalsByTask[g.task_id] ?? [];
      const idx = list.findIndex((x) => x.id === g.id);
      this.goalsByTask[g.task_id] = idx >= 0 ? list.map((x) => (x.id === g.id ? g : x)) : [...list, g];
      this.goalsByTask = { ...this.goalsByTask };
    }
    if (g.project_id) {
      const list = this.goalsByProject[g.project_id] ?? [];
      const idx = list.findIndex((x) => x.id === g.id);
      this.goalsByProject[g.project_id] =
        idx >= 0 ? list.map((x) => (x.id === g.id ? g : x)) : [...list, g];
      this.goalsByProject = { ...this.goalsByProject };
    }
    if (g.kind === 'standing' && !g.task_id && !g.project_id) {
      const idx = this.standingGoals.findIndex((x) => x.id === g.id);
      this.standingGoals =
        idx >= 0 ? this.standingGoals.map((x) => (x.id === g.id ? g : x)) : [...this.standingGoals, g];
    }
  }

  // -- Standing goals (swarm-level templates) --------------------------------

  async loadStandingGoals(sid: string): Promise<void> {
    try {
      this.standingGoals = await api.get<SwarmGoal[]>(`/swarm/swarms/${sid}/standing-goals`);
    } catch {
      this.standingGoals = [];
    }
  }

  async putStandingGoals(sid: string, goals: CreateGoalReq[]): Promise<void> {
    this.standingGoals = await api.put<SwarmGoal[]>(`/swarm/swarms/${sid}/standing-goals`, {
      goals,
    });
  }

  // -- Verification ----------------------------------------------------------

  async verifyTask(tid: string): Promise<{ started: boolean; reason?: string }> {
    const res = await api.post<{ started: boolean; reason?: string }>(
      `/swarm/tasks/${tid}/verify`,
    );
    if (res.started) {
      this.verifyByTask[tid] = { running: true, task_status: 'verifying' };
      this.verifyByTask = { ...this.verifyByTask };
    }
    return res;
  }

  async stopVerify(tid: string): Promise<{ stopped: boolean }> {
    const res = await api.post<{ stopped: boolean }>(`/swarm/tasks/${tid}/verify/stop`);
    const cur = this.verifyByTask[tid];
    if (cur) {
      this.verifyByTask[tid] = { ...cur, running: false };
      this.verifyByTask = { ...this.verifyByTask };
    }
    return res;
  }

  async loadVerification(tid: string): Promise<void> {
    try {
      const v = await api.get<TaskVerification>(`/swarm/tasks/${tid}/verification`);
      this.verifyByTask[tid] = { running: v.running, task_status: v.task_status };
      this.verifyByTask = { ...this.verifyByTask };
      this.goalsByTask[tid] = v.goals;
      this.goalsByTask = { ...this.goalsByTask };
    } catch {
      /* best-effort */
    }
  }

  // -- Channel triggers ------------------------------------------------------

  async loadTriggers(sid: string): Promise<void> {
    try {
      this.triggers = await api.get<SwarmChannelTrigger[]>(`/swarm/swarms/${sid}/triggers`);
    } catch {
      this.triggers = [];
    }
  }

  async createTrigger(sid: string, req: CreateTriggerReq): Promise<SwarmChannelTrigger> {
    const t = await api.post<SwarmChannelTrigger>(`/swarm/swarms/${sid}/triggers`, req);
    this.triggers = [...this.triggers, t];
    return t;
  }

  async updateTrigger(id: string, patch: UpdateTriggerReq): Promise<SwarmChannelTrigger> {
    const updated = await api.patch<SwarmChannelTrigger>(`/swarm/triggers/${id}`, patch);
    this.triggers = this.triggers.map((t) => (t.id === id ? updated : t));
    return updated;
  }

  async deleteTrigger(id: string): Promise<void> {
    await api.del(`/swarm/triggers/${id}`);
    this.triggers = this.triggers.filter((t) => t.id !== id);
  }

  // -- Library skills (for the skill pickers) --------------------------------

  async loadLibrarySkills(): Promise<void> {
    if (this.librarySkills.length) return;
    try {
      this.librarySkills = await api.get<LibrarySkillMeta[]>('/library/skills');
    } catch {
      this.librarySkills = [];
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
        this.scheduleGraphRefresh();
        return true;
      }
      case 'swarm_run_updated': {
        if (this.detail?.id !== ev.swarm_id) return true;
        const run = ev.run as unknown as SwarmRun;
        const idx = this.runs.findIndex((r) => r.id === run.id);
        if (idx < 0 && this.detail) {
          // Keep the header's run-budget meter live: counts come from the detail
          // load (not run events), so without this "N/300" looks frozen.
          this.detail = {
            ...this.detail,
            counts: { ...this.detail.counts, total_runs: (this.detail.counts.total_runs ?? 0) + 1 },
          };
        }
        this.runs = idx >= 0 ? this.runs.map((r) => (r.id === run.id ? run : r)) : [run, ...this.runs];
        this.scheduleGraphRefresh();
        return true;
      }
      case 'swarm_message_posted': {
        if (this.detail?.id !== ev.swarm_id) return true;
        const msg = ev.message as unknown as SwarmMessage;
        if (this.board.some((m) => m.id === msg.id)) return true;
        this.board = [msg, ...this.board];
        return true;
      }
      case 'swarm_goal_updated': {
        if (this.detail?.id !== ev.swarm_id) return true;
        const goal = ev.goal as unknown as SwarmGoal;
        this.mergeGoal(goal);
        // Keep the per-task verification banner in sync with the goal's status.
        const tid = ev.task_id ?? goal.task_id ?? null;
        if (tid) {
          const cur = this.verifyByTask[tid];
          this.verifyByTask[tid] = {
            running: goal.status === 'verifying',
            task_status: cur?.task_status ?? goal.status,
          };
          this.verifyByTask = { ...this.verifyByTask };
        }
        return true;
      }
      default:
        return false;
    }
  }

  private scheduleGraphRefresh(): void {
    if (!this.detail) return;
    if (this.graphDebounce) clearTimeout(this.graphDebounce);
    const sid = this.detail.id;
    this.graphDebounce = setTimeout(() => {
      this.graphDebounce = null;
      void this.loadGraph(sid);
    }, 800);
  }

  private async refreshDetail(): Promise<void> {
    if (this.detail) {
      const sid = this.detail.id;
      this.detail = await api.get<SwarmDetail>(`/swarm/swarms/${sid}`);
    }
  }
}

export const swarm = new SwarmStore();
