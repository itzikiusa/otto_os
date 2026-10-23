// The API client's send path: runtime (session) variables, `execute` (pre
// script → HTTP → post script), `executeGrpc`, GraphQL introspection and the
// history-replay guard. Split out of apiClient.svelte.ts (arch-6-03); the
// store forwards every public name. A send owns its tab's response slot (the
// `tabs` part): the result lands in the initiating tab whether or not it is
// still in front, and a newer send or a closed tab makes it stale.

import { api, isAbortError, newHostConfirmHost } from '../../api/client';
import { toasts } from '../../toast.svelte';
import { loadErrorText, providerErrorText } from '../../loadError';
import { mergeVariableWrites, type PreRequestReq } from '../../api/scripts';
import { runScript } from '../../api/scriptRunner';
import { parseGraphqlVariables } from '../../api/graphqlVars';
import type { ApiAuth, ApiResponse, ExecuteApiReq, GrpcInvokeReq, Id } from '../../api/types';
import type { ApiClientStore } from '../apiClient.svelte';
import { confirmNewHost, draftSecrets, liveKv, type ApiDraft } from './model';

/** The store members this part reads and calls (its whole contract with the façade). */
export type ExecuteHost = Pick<
  ApiClientStore,
  | 'activeEnv'
  | 'base'
  | 'draft'
  | 'environments'
  | 'loadHistory'
  | 'openRequest'
  | 'tabSet'
  | 'workspaceOwner'
  | 'wsId'
>;

export class ApiExecute {
  constructor(private readonly host: ExecuteHost) {}

  /** Runtime/session variables (set by scripts or by hand) sent as overrides. */
  private runtimeScopes: Record<string, Record<string, string>> = $state({});
  get runtimeVars(): Record<string, string> { return this.runtimeScopes[this.host.wsId() ?? ''] ?? {}; }
  set runtimeVars(vars: Record<string, string>) {
    const id = this.host.wsId();
    if (id) this.runtimeScopes = {...this.runtimeScopes, [id]: vars};
  }
  setRuntimeVar(key: string, value: string): void {
    this.runtimeVars = { ...this.runtimeVars, [key]: value };
  }
  renameRuntimeVar(oldKey: string, newKey: string): void {
    if (oldKey === newKey) return;
    const next = { ...this.runtimeVars };
    const v = next[oldKey] ?? '';
    delete next[oldKey];
    if (newKey.trim()) next[newKey] = v;
    this.runtimeVars = next;
  }
  removeRuntimeVar(key: string): void {
    const next = { ...this.runtimeVars };
    delete next[key];
    this.runtimeVars = next;
  }
  /** Explain why a history snapshot cannot be sent as a complete request. */
  replayIssue(draft: ApiDraft = this.host.draft): string | null {
    const replay = draft.historyReplay;
    if (replay && (replay.body === undefined || replay.body === draft.body)) return replay.reason;
    if (draft.body_mode === 'multipart') {
      try {
        const rows = JSON.parse(draft.body);
        if (Array.isArray(rows) && rows.some(r => r.type === 'file' && (r.missing_content || (r.size !== undefined && !r.value)))) {
          return 'History does not retain uploaded files. Reselect each missing file before sending or saving.';
        }
      } catch { /* Request validation reports malformed input. */ }
    }
    return null;
  }

  /** Load today's saved request explicitly, never label it as the historical send. */
  async openHistorySavedRequest(): Promise<void> {
    const id = this.host.draft.historyReplay?.requestId;
    if (id) await this.host.openRequest(id);
  }

