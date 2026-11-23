// OAuth2 token acquisition for the API Client request builder, split out of
// RequestBuilder.svelte so the polling loop is abortable and unit-testable.
import { api, isAbortError, newHostConfirmHost } from '../../lib/api/client';
import { openExternal } from '../../lib/external';
import { apiClient, confirmNewHost, type ApiDraft } from '../../lib/stores/apiClient.svelte';
import type { ApiRequest, ApiSecretable } from '../../lib/api/types';

export function sleepAbortable(ms: number, signal: AbortSignal): Promise<void> {
  if (signal.aborted) return Promise.reject(new DOMException('Aborted', 'AbortError'));
  return new Promise((resolve, reject) => {
    const onAbort = (): void => {
      clearTimeout(t);
      reject(new DOMException('Aborted', 'AbortError'));
    };
    const t = setTimeout(() => {
      signal.removeEventListener('abort', onAbort);
      resolve();
    }, ms);
    signal.addEventListener('abort', onAbort, { once: true });
  });
}

export type OAuthFetchOutcome =
  | { kind: 'cancelled' }
  | { kind: 'signed_in' }
  | { kind: 'token'; access_token: string; token_type: string; refresh_token: ApiSecretable; expires_in?: number };

export async function runOAuthTokenFlow(opts: {
  wid: string;
  draft: ApiDraft;
  signal: AbortSignal;
  isCurrent: () => boolean;
}): Promise<OAuthFetchOutcome> {
  const { wid, draft, signal, isCurrent } = opts;
  if (draft.auth.type !== 'oauth2') return { kind: 'cancelled' };
  const a = draft.auth;
  if (a.grant === 'authorization_code') {
    const colId = apiClient.requests.find((r) => r.id === draft.requestId)?.collection_id ?? null;
    const saved = await apiClient.saveDraft(draft.name || 'OAuth request', colId);
    if (signal.aborted || !saved || !isCurrent()) return { kind: 'cancelled' };
    const flow = await api.post<{ flow_id: string; authorization_url: string; redirect_uri: string }>(
      `/workspaces/${wid}/api-client/oauth2/authorize`,
      { request_id: saved.id },
      signal,
    );
    if (signal.aborted || !isCurrent()) return { kind: 'cancelled' };
    await openExternal(flow.authorization_url);
    const deadline = Date.now() + 600_000;
    while (!signal.aborted && Date.now() < deadline) {
      await sleepAbortable(1000, signal);
      if (signal.aborted || !isCurrent()) return { kind: 'cancelled' };
      const result = await api.get<{ status: string; error?: string }>(
        `/workspaces/${wid}/api-client/oauth2/flows/${flow.flow_id}`,
        signal,
      );
      if (signal.aborted || !isCurrent()) return { kind: 'cancelled' };
      if (result.status === 'failed') throw new Error(result.error || 'Authorization failed');
      if (result.status === 'completed') {
        const request = await api
          .get<ApiRequest>(`/workspaces/${wid}/api-client/requests/${encodeURIComponent(saved.id)}`, signal)
          .catch(() => null);
        if (signal.aborted || !isCurrent()) return { kind: 'cancelled' };
        if (request) {
          apiClient.applySavedAuth(wid, request, saved.auth);
          return { kind: 'signed_in' };
        }
        return { kind: 'cancelled' };
      }
    }
    return { kind: 'cancelled' };
  }

  type TokenResp = { access_token: string; token_type?: string; refresh_token?: string; expires_in?: number };
  const tokenReq = {
    grant: a.grant,
    token_url: a.token_url,
    client_id: a.client_id,
    client_secret: a.client_secret,
    scope: a.scope,
    username: a.username,
    password: a.password,
    refresh_token: a.refresh_token,
  };
  let res: TokenResp;
  try {
    res = await api.post<TokenResp>(`/workspaces/${wid}/api-client/oauth2/token`, tokenReq, signal);
  } catch (e) {
    if (signal.aborted || isAbortError(e)) return { kind: 'cancelled' };
    const host = newHostConfirmHost(e);
    if (
      host === null ||
      !(await confirmNewHost(host, {
        method: 'POST',
        url: a.token_url,
        secrets: ['the saved OAuth client secret / credentials (Keychain)'],
      }))
    ) {
      throw e;
    }
    if (signal.aborted || !isCurrent()) return { kind: 'cancelled' };
    res = await api.post<TokenResp>(
      `/workspaces/${wid}/api-client/oauth2/token`,
      { ...tokenReq, confirm_new_host: true },
      signal,
    );
  }
  if (signal.aborted || !isCurrent()) return { kind: 'cancelled' };
  return {
    kind: 'token',
    access_token: res.access_token,
    token_type: res.token_type || 'Bearer',
    refresh_token: res.refresh_token || a.refresh_token,
    expires_in: res.expires_in,
  };
}
