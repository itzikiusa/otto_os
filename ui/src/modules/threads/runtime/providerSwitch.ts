// "Continue with another provider": the thread's next message runs on
// another provider, handed the history (daemon runtime/fresh.rs, handoff.rs;
// after T3 Code's provider switch, MIT, Copyright (c) 2026 T3 Tools Inc.,
// apps/web/src/components/chat/V2LifecycleRow.tsx). Arming changes nothing
// until a message is accepted. Pure (unit/runtimeProviderSwitch.test.ts).
import type { ContextHandoff, RuntimeDraftContent } from '../../../lib/api/types.ts';
import { harnessLabel } from '../../../lib/providerRegistry.ts';

/** The picker's providers: the thread's own first, then every other one a
 *  thread can run on now (`switchable`: a structured, installed provider). */
export function switchTargets(home: string, all: readonly string[], switchable: (p: string) => boolean): string[] {
  return [home, ...all.filter((p) => p !== home && switchable(p))];
}

/** The quiet line above the composer while a switch is armed. */
export function armedSwitchLine(to: string): string {
  return `Next message continues on ${harnessLabel(to)}`;
}

/** What a send adds for an armed switch: the provider, and the new
 *  provider's model (`null` = its default: none sent) and effort. */
export function switchPromptFields(sent: Pick<RuntimeDraftContent, 'provider' | 'model' | 'effort'>, home: string): { provider?: string; model?: string; effort?: string } {
  const to = sent.provider?.trim();
  if (!to || to === home) return {};
  return { provider: to, ...(sent.model ? { model: sent.model } : {}), ...(sent.effort ? { effort: sent.effort } : {}) };
}

/** The divider: "Context handoff · Claude Code → Codex". */
export function handoffTitle(h: ContextHandoff): string {
  return `Context handoff · ${harnessLabel(h.from_provider)} → ${harnessLabel(h.to_provider)}`;
}

/** What was handed over, in words (the divider's expanded line). */
export function handoffSummary(h: ContextHandoff): string {
  const n = (k: number, one: string, many: string): string => `${k} ${k === 1 ? one : many}`;
  if (h.turns === 0 && h.omitted_turns === 0) return 'Nothing to hand over yet: the thread had no messages.';
  const parts = [`${n(h.turns, 'turn', 'turns')} handed over`];
  if (h.omitted_turns > 0) parts.push(`${n(h.omitted_turns, 'earlier turn', 'earlier turns')} left out (still readable above)`);
  if (h.images_omitted > 0) parts.push(`${n(h.images_omitted, 'image', 'images')} not handed over`);
  return `${parts.join(' · ')}.`;
}
