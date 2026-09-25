// Load-failure text for the Personal Agents views. The shared store
// (lib/stores/personalAgents.svelte.ts) swallows list-load errors today and
// falls back to an empty list, which would show "No personal agents" for a
// daemon that is down. The views read an optional `<name>Error` string field
// so LoadState can show "Couldn't load …" + Retry as soon as the store exposes
// one (`agentsError`, `roomsError`, `runsError`) — until then this is null and
// the views behave as before.
export function loadErrorOf(store: object, key: string): string | null {
  const v = (store as Record<string, unknown>)[key];
  return typeof v === 'string' && v ? v : null;
}
