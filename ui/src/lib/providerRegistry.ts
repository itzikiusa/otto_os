// Pure, zero-Svelte provider constants, family resolution, runtime capability
// helpers, harness ordering, and display labels shared by `lib/providers.ts`,
// `lib/providerModels.ts`, IDE/Threads modules, and Node `unit/*.test.ts` suites.

import type { MetaResp, RuntimeProvider, RuntimeProviderOrSlug, Session } from './api/types.ts';

/** The pseudo-provider that opens a raw shell — can't plan/review/analyze. */
export const SHELL_PROVIDER = 'shell';

/** Canonical CLI model families supported across Otto. */
export const KNOWN_AGENT_FAMILIES = [
  'claude',
  'codex',
  'agy',
  'gemini',
  'grok',
  'opencode',
  'pi',
] as const;

/** Harnesses with a structured runtime driver (Threads / IDE Agent dock). */
export const RUNTIME_PROVIDERS: readonly RuntimeProvider[] = [
  'claude',
  'codex',
  'agy',
  'gemini',
  'grok',
  'opencode',
  'pi',
];

export type ProviderFamilyMeta = Pick<MetaResp, 'provider_models'> &
  Partial<Pick<MetaResp, 'provider_families' | 'providers'>>;

/** Infer the canonical agent CLI family (`'claude'`, `'codex'`, `'agy'`, `'gemini'`,
 *  `'grok'`, `'opencode'`, `'pi'`, `'shell'`) for any built-in or custom provider slug
 *  (e.g. `'claudework'` -> `'claude'`), consulting `/meta.provider_families` first, then
 *  token-boundary matching for short names like `'pi'` (so `'pipeline'` or `'api-agent'`
 *  never false-matches `'pi'`), then `/meta.provider_models` catalog signatures. */
export function inferProviderFamily(
  provider: string,
  meta?: ProviderFamilyMeta | null,
): string {
  const trimmed = provider.trim();
  const p = trimmed.toLowerCase();
  if (!p) return '';
  const serverFam = meta?.provider_families?.[trimmed] ?? meta?.provider_families?.[p];
  if (typeof serverFam === 'string' && serverFam.trim() !== '') {
    return serverFam.trim().toLowerCase();
  }
  if (
    p === 'claude' ||
    p === 'codex' ||
    p === 'agy' ||
    p === 'gemini' ||
    p === 'grok' ||
    p === 'opencode' ||
    p === 'pi' ||
    p === 'shell'
  ) {
    return p;
  }
  if (p === 'antigravity' || p.startsWith('agy') || p.startsWith('antigravity') || p.endsWith('-agy')) {
    return 'agy';
  }
  if (p.startsWith('claude') || p.endsWith('-claude') || p.endsWith('_claude')) return 'claude';
  if (p.startsWith('codex') || p.endsWith('-codex') || p.endsWith('_codex')) return 'codex';
  if (p.startsWith('gemini') || p.endsWith('-gemini') || p.endsWith('_gemini')) return 'gemini';
  if (p.startsWith('grok') || p.endsWith('-grok') || p.endsWith('_grok')) return 'grok';
  if (p.startsWith('opencode') || p.endsWith('-opencode') || p.endsWith('_opencode')) return 'opencode';
  if (
    p === 'pi' ||
    p.startsWith('pi-') ||
    p.startsWith('pi_') ||
    p.endsWith('-pi') ||
    p.endsWith('_pi')
  ) {
    return 'pi';
  }

  const live = meta?.provider_models?.[provider];
  if (live && live.length > 0) {
    if (live.some((m) => m.id.startsWith('gemini-4-') || m.id.startsWith('gemini-pro-orchestrator'))) {
      return 'agy';
    }
    if (live.some((m) => m.id === 'opus' || m.id === 'sonnet' || m.id === 'haiku' || m.id.startsWith('claude-'))) {
      return 'claude';
    }
    if (live.some((m) => m.id.startsWith('gpt-5') || m.id.includes('codex'))) {
      return 'codex';
    }
    if (live.some((m) => m.id.startsWith('grok-'))) {
      return 'grok';
    }
    if (live.some((m) => m.id.startsWith('gemini-'))) {
      return 'gemini';
    }
  }
  return p;
}

