// Pure, zero-Svelte provider constants, runtime capability helpers, harness
// ordering, and display labels shared by `lib/providers.ts`, IDE/Threads
// modules, and Node `unit/*.test.ts` suites.

import type { RuntimeProvider, Session } from './api/types.ts';

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
export type IdeRuntimeProvider = RuntimeProvider;

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
 *  providers for inline edits. Whenever `registeredProviders` is supplied as an
 *  array (even empty `[]`), disabled providers stay excluded. */
export function resolveIdeInlineProviders(
  availableProviders: readonly string[],
  registeredProviders?: readonly string[],
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
    if (isRuntimeProvider(p) && (!regSet || regSet.has(p)) && !filtered.includes(p)) {
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
