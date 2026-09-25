// Agent UI control — API client handlers (`otto.ui_api_*`). Every handler
// drives the SAME store the request builder uses (stores/apiClient.svelte.ts),
// in the document that shows the API module, so what the agent edits or sends
// is the tab the user is looking at.
//
// Risk (docs/contracts/ui-commands.json): reading and editing tabs is `read` /
// `navigate` (nothing leaves the Mac, nothing is persisted but the device-local
// tab set); saving a request is `local_write` (an attributed confirm the user
// may remember for the session); SENDING is `outward` — it always confirms,
// saying where it goes and what is sent, except a loopback target, which is a
// rememberable `local_write` confirm (nothing leaves the Mac). The daemon's own
// new-host secret confirm (409 needs_confirm) still applies on top.

import { registerUiCommands, registerUiState, UiCommandError, type UiCommandCtx } from '../uiCommands';
import { apiClient, draftSecrets, HTTP_METHODS, type ApiDraft } from '../stores/apiClient.svelte';
import { ws } from '../stores/workspace.svelte';
import type { ApiBodyMode, ApiKeyVal, ApiResponse } from '../api/types';
import { asUiError, capList, highlightWhenReady, resolveByIdOrName, waitFor } from './pagePort';

/** Largest response body returned to the agent by default (chars). */
const BODY_MAX = 64 * 1024;
const BODY_MODES: ApiBodyMode[] = ['none', 'json', 'raw', 'form', 'multipart', 'graphql'];
const URL_INPUT = 'input[aria-label="Request URL"]';

/** Workspace open, this document's API client loaded (tabs restored). */
async function ready(signal: AbortSignal): Promise<void> {
  if (!ws.currentId) throw new UiCommandError('failed', 'No workspace is open in this Otto window.');
  await apiClient.ensureLoaded();
  await waitFor(() => !apiClient.loading, signal, 15_000, 'the API client to load');
}

/** Index of `tabId` among the open tabs (the active tab when omitted). */
function tabIndex(tabId: unknown): number {
  if (tabId === undefined || tabId === null || tabId === '') return apiClient.activeTab;
  const i = apiClient.tabs.findIndex((t) => t.tabId === tabId);
  if (i < 0) throw new UiCommandError('not_found', `No open request tab “${String(tabId)}” — otto.ui_api_state lists them.`);
  return i;
}

/** Bring tab `i` and the request editor to the front. */
function focusTab(i: number): void {
  if (i !== apiClient.activeTab) apiClient.switchTab(i);
  apiClient.showRequestView();
}

function tabSummary(d: ApiDraft, i: number) {
  return {
    tab_id: d.tabId ?? null,
    label: apiClient.tabLabel(d),
    method: d.method,
    url: d.url,
    kind: d.kind,
    active: i === apiClient.activeTab,
    unsaved: apiClient.isDirty(d),
    request_id: d.requestId,
  };
}

function kvArg(rows: unknown, what: string): ApiKeyVal[] {
  if (!Array.isArray(rows)) throw new UiCommandError('invalid_args', `\`${what}\` must be an array of {key, value}.`);
  return rows.map((r) => {
    const o = r as { key?: unknown; value?: unknown; enabled?: unknown };
    if (typeof o?.key !== 'string' || typeof o?.value !== 'string') {
      throw new UiCommandError('invalid_args', `Every \`${what}\` entry needs a string key and value.`);
    }
    return { key: o.key, value: o.value, enabled: typeof o.enabled === 'boolean' ? o.enabled : true };
  });
}

/** The URL as it will be sent: plain `{{variables}}` filled in from the
 *  runtime vars and the active environment (secret ones stay as written). */
function shownUrl(url: string): string {
  const env = apiClient.activeEnv;
  const vars = apiClient.runtimeVars;
  return url.replace(/\{\{\s*([^{}]+?)\s*\}\}/g, (m, n: string) => vars[n] ?? env?.variables[n] ?? m);
}

function hostOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^\[|\]$/g, '').toLowerCase();
  } catch {
    return '';
  }
}

/** A loopback target — the request never leaves this Mac. */
export function isLoopbackHost(host: string): boolean {
  return host === 'localhost' || host.endsWith('.localhost') || host === '::1' || /^127(\.\d{1,3}){3}$/.test(host);
}

function preview(s: string, max = 300): string {
  const t = s.trim();
  return t.length > max ? `${t.slice(0, max - 1)}…` : t;
}

function responseResult(resp: ApiResponse, max = BODY_MAX) {
  const body = resp.body ?? '';
  return {
    status: resp.status,
    status_text: resp.status_text,
    duration_ms: resp.duration_ms,
    size_bytes: resp.size_bytes,
    content_type: resp.content_type,
    headers: resp.headers.slice(0, 100).map((h) => ({ key: h.key, value: h.value })),
    body: body.length > max ? body.slice(0, max) : body,
    body_truncated: body.length > max || resp.truncated || resp.too_large,
    tests: apiClient.testResults.map((t) => ({ name: t.name, passed: t.passed, error: t.error ?? null })),
    script_logs: apiClient.scriptLogs.slice(-50),
  };
}

