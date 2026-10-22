import { swarmStore, usageStore } from './stores';
import type { EventOf, EventRoutes } from './types';

type SwarmEvent = EventOf<'swarm_run_updated' | 'swarm_task_updated' | 'swarm_project_cleared' | 'swarm_message_posted' | 'swarm_goal_updated' | 'swarm_status'>;
// The Swarm page loads its swarms on mount: a document without the store has
// nothing to patch.
const toSwarm = (parsed: SwarmEvent): void => {
  swarmStore.peek()?.applyEvent(parsed);
};

export const swarmRoutes: EventRoutes = {
  swarm_run_updated: toSwarm,
  swarm_task_updated: toSwarm,
  swarm_project_cleared: toSwarm,
  swarm_message_posted: toSwarm,
  swarm_goal_updated: toSwarm,
  swarm_status: toSwarm,
  // Drive a near-real-time metrics sparkline refresh without polling.
  // Broadcast on every sampler tick; only an open metrics view wants it.
  usage_metrics_tick: () => usageStore.peek()?.applyMetricsTick(),
};
