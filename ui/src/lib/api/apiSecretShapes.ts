// Client-side mirror of the daemon's `api_secrets` shapes (which auth members
// and which header/query rows carry credentials, and the history mask), plus
// the two pure transforms the API-client store needs:
//   • stripSecretsForStorage — what an open tab may write to localStorage;
//   • unmaskHistory — how a masked history snapshot is replayed.
// Pure + framework-free so `unit/apiSecretShapes.test.ts` can exercise it.

import type { ApiAuth, ApiKeyVal } from './types';

/** The masked value the daemon writes into history snapshots. */
export const MASK = '***';

/** Secret members per auth type — mirrors `api_secrets::secret_members`. */
export const SECRET_MEMBERS: Record<string, readonly string[]> = {
  bearer: ['token'],
  basic: ['password'],
  api_key: ['value'],
  oauth2: ['client_secret', 'refresh_token', 'password', 'access_token'],
};

/** Header/query keys that carry credentials — mirrors
 *  `api_secrets::sensitive_header_key` (name-shaped + well-known headers). */
export function sensitiveKey(key: string): boolean {
  const k = key.toLowerCase();
  return (
    ['token', 'secret', 'passw', 'apikey', 'api_key', 'api-key', 'authorization', 'credential'].some((s) => k.includes(s)) ||
    ['proxy-authorization', 'cookie', 'set-cookie', 'x-api-key', 'x-auth-token'].includes(k)
  );
}

/** A value that references variables (`Bearer {{token}}`) carries no secret itself. */
function isPlaceholder(v: string): boolean {
  return v.includes('{{');
}

/** The request-shaped parts both transforms work on. */
export interface SecretBearing {
  auth: ApiAuth;
  headers: ApiKeyVal[];
  query: ApiKeyVal[];
}

function sameKey(a: string, b: string): boolean {
  return a.toLowerCase() === b.toLowerCase();
}

/** Copy of `d` safe to write to localStorage: every PLAINTEXT secret (auth
 *  secret members, credential-carrying header/query rows) is replaced by the
 *  saved request's stored form (its Keychain marker / its own row value) when
 *  the tab belongs to one, else by ''. Keychain markers and `{{var}}`
 *  references are kept. Secrets typed into a tab therefore never reach disk
 *  outside the Keychain — they just don't survive a relaunch. */
export function stripSecretsForStorage<T extends SecretBearing>(d: T, saved?: SecretBearing): T {
  const auth = { ...d.auth } as Record<string, unknown>;
  const savedAuth = (saved?.auth ?? { type: 'none' }) as Record<string, unknown>;
  for (const m of SECRET_MEMBERS[d.auth.type] ?? []) {
    const v = auth[m];
    if (typeof v === 'string' && v !== '' && !isPlaceholder(v)) {
      auth[m] = savedAuth.type === d.auth.type && savedAuth[m] !== undefined ? savedAuth[m] : '';
    }
  }
  const rows = (list: ApiKeyVal[], savedRows: ApiKeyVal[] | undefined): ApiKeyVal[] =>
    list.map((r) => {
      if (!sensitiveKey(r.key) || r.value === '' || isPlaceholder(r.value)) return r;
      // The saved request's own row (already persisted daemon-side) is the
      // only form written; an unsaved credential is not.
      const stored = savedRows?.find((s) => sameKey(s.key, r.key));
      return { ...r, value: stored ? stored.value : '' };
    });
  return {
    ...d,
    auth: auth as unknown as ApiAuth,
    headers: rows(d.headers, saved?.headers),
    query: rows(d.query, saved?.query),
  };
}

/** Undo the daemon's history masking for a replay: a `***` secret member or
 *  credential row is refilled from `saved` (the saved request the entry ran,
 *  when it still exists — its Keychain markers resolve server-side), else
 *  blanked (rows also disabled) so the person re-enters it. The literal `***`
 *  is NEVER sent as a credential. `blanked` reports whether anything was. */
export function unmaskHistory(
  snap: SecretBearing,
  saved?: SecretBearing,
): SecretBearing & { blanked: boolean } {
  let blanked = false;
  const a = { ...snap.auth } as Record<string, unknown>;
  const savedAuth = (saved?.auth ?? { type: 'none' }) as Record<string, unknown>;
  for (const m of SECRET_MEMBERS[snap.auth.type] ?? []) {
    if (a[m] !== MASK) continue;
    if (savedAuth.type === snap.auth.type && savedAuth[m] !== undefined && savedAuth[m] !== MASK) {
      a[m] = savedAuth[m];
    } else {
      a[m] = '';
      blanked = true;
    }
  }
  const rows = (list: ApiKeyVal[], savedRows: ApiKeyVal[] | undefined): ApiKeyVal[] =>
    list.map((r) => {
      if (r.value !== MASK) return { ...r };
      const stored = savedRows?.find((s) => sameKey(s.key, r.key));
      if (stored && stored.value !== MASK) return { ...r, value: stored.value };
      blanked = true;
      return { ...r, value: '', enabled: false };
    });
  const headers = rows(snap.headers, saved?.headers);
  const query = rows(snap.query, saved?.query);
  return { auth: a as unknown as ApiAuth, headers, query, blanked };
}

/** Copy of a draft for "Duplicate": Keychain markers belong to the ORIGINAL
 *  request's Keychain item, so they are blanked (the person re-enters them, or
 *  keeps using `{{variables}}`); plaintext and `{{var}}` values are kept.
 *  `blanked` reports whether anything was cleared. */
export function forDuplicate<T extends SecretBearing>(d: T): T & { blanked: boolean } {
  let blanked = false;
  const auth = { ...d.auth } as Record<string, unknown>;
  for (const m of SECRET_MEMBERS[d.auth.type] ?? []) {
    const v = auth[m];
    if (typeof v === 'object' && v !== null && typeof (v as { $secret?: unknown }).$secret === 'string') {
      auth[m] = '';
      blanked = true;
    }
  }
  return {
    ...d,
    auth: auth as unknown as ApiAuth,
    headers: d.headers.map((r) => ({ ...r })),
    query: d.query.map((r) => ({ ...r })),
    blanked,
  };
}
