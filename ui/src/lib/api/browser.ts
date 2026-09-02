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
  BrowserAskReq,
  BrowserCreateCredentialReq,
  BrowserCredential,
  BrowserLoginResp,
  BrowserPage,
  BrowserPatchCredentialReq,
  BrowserQueryResp,
  BrowserRevealCredentialResp,
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

/** Submit one "ask" turn into an agent session: page URL + the given marks
 *  (fenced server-side) + the user's question. */
export function ask(ws: string, body: BrowserAskReq) {
  return api.post<void>(`${base(ws)}/ask`, body);
}

export function vaultSave(ws: string, body: { url: string; vault_id: number; summary?: string }) {
  return api.post<BrowserVaultSaveResp>(`${base(ws)}/vault-save`, body);
}

// ---------------------------------------------------------------------------
// Credentials — keychain-backed site credentials. List/get NEVER carry a
// password (the wire type has no such field); only `revealCredential` does.
// ---------------------------------------------------------------------------

export function listCredentials(ws: string) {
  return api.get<BrowserCredential[]>(`${base(ws)}/credentials`);
}

export function createCredential(ws: string, body: BrowserCreateCredentialReq) {
  return api.post<BrowserCredential>(`${base(ws)}/credentials`, body);
}

export function updateCredential(id: string, body: BrowserPatchCredentialReq) {
  return api.patch<BrowserCredential>(`/browser/credentials/${id}`, body);
}

export function deleteCredential(id: string) {
  return api.del<void>(`/browser/credentials/${id}`);
}

/** Caller must have already confirmed with the user (a confirm dialog) —
 *  this always sends `confirm: true`, which is what makes the server return
 *  the plaintext password instead of a 400. */
export function revealCredential(id: string) {
  return api.post<BrowserRevealCredentialResp>(`/browser/credentials/${id}/reveal`, {
    confirm: true,
  });
}

/** Governed AGENT sign-in (the `browser_login` MCP tool's own HTTP route) —
 *  the server resolves the credential for `domain` (must exist and have
 *  `allow_agent_use: true`) and drives an off-screen CDP fill+submit itself;
 *  the password never travels through this call. NOT what user-triggered
 *  autofill in a live tab should call — that fills the user's own visible
 *  webview via `revealCredential` + `nativeBrowser.eval` instead (see
 *  `BrowserView.svelte`'s `autofill()`), since this route drives a separate,
 *  invisible navigation rather than the tab the user is looking at. */
export function browserLogin(ws: string, domain: string) {
  return api.post<BrowserLoginResp>(`${base(ws)}/login`, { domain });
}
