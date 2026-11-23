// Production environment detection & mutation classification for the API
// Client. Pure (no Svelte/store imports) so node unit tests cover it directly.

const NON_PROD_RE = /\b(?:pre[-_\s]?prod|non[-_\s]?prod|staging|stage|stg|uat|dev|local|test)\b/i;
const PROD_TOKEN_RE = /(?:^|[\s\-_/.:])(prod|production|prd|live)(?:$|[\s\-_/.:])/i;
const SAFE_HTTP_METHODS = new Set(['GET', 'HEAD', 'OPTIONS']);

/** True when an API environment name denotes a production target (e.g.
 *  "Production", "prod", "API Prod", "payments-prod"), while excluding
 *  "preprod" / "non-prod" / "staging". */
export function isProdApiEnv(name: string | null | undefined): boolean {
  const s = (name ?? '').trim();
  if (!s) return false;
  if (NON_PROD_RE.test(s)) return false;
  return PROD_TOKEN_RE.test(s);
}

/** True when sending this API request can mutate remote state: any non-safe
 *  HTTP / SSE verb (`POST`, `PUT`, `PATCH`, `DELETE`, …) or any gRPC call. */
export function isMutatingApiCall(draft: { kind: string; method?: string | null }): boolean {
  if (draft.kind === 'grpc') return true;
  if (draft.kind === 'websocket') return false;
  const method = (draft.method ?? 'GET').trim().toUpperCase();
  return !SAFE_HTTP_METHODS.has(method);
}
