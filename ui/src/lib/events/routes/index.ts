// The feature route tables, merged (architecture-2-04). `lib/events.svelte.ts`
// adds the bus routes (its in-file buses) and falls back to `ws.applyEvent`
// for a type no table lists — the `LIVE_ONLY_EVENTS` below, which reach their
// consumers through `liveEvents` subscribers only.
import type { EventRoutes, EventType } from './types';
import { shellRoutes } from './shell';
import { swarmRoutes } from './swarm';
import { apiClientRoutes } from './apiClient';
import { productRoutes } from './product';
import { databaseRoutes } from './database';
import { canvasRoutes } from './canvas';
import { automationRoutes } from './automation';
import { browserRoutes } from './browser';
import { personalAgentsRoutes } from './personalAgents';
import { cloudRoutes } from './cloud';

export type { EventOf, EventRoutes, EventType } from './types';
export { swarmStore, usageStore, databaseStore, personalAgentsStore } from './stores';
export { resyncAutomation } from './automation';

export const featureRoutes: EventRoutes = {
  ...shellRoutes,
  ...swarmRoutes,
  ...apiClientRoutes,
  ...productRoutes,
  ...databaseRoutes,
  ...canvasRoutes,
  ...automationRoutes,
  ...browserRoutes,
  ...personalAgentsRoutes,
  ...cloudRoutes,
};

/** Event types with no store route: their consumers subscribe through
 *  `liveEvents.on(types, fn)` (liveQuery views, pages). Listing one here is a
 *  decision, not a default — `unit/eventRoutes.test.ts` fails on a type that
 *  is neither routed nor listed. */
export const LIVE_ONLY_EVENTS: readonly EventType[] = [
  'improvement_run_started',
  'improvement_run_finished',
  'improvement_edit_applied',
  'improvement_approval_pending',
  'insight_ready',
  'personal_agent_activity',
  'browser_live_session_updated',
  'mcp_approval_changed',
  'resource_access_changed',
  'notifications_changed',
  'workbench_doc_changed',
];
