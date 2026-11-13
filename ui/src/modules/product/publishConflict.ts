// A publish of a rewrite is refused with 409 when the Jira issue or the
// Confluence page changed since Otto last synced it (someone edited it there),
// or when Otto could not read back what Jira held after its last publish.
// That is not an error to toast: the person refreshes, reviews the new source
// and publishes again — so the Rewrite tab offers Refresh instead.

/** The daemon's "changed since the last sync" publish refusal, or `null`
 *  for any other failure. Duck-typed on `status` so it stays free of the
 *  API client (the unit test feeds it plain objects). */
export function publishConflictMessage(e: unknown): string | null {
  if (typeof e !== 'object' || e === null) return null;
  const { status, message } = e as { status?: unknown; message?: unknown };
  if (status !== 409 || typeof message !== 'string') return null;
  // "could not confirm": the last publish's read-back failed — a Refresh
  // records what Jira holds and re-arms the check (otto-product publish_guard).
  return /changed in Jira|page changed since Otto last synced|could not confirm what is in Jira/i.test(message) ? message : null;
}