/** Whether `provider` can run on Structured Runtime (`AgentRuntime`): true for all 7
 *  built-in runtime providers AND any custom provider whose CLI family maps to one of
 *  the 7 structured runtime drivers (e.g. `'claudework'` -> `'claude'`). */
export function isRuntimeProvider(
  provider: string,
  meta?: ProviderFamilyMeta | null,
): provider is RuntimeProvider {
  const clean = provider.trim();
  if (!clean || clean !== clean.toLowerCase() || clean === SHELL_PROVIDER) return false;
  if ((RUNTIME_PROVIDERS as readonly string[]).includes(clean)) return true;
  if (meta?.providers && !meta.providers.includes(clean)) return false;
  const fam = inferProviderFamily(clean, meta);
  return (RUNTIME_PROVIDERS as readonly string[]).includes(fam);
}

/** Return the effective provider slug for a runtime or PTY session:
 *  prefers `s.meta.provider_slug` (e.g. `'claudework'`) when present so custom
 *  providers keep their configured identity, icon, and model catalog even when
 *  `s.provider` stores the underlying protocol driver (`'claude'`). */
export function sessionProviderSlug(
  s: { provider: string; meta?: Record<string, unknown> | null } | null | undefined,
): string {
  if (!s) return 'claude';
  const rawSlug = s.meta?.provider_slug;
  if (typeof rawSlug === 'string' && rawSlug.trim()) return rawSlug.trim();
  return s.provider;
}

/** Whether a runtime provider supports steering mid-turn (`PromptDelivery = 'steer'`).
 *  Supported by Claude (`stream-json`), Codex (`turn/steer`), and pi (`steer` RPC),
 *  including custom providers mapped to those families (e.g. `claudework`). */
export function providerSupportsSteer(
  provider: string,
  meta?: ProviderFamilyMeta | null,
): boolean {
  if (!isRuntimeProvider(provider, meta)) return false;
  const fam = inferProviderFamily(provider, meta);
  return fam === 'claude' || fam === 'codex' || fam === 'pi';
}

/** What a runtime thread's agent takes besides text. Claude (and Claude-family
 *  custom providers like `claudework`): images + PDFs; Codex and pi: images; the
 *  ACP agents (Antigravity, Gemini, Grok, OpenCode): images + PDFs. */
export function attachmentSupport(
  provider: string,
  meta?: ProviderFamilyMeta | null,
): { images: boolean; files: boolean } {
  const fam = inferProviderFamily(provider, meta);
  const files =
    fam === 'claude' ||
    fam === 'agy' ||
    fam === 'gemini' ||
    fam === 'grok' ||
    fam === 'opencode';
  return { images: files || fam === 'codex' || fam === 'pi', files };
}

/** Whether `f` is an attachment `provider` takes. */
export function takesFile(
  provider: string,
  f: Pick<File, 'type'>,
  meta?: ProviderFamilyMeta | null,
): boolean {
  const can = attachmentSupport(provider, meta);
  return (can.images && f.type.startsWith('image/')) || (can.files && f.type === 'application/pdf');
}

/** Whether a session is a raw interactive shell PTY (not a coding-agent CLI). */
export function isPlainShellSession(s: Session | null | undefined): boolean {
  if (!s) return false;
  return (
    s.provider === SHELL_PROVIDER &&
    typeof (s.meta as Record<string, unknown> | undefined)?.nested_provider !== 'string'
  );
}

const PREFERRED_THREAD_HARNESSES: readonly string[] = [
  'claude',
  'codex',
  'agy',
  'opencode',
  'pi',
  'gemini',
  'grok',
];

