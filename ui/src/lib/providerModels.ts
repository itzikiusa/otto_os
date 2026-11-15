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
      id: 'gemini-4-carbon',
      label: 'Gemini 4 Carbon',
      description: 'Balanced flagship Gemini model',
      default_effort: 'medium',
      supported_efforts: ['low', 'medium', 'high'],
    },
    {
      id: 'gemini-4-argon',
      label: 'Gemini 4 Argon',
      description: 'High-capability Gemini reasoning model',
      default_effort: 'medium',
      supported_efforts: ['low', 'medium', 'high'],
    },
    {
      id: 'gemini-4-hydrogen',
      label: 'Gemini 4 Hydrogen',
      description: 'Deep-reasoning Gemini model',
      default_effort: 'high',
      supported_efforts: ['low', 'medium', 'high', 'xhigh'],
    },
    {
      id: 'claude-opus-5-5',
      label: 'Claude Opus 5.5',
      description: 'Anthropic Opus via Antigravity',
      default_effort: 'medium',
      supported_efforts: ['low', 'medium', 'high'],
    },
  ],
};

const VALID_EFFORTS = new Set<string>(['low', 'medium', 'high', 'xhigh', 'max', 'ultra']);
const EFFORT_ORDER: readonly EffortLevel[] = ['low', 'medium', 'high', 'xhigh', 'max', 'ultra'];
const ID_TIER_RE = /^(.+)-(minimal|low|medium|high|xhigh|max|ultra)$/;
const LABEL_TIER_RE = /^(.*?)\s*\((minimal|low|medium|high|xhigh|x-high|extra high|extra|max|ultra)\)\s*$/i;
const unifiedCatalogMemo = new WeakMap<readonly ProviderModelOption[], Map<string, ProviderModelOption[]>>();

export function providerFamily(provider: string): string {
  return provider.trim().toLowerCase();
}

function mapRawTier(raw: string): EffortLevel {
  const v = raw.trim().toLowerCase();
  if (v === 'minimal') return 'low';
  if (v === 'x-high' || v === 'extra high' || v === 'extra') return 'xhigh';
  return (VALID_EFFORTS.has(v) ? v : 'medium') as EffortLevel;
}

export function stripEmbeddedEffortTier(
  provider: string,
  id: string,
  label = id,
): { baseId: string; cleanLabel: string; effort: EffortLevel } | null {
  if (providerFamily(provider) !== 'agy') return null;
  const idMatch = ID_TIER_RE.exec(id);
  if (!idMatch) return null;
  const baseId = idMatch[1];
  const effort = mapRawTier(idMatch[2]);
  if (label === id) {
    return { baseId, cleanLabel: baseId, effort };
  }
  const labelMatch = LABEL_TIER_RE.exec(label);
  if (!labelMatch) return null;
  const cleanLabel = labelMatch[1].trim();
  if (!cleanLabel) return null;
  return { baseId, cleanLabel, effort };
}

/** Collapse tier-suffixed `agy` model rows (`gemini-4-carbon-{low,medium,high}`)
 *  into one unified family row with exact `supported_efforts`, matching the
 *  daemon's parser even when `/meta` holds an older cached list. Non-`agy`
 *  providers return their catalog untouched so custom `-low`/`-high`/`-max`
 *  model IDs are never rewritten. */
export function unifyModelCatalog(
  provider: string,
  list: readonly ProviderModelOption[],
): ProviderModelOption[] {
  const fam = providerFamily(provider);
  let byProvider = unifiedCatalogMemo.get(list);
  const cached = byProvider?.get(fam);
  if (cached) return cached;

  let result: ProviderModelOption[];
  if (fam !== 'agy') {
    result = [...list];
  } else {
    type Group = {
      kind: 'family';
      baseId: string;
      firstFullId: string;
      label: string;
      description: string;
      firstEffort: EffortLevel;
      efforts: EffortLevel[];
    };
    const out: Array<{ kind: 'plain'; opt: ProviderModelOption } | Group> = [];
    for (let i = 0; i < list.length; i++) {
      const item = list[i];
      const split = stripEmbeddedEffortTier(fam, item.id, item.label);
      if (!split) {
        out.push({ kind: 'plain', opt: item });
        continue;
      }
      const existing = out.find(
        (g): g is Group => g.kind === 'family' && g.baseId === split.baseId,
      );
      if (existing) {
        if (!existing.efforts.includes(split.effort)) existing.efforts.push(split.effort);
      } else {
        out.push({
          kind: 'family',
          baseId: split.baseId,
          firstFullId: item.id,
          label: split.cleanLabel,
          description: item.description,
          firstEffort: split.effort,
          efforts: [split.effort],
        });
      }
    }
    result = out.map((entry) => {
      if (entry.kind === 'plain') return entry.opt;
      const sorted = [...entry.efforts].sort(
        (a, b) => EFFORT_ORDER.indexOf(a) - EFFORT_ORDER.indexOf(b),
      );
      const defaultEffort =
        entry.firstEffort !== 'low'
          ? entry.firstEffort
          : sorted.includes('medium')
            ? 'medium'
            : entry.firstEffort;
      return {
        id: sorted.length >= 2 ? entry.baseId : entry.firstFullId,
        label: entry.label,
        description: entry.description,
        default_effort: defaultEffort,
        supported_efforts: sorted,
      };
    });
  }

  if (!byProvider) {
    byProvider = new Map();
    unifiedCatalogMemo.set(list, byProvider);
  }
  byProvider.set(fam, result);
  return result;
}

