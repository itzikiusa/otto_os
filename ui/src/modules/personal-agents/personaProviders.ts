// Which agent CLIs can run a personal agent. A persona (its instructions,
// rules, team and personality) reaches the CLI as a context file handed over
// for one launch (pi: `--append-system-prompt` + `--skill`); Gemini (and a
// custom CLI) have no such mechanism, so the daemon refuses them (`otto_context::materialize::delivers_context`,
// `personal_agents_engine::persona_provider`). Keep the list in step.

import { harnessLabel } from '../../lib/providers';

export const PERSONA_PROVIDERS: readonly string[] = ['claude', 'codex', 'agy', 'grok', 'opencode', 'pi'];

/** Empty = the daemon's default, Claude. */
export function canCarryPersona(provider: string | null | undefined): boolean {
  return PERSONA_PROVIDERS.includes((provider ?? '').trim() || 'claude');
}

/** The registered providers a personal agent may pick. */
export function personaProviders(registered: string[]): string[] {
  return registered.filter(canCarryPersona);
}

/** A provider to start a new agent on: the configured default when it can
 *  carry a persona, else Claude. */
export function personaDefault(preferred: string): string {
  return canCarryPersona(preferred) ? preferred : 'claude';
}

/** Why `provider` is not offered — shown under the picker for an agent saved
 *  on one. */
export function personaRefusal(provider: string): string {
  const names = PERSONA_PROVIDERS.map(harnessLabel).join(', ');
  return `${harnessLabel(provider.trim() || 'custom')} can’t load a persona for one launch, so its runs would start without this agent’s instructions. Pick ${names}.`;
}
