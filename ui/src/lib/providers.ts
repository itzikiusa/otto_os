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

import type { RuntimeProvider, Session } from './api/types';
import { auth } from './stores/auth.svelte';

/** The pseudo-provider that opens a raw shell — can't plan/review/analyze. */
export const SHELL_PROVIDER = 'shell';

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

export function isRuntimeProvider(provider: string): provider is RuntimeProvider {
  return (RUNTIME_PROVIDERS as readonly string[]).includes(provider);
}

/** What a runtime thread's agent takes besides text. Claude: images + PDFs
 *  (content/document blocks); Codex and pi: images; the ACP agents
 *  (Antigravity, Gemini, Grok, OpenCode): images + PDFs, and the driver
 *  refuses with a clear message if that agent did not advertise them. */
export function attachmentSupport(provider: string): { images: boolean; files: boolean } {
  const files =
    provider === 'claude' ||
    provider === 'agy' ||
    provider === 'gemini' ||
    provider === 'grok' ||
    provider === 'opencode';
  return { images: files || provider === 'codex' || provider === 'pi', files };
}

/** Whether `f` is an attachment `provider` takes. */
export function takesFile(provider: string, f: Pick<File, 'type'>): boolean {
  const can = attachmentSupport(provider);
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

/** Built-in fallback used only before `/meta` has loaded (meta is normally
 *  present by the time any picker renders). */
const FALLBACK: readonly string[] = ['claude', 'codex', 'agy', 'opencode', 'pi', 'shell'];

const PREFERRED_THREAD_HARNESSES: readonly string[] = [
  'claude',
  'codex',
  'agy',
  'opencode',
  'pi',
  'gemini',
  'grok',
];

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

/** Shared launch readiness. Older daemons may not probe custom executables;
 *  keep those launchable but say the check is unknown instead of claiming it passed. */
export function providerReadiness(provider: string): { available: boolean; checked: boolean; message: string; version?: string } {
  if (!allProviders().includes(provider)) return { available: false, checked: true, message: 'Provider is not registered. Configure it in Settings → Providers.' };
  const tool = auth.meta?.tools?.find((t) => t.name === provider);
  if (tool) return {
    available: tool.found,
    checked: true,
    message: tool.found ? 'Ready' : 'Executable not found. Install the CLI or update its command in Settings → Providers.',
    version: tool.version ?? undefined,
  };
  return { available: true, checked: provider === SHELL_PROVIDER, message: provider === SHELL_PROVIDER ? 'Ready' : 'Executable availability has not been checked.' };
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
 *  to the built-in CLI harnesses before `/meta` has loaded (or on an older daemon). */
export function providerSupportsModel(provider: string): boolean {
  const flags = auth.meta?.model_flags;
  if (flags && provider in flags) return flags[provider];
  return (
    provider === 'claude' ||
    provider === 'codex' ||
    provider === 'agy' ||
    provider === 'opencode' ||
    provider === 'pi'
  );
}

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
