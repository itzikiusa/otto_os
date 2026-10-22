import { personalAgentsStore } from './stores';
import type { EventRoutes } from './types';

export const personalAgentsRoutes: EventRoutes = {
  // Personal Agents page refreshes the agent's runs + schedule cursors.
  personal_agent_run_updated: (parsed) => personalAgentsStore.peek()?.applyRunEvent(parsed),
  // Agent-room feeds append the event's message (open Rooms view only).
  agent_room_message: (parsed) => personalAgentsStore.peek()?.applyRoomEvent(parsed),
};
