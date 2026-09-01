// Browser API — reader/annotate tabs + on-demand page fetch.
// Mirrors docs/contracts/api.md "Browser (reader/annotate tabs + on-demand
// page fetch)" + crates/otto-server/src/routes/browser.rs.
//
// Tab/annotation list+create and the page/query/summarize/vault-save fetches
// are workspace-scoped (`/workspaces/{wid}/browser/...`); tab/annotation
// mutation-by-id (PATCH/DELETE/send) are not (`/browser/tabs/{id}`,
// `/browser/annotations/{id}`) — matches the route table exactly.

import { api } from './client';
import type {
  BrowserAnnotation,
  BrowserPage,
  BrowserQueryResp,
  BrowserSummarizeResp,
  BrowserTab,
  BrowserVaultSaveResp,
} from './types';

const base = (ws: string) => `/workspaces/${ws}/browser`;
const enc = encodeURIComponent;

export function listTabs(ws: string) {
  return api.get<BrowserTab[]>(`${base(ws)}/tabs`);
}

export function createTab(ws: string, url: string) {
  return api.post<BrowserTab>(`${base(ws)}/tabs`, { url });
}

export function navigateTab(
  id: string,
  body: { url?: string; title?: string; mode?: 'reader' | 'live' },
) {
  return api.patch<BrowserTab>(`/browser/tabs/${id}`, body);
}

export function closeTab(id: string) {
  return api.del<void>(`/browser/tabs/${id}`);
}

export function getPage(ws: string, url: string) {
  return api.get<BrowserPage>(`${base(ws)}/page?url=${enc(url)}`);
}

export function queryPage(ws: string, url: string, selector: string) {
  return api.get<BrowserQueryResp>(
    `${base(ws)}/query?url=${enc(url)}&selector=${enc(selector)}`,
  );
}

export function listAnnotations(ws: string, url?: string) {
  const qs = url ? `?url=${enc(url)}` : '';
  return api.get<BrowserAnnotation[]>(`${base(ws)}/annotations${qs}`);
}

export function createAnnotation(
  ws: string,
  body: {
    url: string;
    selector: string;
    excerpt: string;
    text: string;
    comment?: string;
    color?: string;
    tab_id?: string;
  },
) {
  return api.post<BrowserAnnotation>(`${base(ws)}/annotations`, body);
}

export function updateAnnotation(id: string, comment: string) {
  return api.patch<BrowserAnnotation>(`/browser/annotations/${id}`, { comment });
}

export function deleteAnnotation(id: string) {
  return api.del<void>(`/browser/annotations/${id}`);
}

export function summarize(ws: string, url: string) {
  return api.post<BrowserSummarizeResp>(`${base(ws)}/summarize`, { url });
}

export function sendAnnotation(ws: string, id: string, sessionId: string) {
  return api.post<void>(`${base(ws)}/annotations/${id}/send`, { session_id: sessionId });
}

export function vaultSave(ws: string, body: { url: string; vault_id: number; summary?: string }) {
  return api.post<BrowserVaultSaveResp>(`${base(ws)}/vault-save`, body);
}
