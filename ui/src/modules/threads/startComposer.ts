// The start composer's pure half (RuntimeStartComposer.svelte): what one
// launch carries to its host (Threads' new-thread stage, an embed's draft
// state), which agents it offers, which it starts on, and the plain words for
// an access mode. Import-free (types only), unit-tested in
// unit/threadsStartComposer.test.ts.
import type { RuntimeAccessMode } from '../../lib/api/types';
import type { EffortLevel, InteractionMode } from '../../lib/providerModels';
import type { ComposerFile } from '../../lib/stores/runtimeDrafts.svelte';

/** One "start" from the composer. The host does the starting. */
export interface StartLaunch {
  provider: string;
  /** The provider runs a structured runtime thread (else a terminal session). */
  runtime: boolean;
  /** Ask several agents (Threads only). */
  council: boolean;
  /** ⌘↵: a council's seats start in the background instead of opening in panes. */
  background: boolean;
  model: string | null;
  /** Null when the provider takes no effort. */
  effort: EffortLevel | null;
  interactionMode: InteractionMode;
  /** The opening turn in the agent's plan mode. */
  plan: boolean;
  /** False: ⌘↵ with an empty box — an empty thread. */
  withPrompt: boolean;
  /** The first message: the text plus its `@` context ('' without a prompt). */
  message: string;
  /** What a placeholder title is made from. */
  titleSeed: string;
  attachments: ComposerFile[];
  /** The access chip's mode (null where the composer shows none). */
  accessMode: RuntimeAccessMode | null;
  /** The message came from the host (a suggestion, a menu ask): it is whole,
   *  context included — a host transform of typed text must not run on it. */
  raw: boolean;
}

/** What the composer's `below` snippet gets (Threads' meta row). */
export interface StartMeta {
  modeSupported: boolean;
  /** The access chip is on the composer (a structured agent, not a council). */
  accessShown: boolean;
  planSupported: boolean;
  interactionMode: InteractionMode;
  setMode: (m: InteractionMode) => void;
  openModeMenu: (anchor?: HTMLElement) => void;
}

/** The agents the composer offers: every harness in Threads; only those that
 *  run a structured thread AND are installed in an embed (an embed never
 *  falls back to one that can't start). */
export function startHarnesses(
  ordered: readonly string[],
  runtimeOnly: boolean,
  isRuntime: (p: string) => boolean,
  ready: (p: string) => boolean,
): string[] {
  return runtimeOnly ? ordered.filter((p) => isRuntime(p) && ready(p)) : [...ordered];
}

/** The agent a fresh composer starts on: the preferred one when offered and
 *  ready, else the first ready, else the first offered ('' when none). */
export function initialProvider(harnesses: readonly string[], preferred: string, ready: (p: string) => boolean): string {
  if (harnesses.includes(preferred) && ready(preferred)) return preferred;
  return harnesses.find(ready) ?? harnesses[0] ?? '';
}

/** The same words the access chip uses (RuntimeAccessChip.svelte), as a
 *  sentence for a footer: what the agent may do without asking. */
export const ACCESS_HINT: Readonly<Record<RuntimeAccessMode, string>> = {
  'approval-required': 'Supervised: it asks before running commands or changing files.',
  'auto-accept-edits': 'Auto-accept edits: it changes files without asking, and asks before other actions.',
  'full-access': 'Full access: it runs commands and changes files without asking.',
};

/** How permissive a mode is (a pick above the host's default may be confirmed). */
export const ACCESS_RANK: Readonly<Record<RuntimeAccessMode, number>> = {
  'approval-required': 0,
  'auto-accept-edits': 1,
  'full-access': 2,
};

/** The agent a start composer opened on last time, per scope (Threads: per
 *  workspace; an embed: per host kind — a PR's pick never leaks into
 *  Threads, or Threads' into a PR). Access is NOT remembered: an embed always
 *  starts from its host's default, and Threads from the saved one (Settings ›
 *  Providers), shown on the chip before the first send. */
export interface StartPick {
  provider: string;
  model: string | null;
  effort: string | null;
}

/** The localStorage key of a scope's pick. */
export const startPickKey = (scope: string): string => `otto_start_pick:${scope}`;

type PickStore = Pick<Storage, 'getItem' | 'setItem'>;

/** The scope's remembered pick, if it still names an agent offered and
 *  installed now (else null: the default applies). Never throws. */
export function readStartPick(
  store: PickStore | null | undefined,
  scope: string,
  harnesses: readonly string[],
  ready: (p: string) => boolean,
): StartPick | null {
  let v: unknown;
  try {
    v = JSON.parse(store?.getItem(startPickKey(scope)) ?? 'null');
  } catch {
    return null;
  }
  if (!v || typeof v !== 'object') return null;
  const o = v as Record<string, unknown>;
  const provider = typeof o.provider === 'string' ? o.provider : '';
  if (!provider || !harnesses.includes(provider) || !ready(provider)) return null;
  return {
    provider,
    model: typeof o.model === 'string' && o.model ? o.model : null,
    effort: typeof o.effort === 'string' && o.effort ? o.effort : null,
  };
}

/** Keep the scope's pick (storage full or blocked: not remembered). */
export function writeStartPick(store: PickStore | null | undefined, scope: string, pick: StartPick): void {
  try {
    store?.setItem(startPickKey(scope), JSON.stringify(pick));
  } catch {
    /* not remembered: the default applies next time */
  }
}
