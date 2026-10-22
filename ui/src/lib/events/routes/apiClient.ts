import { lazyModule } from '../../lazyModule';
import type { EventRoutes } from './types';

const apiClientStore = lazyModule(() => import('../../stores/apiClient.svelte').then((m) => m.apiClient), 'apiClient');

export const apiClientRoutes: EventRoutes = {
  // API Client history is shared across human sends and agent MCP runs.
  // Refresh once after a burst so the current workspace stays live.
  // Automation runs emit one per step / row: only a loaded store with a
  // loaded history list refetches.
  api_history_appended: (parsed) => apiClientStore.peek()?.noteHistoryAppended(parsed.workspace_id, parsed.entry_id),
  // A running automation finished a step: fetch its delta now instead of
  // waiting for the fallback poll.
  api_run_progress: (parsed) => apiClientStore.peek()?.noteRunProgress(parsed.run_id, parsed.status),
  // Requests / collections / envs / automations changed (maybe by an agent):
  // keep the API store's 60 s reuse window honest. A store that never loaded
  // has nothing stale — don't load the chunk for it.
  api_client_changed: (parsed) => apiClientStore.peek()?.noteClientChanged(parsed),
};
