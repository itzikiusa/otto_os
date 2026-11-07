// Provider model catalogs, reasoning-effort levels, and interaction modes
// shared between NewThreadStage and the live thread Composer.
import type { MetaResp, ProviderModelOption, Session } from './api/types';

export type EffortLevel = 'low' | 'medium' | 'high' | 'xhigh' | 'max' | 'ultra';
export type InteractionMode = 'full' | 'accept-edits' | 'plan';

export interface InteractionModeOption {
  id: InteractionMode;
  label: string;
  shortLabel: string;
  hint: string;
}

export const INTERACTION_MODES: readonly InteractionModeOption[] = [
  {
    id: 'full',
    label: 'Full access',
    shortLabel: 'Full access',
    hint: 'Auto-approve tools and commands without confirmation prompts',
  },
  {
    id: 'accept-edits',
    label: 'Accept edits',
    shortLabel: 'Accept edits',
    hint: 'Auto-apply file edits; ask before running shell commands',
  },
  {
    id: 'plan',
    label: 'Plan mode',
    shortLabel: 'Plan',
    hint: 'Read-only exploration — propose a plan before making edits (⇧⇥)',
  },
] as const;

const STANDARD_EFFORTS: EffortLevel[] = ['low', 'medium', 'high', 'xhigh', 'max'];

export const FALLBACK_PROVIDER_MODELS: Record<string, ProviderModelOption[]> = {
  claude: [
    {
      id: 'opus',
      label: 'Claude Opus',
      description: 'Most capable model for complex reasoning and everyday coding',
      default_effort: 'high',
      supported_efforts: [...STANDARD_EFFORTS],
    },
    {
      id: 'sonnet',
      label: 'Claude Sonnet',
      description: 'Balanced speed and intelligence for daily engineering',
      default_effort: 'medium',
      supported_efforts: [...STANDARD_EFFORTS],
    },
    {
      id: 'haiku',
      label: 'Claude Haiku',
      description: 'Fastest responses for lightweight edits and questions',
      default_effort: 'low',
      supported_efforts: ['low', 'medium', 'high'],
    },
  ],
  codex: [
    {
      id: 'gpt-5.4',
      label: 'GPT-5.4',
      description: 'Flagship reasoning model for coding and agentic workflows',
      default_effort: 'medium',
      supported_efforts: [...STANDARD_EFFORTS],
    },
    {
      id: 'gpt-5.3-codex',
      label: 'GPT-5.3 Codex',
      description: 'Specialized Codex model tuned for software engineering',
      default_effort: 'medium',
      supported_efforts: [...STANDARD_EFFORTS],
    },
  ],
  agy: [
    {
      id: 'gemini-4-argon-high',
      label: 'Gemini 4 Argon (High)',
      description: 'High-reasoning Gemini model',
      default_effort: 'high',
      supported_efforts: [...STANDARD_EFFORTS],
    },
    {
      id: 'gemini-4-argon-medium',
      label: 'Gemini 4 Argon (Medium)',
      description: 'Balanced Gemini model',
      default_effort: 'medium',
      supported_efforts: [...STANDARD_EFFORTS],
    },
    {
      id: 'claude-opus-5-5-high',
      label: 'Claude Opus 5.5 (High)',
      description: 'Anthropic Opus via Antigravity',
      default_effort: 'high',
      supported_efforts: [...STANDARD_EFFORTS],
    },
  ],
};

const VALID_EFFORTS = new Set<string>(['low', 'medium', 'high', 'xhigh', 'max', 'ultra']);

export function supportsModelSelection(
  provider: string,
  meta?: Pick<MetaResp, 'model_flags'> | null,
): boolean {
  if (!provider || provider === 'shell') return false;
  if (meta?.model_flags && provider in meta.model_flags) {
    return meta.model_flags[provider] === true;
  }
  return provider === 'claude' || provider === 'codex' || provider === 'agy';
}

export function supportsEffortSelection(provider: string): boolean {
  return provider === 'claude' || provider === 'codex' || provider === 'agy';
}

export function supportsInteractionMode(provider: string): boolean {
  return provider === 'claude' || provider === 'codex' || provider === 'agy';
}

export function modelsForProvider(
  provider: string,
  meta?: Pick<MetaResp, 'provider_models'> | null,
): ProviderModelOption[] {
  const live = meta?.provider_models?.[provider];
  if (live && live.length > 0) return live;
  return FALLBACK_PROVIDER_MODELS[provider] ?? [];
}