  /** gRPC shares HTTP's per-tab cancellation and replacement ownership. */
  async executeGrpc(draft: ApiDraft = this.host.draft): Promise<ApiResponse | null> {
    const base = this.host.base(), wid = this.host.wsId();
    if (!base || !wid) return null;
    if (!draft.grpc_method) { toasts.error('Pick a method first'); return null; }
    const issue = this.replayIssue(draft);
    if (issue) { this.host.tabSet.patchSlot(draft.tabId, {error: issue}); return null; }
    draft = $state.snapshot(draft);
    const tabId = draft.tabId;
    this.host.tabSet.cancelExecute(tabId);
    const controller = new AbortController(), signal = controller.signal;
    this.host.tabSet.patchSlot(tabId, {ctrl:controller, sending:true, error:null, tests:[], logs:[], history:null});
    const current = () => this.host.tabSet.slot(tabId)?.ctrl === controller && !signal.aborted
      && this.host.wsId() === wid && this.host.tabSet.tabs.some(t => t.tabId === tabId);
    const env = this.host.activeEnv;
    const body: GrpcInvokeReq = {url:draft.url,proto:draft.proto ?? '',method:draft.grpc_method!,body:draft.body,
      headers:liveKv(draft.headers),environment_id:env?.id ?? null,vars:{...this.runtimeVars}};
    try {
      let resp: ApiResponse;
      try { resp = await api.long.post<ApiResponse>(`${base}/grpc/invoke`, body, signal); }
      catch (e) {
        const host = newHostConfirmHost(e);
        if (host === null || !current()) throw e;
        const ok = await confirmNewHost(host, {method:'gRPC',url:body.url,secrets:draftSecrets(draft,env)});
        if (!ok || !current()) return null;
        resp = await api.long.post<ApiResponse>(`${base}/grpc/invoke`, {...body,confirm_new_host:true}, signal);
      }
      if (!current()) return null;
      this.host.tabSet.patchSlot(tabId, {resp,error:null});
      return resp;
    } catch (e) {
      if (current() && !isAbortError(e)) this.host.tabSet.patchSlot(tabId, {resp:null,error:providerErrorText(e)});
      return null;
    } finally {
      if (this.host.tabSet.slot(tabId)?.ctrl === controller) this.host.tabSet.patchSlot(tabId, {sending:false,ctrl:null});
    }
  }

