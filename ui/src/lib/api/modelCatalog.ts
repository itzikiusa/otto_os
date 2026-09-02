// Model catalog API client — per-provider model lists refreshed server-side
// with no API keys. Mirrors docs/contracts/api.md (## Model catalog). The
// shared ModelPicker keeps its own tolerant fetch; these typed wrappers are
// for surfaces that show catalog freshness or trigger a refresh.

import { api } from './client';
import type { ModelCatalogResp } from './types';

export const modelCatalogApi = {
  list: () => api.get<ModelCatalogResp>('/providers/models'),
  /** Absent provider = refresh all. A failed refresh never wipes the last good list. */
  refresh: (provider?: string) =>
    api.post<ModelCatalogResp>('/providers/models/refresh', provider ? { provider } : {}),
};
