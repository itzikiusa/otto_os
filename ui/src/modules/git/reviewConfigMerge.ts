import type { ReviewAgentCfg, ReviewConfig, ReviewMode } from '../../lib/api/types';

// The "Configure agents" modal edits only part of a ReviewConfig (agents,
// summarizer, presets, cross-provider check, and the Advanced fields). The PUT
// replaces the WHOLE stored config, so a save built from the edited fields
// alone silently wiped everything else set through the API, MCP or a preset
// (mode, attempts, timeout, PR-size thresholds, fields a newer daemon added).
// Every save therefore starts from the config that was loaded.

/** The run settings the modal's Advanced section edits. `null` = the daemon default. */
export interface ReviewAdvanced {
  mode: ReviewMode | null;
  max_attempts: number | null;
  timeout_secs: number | null;
  medium_pr_lines: number | null;
  large_pr_lines: number | null;
}

/** The daemon's defaults for the Advanced fields (shown as placeholders). */
export const ADVANCED_DEFAULTS = {
  mode: 'fan_out' as ReviewMode,
  max_attempts: 3,
  medium_pr_lines: 1000,
  large_pr_lines: 5000,
};

export function advancedOf(cfg: ReviewConfig | null | undefined): ReviewAdvanced {
  return {
    mode: cfg?.mode ?? null,
    max_attempts: cfg?.max_attempts ?? null,
    timeout_secs: cfg?.timeout_secs ?? null,
    medium_pr_lines: cfg?.medium_pr_lines ?? null,
    large_pr_lines: cfg?.large_pr_lines ?? null,
  };
}

/** `providers` always set and `provider` = its first entry (the old single-provider field). */
export function syncProviders(a: ReviewAgentCfg, fallback: string): ReviewAgentCfg {
  const ps = a.providers && a.providers.length > 0 ? a.providers : [a.provider || fallback];
  return { ...a, provider: ps[0], providers: ps };
}

/** The fields the modal edits directly. */
export type ReviewConfigEdits = Pick<ReviewConfig, 'cross_provider_check' | 'agents' | 'summarizer' | 'custom_presets'>;

/**
 * The config to save: the loaded `base` with the edited fields laid over it.
 * Unknown / extra fields of `base` survive; an Advanced field cleared to empty
 * is dropped (the daemon then uses its default) rather than sent as `null`
 * over a value the server would keep.
 */
export function mergeReviewConfig(
  base: ReviewConfig | null | undefined,
  edits: ReviewConfigEdits,
  advanced: ReviewAdvanced,
): ReviewConfig {
  const out: Record<string, unknown> = { ...(base ?? {}), ...edits };
  for (const [k, v] of Object.entries(advanced)) {
    const valid = typeof v === 'number' ? Number.isFinite(v) && v > 0 : v != null && v !== '';
    if (valid) out[k] = v;
    else delete out[k];
  }
  return out as unknown as ReviewConfig;
}
