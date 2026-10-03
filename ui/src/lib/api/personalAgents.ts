// Personal Agents API client — thin typed wrappers over the generic `api`
// helper. Mirrors docs/contracts/api.md (## Personal Agents + ### Agent rooms).

import { api } from './client';
import type {
  AgentRoom,
  AgentRoomMessage,
  AgentRoomWithMembers,
  PersonalAgent,
  PersonalAgentActivity,
  PersonalAgentAutonomy,
  PersonalAgentDocument,
  PersonalAgentMemories,
  PersonalAgentPermission,
  PersonalAgentRun,
  PersonalAgentSchedule,
  SavePersonalAgentAutonomyReq,
} from './types';

export interface PersonalAgentInput {
  name?: string;
  avatar?: string;
  soul_md?: string;
  provider?: string;
  model?: string;
  cwd?: string;
  browser?: boolean;
  delivery?: Record<string, unknown>;
  enabled?: boolean;
}

export interface AgentScheduleInput {
  schedule?: Record<string, unknown>;
  timezone?: string;
  directive?: string;
  enabled?: boolean;
  /** The schedule's own permission set (default `directed`). */
  permission?: PersonalAgentPermission;
}

export const personalAgentsApi = {
  list: (ws: string) => api.get<PersonalAgent[]>(`/workspaces/${ws}/personal-agents`),
  create: (ws: string, body: PersonalAgentInput & { name: string }) =>
    api.post<PersonalAgent>(`/workspaces/${ws}/personal-agents`, body),
  get: (id: string) => api.get<PersonalAgent>(`/personal-agents/${id}`),
  update: (id: string, body: PersonalAgentInput) =>
    api.patch<PersonalAgent>(`/personal-agents/${id}`, body),
  remove: (id: string) => api.del<{ ok: boolean }>(`/personal-agents/${id}`),

  schedules: (agentId: string) =>
    api.get<PersonalAgentSchedule[]>(`/personal-agents/${agentId}/schedules`),
  createSchedule: (agentId: string, body: AgentScheduleInput & { schedule: Record<string, unknown> }) =>
    api.post<PersonalAgentSchedule>(`/personal-agents/${agentId}/schedules`, body),
  updateSchedule: (scheduleId: string, body: AgentScheduleInput) =>
    api.patch<PersonalAgentSchedule>(`/personal-agents/schedules/${scheduleId}`, body),
  deleteSchedule: (scheduleId: string) =>
    api.del<{ ok: boolean }>(`/personal-agents/schedules/${scheduleId}`),

  /** Manual fire — never moves a schedule cursor. `scheduleId` picks a directive. */
  run: (agentId: string, scheduleId?: string) =>
    api.post<PersonalAgentRun>(
      `/personal-agents/${agentId}/run`,
      scheduleId ? { schedule_id: scheduleId } : {},
    ),
  runs: (agentId: string) => api.get<PersonalAgentRun[]>(`/personal-agents/${agentId}/runs`),
  /** Stop a running run (its session is killed; it settles `canceled`). */
  cancelRun: (runId: string) => api.post<{ ok: boolean }>(`/personal-agents/runs/${runId}/cancel`, {}),
  /** The stored report path for a run (fetched as text/markdown via authedText). */
  reportPath: (runId: string) => `/personal-agents/runs/${runId}/report`,

  /** Return (creating if absent or dead) the agent's single chat session. */
  chatSession: (agentId: string) =>
    api.post<{ session_id: string; created: boolean }>(`/personal-agents/${agentId}/chat-session`, {}),

  document: (agentId: string, kind: 'memory' | 'context') =>
    api.get<PersonalAgentDocument>(`/personal-agents/${agentId}/${kind}`),
  saveDocument: (agentId: string, kind: 'memory' | 'context', body: { content: string; version: string }) =>
    api.put<PersonalAgentDocument>(`/personal-agents/${agentId}/${kind}`, body),

  // -- Autonomy (modes, goals, rules, primary), activity, memory inspector --
  autonomy: (agentId: string) =>
    api.get<PersonalAgentAutonomy>(`/personal-agents/${agentId}/autonomy`),
  saveAutonomy: (agentId: string, body: SavePersonalAgentAutonomyReq) =>
    api.put<PersonalAgentAutonomy>(`/personal-agents/${agentId}/autonomy`, body),
  /** Work a standing goal now (a proactive, read-only run). */
  runGoal: (agentId: string, goalId: string) =>
    api.post<PersonalAgentRun>(`/personal-agents/${agentId}/goals/${goalId}/run`, {}),
  activity: (agentId: string) =>
    api.get<PersonalAgentActivity>(`/personal-agents/${agentId}/activity`),
  memories: (agentId: string) =>
    api.get<PersonalAgentMemories>(`/personal-agents/${agentId}/memories`),
  /** Edit (`text`) or forget (`text: null`) one memory item. */
  editMemory: (
    agentId: string,
    body: { version: string; line: number; raw: string; text: string | null },
  ) => api.post<PersonalAgentMemories>(`/personal-agents/${agentId}/memories/edit`, body),
  /** Forget everything the agent learned and did (`confirm` = the agent's name). */
  reset: (agentId: string, confirm: string) =>
    api.post<{ ok: boolean }>(`/personal-agents/${agentId}/reset`, { confirm }),

  // -- Rooms ---------------------------------------------------------------
  rooms: (ws: string) => api.get<AgentRoomWithMembers[]>(`/workspaces/${ws}/agent-rooms`),
  createRoom: (ws: string, name: string) =>
    api.post<AgentRoom>(`/workspaces/${ws}/agent-rooms`, { name }),
  getRoom: (id: string) => api.get<Pick<AgentRoomWithMembers, 'room' | 'members'>>(`/agent-rooms/${id}`),
  renameRoom: (id: string, name: string) => api.patch<AgentRoom>(`/agent-rooms/${id}`, { name }),
  deleteRoom: (id: string) => api.del<{ ok: boolean }>(`/agent-rooms/${id}`),
  addMember: (roomId: string, agentId: string) =>
    api.post<{ ok: boolean }>(`/agent-rooms/${roomId}/members`, { agent_id: agentId }),
  removeMember: (roomId: string, agentId: string) =>
    api.del<{ ok: boolean }>(`/agent-rooms/${roomId}/members/${agentId}`),
  /** Chronological page: messages with `id > after`, oldest first (ULID ids). */
  messages: (roomId: string, after?: string, limit = 200) => {
    const q = new URLSearchParams();
    if (after) q.set('after', after);
    q.set('limit', String(limit));
    return api.get<AgentRoomMessage[]>(`/agent-rooms/${roomId}/messages?${q}`);
  },
  /** Backwards page, oldest first: the `limit` messages before `before`, or
   *  the room's newest `limit` when `before` is omitted (`tail=true`). */
  messagesBefore: (roomId: string, before?: string, limit = 200) => {
    const q = new URLSearchParams();
    if (before) q.set('before', before);
    else q.set('tail', 'true');
    q.set('limit', String(limit));
    return api.get<AgentRoomMessage[]>(`/agent-rooms/${roomId}/messages?${q}`);
  },
  /** A user post (no session_id — agent posts go through the room MCP tools). */
  postMessage: (roomId: string, text: string) =>
    api.post<AgentRoomMessage>(`/agent-rooms/${roomId}/messages`, { text }),
};
