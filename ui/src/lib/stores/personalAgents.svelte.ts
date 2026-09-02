// Personal Agents store: agents + per-agent schedules/runs, agent rooms with
// live message feeds, REST loaders, and live-event application. Like the
// scheduledTasks/loops stores it does NOT import events.svelte.ts — the event
// dispatcher calls `personalAgents.apply*Event(...)` on this singleton.

import { personalAgentsApi, type AgentScheduleInput, type PersonalAgentInput } from '../api/personalAgents';
import type {
  AgentRoomMessage,
  AgentRoomWithMembers,
  OttoEvent,
  PersonalAgent,
  PersonalAgentRun,
  PersonalAgentSchedule,
} from '../api/types';

class PersonalAgentsStore {
  agents: PersonalAgent[] = $state([]);
  loadingAgents = $state(false);
  /** agent_id → its schedules (loaded with the list so cards can show next-run). */
  schedulesByAgent: Record<string, PersonalAgentSchedule[]> = $state({});
  /** agent_id → its recent runs (loaded on demand when the Runs tab opens). */
  runsByAgent: Record<string, PersonalAgentRun[]> = $state({});
  rooms: AgentRoomWithMembers[] = $state([]);
  /** room_id → its messages, oldest first (appended via `after` paging). */
  messagesByRoom: Record<string, AgentRoomMessage[]> = $state({});
  private wsId = '';

  agent(id: string): PersonalAgent | undefined {
    return this.agents.find((a) => a.id === id);
  }

  /** Earliest next_run_at across an agent's enabled schedules (card display). */
  nextRunAt(agentId: string): string | null {
    const times = (this.schedulesByAgent[agentId] ?? [])
      .filter((s) => s.enabled && s.next_run_at)
      .map((s) => s.next_run_at as string)
      .sort();
    return times[0] ?? null;
  }

  async loadAgents(workspaceId: string): Promise<void> {
    this.wsId = workspaceId;
    this.loadingAgents = true;
    try {
      this.agents = await personalAgentsApi.list(workspaceId);
    } catch {
      this.agents = [];
    } finally {
      this.loadingAgents = false;
    }
    // Schedules feed the cards' next-run + the Schedules tab; best-effort.
    await Promise.all(this.agents.map((a) => this.loadSchedules(a.id)));
  }

  async loadSchedules(agentId: string): Promise<void> {
    try {
      this.schedulesByAgent = {
        ...this.schedulesByAgent,
        [agentId]: await personalAgentsApi.schedules(agentId),
      };
    } catch {
      this.schedulesByAgent = { ...this.schedulesByAgent, [agentId]: [] };
    }
  }

  async loadRuns(agentId: string): Promise<void> {
    try {
      this.runsByAgent = { ...this.runsByAgent, [agentId]: await personalAgentsApi.runs(agentId) };
    } catch {
      this.runsByAgent = { ...this.runsByAgent, [agentId]: [] };
    }
  }

  async create(workspaceId: string, body: PersonalAgentInput & { name: string }): Promise<PersonalAgent> {
    const a = await personalAgentsApi.create(workspaceId, body);
    await this.loadAgents(workspaceId);
    return a;
  }

  async update(id: string, body: PersonalAgentInput): Promise<PersonalAgent> {
    const updated = await personalAgentsApi.update(id, body);
    this.agents = this.agents.map((a) => (a.id === id ? updated : a));
    return updated;
  }

  async setEnabled(id: string, enabled: boolean): Promise<void> {
    await this.update(id, { enabled });
  }

  async remove(id: string): Promise<void> {
    await personalAgentsApi.remove(id);
    this.agents = this.agents.filter((a) => a.id !== id);
    const sch = { ...this.schedulesByAgent };
    delete sch[id];
    this.schedulesByAgent = sch;
    const runs = { ...this.runsByAgent };
    delete runs[id];
    this.runsByAgent = runs;
  }

  async runNow(agentId: string, scheduleId?: string): Promise<void> {
    await personalAgentsApi.run(agentId, scheduleId);
    await this.loadRuns(agentId);
  }