  /** Send the given draft through the daemon. Sets lastResponse + refreshes history. */
  async execute(draft: ApiDraft = this.host.draft): Promise<ApiResponse | null> {
    const base = this.host.base();
    if (!base) { toasts.error('No workspace selected'); return null; }
    if (!draft.url.trim()) { toasts.error('URL is empty'); return null; }
    const issue = this.replayIssue(draft);
    if (issue) { this.host.tabSet.patchSlot(draft.tabId, {error: issue}); return null; }

    // Own the complete pre → HTTP → post sequence before any user code runs.
    // Snapshot reactive fields so editing another draft cannot alter this send.
    draft = $state.snapshot(draft);
    const wid = this.host.wsId()!, tabId = draft.tabId ?? this.host.draft.tabId;
    const environmentId = this.host.activeEnv?.id ?? null;
    // Only this tab's previous send is replaced; other tabs keep theirs.
    this.host.tabSet.cancelExecute(tabId);
    const controller = new AbortController();
    this.host.tabSet.patchSlot(tabId, { ctrl: controller, sending: true, tests: [], logs: [], error: null, history: null });
    const { signal } = controller;
    // The result belongs to the initiating tab's slot whether or not that tab
    // is still in front — a background tab's response is kept, not dropped.
    const ownsExecution = () => this.host.tabSet.slot(tabId)?.ctrl === controller && !signal.aborted
      && this.host.wsId() === wid && this.host.tabSet.tabs.some(tab => tab.tabId === tabId);
    const ownsView = ownsExecution;
    const checkCurrent = () => { if (!ownsExecution()) throw new DOMException('Request canceled', 'AbortError'); };
    let runtimeVars = {...this.runtimeVars};
    // Plain environment values are readable by scripts; only explicit writes
    // become session overrides. Keychain values remain on the daemon.
    const plainEnv = {...this.host.activeEnv?.variables};
    let scriptVars = {...plainEnv, ...runtimeVars};
    const logs: string[] = [];
    let reqCtx: PreRequestReq = {
      method: draft.method, url: draft.url,
      headers: draft.headers.map(h => ({...h})), body: draft.body,
    };
    try {
      // Bad variables JSON fails the send here, inline, before any script runs.
      const gqlVariables = draft.body_mode === 'graphql' ? parseGraphqlVariables(draft.graphql_variables) : null;
      if (draft.pre_request_script?.trim()) {
        const pre = await runScript({ kind:'pre', code:draft.pre_request_script, request:reqCtx, vars:scriptVars }, signal);
        checkCurrent();
        logs.push(...pre.run.logs.map(l => `[pre] ${l}`));
        if (pre.run.error) throw new Error(`Pre-request script failed: ${pre.run.error}`);
        if (!pre.request) throw new Error('Pre-request script returned no request.');
        reqCtx = pre.request; scriptVars = pre.vars;
        runtimeVars = mergeVariableWrites(runtimeVars, pre.run.writes ?? {});
        this.runtimeScopes = {...this.runtimeScopes, [wid]: mergeVariableWrites(this.runtimeScopes[wid] ?? {}, pre.run.writes ?? {})};
      }
      checkCurrent();
      let effectiveBody = reqCtx.body;
      if (draft.body_mode === 'graphql') {
        effectiveBody = JSON.stringify({query:reqCtx.body, variables: gqlVariables ?? {}});
      }
      const settings = draft.settings;
      const body: ExecuteApiReq = {
        method:reqCtx.method, url:reqCtx.url, headers:liveKv(reqCtx.headers), query:liveKv(draft.query),
        body_mode:draft.body_mode, body:effectiveBody, auth:draft.auth,
        environment_id:environmentId,
        timeout_ms:settings?.timeout_ms ?? null, follow_redirects:settings?.follow_redirects ?? true,
        verify_ssl:settings?.verify_ssl ?? true,
        vars:Object.keys(runtimeVars).length ? runtimeVars : undefined,
        ssh_connection_id:draft.ssh_connection_id ?? null,
      };
      let resp: ApiResponse;
      try {
        resp = await api.long.post<ApiResponse>(`${base}/execute`, body, signal);
      } catch (e) {
        // A stored secret would leave the host it is bound to (e.g. the URL of
        // a saved request was edited before saving): the daemon refuses until
        // a person confirms — ask, then re-send once with the confirmation.
        const host = newHostConfirmHost(e);
        if (host === null) throw e;
        checkCurrent();
        // Show the URL as it will be sent: plain variables filled in (secret
        // values are never resolved client-side).
        const env = this.host.environments.find((e) => e.id === environmentId) ?? null;
        const shownUrl = reqCtx.url.replace(/\{\{\s*([^{}]+?)\s*\}\}/g, (m, n: string) => runtimeVars[n] ?? env?.variables[n] ?? m);
        const ok = await confirmNewHost(host, { method: reqCtx.method, url: shownUrl, secrets: draftSecrets(draft, env) });
        checkCurrent();
        if (!ok) throw new DOMException('Request canceled', 'AbortError');
        resp = await api.long.post<ApiResponse>(`${base}/execute`, { ...body, confirm_new_host: true }, signal);
      }
      checkCurrent();
      if (ownsView()) this.host.tabSet.patchSlot(tabId, { resp, error: null });
      void this.host.loadHistory();
      if (draft.post_response_script?.trim()) {
        const headers: Record<string, string> = {};
        for (const h of resp.headers) headers[h.key.toLowerCase()] = h.value;
        const post = await runScript({kind:'post', code:draft.post_response_script,
          response:{code:resp.status,status:resp.status_text,responseTime:resp.duration_ms,headers,bodyText:resp.body}, vars:scriptVars}, signal);
        checkCurrent();
        logs.push(...post.run.logs.map(l => `[test] ${l}`));
        if (ownsView()) this.host.tabSet.patchSlot(tabId, { tests: post.run.tests });
        if (post.run.error) logs.push(`[test] error: ${post.run.error}`);
        else this.runtimeScopes = {...this.runtimeScopes, [wid]: mergeVariableWrites(this.runtimeScopes[wid] ?? {}, post.run.writes ?? {})};
      }
      if (ownsView()) this.host.tabSet.patchSlot(tabId, { logs });
      return resp;
    } catch (e) {
      if (ownsView()) this.host.tabSet.patchSlot(tabId, { logs: [...logs, `[error] ${providerErrorText(e)}`] });
      if (!signal.aborted && ownsExecution() && !isAbortError(e)) {
        // The tab's response pane shows the failure in place (with the
        // reason); a send failing on a background tab also toasts, since
        // nobody is looking at that pane.
        this.host.tabSet.patchSlot(tabId, { error: providerErrorText(e), resp: null });
        if (this.host.draft.tabId !== tabId) toasts.error('Couldn’t send the request', providerErrorText(e));
      }
      return null;
    } finally {
      if (this.host.tabSet.slot(tabId)?.ctrl === controller) this.host.tabSet.patchSlot(tabId, { sending: false, ctrl: null });
    }
  }

