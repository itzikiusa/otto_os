import { lazyModule } from '../../lazyModule';
import type { EventRoutes } from './types';

const productStore = lazyModule(() => import('../../stores/product.svelte').then((m) => m.product), 'product');
const mockupAssistStore = lazyModule(() => import('../../stores/mockup-assist.svelte').then((m) => m.mockupAssist));

export const productRoutes: EventRoutes = {
  // Let product section tabs know a run completed (kills a poll cycle).
  product_changed: (parsed) => productStore.peek()?.applyEvent(parsed),
  // Multi-agent plan kickoff: the Plan tab tiles the live sessions.
  plan_run: (parsed) => productStore.peek()?.applyPlanRun(parsed),
  // Live design edits: the arena's Assistant preview + the open artifact
  // reload. `content` is an explicit null for binary / oversized payloads (a
  // glb, a Blender render) — the store then re-fetches the bytes rather than
  // treating "null" as an empty document.
  mockup_updated: (parsed) =>
    mockupAssistStore.use((mockupAssist) =>
      mockupAssist.ingestLive(parsed.attachment_id, parsed.story_id, parsed.format, parsed.content ?? null),
    ),
  // The mockup agent session is live (turn start) → attach its shell.
  mockup_session_started: (parsed) =>
    mockupAssistStore.use((mockupAssist) =>
      mockupAssist.setSession(parsed.attachment_id, parsed.story_id, parsed.session_id),
    ),
};