/** Order enabled agent providers for Threads/stage pickers without ever
 *  re-introducing a provider hidden via `disabled_providers`. */
export function orderThreadHarnesses(registered: readonly string[]): string[] {
  const enabled = registered.map((p) => p.trim()).filter((p) => p && p !== SHELL_PROVIDER);
  const ordered: string[] = [];
  for (const preferred of PREFERRED_THREAD_HARNESSES) {
    if (enabled.includes(preferred)) ordered.push(preferred);
  }
  for (const p of enabled) {
    if (!ordered.includes(p)) ordered.push(p);
  }
  return ordered;
}

export const IDE_RUNTIME_PROVIDERS: readonly RuntimeProvider[] = orderThreadHarnesses(
  RUNTIME_PROVIDERS,
) as RuntimeProvider[];
export type IdeRuntimeProvider = RuntimeProviderOrSlug;

/** Build the ordered IDE harness list from enabled `/meta.providers`, preserving
 *  an explicit empty registry (`[]`) when every built-in provider is disabled
 *  while appending any non-shell session providers already open. */
export function resolveIdeHarnesses(
  registeredProviders: readonly string[],
  extraProviders: readonly (string | null | undefined)[] = [],
): string[] {
  const merged = orderThreadHarnesses(registeredProviders);
  for (let i = 0; i < extraProviders.length; i++) {
    const clean = (extraProviders[i] ?? '').trim();
    if (clean && clean !== SHELL_PROVIDER && !merged.includes(clean)) {
      merged.push(clean);
    }
  }
  return merged;
}

/** Filter available harnesses down to enabled structured `/runtime-sessions`
 *  providers for inline edits (including custom providers like `claudework`). */
export function resolveIdeInlineProviders(
  availableProviders: readonly string[],
  registeredProviders?: readonly string[],
  meta?: ProviderFamilyMeta | null,
): IdeRuntimeProvider[] {
  const hasRegistry = Array.isArray(registeredProviders);
  const regSet = hasRegistry
    ? new Set(
        registeredProviders
          .map((p) => p.trim())
          .filter((p) => p && p !== SHELL_PROVIDER),
      )
    : null;
  const filtered: IdeRuntimeProvider[] = [];
  for (let i = 0; i < availableProviders.length; i++) {
    const p = availableProviders[i]?.trim() ?? '';
    if (isRuntimeProvider(p, meta) && (!regSet || regSet.has(p)) && !filtered.includes(p)) {
      filtered.push(p);
    }
  }
  if (filtered.length > 0) return filtered;
  return hasRegistry ? [] : [...IDE_RUNTIME_PROVIDERS];
}

/** Harness display name for pickers, cards and badges. */
export function harnessLabel(provider: string): string {
  switch (provider) {
    case 'claude':
      return 'Claude Code';
    case 'codex':
      return 'Codex';
    case 'agy':
      return 'Antigravity';
    case 'opencode':
      return 'OpenCode';
    case 'pi':
      return 'Pi';
    case 'gemini':
      return 'Gemini';
    case 'grok':
      return 'Grok';
    case 'shell':
      return 'Shell';
    default:
      return provider.charAt(0).toUpperCase() + provider.slice(1);
  }
}

/** Short CLI subtitle for New Session, Handover, and provider cards. */
export function providerSubtitle(provider: string): string {
  switch (provider) {
    case 'claude':
      return 'Claude Code CLI';
    case 'codex':
      return 'Codex CLI';
    case 'agy':
      return 'Antigravity CLI';
    case 'opencode':
      return 'OpenCode CLI';
    case 'pi':
      return 'Pi CLI';
    case 'gemini':
      return 'Gemini CLI';
    case 'grok':
      return 'Grok CLI';
    case 'shell':
      return 'Plain shell';
    default:
      return 'Custom provider';
  }
}