  /** Introspected GraphQL schema (types + fields), or null. */
  private graphqlLoaded: {
    wid: Id | null; tabId?: string; url: string; auth: ApiAuth; environmentId: Id | null;
    types: { name: string; kind: string; fields: string[] }[];
  } | null = $state.raw(null);
  get graphqlSchema(): { name: string; kind: string; fields: string[] }[] | null {
    const loaded = this.graphqlLoaded;
    return loaded && loaded.wid === this.host.wsId() && loaded.tabId === this.host.draft.tabId
      && loaded.url === this.host.draft.url && loaded.auth === this.host.draft.auth
      && loaded.environmentId === (this.host.activeEnv?.id ?? null) ? loaded.types : null;
  }
  graphqlIntrospecting = $state(false);
  private graphqlGeneration = 0;

  /** Run a GraphQL introspection query against the draft URL. */
  async graphqlIntrospect(): Promise<void> {
    const base = this.host.base();
    if (!base || !this.host.draft.url.trim()) {
      toasts.error('No URL', 'Enter the GraphQL endpoint URL first.');
      return;
    }
    const ownsWorkspace = this.host.workspaceOwner();
    const draft = this.host.draft;
    const environmentId = this.host.activeEnv?.id ?? null;
    const generation = ++this.graphqlGeneration;
    const current = () => ownsWorkspace() && this.host.draft === draft
      && (this.host.activeEnv?.id ?? null) === environmentId && this.graphqlGeneration === generation;
    const q = `query{__schema{queryType{name}mutationType{name}types{name kind fields{name}}}}`;
    this.graphqlIntrospecting = true;
    try {
      const resp = await api.long.post<ApiResponse>(`${base}/execute`, {
        method: 'POST', url: draft.url,
        headers: [{ key: 'Content-Type', value: 'application/json', enabled: true }],
        query: [], body_mode: 'json', body: JSON.stringify({ query: q }),
        auth: draft.auth, environment_id: environmentId,
      });
      if (!current()) return;
      const data = JSON.parse(resp.body) as { data?: { __schema?: { types?: { name: string; kind: string; fields?: { name: string }[] }[] } } };
      const types = (data.data?.__schema?.types ?? [])
        .filter((t) => !t.name.startsWith('__') && (t.kind === 'OBJECT' || t.kind === 'INPUT_OBJECT' || t.kind === 'ENUM' || t.kind === 'INTERFACE'))
        .map((t) => ({ name: t.name, kind: t.kind, fields: (t.fields ?? []).map((f) => f.name) }));
      this.graphqlLoaded = {wid: this.host.wsId(), tabId: draft.tabId, url: draft.url, auth: draft.auth, environmentId, types};
      toasts.success('Schema introspected', `${types.length} types`);
    } catch (e) {
      if (current()) toasts.error('Couldn’t introspect the schema', loadErrorText(e));
    } finally {
      if (this.graphqlGeneration === generation) this.graphqlIntrospecting = false;
    }
  }
}
