import { personalAgentsStore } from './stores';
import type { EventRoutes } from './types';

export const personalAgentsRoutes: EventRoutes = {
  // Personal Agents page refreshes the agent's runs + schedule cursors.
  personal_agent_run_updated: (parsed) => personalAgentsStore.peek()?.applyRunEvent(parsed),
  // Agents / routines / delegations changed from any path (incl. an agent's
  // MCP tools): refetch what changed, so a routine set up in chat shows.
  personal_agent_changed: (parsed) => personalAgentsStore.peek()?.applyChangedEvent(parsed),
  // Agent-room feeds append the event's message (open Rooms view only).
  agent_room_message: (parsed) => personalAgentsStore.peek()?.applyRoomEvent(parsed),
};
