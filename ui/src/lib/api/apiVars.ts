// Pure helpers behind the API client's plain-language surface: how a
// `{{variable}}` resolves (for the "variables in this request" line), the
// method/status → tone mapping (one place, like McpPill), collection paths for
// the save sheet, and small URL / cookie parsers. Framework-free so
// `unit/apiVars.test.ts` can exercise it.

import type { ApiCollection, ApiKeyVal } from './types';

/** Built-ins the daemon generates at send time (they always win). */
export const DYNAMIC_VARS: Record<string, string> = {
  $guid: 'a new random UUID for every send',
  $randomUUID: 'a new random UUID for every send',
  $timestamp: 'the current Unix time in seconds',
  $isoTimestamp: 'the current time as ISO 8601',
  $randomInt: 'a random number from 0 to 999',
};

export interface VarEnv {
  name: string;
  variables: Record<string, string>;
  secret_keys: string[];
}

export type VarKind = 'dynamic' | 'session' | 'env' | 'secret' | 'missing';

export interface VarResolution {
  name: string;
  kind: VarKind;
  /** The value that will be sent (plain variables only). */
  value?: string;
  /** One plain-language sentence: where the value comes from. */
  detail: string;
}

const VAR_RE = /\{\{\s*([^{}]+?)\s*\}\}/g;

/** Unique `{{name}}` references across `texts`, in first-seen order. */
export function varNames(...texts: (string | undefined | null)[]): string[] {
  const seen = new Set<string>();
  for (const t of texts) {
    if (!t) continue;
    for (const m of t.matchAll(VAR_RE)) seen.add(m[1]);
  }
  return [...seen];
}

/** Resolve one variable the way the daemon does at send time:
 *  built-ins → session variables → the environment → left as-is. */
export function resolveVar(name: string, runtime: Record<string, string>, env: VarEnv | null): VarResolution {
  if (name in DYNAMIC_VARS) return { name, kind: 'dynamic', detail: `Generated when sent: ${DYNAMIC_VARS[name]}` };
  if (name in runtime) return { name, kind: 'session', value: runtime[name], detail: 'Session variable (set by hand or by a script; overrides the environment)' };
  if (env && name in env.variables) return { name, kind: 'env', value: env.variables[name], detail: `From the “${env.name}” environment` };
  if (env && env.secret_keys.includes(name)) return { name, kind: 'secret', detail: `Secret in “${env.name}”, stored in the macOS Keychain and filled in only when sent` };
  return {
    name,
    kind: 'missing',
    detail: env ? `Not set in “${env.name}”, so it is sent literally as {{${name}}}` : 'No environment is selected, so it is sent literally',
  };
}

/** Split `s` into plain / `{{var}}` segments for the URL highlight layer. */
export function splitVars(s: string): { text: string; isVar: boolean; name?: string }[] {
  const out: { text: string; isVar: boolean; name?: string }[] = [];
  let last = 0;
  for (const m of s.matchAll(VAR_RE)) {
    const i = m.index ?? 0;
    if (i > last) out.push({ text: s.slice(last, i), isVar: false });
    out.push({ text: m[0], isVar: true, name: m[1] });
    last = i + m[0].length;
  }
  if (last < s.length) out.push({ text: s.slice(last), isVar: false });
  return out;
}

/** Every text a request sends, for variable discovery. */
export function requestTexts(d: { url: string; headers: ApiKeyVal[]; query: ApiKeyVal[]; body: string; auth: unknown }): string[] {
  const live = (rows: ApiKeyVal[]) => rows.filter((r) => r.enabled !== false).flatMap((r) => [r.key, r.value]);
  const auth = Object.values((d.auth ?? {}) as Record<string, unknown>).filter((v): v is string => typeof v === 'string');
  return [d.url, ...live(d.query), ...live(d.headers), d.body, ...auth];
}

export type MethodTone = 'get' | 'post' | 'put' | 'delete' | 'other';

/** Method → tone: read (success), create (info), change (warning), destroy (danger). */
export function methodTone(method: string): MethodTone {
  switch (method.toUpperCase()) {
    case 'GET': return 'get';
    case 'POST': return 'post';
    case 'PUT':
    case 'PATCH': return 'put';
    case 'DELETE': return 'delete';
    default: return 'other';
  }
}

export type StatusTone = 'ok' | 'redirect' | 'client' | 'server' | 'none';

export function statusTone(status: number | null | undefined): StatusTone {
  if (status == null || status < 100) return 'none';
  if (status < 300) return 'ok';
  if (status < 400) return 'redirect';
  if (status < 500) return 'client';
  return 'server';
}

/** A short word for the status class, so colour is never the only signal. */
export function statusWord(status: number | null | undefined): string {
  switch (statusTone(status)) {
    case 'ok': return 'Success';
    case 'redirect': return 'Redirect';
    case 'client': return 'Client error';
    case 'server': return 'Server error';
    default: return 'No response';
  }
}

/** Collections as "Parent / Child" paths in tree order (for pickers). */
export function collectionPaths(collections: ApiCollection[]): { id: string; path: string; depth: number }[] {
  const out: { id: string; path: string; depth: number }[] = [];
  const walk = (parent: string | null, prefix: string, depth: number): void => {
    collections
      .filter((c) => (c.parent_id ?? null) === parent)
      .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name))
      .forEach((c) => {
        const path = prefix ? `${prefix} / ${c.name}` : c.name;
        out.push({ id: c.id, path, depth });
        walk(c.id, path, depth + 1);
      });
  };
  walk(null, '', 0);
  return out;
}

/** Host + path of a URL for list rows; falls back to the raw text. */
export function splitUrl(url: string): { host: string; path: string } {
  try {
    const u = new URL(url);
    return { host: u.host, path: `${u.pathname}${u.search}` || '/' };
  } catch {
    return { host: '', path: url };
  }
}

export interface ResponseCookie {
  name: string;
  value: string;
  attributes: string;
}

/** Parse the response's `Set-Cookie` headers. */
export function parseSetCookies(headers: ApiKeyVal[]): ResponseCookie[] {
  return headers
    .filter((h) => h.key.toLowerCase() === 'set-cookie')
    .map((h) => {
      const [pair, ...attrs] = h.value.split(';');
      const eq = pair.indexOf('=');
      return {
        name: (eq < 0 ? pair : pair.slice(0, eq)).trim(),
        value: eq < 0 ? '' : pair.slice(eq + 1).trim(),
        attributes: attrs.map((a) => a.trim()).filter(Boolean).join('; '),
      };
    });
}
