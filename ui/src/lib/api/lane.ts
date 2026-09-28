// Ambient request lane (TRANSPORT_PLAN §4: "pollWhileVisible's run should
// receive a bg client by default, so the helper makes it the rule rather than
// opt-in"). A poll tick runs its `run` inside `inLane('bg', …)`: every `api.*`
// call made in the run's SYNCHRONOUS prefix (the usual `load()` →
// `api.get(…)` / `Promise.all([...])` shape) resolves to the background lane,
// and so does any later call that threads the tick's AbortSignal (the signal
// is tagged). Explicit `api.bg` / `api.long` still win; a plain `api.get`
// outside a tick keeps the interactive lane.
//
// Deliberately import-free: poll.ts and api/client.ts both read it, and the
// unit harness loads each of them in isolation.

export type Lane = 'int' | 'bg' | 'long';

let ambient: Lane | null = null;
const tagged = new WeakMap<AbortSignal, Lane>();

/** Run `fn` with `lane` as the default for requests it issues synchronously. */
export function inLane<T>(lane: Lane, fn: () => T): T {
  const prev = ambient;
  ambient = lane;
  try {
    return fn();
  } finally {
    ambient = prev;
  }
}

/** Requests made with `signal` default to `lane` (a poll tick's signal). */
export function tagSignal(signal: AbortSignal, lane: Lane): void {
  tagged.set(signal, lane);
}

/** The lane a request without an explicit one inherits (ambient scope first,
 *  then its signal's tag), or `null` → the path decides. */
export function inheritedLane(signal?: AbortSignal): Lane | null {
  return ambient ?? (signal ? tagged.get(signal) ?? null : null);
}
