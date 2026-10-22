// Resources-table auto-refresh cadence (perf review K2). A small list re-reads
// every 10 s; a big one (≥ 1000 rows — a multi-MB `kubectl get -o json`) waits
// at least 30 s and never less than 3× the last load took, so a slow cluster
// is never asked again before it has had time to recover.
//
// Import-free so the unit harness (`node --test`) can load it directly.

export const RESOURCES_POLL_MS = 10_000;
export const RESOURCES_BIG_LIST_ROWS = 1000;
export const RESOURCES_BIG_POLL_MS = 30_000;

/** The delay before the next resources poll, given the last list's row count
 *  and how long that load took (ms; 0 / unknown ⇒ just the floor). */
export function resourcesPollMs(rows: number, lastLoadMs: number): number {
  if (rows < RESOURCES_BIG_LIST_ROWS) return RESOURCES_POLL_MS;
  const load = Number.isFinite(lastLoadMs) && lastLoadMs > 0 ? lastLoadMs : 0;
  return Math.max(RESOURCES_BIG_POLL_MS, Math.round(3 * load));
}

/** The `If-None-Match` validator for a resources response — its `ETag` header
 *  when the daemon exposes it (CORS), else the body's `version` quoted the way
 *  the daemon quotes its ETag; null when neither is present (an older daemon:
 *  every poll is then a plain full read). */
export function resourcesValidator(etag: string | null, version: string | undefined): string | null {
  if (etag) return etag;
  return version ? `"${version}"` : null;
}
