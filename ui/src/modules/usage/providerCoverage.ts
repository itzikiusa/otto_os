// Which providers' tokens Usage can measure, and how to name them.
//
// Usage reads the CLIs' own records (transcripts / session stores) for Claude,
// Codex, Grok and Antigravity — terminal sessions and Threads alike. Gemini,
// OpenCode, Pi and Muse keep no record Otto reads, so only their Threads
// (structured runtime) turns are counted, from the usage each turn reports.
// Their terminal sessions are not measured at all: the page says so instead of
// letting a missing bar read as "used nothing". Mirrors
// `TRANSCRIPT_TAILED_PROVIDERS` in crates/otto-usage/src/engine/runtime_turns.rs.

import { harnessLabel, KNOWN_AGENT_FAMILIES } from '../../lib/providerRegistry.ts';

/** Providers whose own on-disk records the usage tailer reads. */
export const TRANSCRIPT_TAILED: readonly string[] = ['claude', 'codex', 'grok', 'agy'];

/** A known agent CLI whose TERMINAL sessions Usage cannot measure. */
export function terminalUnmeasured(provider: string): boolean {
  return (KNOWN_AGENT_FAMILIES as readonly string[]).includes(provider) && !TRANSCRIPT_TAILED.includes(provider);
}

/** Display name for a usage row's provider slug (custom slugs stay as is). */
export function usageProviderLabel(provider: string): string {
  return (KNOWN_AGENT_FAMILIES as readonly string[]).includes(provider) ? harnessLabel(provider) : provider;
}

/** "Gemini and Muse tokens count in Threads only: …" for the unmeasured
 *  providers among `providers` (used or installed), in registry order;
 *  `null` when none. */
export function unmeasuredTerminalNote(providers: readonly string[]): string | null {
  const names = KNOWN_AGENT_FAMILIES.filter((p) => providers.includes(p) && terminalUnmeasured(p)).map(harnessLabel);
  if (names.length === 0) return null;
  const list = names.length === 1 ? names[0] : `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
  return `${list} tokens count in Threads only: their terminal sessions are not measured (the CLI keeps no usage record Otto can read).`;
}
