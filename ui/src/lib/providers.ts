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

/** The first registered agent provider (preferring the configured default),
 *  used as a safe default pick. */
export function defaultAgentProvider(): string {
  const list = agentProviders();
  const def = auth.meta?.default_provider;
  if (def && list.includes(def)) return def;
  return list[0] ?? 'claude';
}

/** token → canonical provider alias map for the ⌘K deterministic parser: the
 *  built-in aliases PLUS every registered provider name mapped to itself, so a
 *  custom provider (`grok`) resolves in the no-LLM fast-path. */
export function providerAliasMap(): Record<string, string> {
  const map: Record<string, string> = {};
  for (const name of allProviders()) map[name.toLowerCase()] = name;
  return map;
}
