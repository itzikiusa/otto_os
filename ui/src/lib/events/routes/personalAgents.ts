import { personalAgentsStore } from './stores';
import { agentRunsWorking } from '../../stores/agentRunsWorking.svelte';
import type { EventRoutes } from './types';

export const personalAgentsRoutes: EventRoutes = {
  // Personal Agents page refreshes the agent's runs + schedule cursors.
  personal_agent_run_updated: (parsed) => {
    // The status bar's "N working" counts personal-agent runs too.
    agentRunsWorking.set(parsed.agent_id, parsed.status === 'running');
    personalAgentsStore.peek()?.applyRunEvent(parsed);
  },
  // Agents / routines / delegations changed from any path (incl. an agent's
  // MCP tools): refetch what changed, so a routine set up in chat shows.
  personal_agent_changed: (parsed) => personalAgentsStore.peek()?.applyChangedEvent(parsed),
  // Agent-room feeds append the event's message (open Rooms view only).
  agent_room_message: (parsed) => personalAgentsStore.peek()?.applyRoomEvent(parsed),
};
