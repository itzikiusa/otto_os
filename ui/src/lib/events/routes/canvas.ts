import { lazyModule } from '../../lazyModule';
import type { EventRoutes } from './types';

const canvasStore = lazyModule(() => import('../../stores/canvas.svelte').then((m) => m.canvas));

export const canvasRoutes: EventRoutes = {
  // The agent session is live (turn start) → attach its shell immediately by
  // setting the open scene's session id.
  canvas_session_started: (parsed) =>
    canvasStore.use((canvas) => {
      if (parsed.scene_id === canvas.currentId) canvas.sessionId = parsed.session_id;
    }),
};