registerUiState('api', () => ({
  active_tab_id: apiClient.draft?.tabId ?? null,
  tabs: apiClient.tabs.map((d, i) => ({ tab_id: d.tabId ?? null, label: apiClient.tabLabel(d), active: i === apiClient.activeTab })),
  active_environment: apiClient.activeEnv?.name ?? null,
  sending: apiClient.sending,
  response_status: apiClient.lastResponse?.status ?? null,
}));

registerUiCommands('api', {
  async api_state(args: { query?: string }, ctx: UiCommandCtx) {
    await ready(ctx.signal);
    const q = (args.query ?? '').trim().toLowerCase();
    const reqs = apiClient.requests
      .filter((r) => !q || r.name.toLowerCase().includes(q) || r.url.toLowerCase().includes(q))
      .map((r) => ({ id: r.id, name: r.name, method: r.method, url: r.url, collection_id: r.collection_id }));
    return {
      workspace_id: ws.currentId,
      tabs: apiClient.tabs.map(tabSummary),
      active_tab_id: apiClient.draft?.tabId ?? null,
      active_environment: apiClient.activeEnv ? { id: apiClient.activeEnv.id, name: apiClient.activeEnv.name } : null,
      environments: apiClient.environments.map((e) => ({ id: e.id, name: e.name, active: e.is_active })),
      collections: apiClient.collections.map((c) => ({ id: c.id, name: c.name, parent_id: c.parent_id })),
      requests: capList(reqs),
    };
  },

  async api_open_request(args: { request_id: string }, ctx: UiCommandCtx) {
    await ready(ctx.signal);
    const r = resolveByIdOrName(apiClient.requests, args.request_id, (x) => x.id, (x) => x.name, 'saved request');
    apiClient.loadRequestIntoDraft(r);
    apiClient.showRequestView();
    await highlightWhenReady(ctx, URL_INPUT);
    return { tab_id: apiClient.draft.tabId ?? null, request_id: r.id, name: r.name, method: r.method, url: r.url };
  },

  async api_set_draft(
    args: {
      tab_id?: string;
      new_tab?: boolean;
      name?: string;
      method?: string;
      url?: string;
      headers?: unknown;
      query?: unknown;
      body_mode?: string;
      body?: string;
    },
    ctx: UiCommandCtx,
  ) {
    await ready(ctx.signal);
    const method = args.method?.toUpperCase();
    if (method !== undefined && !HTTP_METHODS.includes(method)) {
      throw new UiCommandError('invalid_args', `Unknown method “${args.method}”.`);
    }
    if (args.body_mode !== undefined && !BODY_MODES.includes(args.body_mode as ApiBodyMode)) {
      throw new UiCommandError('invalid_args', `Unknown body_mode “${args.body_mode}”.`);
    }
    const headers = args.headers === undefined ? undefined : kvArg(args.headers, 'headers');
    const query = args.query === undefined ? undefined : kvArg(args.query, 'query');
    if (args.new_tab) {
      if (args.tab_id) throw new UiCommandError('invalid_args', 'Pass either `tab_id` or `new_tab`, not both.');
      apiClient.newDraft();
      apiClient.showRequestView();
    } else {
      focusTab(tabIndex(args.tab_id));
    }
    const d = apiClient.draft;
    apiClient.draft = {
      ...d,
      ...(args.name !== undefined ? { name: args.name } : {}),
      ...(method !== undefined ? { method } : {}),
      ...(args.url !== undefined ? { url: args.url.trim() } : {}),
      ...(headers !== undefined ? { headers } : {}),
      ...(query !== undefined ? { query } : {}),
      ...(args.body_mode !== undefined ? { body_mode: args.body_mode as ApiBodyMode } : {}),
      ...(args.body !== undefined ? { body: args.body } : {}),
    };
    await highlightWhenReady(ctx, URL_INPUT);
    const now = apiClient.draft;
    return { tab_id: now.tabId ?? null, method: now.method, url: now.url, unsaved: apiClient.isDirty(now) };
  },

  async api_import_curl(args: { curl: string }, ctx: UiCommandCtx) {
    await ready(ctx.signal);
    if (!args.curl?.trim()) throw new UiCommandError('invalid_args', '`curl` is empty.');
    const ok = await apiClient.importCurl(args.curl);
    if (!ok) throw new UiCommandError('failed', 'The curl command could not be parsed.');
    apiClient.showRequestView();
    await highlightWhenReady(ctx, URL_INPUT);
    const d = apiClient.draft;
    return { tab_id: d.tabId ?? null, method: d.method, url: d.url };
  },

  async api_select_env(args: { environment_id: string }, ctx: UiCommandCtx) {
    await ready(ctx.signal);
    const env = resolveByIdOrName(apiClient.environments, args.environment_id, (e) => e.id, (e) => e.name, 'environment');
    await apiClient.activateEnvironment(env.id);
    if (apiClient.activeEnv?.id !== env.id) throw new UiCommandError('failed', `Couldn't activate “${env.name}”.`);
    return { environment_id: env.id, name: env.name };
  },

  async api_send(args: { tab_id?: string }, ctx: UiCommandCtx) {
    await ready(ctx.signal);
    focusTab(tabIndex(args.tab_id));
    const d = apiClient.draft;
    if (!d.url.trim()) throw new UiCommandError('invalid_args', 'The request has no URL — set one with otto.ui_api_set_draft.');
    if ((d.kind ?? 'http') !== 'http') {
      throw new UiCommandError('invalid_args', `Only HTTP requests can be sent this way (this tab is ${d.kind}).`);
    }
    const url = shownUrl(d.url);
    const host = hostOf(url);
    const local = host !== '' && isLoopbackHost(host);
    const env = apiClient.activeEnv;
    const secrets = draftSecrets(d, env);
    const what = [
      `${d.method} ${url}`,
      d.body_mode !== 'none' && d.body.trim() ? `Body (${d.body_mode}): ${preview(d.body)}` : '',
      env ? `Environment: ${env.name}` : '',
      secrets.length ? `Stored secrets sent: ${secrets.join(', ')}` : '',
      local ? '' : `Who sees it: the server at ${host || 'that URL'} receives this request.`,
    ]
      .filter(Boolean)
      .join('\n\n');
    await highlightWhenReady(ctx, URL_INPUT);
    const ok = await ctx.confirmWrite({
      what,
      where: host || 'the request URL',
      verb: 'Send',
      outward: !local,
      connId: local ? `api:${host}` : undefined,
    });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined to send the request.');
    const tabId = d.tabId;
    const stop = (): void => {
      if (apiClient.sending && apiClient.draft.tabId === tabId) apiClient.cancelExecute();
    };
    ctx.signal.addEventListener('abort', stop, { once: true });
    ctx.progress(`Sending ${d.method} ${host || url}`);
    try {
      const resp = await apiClient.execute();
      if (!resp) {
        if (apiClient.lastError) throw new UiCommandError('failed', apiClient.lastError);
        throw new UiCommandError('cancelled_by_user', 'The send was cancelled (a new-host secret confirm was declined, or it was stopped).');
      }
      return responseResult(resp);
    } catch (e) {
      throw asUiError(e);
    } finally {
      ctx.signal.removeEventListener('abort', stop);
    }
  },

  async api_get_response(args: { tab_id?: string; max_body_chars?: number }, ctx: UiCommandCtx) {
    await ready(ctx.signal);
    const i = tabIndex(args.tab_id);
    if (i !== apiClient.activeTab) {
      throw new UiCommandError('not_found', 'Only the active tab shows a response — send it (otto.ui_api_send) to see one.');
    }
    const resp = apiClient.lastResponse;
    if (!resp) {
      if (apiClient.lastError) return { tab_id: apiClient.draft.tabId ?? null, error: apiClient.lastError };
      throw new UiCommandError('not_found', 'No response is shown for this tab — send it first (otto.ui_api_send).');
    }
    const max = Math.min(Math.max(1, Math.trunc(args.max_body_chars ?? BODY_MAX)), 262_144);
    return { tab_id: apiClient.draft.tabId ?? null, ...responseResult(resp, max) };
  },

  async api_save_request(args: { tab_id?: string; name: string; collection_id?: string }, ctx: UiCommandCtx) {
    await ready(ctx.signal);
    const name = args.name?.trim();
    if (!name) throw new UiCommandError('invalid_args', '`name` is empty.');
    focusTab(tabIndex(args.tab_id));
    const d = apiClient.draft;
    let collectionId: string | null;
    if (args.collection_id !== undefined) {
      collectionId = resolveByIdOrName(apiClient.collections, args.collection_id, (c) => c.id, (c) => c.name, 'collection').id;
    } else {
      // Keep a saved request where it is; a new one goes where it was started.
      const saved = d.requestId ? apiClient.requests.find((r) => r.id === d.requestId) : undefined;
      collectionId = saved?.collection_id ?? d.collectionHint ?? null;
    }
    const where = collectionId ? (apiClient.collections.find((c) => c.id === collectionId)?.name ?? 'a collection') : 'the API client';
    const ok = await ctx.confirmWrite({
      what: `${d.requestId ? 'Update' : 'Save'} “${name}” — ${d.method} ${d.url || '(no URL)'}`,
      where,
      verb: 'Save',
      connId: `api-save:${ws.currentId}`,
    });
    if (!ok) throw new UiCommandError('cancelled_by_user', 'The user declined to save the request.');
    const saved = await apiClient.saveDraft(name, collectionId);
    if (!saved) throw new UiCommandError('failed', 'Saving the request failed (see the toast in Otto).');
    return { request_id: saved.id, name: saved.name, collection_id: saved.collection_id };
  },
});
