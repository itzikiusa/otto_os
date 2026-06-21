// Module-local types for the usage attribution + forecast features (B1).
// Do NOT export from ui/src/lib/api/types.ts — these are usage-module-scoped.

/** One row in an attribution GROUP BY response from `GET /usage/attribution`. */
export interface AttributionRow {
  /** The grouped dimension value (repo id, branch name, origin tag, …). */
  key: string;
  cost_usd: number;
  tokens: number;
  sessions: number;
}

/** Valid `by=` dimension keys for the attribution endpoint. */
export type AttributionDim =
  | 'repo'
  | 'branch'
  | 'pr'
  | 'story'
  | 'swarm_task'
  | 'workflow'
  | 'channel'
  | 'review'
  | 'origin';

/** Human-readable labels for each dimension, shown in the picker. */
export const DIM_LABELS: Record<AttributionDim, string> = {
  repo: 'Repository',
  branch: 'Branch',
  pr: 'Pull Request',
  story: 'Story',
  swarm_task: 'Swarm Task',
  workflow: 'Workflow',
  channel: 'Channel',
  review: 'Code Review',
  origin: 'Origin',
};

/** Request body for `POST /usage/forecast`. */
export interface ForecastReq {
  feature: string;
  provider: string;
  est_tokens?: number;
}

/** Response from `POST /usage/forecast`. */
export interface ForecastResp {
  projected_cost_usd: number;
  basis: string;
}
