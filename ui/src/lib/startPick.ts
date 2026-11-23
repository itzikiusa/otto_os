// The last agent · model · effort a person picked to start an agent, kept
// per scope in localStorage: Threads' start composer (RuntimeStartComposer)
// and Create's planner chip (design-hall/create/SmartHome). Import-free,
// unit-tested in unit/threadsEmbedStart.test.ts.

/** The agent a start composer opened on last time, per scope (Threads: per
 *  workspace; an embed: per host kind — a PR's pick never leaks into
 *  Threads, or Threads' into a PR; Create's planner: per workspace). Access
 *  is NOT remembered: an embed always starts from its host's default, and
 *  Threads from the saved one (Settings › Providers), shown on the chip
 *  before the first send. */
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
