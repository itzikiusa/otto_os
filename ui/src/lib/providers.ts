// Single source of truth for the agent-provider list across the whole UI.
//
// The daemon's provider registry (built-ins claude/codex/agy/shell + any CUSTOM
// providers the user configures in Settings → Providers, e.g. `grok`) is
// surfaced on `GET /meta`.providers and cached on the auth store. EVERY surface
// that lets the user pick which agent CLI to launch/use — PR review, ⌘K
// orchestrator, swarm, workflows, scheduled tasks, self-improve, evals, … —
// must render from THIS list so a custom provider is a first-class citizen,
// never a hardcoded subset. Supports any number of providers (10, 50, …).
//
// These are plain functions (not $derived) so any module can import them; call
// them inside a component `$derived(...)` and they stay reactive, because they
// read `auth.meta` (a $state field).
//
// NOTE: the Product tabs' `claude/openai/gemini` picker is a DIFFERENT axis
// (direct LLM-API model providers, not spawnable agent CLIs) and intentionally
// does not use this registry.

import { auth } from './stores/auth.svelte';

/** The pseudo-provider that opens a raw shell — can't plan/review/analyze. */
export const SHELL_PROVIDER = 'shell';

/** Built-in fallback used only before `/meta` has loaded (meta is normally
 *  present by the time any picker renders). */
const FALLBACK: readonly string[] = ['claude', 'codex', 'agy', 'shell'];

/** All registered provider names (built-ins + custom), INCLUDING `shell`.
 *  Already sorted by the daemon. Use for surfaces where a shell is valid
 *  (new session, scheduled tasks). */
export function allProviders(): string[] {
  const p = auth.meta?.providers;
  return p && p.length > 0 ? [...p] : [...FALLBACK];
}

/** Registered AGENT providers — `allProviders()` minus `shell`. Use for every
 *  surface that needs a reasoning agent (review, swarm, workflows, plan, eval,
 *  self-improve, channels, run-with-otto, goal loops, …). */
export function agentProviders(): string[] {
  return allProviders().filter((p) => p !== SHELL_PROVIDER);
}

/** `agentProviders()` but guaranteed to include `current` (even if it is not
 *  registered), so an editor showing a previously-saved provider never drops
 *  it. Preserves registry order, appending `current` if missing. */
export function agentProvidersWith(current: string | null | undefined): string[] {
  const list = agentProviders();
  const c = (current ?? '').trim();
  return c && !list.includes(c) ? [c, ...list] : list;
}

/** The provider a surface should default to when the user hasn't picked one.
 *  Mirrors the daemon's `resolve_provider` precedence so the UI never labels a
 *  different provider than what would actually run:
 *    configured default (Settings → Providers → Default agent)
 *      → `claude` (the daemon's FALLBACK_PROVIDER, i.e. "Auto")
 *      → the first registered agent provider (only if claude isn't registered).
 *  Notably this must NOT return the alphabetically-first provider (`agy`) when
 *  nothing is configured — "Auto" means claude. */
export function defaultAgentProvider(): string {
  const list = agentProviders();
  const def = (auth.meta?.default_provider ?? '').trim();
  if (def && list.includes(def)) return def;
  if (list.includes('claude')) return 'claude';
  return list[0] ?? 'claude';
}

/** Whether `provider` accepts a model flag — its daemon spec carries a
 *  `model_args` template (`/meta.model_flags`). Surfaces hide their model
 *  control when false, so a picked model is never silently dropped. Falls back
 *  to the built-in trio before `/meta` has loaded (or on an older daemon). */
export function providerSupportsModel(provider: string): boolean {
  const flags = auth.meta?.model_flags;
  if (flags && provider in flags) return flags[provider];
  return provider === 'claude' || provider === 'codex' || provider === 'agy';
}

/** token → canonical provider alias map for the ⌘K deterministic parser: the
 *  built-in aliases PLUS every registered provider name mapped to itself, so a
 *  custom provider (`grok`) resolves in the no-LLM fast-path. */
export function providerAliasMap(): Record<string, string> {
  const map: Record<string, string> = {};
  for (const name of allProviders()) map[name.toLowerCase()] = name;
  return map;
}
