// Where a Jira / Confluence account's stored token is sent: mirrors
// otto-issues `credential_origin` / `require_token_for_host_change`
// (crates/otto-issues/src/http.rs) so the form asks for a fresh token exactly
// when the daemon will demand one (S17-306) — scheme + host + port, not just
// host:port (`https://x` → `http://x` moves the token too).

/** `scheme://host[:port]` (host lower-cased, default port dropped — the same
 *  tuple as reqwest's `port_or_known_default`), or `null` when unparseable. */
export function credentialOrigin(u: string): string | null {
  try {
    const url = new URL(u.trim());
    if (!url.host) return null;
    return url.origin.toLowerCase();
  } catch {
    return null;
  }
}

/** True when repointing `current` → `next` moves the token to another origin.
 *  Unparseable on either side: only an identical (trimmed) string is the same. */
export function originChanged(current: string, next: string): boolean {
  const a = credentialOrigin(current);
  const b = credentialOrigin(next);
  if (a === null || b === null) return current.trim() !== next.trim();
  return a !== b;
}
