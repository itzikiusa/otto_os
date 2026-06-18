// Insights API helpers: read/write the scheduled-report opt-in config, list the
// generated reports, kick off an ad-hoc run, and resolve a report's HTML for
// in-app viewing. Reports are catch-up: a scheduled run that was missed while
// the app was closed is generated the next time the app is open.

import { api, authedBlobUrl } from './client';
import type {
  InsightReport,
  InsightsConfig,
  RunInsightsReq,
  RunInsightsResp,
} from './types';

export const insightsApi = {
  /** Current opt-in toggles (daily / weekly / monthly). All off by default. */
  getConfig: () => api.get<InsightsConfig>('/insights/config'),
  putConfig: (body: InsightsConfig) =>
    api.put<InsightsConfig>('/insights/config', body),

  /** All generated reports, newest first. */
  listReports: () => api.get<InsightReport[]>('/insights/reports'),

  /** Start an ad-hoc run for a period; the report appears in the list shortly. */
  run: (body: RunInsightsReq) =>
    api.post<RunInsightsResp>('/insights/run', body),

  /**
   * Resolve a report's `html_path` to a revocable object URL the webview can
   * load into an <iframe> (the daemon serves the file with the auth token).
   * The caller MUST URL.revokeObjectURL() it when done (e.g. on unmount).
   */
  reportUrl: (htmlPath: string): Promise<string> =>
    authedBlobUrl(`/insights/report?path=${encodeURIComponent(htmlPath)}`),
};
