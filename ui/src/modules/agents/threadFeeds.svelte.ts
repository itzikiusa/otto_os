// The structured threads the Agents inbox lists (the Threads runtime store),
// loaded once per app load in their own chunk, after first paint: the list is
// read and its live feeds (and the MCP asks') started, then events and a
// reconnect's resync keep it current. A failed read is retried by the next
// caller, and an identity change (sign-out / sign-in) reads again.
import { getToken } from '../../lib/api/client';

type Modules = [
  typeof import('../threads/runtime/runtimeThreads.svelte'), // ui-guards: allow — the inbox lists the Threads store's rows (one store)
  typeof import('../threads/threadNav.svelte'), // ui-guards: allow — their state, the Threads way (one derivation)
  typeof import('../threads/threadInbox.svelte'), // ui-guards: allow — the MCP asks (one store)
];

/** `settled`: the first list read has answered (either way) — the first-run coach waits for it. */
export const threadFeeds = $state({ settled: false });
let mods: Promise<Modules> | null = null;
let listedFor: string | null | undefined;
let listing: Promise<void> | null = null;

export function startThreadFeeds(): Promise<Modules> {
  mods ??= Promise.all([
    import('../threads/runtime/runtimeThreads.svelte'), // ui-guards: allow — see Modules
    import('../threads/threadNav.svelte'), // ui-guards: allow — see Modules
    import('../threads/threadInbox.svelte'), // ui-guards: allow — see Modules
  ]).then((m) => {
    m[0].runtimeThreads.mount();
    m[2].inboxLive.mount();
    return m;
  });
  return mods.then((m) => {
    const token = getToken();
    if (listedFor !== token && !listing) {
      listing = m[0].runtimeThreads.loadAll().then(
        () => void (listedFor = token),
        () => {},
      ).finally(() => {
        listing = null;
        threadFeeds.settled = true;
      });
    }
    return m;
  });
}
