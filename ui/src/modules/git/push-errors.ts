// Pure classifier for the daemon's push refusals (no imports — unit-tested
// under node). The prefixes are otto-git's `PUSH_REJECTED` /
// `PUSH_LEASE_REFUSED` in crates/otto-git/src/local.rs; keep them in sync.

/** `rejected`: the remote has commits the branch lacks (pull, or — after a
 *  deliberate rewrite — force with lease). `lease`: a force-with-lease the
 *  remote refused because the branch moved since it was last fetched and
 *  integrated. `null`: any other failure. Both refusals are 409s. */
export type PushRefusal = 'rejected' | 'lease' | null;

export function pushRefusal(status: number | undefined, message: string): PushRefusal {
  if (status !== 409) return null;
  if (message.includes('force push refused:')) return 'lease';
  if (message.includes('push rejected:')) return 'rejected';
  return null;
}
