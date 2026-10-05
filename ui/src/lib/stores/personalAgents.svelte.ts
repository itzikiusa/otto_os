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
import { announceModule } from '../lazyModule';

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
  private scheduleRequests = new Map<string, number>();
  private runRequests = new Map<string, number>();
  /** Rooms whose held feed may have a hole (a WS gap, or events skipped while
   *  the Rooms page was closed): the next {@link loadMessages} re-reads the
   *  TAIL and replaces, instead of paging forward from a stale cursor. */
  private tailReload = new Set<string>();
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
    const request = (this.scheduleRequests.get(agentId) ?? 0) + 1;
    this.scheduleRequests.set(agentId, request);
    try {
      const schedules = await personalAgentsApi.schedules(agentId);
      if (this.scheduleRequests.get(agentId) !== request) return;
      this.schedulesByAgent = { ...this.schedulesByAgent, [agentId]: schedules };
    } catch {
      if (this.scheduleRequests.get(agentId) !== request) return;
      this.schedulesByAgent = { ...this.schedulesByAgent, [agentId]: [] };
    }
  }

  async loadRuns(agentId: string): Promise<void> {
    const request = (this.runRequests.get(agentId) ?? 0) + 1;
    this.runRequests.set(agentId, request);
    try {
      const runs = await personalAgentsApi.runs(agentId);
      if (this.runRequests.get(agentId) !== request) return;
      this.runsByAgent = { ...this.runsByAgent, [agentId]: runs };
      if (agentId in this.runsError) {
        const { [agentId]: _cleared, ...rest } = this.runsError;
        this.runsError = rest;
      }
    } catch (e) {
      if (this.runRequests.get(agentId) !== request) return;
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
      if (have.length === 0 || this.tailReload.has(roomId)) {
        const tail = await personalAgentsApi.messagesBefore(roomId, undefined, PAGE);
        if (this.messageRequests.get(roomId) !== request) return;
        this.tailReload.delete(roomId);
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
    const msg = await personalAgentsApi.postMessage(roomId, text);
    // Append the stored message the POST returned — no refetch. The WS
    // broadcast of the same message dedupes by id in appendMessage. A reply
    // that isn't a message of this room falls back to the cursor fetch.
    if (msg?.room_id === roomId && typeof msg.text === 'string') this.appendMessage(msg);
    else await this.loadMessages(roomId);
  }

  /** Append one live message to a held feed (dedup by id, capped at
   *  ROOM_KEEP). A room with a fetch in flight or a suspected hole re-reads
   *  instead, so a live append never lands past a gap. */
  private appendMessage(msg: AgentRoomMessage): void {
    const roomId = msg.room_id;
    const have = this.messagesByRoom[roomId];
    if (!have) return;
    if (this.messagesLoading[roomId] || this.tailReload.has(roomId)) {
      void this.loadMessages(roomId);
      return;
    }
    if (have.some((m) => m.id === msg.id)) return;
    let next = have.concat(msg);
    if (next.length > ROOM_KEEP) {
      next = next.slice(next.length - ROOM_KEEP);
      this.olderByRoom[roomId] = true;
    }
    this.messagesByRoom = { ...this.messagesByRoom, [roomId]: next };
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

  /** Mounted Rooms views. Room events touch message feeds only while one is. */
  private roomViewers = 0;
  /** The room the Rooms view shows (kept across unmount; others are evicted). */
  private activeRoom: string | null = null;

  /** Register a mounted Rooms view; call the returned fn on unmount. The last
   *  view leaving evicts every held feed but the selected room's, and marks
   *  that one for a tail re-read (events stop applying while it's closed). */
  watchRooms(): () => void {
    this.roomViewers += 1;
    let released = false;
    return () => {
      if (released) return;
      released = true;
      this.roomViewers = Math.max(0, this.roomViewers - 1);
      if (this.roomViewers > 0) return;
      const keep = this.activeRoom;
      const kept: Record<string, AgentRoomMessage[]> = {};
      if (keep && this.messagesByRoom[keep]) {
        kept[keep] = this.messagesByRoom[keep];
        this.tailReload.add(keep);
      }
      this.messagesByRoom = kept;
    };
  }

  /** The Rooms view's selection (see {@link watchRooms}). */
  setActiveRoom(roomId: string | null): void {
    this.activeRoom = roomId;
  }

  /** Events were missed (WS reconnect / lag): held feeds may have holes. The
   *  shown room re-reads its tail now; the rest on their next open. */
  resyncRooms(): void {
    for (const id of Object.keys(this.messagesByRoom)) this.tailReload.add(id);
    if (this.roomViewers > 0 && this.activeRoom && this.activeRoom in this.messagesByRoom) {
      void this.loadMessages(this.activeRoom);
    }
  }

  /** Live WS tick. The event carries the whole message: an open Rooms view
   *  appends it to a held feed with no GET (R3); with no Rooms view mounted,
   *  or for a room never opened, only the list's activity line moves (R2). */
  applyRoomEvent(ev: Extract<OttoEvent, { type: 'agent_room_message' }>): void {
    if (this.wsId && ev.workspace_id !== this.wsId) return;
    // Keep the rooms list's activity line current without a list reload
    // (one event per persisted message, so the count stays exact).
    const r = this.rooms.find((x) => x.room.id === ev.room_id);
    if (r) {
      r.message_count = (r.message_count ?? 0) + 1;
      r.last_message_at = ev.created_at ?? new Date().toISOString();
    }
    if (this.roomViewers === 0 || !(ev.room_id in this.messagesByRoom)) return;
    // An older daemon sends ids only — fall back to the cursor fetch.
    if (typeof ev.text !== 'string' || !ev.created_at) {
      void this.loadMessages(ev.room_id);
      return;
    }
    this.appendMessage({
      id: ev.message_id,
      room_id: ev.room_id,
      author_kind: ev.author_kind === 'agent' ? 'agent' : 'user',
      author_id: ev.author_id,
      text: ev.text,
      created_at: ev.created_at,
    });
  }
}

export const personalAgents = new PersonalAgentsStore();
// Routed by `peek()` in lib/events.svelte.ts (perf G2): let it see this store
// however it was first imported.
announceModule('personalAgents', personalAgents);
