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
import { loadErrorText } from '../loadError';

/** Room messages per request / kept in memory on live appends. */
const ROOM_PAGE = 200;
const ROOM_KEEP = 500;

class PersonalAgentsStore {
  agents: PersonalAgent[] = $state([]);
  loadingAgents = $state(false);
  /** Why the last agents / rooms list load failed (null = it didn't). A
   *  failed load keeps what was on screen and says so inline (LoadState),
   *  instead of falling through to "No personal agents yet". */
  agentsError: string | null = $state(null);
  roomsError: string | null = $state(null);
  /** agent_id → its schedules (loaded with the list so cards can show next-run). */
  schedulesByAgent: Record<string, PersonalAgentSchedule[]> = $state({});
  /** agent_id → its recent runs (loaded on demand when the Runs tab opens). */
  runsByAgent: Record<string, PersonalAgentRun[]> = $state({});
  /** agent_id → its last runs-load failure (human text); cleared on success. A
   *  failed load keeps the last known runs, so it never reads as "no runs yet". */
  runsError: Record<string, string> = $state({});
  rooms: AgentRoomWithMembers[] = $state([]);
  /** room_id → its messages, oldest first (appended via `after` paging).
   *  Raw (replaced wholesale) and capped at ROOM_KEEP on live appends — a busy
   *  room used to load up to 5,000 messages into deep state (backlog B6 /
   *  SA-08). Older history pages in on demand via {@link loadOlder}. */
  messagesByRoom: Record<string, AgentRoomMessage[]> = $state.raw({});
  /** room_id → the server has messages older than the first one held. */
  olderByRoom: Record<string, boolean> = $state({});
  olderLoading: Record<string, boolean> = $state({});
  messagesLoading: Record<string, boolean> = $state({});
  messagesError: Record<string, string> = $state({});
  private messageRequests = new Map<string, number>();
  private wsId = '';
  /** Workspace each list was last loaded for (a late reply for another is dropped). */
  private agentsWs = '';
  private roomsWs = '';

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
    if (this.agentsWs !== workspaceId) this.agents = [];
    this.wsId = workspaceId;
    this.agentsWs = workspaceId;
    this.loadingAgents = true;
    this.agentsError = null;
    try {
      const list = await personalAgentsApi.list(workspaceId);
      if (this.agentsWs === workspaceId) this.agents = list;
    } catch (e) {
      if (this.agentsWs === workspaceId) this.agentsError = loadErrorText(e);
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
      if (agentId in this.runsError) {
        const { [agentId]: _cleared, ...rest } = this.runsError;
        this.runsError = rest;
      }
    } catch (e) {
      this.runsError = { ...this.runsError, [agentId]: loadErrorText(e) };
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
    if (this.roomsWs !== workspaceId) this.rooms = [];
    this.wsId = workspaceId;
    this.roomsWs = workspaceId;
    this.roomsError = null;
    try {
      const rooms = await personalAgentsApi.rooms(workspaceId);
      if (this.roomsWs === workspaceId) this.rooms = rooms;
    } catch (e) {
      if (this.roomsWs === workspaceId) this.roomsError = loadErrorText(e);
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

  /** Fetch messages after the cached cursor and append. A room with nothing
   *  cached opens on its TAIL (newest ROOM_PAGE); after that it pages
   *  forward (ULID ids are chronological) until a short page, bounded, and
   *  keeps only the newest ROOM_KEEP in memory. */
  async loadMessages(roomId: string): Promise<void> {
    const PAGE = ROOM_PAGE;
    const MAX_PAGES = 5;
    const request = (this.messageRequests.get(roomId) ?? 0) + 1;
    this.messageRequests.set(roomId, request);
    this.messagesLoading[roomId] = true;
    this.messagesError[roomId] = '';
    let have = this.messagesByRoom[roomId] ?? [];
    try {
      if (have.length === 0) {
        const tail = await personalAgentsApi.messagesBefore(roomId, undefined, PAGE);
        if (this.messageRequests.get(roomId) !== request) return;
        this.messagesByRoom = { ...this.messagesByRoom, [roomId]: tail };
        this.olderByRoom[roomId] = tail.length >= PAGE;
        return;
      }
      for (let i = 0; i < MAX_PAGES; i++) {
        const after = have.length > 0 ? have[have.length - 1].id : undefined;
        const page = await personalAgentsApi.messages(roomId, after, PAGE);
        if (this.messageRequests.get(roomId) !== request) return;
        if (page.length === 0) break;
        have = have.concat(page);
        if (have.length > ROOM_KEEP) {
          have = have.slice(have.length - ROOM_KEEP);
          this.olderByRoom[roomId] = true;
        }
        this.messagesByRoom = { ...this.messagesByRoom, [roomId]: have };
        if (page.length < PAGE) break;
      }
    } catch (e) {
      if (this.messageRequests.get(roomId) === request) this.messagesError[roomId] = loadErrorText(e);
    } finally {
      if (this.messageRequests.get(roomId) === request) this.messagesLoading[roomId] = false;
    }
  }

  /** Page one batch of OLDER history in front of what is held ("Load older"). */
  async loadOlder(roomId: string): Promise<void> {
    const have = this.messagesByRoom[roomId] ?? [];
    if (this.olderLoading[roomId] || have.length === 0) return;
    this.olderLoading[roomId] = true;
    try {
      const page = await personalAgentsApi.messagesBefore(roomId, have[0].id, ROOM_PAGE);
      const cur = this.messagesByRoom[roomId] ?? [];
      // A reset/delete in between: don't resurrect the room.
      if (cur.length === 0 || cur[0].id !== have[0].id) return;
      this.messagesByRoom = { ...this.messagesByRoom, [roomId]: page.concat(cur) };
      this.olderByRoom[roomId] = page.length >= ROOM_PAGE;
    } catch (e) {
      this.messagesError[roomId] = loadErrorText(e);
    } finally {
      this.olderLoading[roomId] = false;
    }
  }

  async postMessage(roomId: string, text: string): Promise<void> {
    await personalAgentsApi.postMessage(roomId, text);
    // The WS broadcast also lands here; loadMessages appends after the cursor
    // so the double refresh is idempotent.
    await this.loadMessages(roomId);
  }

  // -- Live events ----------------------------------------------------------

  /** Mounted Personal Agents pages. Run events refetch only while one is. */
  private viewers = 0;

  /** Register a mounted page; call the returned fn on unmount. */
  watch(): () => void {
    this.viewers += 1;
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.viewers = Math.max(0, this.viewers - 1);
    };
  }

  /** Live WS tick: refresh the affected agent's runs + schedules (cursors
   *  moved) — only what is cached AND only while a page shows it (SI-10). A
   *  15-min check agent used to refetch 100 runs + schedules per run event for
   *  the app's lifetime. Off-page, both reload when the page mounts (the list
   *  load refreshes schedules; the Runs tab reloads runs). */
  applyRunEvent(ev: Extract<OttoEvent, { type: 'personal_agent_run_updated' }>): void {
    if (this.wsId && ev.workspace_id !== this.wsId) return;
    if (this.viewers === 0) return;
    if (ev.agent_id in this.runsByAgent) void this.loadRuns(ev.agent_id);
    if (ev.agent_id in this.schedulesByAgent) void this.loadSchedules(ev.agent_id);
  }

  /** Live WS tick: fetch the room's messages after our cursor. */
  applyRoomEvent(ev: Extract<OttoEvent, { type: 'agent_room_message' }>): void {
    if (this.wsId && ev.workspace_id !== this.wsId) return;
    void this.loadMessages(ev.room_id);
  }
}

export const personalAgents = new PersonalAgentsStore();