export function normalizeEffort(raw: unknown): EffortLevel | null {
  if (typeof raw !== 'string') return null;
  const v = raw.trim().toLowerCase();
  return VALID_EFFORTS.has(v) ? (v as EffortLevel) : null;
}

export function normalizeInteractionMode(raw: unknown): InteractionMode {
  if (typeof raw !== 'string') return 'full';
  const v = raw.trim().toLowerCase();
  if (v === 'plan') return 'plan';
  if (v === 'accept-edits' || v === 'standard') return 'accept-edits';
  return 'full';
}

export function effortsForModel(
  provider: string,
  modelId: string | null | undefined,
  meta?: Pick<MetaResp, 'provider_models'> | null,
): EffortLevel[] {
  if (!supportsEffortSelection(provider)) return [];
  const catalog = modelsForProvider(provider, meta);
  const match = modelId ? catalog.find((m) => m.id === modelId) : catalog[0];
  const supported = (match?.supported_efforts ?? [])
    .map((e) => normalizeEffort(e))
    .filter((e): e is EffortLevel => e !== null);
  return supported.length > 0 ? supported : [...STANDARD_EFFORTS];
}

export function defaultEffortForModel(
  provider: string,
  modelId: string | null | undefined,
  meta?: Pick<MetaResp, 'provider_models'> | null,
): EffortLevel {
  const catalog = modelsForProvider(provider, meta);
  const match = modelId ? catalog.find((m) => m.id === modelId) : catalog[0];
  return normalizeEffort(match?.default_effort) ?? 'medium';
}

export function formatEffortLabel(effort: string | null | undefined): string {
  switch ((effort ?? '').trim().toLowerCase()) {
    case 'low':
      return 'Low';
    case 'medium':
      return 'Medium';
    case 'high':
      return 'High';
    case 'xhigh':
      return 'Extra High';
    case 'max':
      return 'Max';
    case 'ultra':
      return 'Ultra';
    default:
      return 'Default';
  }
}

export function formatEffortShort(effort: string | null | undefined): string {
  switch ((effort ?? '').trim().toLowerCase()) {
    case 'low':
      return 'Low';
    case 'medium':
      return 'Med';
    case 'high':
      return 'High';
    case 'xhigh':
      return 'X-High';
    case 'max':
      return 'Max';
    case 'ultra':
      return 'Ultra';
    default:
      return 'Auto';
  }
}

export function formatModelLabel(
  provider: string,
  modelId: string | null | undefined,
  meta?: Pick<MetaResp, 'provider_models'> | null,
): string {
  const trimmed = (modelId ?? '').trim();
  if (!trimmed) return 'Default model';
  // The live catalog first, then the built-in aliases ("opus" → "Claude Opus").
  const found =
    modelsForProvider(provider, meta).find((m) => m.id === trimmed) ??
    FALLBACK_PROVIDER_MODELS[provider]?.find((m) => m.id === trimmed);
  if (found) return found.label;
  return trimmed;
}

export function formatInteractionModeLabel(mode: InteractionMode): string {
  return INTERACTION_MODES.find((m) => m.id === mode)?.shortLabel ?? 'Full access';
}

export function nextInteractionModeToggle(mode: InteractionMode): InteractionMode {
  return mode === 'plan' ? 'full' : 'plan';
}

export interface SessionThreadConfig {
  /** Explicit `meta.model` if pinned, else null. */
  pinnedModel: string | null;
  /** Effective display model (`meta.model` or transcript model). */
  effectiveModel: string | null;
  effort: EffortLevel | null;
  interactionMode: InteractionMode;
}

export function resolveSessionThreadConfig(
  session: Pick<Session, 'meta'> | null | undefined,
  transcriptModel?: string | null,
): SessionThreadConfig {
  const m = session?.meta ?? {};
  const pinned = typeof m.model === 'string' && m.model.trim() !== '' ? m.model.trim() : null;
  const fallback =
    typeof transcriptModel === 'string' && transcriptModel.trim() !== ''
      ? transcriptModel.trim()
      : null;
  const rawMode =
    typeof m.interaction_mode === 'string'
      ? m.interaction_mode
      : m.autonomy_mode === 'standard'
        ? 'accept-edits'
        : 'full';
  return {
    pinnedModel: pinned,
    effectiveModel: pinned ?? fallback,
    effort: normalizeEffort(m.effort),
    interactionMode: normalizeInteractionMode(rawMode),
  };
}