  async createSchedule(
    agentId: string,
    body: AgentScheduleInput & { schedule: Record<string, unknown> },
  ): Promise<void> {
    await personalAgentsApi.createSchedule(agentId, body);
    await this.loadSchedules(agentId);
  }

  async updateSchedule(agentId: string, scheduleId: string, body: AgentScheduleInput): Promise<void> {
    await personalAgentsApi.updateSchedule(scheduleId, body);
    await this.loadSchedules(agentId);
  }

  async deleteSchedule(agentId: string, scheduleId: string): Promise<void> {
    await personalAgentsApi.deleteSchedule(scheduleId);
    await this.loadSchedules(agentId);
  }

  // -- Rooms ----------------------------------------------------------------

  async loadRooms(workspaceId: string): Promise<void> {
    this.wsId = workspaceId;
    try {
      this.rooms = await personalAgentsApi.rooms(workspaceId);
    } catch {
      this.rooms = [];
    }
  }

  async createRoom(workspaceId: string, name: string): Promise<string> {
    const room = await personalAgentsApi.createRoom(workspaceId, name);
    await this.loadRooms(workspaceId);
    return room.id;
  }

  async renameRoom(roomId: string, name: string): Promise<void> {
    await personalAgentsApi.renameRoom(roomId, name);
    if (this.wsId) await this.loadRooms(this.wsId);
  }

  async deleteRoom(roomId: string): Promise<void> {
    await personalAgentsApi.deleteRoom(roomId);
    this.rooms = this.rooms.filter((r) => r.room.id !== roomId);
    const msgs = { ...this.messagesByRoom };
    delete msgs[roomId];
    this.messagesByRoom = msgs;
  }

  async addMember(roomId: string, agentId: string): Promise<void> {
    await personalAgentsApi.addMember(roomId, agentId);
    if (this.wsId) await this.loadRooms(this.wsId);
  }

  async removeMember(roomId: string, agentId: string): Promise<void> {
    await personalAgentsApi.removeMember(roomId, agentId);
    if (this.wsId) await this.loadRooms(this.wsId);
  }

  /** Fetch messages after the cached cursor and append. Pages forward (ULID
   *  ids are chronological) until a short page, bounded so a huge backlog
   *  can't wedge the tab. */
  async loadMessages(roomId: string): Promise<void> {
    const PAGE = 200;
    const MAX_PAGES = 25;
    let have = this.messagesByRoom[roomId] ?? [];
    for (let i = 0; i < MAX_PAGES; i++) {
      const after = have.length > 0 ? have[have.length - 1].id : undefined;
      let page: AgentRoomMessage[];
      try {
        page = await personalAgentsApi.messages(roomId, after, PAGE);
      } catch {
        break;
      }
      if (page.length > 0) {
        have = [...have, ...page];
        this.messagesByRoom = { ...this.messagesByRoom, [roomId]: have };
      } else if (!(roomId in this.messagesByRoom)) {
        this.messagesByRoom = { ...this.messagesByRoom, [roomId]: [] };
      }
      if (page.length < PAGE) break;
    }
  }

  async postMessage(roomId: string, text: string): Promise<void> {
    await personalAgentsApi.postMessage(roomId, text);
    // The WS broadcast also lands here; loadMessages appends after the cursor
    // so the double refresh is idempotent.
    await this.loadMessages(roomId);
  }

  // -- Live events ----------------------------------------------------------

  /** Live WS tick: refresh the affected agent's runs + schedules (cursors moved). */
  applyRunEvent(ev: Extract<OttoEvent, { type: 'personal_agent_run_updated' }>): void {
    if (this.wsId && ev.workspace_id !== this.wsId) return;
    void this.loadRuns(ev.agent_id);
    void this.loadSchedules(ev.agent_id);
  }

  /** Live WS tick: fetch the room's messages after our cursor. */
  applyRoomEvent(ev: Extract<OttoEvent, { type: 'agent_room_message' }>): void {
    if (this.wsId && ev.workspace_id !== this.wsId) return;
    void this.loadMessages(ev.room_id);
  }
}

export const personalAgents = new PersonalAgentsStore();