export function supportsModelSelection(
  provider: string,
  meta?: Pick<MetaResp, 'model_flags'> | null,
): boolean {
  if (!provider || provider === 'shell') return false;
  if (meta?.model_flags && provider in meta.model_flags) {
    return meta.model_flags[provider] === true;
  }
  return (
    provider === 'claude' ||
    provider === 'codex' ||
    provider === 'agy' ||
    provider === 'opencode' ||
    provider === 'pi'
  );
}

export function supportsEffortSelection(provider: string): boolean {
  return (
    provider === 'claude' ||
    provider === 'codex' ||
    provider === 'agy' ||
    provider === 'pi'
  );
}

export function supportsInteractionMode(provider: string): boolean {
  return provider === 'claude' || provider === 'codex' || provider === 'agy';
}

export function modelsForProvider(
  provider: string,
  meta?: Pick<MetaResp, 'provider_models'> | null,
): ProviderModelOption[] {
  const live = meta?.provider_models?.[provider];
  if (live && live.length > 0) return unifyModelCatalog(provider, live);
  return FALLBACK_PROVIDER_MODELS[provider] ?? [];
}

/** Resolve a saved or legacy `agy` tier-suffixed model ID (`gemini-4-carbon-high`)
 *  to its unified catalog entry ID (`gemini-4-carbon`) when that family exists. */
export function canonicalModelId(
  provider: string,
  modelId: string | null | undefined,
  meta?: Pick<MetaResp, 'provider_models'> | null,
): string | null {
  const trimmed = (modelId ?? '').trim();
  if (!trimmed) return null;
  const catalog = modelsForProvider(provider, meta);
  if (catalog.some((m) => m.id === trimmed)) return trimmed;
  if (providerFamily(provider) === 'agy') {
    const m = ID_TIER_RE.exec(trimmed);
    if (m && catalog.some((x) => x.id === m[1])) return m[1];
  }
  return trimmed;
}

export function canonicalizeModelSelection(
  provider: string,
  modelId: string | null | undefined,
  effort: EffortLevel | null = null,
  meta?: Pick<MetaResp, 'provider_models'> | null,
): { model: string | null; effort: EffortLevel | null } {
  const trimmed = (modelId ?? '').trim();
  if (!trimmed) return { model: null, effort };
  const resolved = canonicalModelId(provider, trimmed, meta) ?? trimmed;
  let nextEffort = effort;
  if (!nextEffort && providerFamily(provider) === 'agy' && resolved !== trimmed) {
    const tier = ID_TIER_RE.exec(trimmed);
    if (tier) nextEffort = mapRawTier(tier[2]);
  }
  return { model: resolved, effort: nextEffort };
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
  const resolved = canonicalModelId(provider, modelId, meta);
  const match = resolved ? catalog.find((m) => m.id === resolved) : catalog[0];
  if (!match) return [...STANDARD_EFFORTS];
  return (match.supported_efforts ?? [])
    .map((e) => normalizeEffort(e))
    .filter((e): e is EffortLevel => e !== null);
}

export function defaultEffortForModel(
  provider: string,
  modelId: string | null | undefined,
  meta?: Pick<MetaResp, 'provider_models'> | null,
): EffortLevel {
  const catalog = modelsForProvider(provider, meta);
  const resolved = canonicalModelId(provider, modelId, meta);
  const match = resolved ? catalog.find((m) => m.id === resolved) : catalog[0];
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
  const resolved = canonicalModelId(provider, trimmed, meta) ?? trimmed;
  // The live catalog first, then the built-in aliases ("opus" → "Claude Opus").
  const found =
    modelsForProvider(provider, meta).find((m) => m.id === resolved) ??
    FALLBACK_PROVIDER_MODELS[provider]?.find((m) => m.id === resolved);
  if (found) return found.label;
  if (providerFamily(provider) === 'agy') {
    const labelTier = LABEL_TIER_RE.exec(trimmed);
    if (labelTier && labelTier[1].trim()) return labelTier[1].trim();
  }
  return trimmed;
}

export const formatModelChipLabel = formatModelLabel;

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
  session: (Pick<Session, 'meta'> & { provider?: string }) | null | undefined,
  transcriptModel?: string | null,
): SessionThreadConfig {
  const m = session?.meta ?? {};
  const pinned = typeof m.model === 'string' && m.model.trim() !== '' ? m.model.trim() : null;
  const fallback =
    typeof transcriptModel === 'string' && transcriptModel.trim() !== ''
      ? transcriptModel.trim()
      : null;
  const effectiveModel = pinned ?? fallback;
  let effort = normalizeEffort(m.effort);
  if (!effort && session?.provider === 'agy' && pinned) {
    const tier = ID_TIER_RE.exec(pinned);
    if (tier) effort = mapRawTier(tier[2]);
  }
  const rawMode =
    typeof m.interaction_mode === 'string'
      ? m.interaction_mode
      : m.autonomy_mode === 'standard'
        ? 'accept-edits'
        : 'full';
  return {
    pinnedModel: pinned,
    effectiveModel,
    effort,
    interactionMode: normalizeInteractionMode(rawMode),
  };
}
