// curl import / export for the API client. Split out of apiClient.svelte.ts
// (arch-6-03): `toCurl` is pure (a draft → a shell command, secrets masked);
// `importCurl` asks the daemon to parse a command and places the result as a
// draft through the store (the store forwards both names).

import { api } from '../../api/client';
import { toasts } from '../../toast.svelte';
import { loadErrorText } from '../../loadError';
import type { ImportCurlReq, ParsedCurl } from '../../api/types';
import { isSecretRef } from '../../api/types';
import type { ApiClientStore } from '../apiClient.svelte';
import { liveKv, secretMasked, type ApiDraft } from './model';

/** The store members `importCurl` reads and calls. */
export type CurlHost = Pick<ApiClientStore, 'workspaceOwner' | 'placeDraft' | 'isDirty' | 'draft'>;

/** Parse a curl command (daemon) and fill the draft with the result. */
export async function importCurl(host: CurlHost, curl: string): Promise<boolean> {
  if (!curl.trim()) return false;
  const current = host.workspaceOwner();
  const req: ImportCurlReq = { curl };
  try {
    const p = await api.post<ParsedCurl>('/api-client/import-curl', req);
    if (!current()) return false;
    // Fills the current tab only while it holds nothing unsaved; otherwise
    // the import opens in a new tab (placeDraft).
    host.placeDraft({
      tabId: crypto.randomUUID(),
      requestId: null,
      name: host.isDirty(host.draft) ? '' : host.draft.name,
      kind: 'http',
      method: p.method,
      url: p.url,
      headers: p.headers,
      query: p.query,
      body_mode: p.body_mode,
      body: p.body,
      auth: p.auth,
      proto: '',
      grpc_method: '',
    });
    toasts.success('Imported curl', `${p.method} ${p.url}`);
    return true;
  } catch (e) {
    if (!current()) return false;
    toasts.error('Couldn’t import curl', loadErrorText(e));
    return false;
  }
}

/** Build a curl command string from a draft (for "Copy as curl"). */
export function toCurl(draft: ApiDraft): string {
  const sh = (s: string): string => `'${s.replace(/'/g, `'\\''`)}'`;
  const parts: string[] = ['curl'];
  if (draft.method && draft.method.toUpperCase() !== 'GET') {
    parts.push('-X', draft.method.toUpperCase());
  }

  // URL with enabled query params appended.
  let url = draft.url;
  const qs = liveKv(draft.query)
    .map((q) => `${encodeURIComponent(q.key)}=${encodeURIComponent(q.value)}`)
    .join('&');
  if (qs) url += (url.includes('?') ? '&' : '?') + qs;
  parts.push(sh(url));

  // Headers.
  for (const h of liveKv(draft.headers)) {
    parts.push('-H', sh(`${h.key}: ${h.value}`));
  }

  // Auth. Keychain-backed members render as `***` — a copied snippet must
  // never leak a resolved secret.
  const a = draft.auth;
  if (a.type === 'bearer' && (isSecretRef(a.token) || a.token)) {
    parts.push('-H', sh(`Authorization: Bearer ${secretMasked(a.token)}`));
  } else if (a.type === 'basic') {
    parts.push('-u', sh(`${a.username}:${secretMasked(a.password)}`));
  } else if (a.type === 'api_key' && a.key) {
    if (a.in === 'header') parts.push('-H', sh(`${a.key}: ${secretMasked(a.value)}`));
    else {
      const sep = url.includes('?') ? '&' : '?';
      // already appended to url above only for query rows; add the api_key here
      parts[parts.indexOf(sh(url))] = sh(
        url + sep + `${encodeURIComponent(a.key)}=${encodeURIComponent(secretMasked(a.value))}`,
      );
    }
  }

  // Body.
  if (draft.body_mode !== 'none' && draft.body.trim()) {
    if (draft.body_mode === 'json') {
      if (!liveKv(draft.headers).some((h) => h.key.toLowerCase() === 'content-type')) {
        parts.push('-H', sh('Content-Type: application/json'));
      }
      parts.push('--data', sh(draft.body));
    } else if (draft.body_mode === 'multipart') {
      // multipart/form-data → -F per field; files as -F key=@filename.
      try {
        const rows = JSON.parse(draft.body) as { key: string; type?: string; value: string; filename?: string }[];
        for (const r of rows) {
          if (!r.key) continue;
          if (r.type === 'file') parts.push('-F', sh(`${r.key}=@${r.filename || 'file'}`));
          else parts.push('-F', sh(`${r.key}=${r.value}`));
        }
      } catch {
        parts.push('--data', sh(draft.body));
      }
    } else if (draft.body_mode === 'form') {
      parts.push('--data', sh(draft.body));
    } else {
      parts.push('--data', sh(draft.body));
    }
  }

  return parts.join(' ');
}
