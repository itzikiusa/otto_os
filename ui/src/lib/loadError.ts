// The one way a failed LIST/PAGE load is described to a person (content.md §4):
// a human cause, never the raw exception as the headline. Load failures render
// INLINE through `LoadState` (with Retry) — a toast is for failed *actions* only,
// and a swallowed load error must never fall through to a "No X yet" empty state.
//
// Pure + dependency-free on purpose (duck-types `ApiError`), so it is unit-tested
// without the fetch client.

interface StatusError {
  status: number;
  message?: string;
}

function isStatusError(e: unknown): e is StatusError {
  return typeof e === 'object' && e !== null && typeof (e as { status?: unknown }).status === 'number';
}

/** Secondary detail line for "Couldn't load X" — the cause, in the user's terms. */
export function loadErrorText(e: unknown): string {
  if (isStatusError(e)) {
    if (e.status === 401) return 'Your session expired. Sign in again, then retry.';
    if (e.status === 403) return 'You don’t have access to this.';
    if (e.status === 404) return 'It no longer exists, or this daemon doesn’t serve it.';
    return e.message ? e.message : `The daemon answered ${e.status}.`;
  }
  // `fetch` rejects with a TypeError when the daemon is unreachable.
  if (e instanceof TypeError) return 'Otto can’t reach the daemon.';
  if (e instanceof Error) return e.message || 'Something went wrong.';
  const s = String(e ?? '');
  return s && s !== 'undefined' ? s : 'Something went wrong.';
}
