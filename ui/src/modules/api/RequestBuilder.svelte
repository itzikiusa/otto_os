<script lang="ts">
  // The request editor, shared by the full API page and the compact right
  // panel. Top to bottom: name + where it's saved (Save ⌘S, ⋯ for everything
  // else) · type + method + URL + Send · the {{variables}} this request uses
  // and what they resolve to · Params / Headers / Body / Auth / Scripts / Docs
  // / Settings, each with a one-line explanation.
  import { openExternal } from '../../lib/external';
  import { baseUrl } from '../../lib/api/client';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import MethodTag from './MethodTag.svelte';
  import SaveRequestDialog from './SaveRequestDialog.svelte';
  import ImportDialog from './ImportDialog.svelte';
  import { apiClient, HTTP_METHODS, defaultSettings, confirmNewHost, type ApiDraft, type ApiRequestKind, type ApiSettings } from '../../lib/stores/apiClient.svelte';
  import { apiStream } from '../../lib/stores/apiStream.svelte';
  import { api, newHostConfirmHost } from '../../lib/api/client';
  import { generateCode, CODE_LANGS, type CodeLang } from '../../lib/api/codegen';
  import { collectionPaths, requestTexts, resolveVar, splitVars, varNames } from '../../lib/api/apiVars';
  import { marked } from 'marked';
  import { sanitizeHtml } from '../../lib/sanitize';
  import type { ApiAuth, ApiBodyMode, ApiKeyVal, ApiResponse, ApiSecretable } from '../../lib/api/types';
  import { isSecretRef } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';

  type Tab = 'params' | 'headers' | 'body' | 'auth' | 'settings' | 'scripts' | 'docs';

  interface Props {
    /** Compact mode (right panel): tighter, textareas instead of code editors. */
    compact?: boolean;
    /** The open editor tab (bindable, so the response pane can jump to Settings). */
    tab?: Tab;
  }
  let { compact = false, tab = $bindable('params') }: Props = $props();

  // The draft lives in the store so the page + panel share one editing target.
  const draft = $derived(apiClient.draft);
  const canEdit = $derived(ws.myRole !== 'viewer');

  // ── Body types ─────────────────────────────────────────────────────────────
  // One plain-language picker over the backend modes: "Text" covers 'raw' (with
  // a sub-type for the Content-Type), "JSON" is 'json'.
  type RawType = 'Text' | 'JavaScript' | 'JSON' | 'HTML' | 'XML';
  const TEXT_TYPES: RawType[] = ['Text', 'JavaScript', 'HTML', 'XML'];
  type BodyChoice = 'none' | 'json' | 'text' | 'form' | 'multipart' | 'graphql';
  const BODY_CHOICES: { id: BodyChoice; label: string; help: string }[] = [
    { id: 'none', label: 'No body', help: 'Most GET and DELETE requests don’t send a body.' },
    { id: 'json', label: 'JSON', help: 'Sent as application/json. Use {{variables}} anywhere in it.' },
    { id: 'text', label: 'Text / XML / HTML', help: 'Sent as-is with the Content-Type you pick.' },
    { id: 'form', label: 'Form (URL-encoded)', help: 'key=value pairs, like an HTML form (application/x-www-form-urlencoded).' },
    { id: 'multipart', label: 'Multipart form (files)', help: 'Fields and file uploads (multipart/form-data).' },
    { id: 'graphql', label: 'GraphQL', help: 'A query plus JSON variables, sent as {"query","variables"}.' },
  ];
  const bodyChoice = $derived<BodyChoice>(
    draft.body_mode === 'raw' ? 'text' : (draft.body_mode as BodyChoice),
  );
  const rawActive = $derived(draft.body_mode === 'raw' || draft.body_mode === 'json');
  const formActive = $derived(draft.body_mode === 'form' || draft.body_mode === 'multipart');

  // Current raw sub-type, inferred from body_mode + the Content-Type header.
  const rawType = $derived(deriveRawType());
  function deriveRawType(): RawType {
    if (draft.body_mode === 'json') return 'JSON';
    if (draft.body_mode === 'raw') {
      const ct = currentContentType().toLowerCase();
      if (ct.includes('html')) return 'HTML';
      if (ct.includes('xml')) return 'XML';
      if (ct.includes('javascript')) return 'JavaScript';
      return 'Text';
    }
    return 'JSON';
  }
  function currentContentType(): string {
    return draft.headers.find((h) => h.key.trim().toLowerCase() === 'content-type')?.value ?? '';
  }
  const RAW_PATH: Record<RawType, string> = { Text: 'body.txt', JavaScript: 'body.js', JSON: 'body.json', HTML: 'body.html', XML: 'body.xml' };
  const RAW_LANG: Record<RawType, string> = { Text: '', JavaScript: 'js', JSON: 'json', HTML: 'html', XML: 'xml' };
  const editorPath = $derived(draft.body_mode === 'graphql' ? 'body.graphql' : RAW_PATH[rawType]);
  const editorLang = $derived(draft.body_mode === 'graphql' ? '' : RAW_LANG[rawType]);

  // ── Content-Type auto-management ───────────────────────────────────────────
  // Values we consider "auto-set" and therefore safe to overwrite/remove when
  // the body type changes. A Content-Type the user typed by hand is preserved.
  const AUTO_CONTENT_TYPES = new Set([
    'application/json', 'application/x-www-form-urlencoded', 'multipart/form-data',
    'text/plain', 'application/javascript', 'text/html', 'application/xml', 'text/xml',
    'application/graphql',
  ]);
  function contentTypeFor(mode: ApiBodyMode, rt: RawType): string | null {
    switch (mode) {
      case 'json':
      case 'graphql': return 'application/json';
      case 'form': return 'application/x-www-form-urlencoded';
      case 'multipart': return null; // reqwest sets the boundary itself
      case 'raw':
        switch (rt) {
          case 'JavaScript': return 'application/javascript';
          case 'HTML': return 'text/html';
          case 'XML': return 'application/xml';
          default: return 'text/plain';
        }
      default: return null;
    }
  }
  /** Returns headers with the implied Content-Type applied (custom values kept). */
  function withAutoContentType(rows: ApiKeyVal[], ct: string | null): ApiKeyVal[] {
    const headers = rows.map((r) => ({ ...r }));
    const idx = headers.findIndex((h) => h.key.trim().toLowerCase() === 'content-type');
    if (ct === null) {
      if (idx >= 0 && AUTO_CONTENT_TYPES.has(headers[idx].value.trim().toLowerCase())) headers.splice(idx, 1);
      return headers;
    }
    if (idx < 0) {
      headers.push({ key: 'Content-Type', value: ct, enabled: true });
      return headers;
    }
    const cur = headers[idx].value.trim().toLowerCase();
    if (cur === '' || AUTO_CONTENT_TYPES.has(cur)) headers[idx] = { ...headers[idx], value: ct };
    return headers;
  }
  /** Set body mode + raw sub-type in one shot, syncing the Content-Type header. */
  function applyBody(mode: ApiBodyMode, rt: RawType): void {
    const headers = withAutoContentType(draft.headers, contentTypeFor(mode, rt));
    apiClient.draft = { ...draft, body_mode: mode, headers };
  }
  function onBodyChoice(c: BodyChoice): void {
    switch (c) {
      case 'none': applyBody('none', 'Text'); break;
      case 'json': applyBody('json', 'JSON'); break;
      case 'text': applyBody('raw', rawType === 'JSON' ? 'Text' : rawType); break;
      default: applyBody(c, 'Text');
    }
  }
  function onRawType(rt: RawType): void {
    applyBody(rt === 'JSON' ? 'json' : 'raw', rt);
  }

  // ── Beautify (raw body pretty-printer) ─────────────────────────────────────
  function beautify(): void {
    const body = draft.body;
    if (!body.trim()) return;
    try {
      let out: string;
      if (rawType === 'JSON' || draft.kind === 'grpc') out = JSON.stringify(JSON.parse(body), null, 2);
      else if (rawType === 'XML' || rawType === 'HTML') out = beautifyXml(body);
      else { toasts.info('Nothing to format', 'Formatting works for JSON, XML and HTML bodies.'); return; }
      setField('body', out);
    } catch {
      toasts.error('Couldn’t format the body', rawType === 'JSON' ? 'The body isn’t valid JSON.' : 'The body couldn’t be parsed.');
    }
  }
  function beautifyXml(xml: string): string {
    const PAD = '  ';
    const withBreaks = xml.replace(/>\s*</g, '>\n<').trim();
    let pad = 0;
    const lines: string[] = [];
    for (const raw of withBreaks.split('\n')) {
      const node = raw.trim();
      if (!node) continue;
      const isClose = /^<\//.test(node);
      const isSelfContained = /^<[^>]+>.*<\/[^>]+>$/.test(node);
      const isOpen = /^<[^/!?][^>]*[^/]>$/.test(node) && !isSelfContained;
      if (isClose && pad > 0) pad--;
      lines.push(PAD.repeat(pad) + node);
      if (isOpen) pad++;
    }
    return lines.join('\n');
  }

  // ── {{variables}} ──────────────────────────────────────────────────────────
  const urlSegments = $derived(splitVars(draft.url));
  const envForVars = $derived(apiClient.activeEnv);
  const usedVars = $derived(
    varNames(...requestTexts(draft)).map((n) => resolveVar(n, apiClient.runtimeVars, envForVars)),
  );
  const missingVars = $derived(usedVars.filter((v) => v.kind === 'missing').length);

  // ── key/value rows ────────────────────────────────────────────────────────
  function addRow(which: 'headers' | 'query', key = '', value = ''): void {
    apiClient.draft = { ...draft, [which]: [...draft[which], { key, value, enabled: true }] };
  }
  function updateRow(which: 'headers' | 'query', i: number, patch: Partial<ApiKeyVal>): void {
    const rows = draft[which].map((r, idx) => (idx === i ? { ...r, ...patch } : r));
    apiClient.draft = { ...draft, [which]: rows };
  }
  function removeRow(which: 'headers' | 'query', i: number): void {
    apiClient.draft = { ...draft, [which]: draft[which].filter((_, idx) => idx !== i) };
  }

  // ── form-body rows ─────────────────────────────────────────────────────────
  // Kept in LOCAL state (not derived from the encoded body) so that *empty* rows
  // persist in the editor — the encoders drop empty-key pairs when sending, so a
  // derived approach would make "Add field" appear to do nothing.
  //   • x-www-form-urlencoded ('form')  → `k=v&…`
  //   • multipart/form-data ('multipart') → JSON `[{key,type,value,filename}]`,
  //     so a row can be a File (value = base64 of the file's bytes).
  type FieldType = 'text' | 'file';
  interface FormRow { key: string; value: string; type: FieldType; filename?: string; }
  function parseForm(s: string, mode: ApiBodyMode): FormRow[] {
    if (mode === 'multipart' && s.trim().startsWith('[')) {
      try {
        const arr = JSON.parse(s) as Partial<FormRow>[];
        return arr.map((r) => ({ key: r.key ?? '', value: r.value ?? '', type: r.type === 'file' ? 'file' : 'text', filename: r.filename }));
      } catch { /* fall through to urlencoded parsing */ }
    }
    if (!s) return [];
    return s.split('&').map((pair) => {
      const eq = pair.indexOf('=');
      if (eq < 0) return { key: decode(pair), value: '', type: 'text' as FieldType };
      return { key: decode(pair.slice(0, eq)), value: decode(pair.slice(eq + 1)), type: 'text' as FieldType };
    });
  }
  function encodeForm(rows: FormRow[], mode: ApiBodyMode): string {
    const live = rows.filter((r) => r.key.trim() !== '');
    if (mode === 'multipart') {
      return JSON.stringify(live.map((r) => (r.type === 'file'
        ? { key: r.key, type: 'file', value: r.value, filename: r.filename ?? '' }
        : { key: r.key, type: 'text', value: r.value })));
    }
    return live.map((r) => `${encodeURIComponent(r.key)}=${encodeURIComponent(r.value)}`).join('&');
  }
  function decode(s: string): string {
    try { return decodeURIComponent(s.replace(/\+/g, ' ')); } catch { return s; }
  }
  let formRows = $state<FormRow[]>([]);
  let lastFormEncoded = $state('');
  // Re-seed when the body changes EXTERNALLY (import curl / load a saved
  // request) — but NOT from our own encodeForm writes (tracked by lastFormEncoded).
  $effect(() => {
    const body = draft.body;
    if ((draft.body_mode === 'form' || draft.body_mode === 'multipart') && body !== lastFormEncoded) {
      formRows = parseForm(body, draft.body_mode);
      lastFormEncoded = body;
    }
  });
  function syncFormBody(): void {
    const enc = encodeForm(formRows, apiClient.draft.body_mode);
    lastFormEncoded = enc;
    apiClient.draft = { ...apiClient.draft, body: enc };
  }
  function addFormRow(): void {
    formRows = [...formRows, { key: '', value: '', type: 'text' }];
    syncFormBody();
  }
  function updateFormRow(i: number, patch: Partial<FormRow>): void {
    formRows = formRows.map((r, idx) => (idx === i ? { ...r, ...patch } : r));
    syncFormBody();
  }
  function removeFormRow(i: number): void {
    formRows = formRows.filter((_, idx) => idx !== i);
    syncFormBody();
  }
  const multipartActive = $derived(draft.body_mode === 'multipart');
  async function pickFile(i: number, input: HTMLInputElement): Promise<void> {
    const file = input.files?.[0];
    if (!file) return;
    try {
      updateFormRow(i, { value: await fileToBase64(file), filename: file.name });
    } catch {
      toasts.error('Couldn’t read the file', file.name);
    }
  }
  function fileToBase64(file: File): Promise<string> {
    return new Promise((resolve, reject) => {
      const reader = new FileReader();
      reader.onload = () => {
        const res = reader.result as string;
        resolve(res.slice(res.indexOf(',') + 1)); // strip the `data:…;base64,` prefix
      };
      reader.onerror = () => reject(reader.error);
      reader.readAsDataURL(file);
    });
  }
  function setRowType(i: number, type: FieldType): void {
    updateFormRow(i, { type, value: '', filename: undefined }); // text ⇄ file aren't interchangeable
  }

  // ── header name/value completions (datalist) ───────────────────────────────
  const COMMON_HEADERS = [
    'Accept', 'Accept-Encoding', 'Accept-Language', 'Authorization', 'Cache-Control',
    'Connection', 'Content-Disposition', 'Content-Encoding', 'Content-Length',
    'Content-Type', 'Cookie', 'Date', 'ETag', 'Host', 'If-Match', 'If-None-Match',
    'If-Modified-Since', 'Location', 'Origin', 'Pragma', 'Range', 'Referer',
    'User-Agent', 'WWW-Authenticate', 'X-Api-Key', 'X-Correlation-Id',
    'X-CSRF-Token', 'X-Forwarded-For', 'X-Forwarded-Host', 'X-Forwarded-Proto',
    'X-Request-Id', 'X-Requested-With',
  ];
  const HEADER_VALUES: Record<string, string[]> = {
    'content-type': ['application/json', 'application/x-www-form-urlencoded', 'multipart/form-data', 'text/plain', 'text/html', 'application/xml', 'text/xml', 'application/octet-stream', 'application/graphql'],
    accept: ['application/json', '*/*', 'text/html', 'application/xml', 'text/plain'],
    'accept-encoding': ['gzip, deflate, br', 'gzip', 'deflate', 'br', 'identity'],
    'cache-control': ['no-cache', 'no-store', 'max-age=0', 'must-revalidate', 'public', 'private'],
    connection: ['keep-alive', 'close'],
    authorization: ['Bearer ', 'Basic '],
    pragma: ['no-cache'],
    'x-requested-with': ['XMLHttpRequest'],
    'content-encoding': ['gzip', 'deflate', 'br', 'identity'],
  };
  function headerValues(key: string): string[] {
    return HEADER_VALUES[key.trim().toLowerCase()] ?? [];
  }
  const QUICK_HEADERS: [string, string][] = [['Accept', 'application/json'], ['Content-Type', 'application/json'], ['Authorization', 'Bearer {{api_token}}']];

  // ── field setters ──────────────────────────────────────────────────────────
  function setField<K extends keyof ApiDraft>(k: K, v: ApiDraft[K]): void {
    apiClient.draft = { ...draft, [k]: v };
  }
  // Request docs arrive from Postman imports, git-pulled collections and agent
  // upserts — untrusted markdown. marked passes raw HTML through, so the output
  // MUST go through the allowlist sanitizer before the `{@html}` sink.
  const docsHtml = $derived.by(() => {
    try { return sanitizeHtml(marked.parse(draft.docs ?? '', { async: false, gfm: true, breaks: true }) as string); }
    catch { return ''; }
  });
  const settings = $derived(draft.settings ?? defaultSettings());
  const settingsChanged = $derived(settings.timeout_ms != null || !settings.follow_redirects || !settings.verify_ssl || !!draft.ssh_connection_id);
  function setSetting<K extends keyof ApiSettings>(k: K, v: ApiSettings[K]): void {
    apiClient.draft = { ...draft, settings: { ...settings, [k]: v } };
  }
  function setAuth(patch: Partial<ApiAuth>): void {
    apiClient.draft = { ...draft, auth: { ...draft.auth, ...patch } as ApiAuth };
  }
  const AUTH_TYPES: { id: ApiAuth['type']; label: string; help: string }[] = [
    { id: 'none', label: 'No auth', help: 'Nothing is added. You can still set an Authorization header yourself.' },
    { id: 'bearer', label: 'Bearer token', help: 'Adds “Authorization: Bearer <token>”. Common for API tokens and JWTs.' },
    { id: 'basic', label: 'Basic auth', help: 'Adds “Authorization: Basic …” from a username and password.' },
    { id: 'api_key', label: 'API key', help: 'Sends a key/value pair as a header or a query parameter.' },
    { id: 'oauth2', label: 'OAuth 2.0', help: 'Gets an access token from a token endpoint (or your browser) and sends it as a bearer token.' },
  ];
  const AUTH_SHORT: Record<string, string> = { none: '', bearer: 'Bearer', basic: 'Basic', api_key: 'API key', oauth2: 'OAuth' };
  function setAuthType(type: ApiAuth['type']): void {
    let auth: ApiAuth;
    switch (type) {
      case 'bearer': auth = { type, token: '' }; break;
      case 'basic': auth = { type, username: '', password: '' }; break;
      case 'api_key': auth = { type, key: '', value: '', in: 'header' }; break;
      case 'oauth2': auth = { type, grant: 'client_credentials', token_url: '', client_id: '', client_secret: '', scope: '', username: '', password: '', refresh_token: '', access_token: '', token_type: 'Bearer' }; break;
      default: auth = { type: 'none' };
    }
    apiClient.draft = { ...draft, auth };
  }
  // ── Keychain-backed secret members ─────────────────────────────────────────
  // A saved secret comes back as a {"$secret": ref} marker: render it masked
  // (empty input + placeholder); typing replaces it with plaintext, which the
  // daemon re-migrates to the Keychain on the next save.
  function secretValue(v: ApiSecretable | undefined): string {
    return typeof v === 'string' ? v : '';
  }
  function secretPlaceholder(v: ApiSecretable | undefined, fallback: string): string {
    return isSecretRef(v) ? '•••••• stored in Keychain — type to replace' : fallback;
  }
  let fetchingToken = $state(false);
  async function fetchOAuthToken(): Promise<void> {
    if (draft.auth.type !== 'oauth2') return;
    const wid = ws.currentId;
    if (!wid) return;
    const a = draft.auth;
    const tabId = draft.tabId;
    if (!a.token_url.trim()) { toasts.error('Add a token URL first', 'The OAuth token endpoint is required.'); return; }
    fetchingToken = true;
    try {
      if (a.grant === 'authorization_code') {
        // Save first so the daemon owns the resulting token under this request's Keychain reference.
        const saved = await apiClient.saveDraft(draft.name || 'OAuth request', apiClient.requests.find((r) => r.id === draft.requestId)?.collection_id ?? null);
        if (!saved || ws.currentId !== wid || apiClient.draft.tabId !== tabId) return;
        const flow = await api.post<{ flow_id: string; authorization_url: string; redirect_uri: string }>(`/workspaces/${wid}/api-client/oauth2/authorize`, { request_id: saved.id });
        await openExternal(flow.authorization_url);
        const deadline = Date.now() + 600_000;
        while (Date.now() < deadline) {
          await new Promise((resolve) => setTimeout(resolve, 1000));
          const result = await api.get<{ status: string; error?: string }>(`/workspaces/${wid}/api-client/oauth2/flows/${flow.flow_id}`);
          if (result.status === 'failed') throw new Error(result.error || 'Authorization failed');
          if (result.status === 'completed') {
            const requests = await api.get<import('../../lib/api/types').ApiRequest[]>(`/workspaces/${wid}/api-client/requests`);
            const request = requests.find((r) => r.id === saved.id);
            if (request) {
              apiClient.applySavedAuth(wid, request, saved.auth);
              toasts.success('Signed in', 'The access token is stored in the Keychain.');
            }
            return;
          }
        }
        return;
      }
      type TokenResp = { access_token: string; token_type?: string; refresh_token?: string; expires_in?: number };
      const tokenReq = { grant: a.grant, token_url: a.token_url, client_id: a.client_id, client_secret: a.client_secret, scope: a.scope, username: a.username, password: a.password, refresh_token: a.refresh_token };
      let res: TokenResp;
      try {
        res = await api.post<TokenResp>(`/workspaces/${wid}/api-client/oauth2/token`, tokenReq);
      } catch (e) {
        // A stored secret would go to a token endpoint other than the saved
        // one — the daemon waits for a person's confirmation.
        const host = newHostConfirmHost(e);
        if (host === null || !(await confirmNewHost(host, { method: 'POST', url: a.token_url, secrets: ['the saved OAuth client secret / credentials (Keychain)'] }))) throw e;
        if (ws.currentId !== wid || apiClient.draft.tabId !== tabId) return;
        res = await api.post<TokenResp>(`/workspaces/${wid}/api-client/oauth2/token`, { ...tokenReq, confirm_new_host: true });
      }
      if (ws.currentId !== wid || apiClient.draft.tabId !== tabId) return;
      setAuth({ access_token: res.access_token, token_type: res.token_type || 'Bearer', refresh_token: res.refresh_token || a.refresh_token });
      toasts.success('Access token received', `${res.token_type || 'Bearer'} · expires in ${res.expires_in ?? '?'} s`);
    } catch (e) {
      toasts.error('Couldn’t get a token', e instanceof Error ? e.message : String(e));
    } finally {
      fetchingToken = false;
    }
  }

  // ── request kind (HTTP / SSE / WebSocket / gRPC) ────────────────────────────
  const REQUEST_KINDS: { id: ApiRequestKind; label: string; title: string }[] = [
    { id: 'http', label: 'HTTP', title: 'A normal HTTP request (REST, GraphQL)' },
    { id: 'sse', label: 'SSE', title: 'Server-sent events: keep the connection open and stream events' },
    { id: 'websocket', label: 'WebSocket', title: 'A two-way WebSocket connection' },
    { id: 'grpc', label: 'gRPC', title: 'Call a gRPC method (unary or server-streaming)' },
  ];
  const isStreaming = $derived(draft.kind === 'sse' || draft.kind === 'websocket');
  function setKind(k: ApiRequestKind): void {
    if (apiStream.active) apiStream.disconnect();
    apiClient.draft = { ...draft, kind: k, ...(k === 'websocket' ? { method: 'GET', body_mode: 'none', body: '' } : {}) };
  }

  // ── editor tabs ────────────────────────────────────────────────────────────
  const liveCount = (rows: ApiKeyVal[]) => rows.filter((r) => r.enabled !== false && r.key.trim() !== '').length;
  const scriptCount = $derived((draft.pre_request_script?.trim() ? 1 : 0) + (draft.post_response_script?.trim() ? 1 : 0));
  const tabs = $derived.by((): { id: Tab; label: string; count?: string }[] => {
    const n = (v: number) => (v ? String(v) : undefined);
    if (draft.kind === 'grpc') return [{ id: 'body', label: 'Message' }, { id: 'headers', label: 'Metadata', count: n(liveCount(draft.headers)) }];
    const params = { id: 'params' as Tab, label: 'Params', count: n(liveCount(draft.query)) };
    const headers = { id: 'headers' as Tab, label: 'Headers', count: n(liveCount(draft.headers)) };
    const auth = { id: 'auth' as Tab, label: 'Auth', count: AUTH_SHORT[draft.auth.type] || undefined };
    const settingsTab = { id: 'settings' as Tab, label: 'Settings', count: settingsChanged ? 'custom' : undefined };
    if (draft.kind === 'websocket') return [params, headers, auth, settingsTab];
    return [
      params, headers,
      { id: 'body', label: 'Body', count: draft.body_mode === 'none' ? undefined : BODY_CHOICES.find((c) => c.id === bodyChoice)?.label.split(' ')[0] },
      auth,
      { id: 'scripts', label: 'Scripts', count: n(scriptCount) },
      { id: 'docs', label: 'Docs' },
      settingsTab,
    ];
  });
  // Keep the active tab valid for the current kind's tab strip.
  $effect(() => {
    if (!tabs.some((t) => t.id === tab)) tab = tabs[0].id;
  });
  function onTabKey(e: KeyboardEvent, i: number): void {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft' && e.key !== 'Home' && e.key !== 'End') return;
    e.preventDefault();
    const j = e.key === 'Home' ? 0 : e.key === 'End' ? tabs.length - 1 : (i + (e.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length;
    tab = tabs[j].id;
    (e.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLElement>('[role=tab]')[j]?.focus();
  }

  // ── gRPC state ──────────────────────────────────────────────────────────────
  interface GrpcMethod { name: string; full: string; input_type: string; output_type: string; input_schema: string; client_streaming: boolean; server_streaming: boolean; }
  interface GrpcService { name: string; methods: GrpcMethod[]; }
  let grpcServices = $state<GrpcService[]>([]);
  let grpcParsing = $state(false);
  let grpcInvoking = $state(false);

  async function onProtoFile(input: HTMLInputElement): Promise<void> {
    const f = input.files?.[0];
    if (!f) return;
    const text = await f.text();
    apiClient.draft = { ...apiClient.draft, proto: text, grpc_method: '' };
    grpcServices = [];
    await parseProto();
  }
  async function parseProto(): Promise<void> {
    const wid = ws.currentId;
    if (!wid) return;
    if (!draft.proto?.trim()) { toasts.error('No .proto file', 'Upload a .proto file first.'); return; }
    const tabId = draft.tabId;
    const proto = draft.proto;
    grpcParsing = true;
    try {
      const res = await api.post<{ services: GrpcService[] }>(`/workspaces/${wid}/api-client/grpc/describe`, { proto: draft.proto });
      if (ws.currentId !== wid || apiClient.draft.tabId !== tabId || apiClient.draft.proto !== proto) return;
      grpcServices = res.services;
      const first = res.services.find((s) => s.methods.length);
      if (first && !draft.grpc_method) selectGrpcMethod(first.methods[0]);
      toasts.success('Read the .proto file', `${res.services.length} service(s)`);
    } catch (e) {
      toasts.error('Couldn’t read the .proto file', e instanceof Error ? e.message : String(e));
    } finally {
      grpcParsing = false;
    }
  }
  let grpcReflecting = $state(false);
  async function reflectGrpc(): Promise<void> {
    const wid = ws.currentId;
    if (!wid) return;
    if (!draft.url.trim()) { toasts.error('Enter the server URL first', 'Reflection asks the gRPC server for its services.'); return; }
    const tabId = draft.tabId;
    grpcReflecting = true;
    try {
      // Reflection-based: clear any uploaded proto so invoke uses reflection too.
      apiClient.draft = { ...apiClient.draft, proto: '' };
      const res = await api.post<{ services: GrpcService[] }>(`/workspaces/${wid}/api-client/grpc/reflect`, { url: draft.url, headers: draft.headers.filter((h) => h.enabled !== false && h.key.trim() !== '') });
      if (ws.currentId !== wid || apiClient.draft.tabId !== tabId) return;
      grpcServices = res.services;
      const first = res.services.find((s) => s.methods.length);
      if (first) selectGrpcMethod(first.methods[0]);
      toasts.success('Loaded services from the server', `${res.services.length} service(s)`);
    } catch (e) {
      toasts.error('Server reflection failed', e instanceof Error ? e.message : String(e));
    } finally {
      grpcReflecting = false;
    }
  }
  let grpcTabId = $state<string | undefined>();
  $effect(() => { if (draft.tabId !== grpcTabId) { grpcTabId = draft.tabId; grpcServices = []; } });
  function selectGrpcMethod(m: GrpcMethod): void {
    apiClient.draft = { ...apiClient.draft, grpc_method: m.full, body: draft.body?.trim() ? draft.body : m.input_schema };
  }
  function onGrpcMethodChange(full: string): void {
    const m = grpcServices.flatMap((s) => s.methods).find((x) => x.full === full);
    if (m) selectGrpcMethod(m);
  }
  async function invokeGrpc(): Promise<void> {
    const wid = ws.currentId;
    if (!wid) return;
    if (!draft.grpc_method) { toasts.error('Pick a method first', 'Upload a .proto or load services from the server.'); return; }
    const tabId = draft.tabId;
    grpcInvoking = true;
    try {
      const res = await api.post<ApiResponse>(`/workspaces/${wid}/api-client/grpc/invoke`, {
        url: draft.url, proto: draft.proto ?? '', method: draft.grpc_method, body: draft.body,
        headers: draft.headers.filter((h) => h.enabled !== false && h.key.trim() !== ''),
      });
      if (ws.currentId === wid && apiClient.draft.tabId === tabId) { apiClient.lastResponse = res; apiClient.lastError = null; }
    } catch (e) {
      if (ws.currentId === wid && apiClient.draft.tabId === tabId) apiClient.lastError = e instanceof Error ? e.message : String(e);
    } finally {
      grpcInvoking = false;
    }
  }

  $effect(() => {
    if (apiStream.workspaceId && apiStream.workspaceId !== ws.currentId) {
      apiStream.disconnect(); apiStream.clear();
    }
  });

  // ── actions ────────────────────────────────────────────────────────────────
  function send(): void {
    switch (draft.kind) {
      case 'http': void apiClient.execute(); break;
      case 'grpc': void invokeGrpc(); break;
      case 'sse':
      case 'websocket':
        if (apiStream.active) apiStream.disconnect();
        else if (ws.currentId) apiStream.connect(ws.currentId, draft.kind, {
          method: draft.method, url: draft.url, headers: draft.headers, query: draft.query,
          body: draft.body, body_mode: draft.body_mode, auth: draft.auth,
          environment_id: apiClient.activeEnv?.id ?? null, vars: { ...apiClient.runtimeVars },
          timeout_ms: draft.settings?.timeout_ms ?? null,
          follow_redirects: draft.settings?.follow_redirects ?? true,
          verify_ssl: draft.settings?.verify_ssl ?? true,
          ssh_connection_id: draft.ssh_connection_id ?? null,
        });
        break;
    }
  }
  const sendLabel = $derived(draft.kind === 'grpc' ? 'Invoke' : isStreaming ? (apiStream.active ? 'Disconnect' : 'Connect') : 'Send');
  const sendBusy = $derived(apiClient.sending || grpcInvoking || apiStream.status === 'connecting');

  function onUrlKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') { e.preventDefault(); send(); }
  }

  // ⌘↵ send · ⌘S save (⌘T / ⌘D are claimed by the page via keyContext).
  function onDocKeydown(e: KeyboardEvent): void {
    if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey || ui.overlayOpen) return;
    if (e.key === 'Enter') { e.preventDefault(); send(); }
    else if (e.key === 's') { e.preventDefault(); void save(); }
  }

  function stopRequest(): void {
    if (draft.kind === 'http') apiClient.cancelExecute();
    else apiStream.disconnect();
  }
  const canStop = $derived((draft.kind === 'http' && apiClient.sending) || (isStreaming && apiStream.active));

  /** Paste a curl command straight into the address bar → import it. */
  function onUrlPaste(e: ClipboardEvent): void {
    const text = e.clipboardData?.getData('text') ?? '';
    if (/^\s*curl[\s]/i.test(text)) {
      e.preventDefault();
      void apiClient.importCurl(text);
    }
  }

  // Session variables (in-memory, per workspace).
  let varsOpen = $state(false);
  const varEntries = $derived(Object.entries(apiClient.runtimeVars));
  let newVarKey = $state('');
  let newVarVal = $state('');
  function addVar(): void {
    if (!newVarKey.trim()) return;
    apiClient.setRuntimeVar(newVarKey.trim(), newVarVal);
    newVarKey = '';
    newVarVal = '';
  }

  let cookiesOpen = $state(false);
  function openCookies(): void {
    cookiesOpen = true;
    void apiClient.loadCookies();
  }
  async function clearCookies(): Promise<void> {
    if (!(await confirmer.ask('Delete every cookie Otto has collected for this workspace? Later requests won’t send them.', { title: 'Clear cookies', confirmLabel: 'Clear cookies' }))) return;
    await apiClient.clearCookies();
  }

  let codeOpen = $state(false);
  let codeLang = $state<CodeLang>('curl');
  const codeSnippet = $derived(codeOpen ? generateCode(draft, codeLang) : '');
  async function copyText(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${what}`);
    } catch {
      toasts.error('Couldn’t copy', 'The clipboard isn’t available.');
    }
  }

  let importOpen = $state(false);

  // ── save ─────────────────────────────────────────────────────────────────
  let saveOpen = $state(false);
  const saved = $derived(draft.requestId ? apiClient.requests.find((r) => r.id === draft.requestId) : undefined);
  const paths = $derived(collectionPaths(apiClient.collections));
  const location = $derived.by(() => {
    if (!saved) return null;
    return saved.collection_id ? (paths.find((p) => p.id === saved.collection_id)?.path ?? 'Collection') : 'Ungrouped';
  });
  const dirty = $derived(apiClient.isDirty(draft));

  /** ⌘S: a saved request saves in place; a new one asks for a name and a place. */
  async function save(): Promise<void> {
    if (!canEdit) {
      toasts.error('Read-only', 'You have viewer access to this workspace, so you can’t save requests.');
      return;
    }
    if (saved) {
      await apiClient.saveDraft(draft.name?.trim() || saved.name, saved.collection_id ?? null);
      return;
    }
    saveOpen = true;
  }
  function saveAsCopy(): void {
    apiClient.duplicateDraft();
    saveOpen = true;
  }
  async function deleteSaved(): Promise<void> {
    if (!saved) return;
    if (!(await confirmer.ask(`Delete the saved request “${saved.name}”? Its stored credentials are removed from the Keychain. This tab stays open as an unsaved copy.`, { title: 'Delete request' }))) return;
    await apiClient.deleteRequest(saved.id);
  }

  function moreMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'Duplicate  ⌘D', icon: 'copy', action: () => apiClient.duplicateDraft() },
      ...(saved && canEdit ? [{ label: 'Save as a copy…', icon: 'plus', action: saveAsCopy }] : []),
      { separator: true },
      { label: 'Copy as curl', icon: 'terminal', action: () => void copyText(apiClient.toCurl(), 'as curl') },
      { label: 'Generate code…', icon: 'function', action: () => (codeOpen = true) },
      { label: 'Import from curl…', icon: 'download', action: () => (importOpen = true) },
      { separator: true },
      { label: 'Session variables…', icon: 'key', action: () => (varsOpen = true) },
      { label: 'Cookie jar…', icon: 'archive', action: openCookies },
    ];
    if (saved && canEdit) items.push({ separator: true }, { label: 'Delete request…', icon: 'trash', danger: true, action: () => void deleteSaved() });
    ctxMenu.show(e, items);
  }
</script>

<svelte:window onkeydown={onDocKeydown} />

<div class="builder" class:compact>
  <div class="name-row">
      <input
        class="name-input"
        size={Math.min(48, Math.max(14, (draft.name || apiClient.tabLabel({ ...draft, name: '' })).length + 2))}
        value={draft.name}
        placeholder={apiClient.tabLabel({ ...draft, name: '' })}
        aria-label="Request name"
        oninput={(e) => setField('name', (e.currentTarget as HTMLInputElement).value)}
        spellcheck="false"
      />
      <span class="where" title={location ?? 'Not saved yet'}>
        {#if location}<Icon name="folder" size={12} />{location}{:else}Not saved{/if}
        {#if dirty && saved}<span class="edited">· Edited</span>{/if}
      </span>
      <span class="grow"></span>
      <button class="btn small" onclick={() => void save()} title="Save request (⌘S)" disabled={!canEdit}>Save</button>
      <button class="icon-btn" onclick={moreMenu} aria-label="More request actions" title="More request actions"><Icon name="more" size={14} /></button>
  </div>

  <!-- type + method + URL + Send -->
  <div class="urlbar">
    <select class="kind" value={draft.kind} aria-label="Request type" title={REQUEST_KINDS.find((k) => k.id === draft.kind)?.title}
      onchange={(e) => setKind((e.currentTarget as HTMLSelectElement).value as ApiRequestKind)}>
      {#each REQUEST_KINDS as k (k.id)}<option value={k.id} title={k.title}>{k.label}</option>{/each}
    </select>
    {#if draft.kind === 'http' || draft.kind === 'sse'}
      <select class="method" value={draft.method} aria-label="HTTP method" onchange={(e) => setField('method', (e.currentTarget as HTMLSelectElement).value)}>
        {#each HTTP_METHODS as m (m)}<option value={m}>{m}</option>{/each}
      </select>
    {/if}
    <div class="url-wrap">
      <!-- Highlight layer mirrors the input value exactly (same font + padding). -->
      <div class="url-layer url-highlight" aria-hidden="true">
        {#each urlSegments as seg, i (i)}<span class:var={seg.isVar} class:missing={seg.isVar && usedVars.find((v) => v.name === seg.name)?.kind === 'missing'}>{seg.text}</span>{/each}
      </div>
      <input
        class="url-layer url-input"
        placeholder={draft.kind === 'websocket' ? 'wss://echo.example.com/socket' : draft.kind === 'grpc' ? 'https://grpc.example.com:443' : 'https://api.example.com/v1/users  or  {{base_url}}/users'}
        value={draft.url}
        oninput={(e) => setField('url', (e.currentTarget as HTMLInputElement).value)}
        onkeydown={onUrlKeydown}
        onpaste={onUrlPaste}
        spellcheck="false"
        autocomplete="off"
        aria-label="Request URL"
        dir="ltr"
      />
    </div>
    <button class="btn primary send" onclick={send} disabled={sendBusy} title="{sendLabel} (⌘↵)">
      <Icon name="send" size={13} />{sendBusy ? 'Sending…' : sendLabel}
    </button>
    {#if canStop}
      <button class="btn stop" onclick={stopRequest} title="Cancel in-flight request"><Icon name="x" size={12} />Stop</button>
    {/if}
  </div>

  <!-- what the {{variables}} resolve to -->
  {#if usedVars.length > 0}
    <div class="vars" aria-label="Variables used by this request">
      <span class="vars-label">Variables{envForVars ? ` from ${envForVars.name}` : ''}</span>
      {#each usedVars as v (v.name)}
        <span class="var-chip {v.kind}" title={v.detail}>
          <span class="vn mono">{v.name}</span>
          <span class="vv mono">
            {#if v.kind === 'secret'}<Icon name="lock" size={12} />secret
            {:else if v.kind === 'missing'}not set
            {:else if v.kind === 'dynamic'}generated
            {:else}{v.value === '' ? '(empty)' : v.value}{/if}
          </span>
        </span>
      {/each}
      {#if missingVars}<span class="vars-warn">{missingVars === 1 ? '1 variable isn’t set' : `${missingVars} variables aren’t set`}; {envForVars ? 'add it to the environment' : 'pick an environment'} or it’s sent as typed.</span>{/if}
    </div>
  {:else if !compact && !draft.url.trim()}
    <p class="tip">Type or paste a URL, or paste a whole <code>curl</code> command. Use <code>{'{{name}}'}</code> to insert a value from the active environment.</p>
  {/if}

  {#if draft.kind === 'grpc'}
    <div class="grpc-panel">
      <p class="tab-help">gRPC needs the service definition: upload the <code>.proto</code> file, or ask a server with reflection enabled to describe itself.</p>
      <div class="grpc-row">
        <label class="btn small file-btn">
          <Icon name="file" size={12} />{draft.proto?.trim() ? 'Replace .proto…' : 'Upload .proto…'}
          <input type="file" accept=".proto" hidden onchange={(e) => onProtoFile(e.currentTarget as HTMLInputElement)} />
        </label>
        <button class="btn small" onclick={reflectGrpc} disabled={grpcReflecting} title="List services via server reflection (no .proto needed)">
          <Icon name="refresh" size={12} />{grpcReflecting ? 'Loading…' : 'Load from server'}
        </button>
        {#if draft.proto?.trim()}
          <button class="btn small ghost" onclick={parseProto} disabled={grpcParsing}>{grpcParsing ? 'Reading…' : 'Re-read .proto'}</button>
        {/if}
        {#if grpcServices.length > 0}
          <select class="input grpc-method" value={draft.grpc_method} onchange={(e) => onGrpcMethodChange((e.currentTarget as HTMLSelectElement).value)} aria-label="gRPC method">
            {#each grpcServices as svc (svc.name)}
              <optgroup label={svc.name}>
                {#each svc.methods as m (m.full)}<option value={m.full}>{m.name}{m.client_streaming || m.server_streaming ? ' (streaming)' : ''}</option>{/each}
              </optgroup>
            {/each}
          </select>
        {:else if draft.grpc_method}
          <span class="dim-text">Method: <code>{draft.grpc_method}</code></span>
        {/if}
      </div>
    </div>
  {/if}

  {#if draft.kind === 'websocket'}<p class="tab-help">WebSocket connections support query parameters, headers, auth and a connection timeout. They can’t send a body, use an SSH tunnel, follow redirects or skip TLS verification.</p>{/if}
  {#if isStreaming && scriptCount}<p class="tab-help">Scripts only run for HTTP requests; this streaming connection ignores them.</p>{/if}

  <div class="tabstrip" role="tablist" aria-label="Request parts">
    {#each tabs as t, i (t.id)}
      <button class="tab" class:active={tab === t.id} role="tab" aria-selected={tab === t.id} tabindex={tab === t.id ? 0 : -1}
        onclick={() => (tab = t.id)} onkeydown={(e) => onTabKey(e, i)}>
        {t.label}{#if t.count}<span class="count" aria-hidden="true">{t.count}</span>{/if}
      </button>
    {/each}
  </div>

  <div class="tabbody" role="tabpanel">
    {#if tab === 'params'}
      <p class="tab-help">Added to the URL as <code>?key=value</code>. Untick a row to skip it without deleting it.</p>
      {@render kvEditor('query', draft.query)}
    {:else if tab === 'headers'}
      <p class="tab-help">{draft.kind === 'grpc' ? 'gRPC metadata sent with the call.' : 'Sent with the request. Content-Type follows the Body type unless you set it yourself.'}</p>
      {@render kvEditor('headers', draft.headers)}
      {#if draft.kind !== 'grpc'}
        <div class="quick">
          <span class="dim-text">Add:</span>
          {#each QUICK_HEADERS as [k, v] (k)}
            {#if !draft.headers.some((h) => h.key.toLowerCase() === k.toLowerCase())}
              <button class="btn ghost small" onclick={() => addRow('headers', k, v)}>{k}</button>
            {/if}
          {/each}
        </div>
      {/if}
    {:else if tab === 'body' && draft.kind === 'grpc'}
      <div class="bodybar">
        <span class="tab-help inline">The request message, as JSON.</span>
        <span class="grow"></span>
        <button class="btn ghost small" onclick={beautify}>Format JSON</button>
      </div>
      {#if compact}
        <textarea class="input body-area mono" aria-label="Request message" value={draft.body} oninput={(e) => setField('body', (e.currentTarget as HTMLTextAreaElement).value)} placeholder={'{ }'} spellcheck="false"></textarea>
      {:else}
        <div class="body-editor"><CodeEditor path="message.json" content={draft.body} root={ws.current?.root_path ?? ''} language="json" readOnly={false} onchange={(v) => setField('body', v)} /></div>
      {/if}
    {:else if tab === 'body'}
      <div class="bodybar">
        <label class="inline-field">
          <span>Body type</span>
          <select class="input" value={bodyChoice} onchange={(e) => onBodyChoice((e.currentTarget as HTMLSelectElement).value as BodyChoice)} aria-label="Body type">
            {#each BODY_CHOICES as c (c.id)}<option value={c.id}>{c.label}</option>{/each}
          </select>
        </label>
        {#if bodyChoice === 'text'}
          <label class="inline-field">
            <span>Format</span>
            <select class="input" value={rawType} onchange={(e) => onRawType((e.currentTarget as HTMLSelectElement).value as RawType)} aria-label="Text format">
              {#each TEXT_TYPES as t (t)}<option value={t}>{t === 'Text' ? 'Plain text' : t}</option>{/each}
            </select>
          </label>
        {/if}
        <span class="grow"></span>
        {#if rawActive && rawType !== 'Text' && rawType !== 'JavaScript'}
          <button class="btn ghost small" onclick={beautify} title="Pretty-print the body">Format {rawType}</button>
        {/if}
      </div>
      <p class="tab-help">{BODY_CHOICES.find((c) => c.id === bodyChoice)?.help}</p>
      {#if draft.body_mode === 'none'}
        <!-- nothing to edit -->
      {:else if formActive}
        <div class="kv-list">
          <div class="kv-head" aria-hidden="true"><span>Key</span>{#if multipartActive}<span class="kv-type-h">Type</span>{/if}<span>Value</span></div>
          {#each formRows as row, i (i)}
            <div class="kv-row">
              <input class="input kv-key mono" placeholder="key" aria-label="Field name" value={row.key} oninput={(e) => updateFormRow(i, { key: (e.currentTarget as HTMLInputElement).value })} />
              {#if multipartActive}
                <select class="input row-type" value={row.type} onchange={(e) => setRowType(i, (e.currentTarget as HTMLSelectElement).value as FieldType)} aria-label="Field type">
                  <option value="text">Text</option>
                  <option value="file">File</option>
                </select>
              {/if}
              {#if multipartActive && row.type === 'file'}
                <label class="btn small file-btn kv-val" title={row.filename ?? 'Choose a file'}>
                  <Icon name="file" size={12} /><span class="file-name">{row.filename || 'Choose file…'}</span>
                  <input type="file" hidden onchange={(e) => pickFile(i, e.currentTarget as HTMLInputElement)} />
                </label>
              {:else}
                <input class="input kv-val mono" placeholder="value" aria-label="Field value" value={row.value} oninput={(e) => updateFormRow(i, { value: (e.currentTarget as HTMLInputElement).value })} />
              {/if}
              <button class="icon-btn" title="Remove field" aria-label="Remove field" onclick={() => removeFormRow(i)}><Icon name="x" size={12} /></button>
            </div>
          {/each}
          <button class="btn small ghost add-row" onclick={addFormRow}><Icon name="plus" size={12} />Add field</button>
        </div>
      {:else if compact}
        <textarea class="input body-area mono" aria-label="Request body" value={draft.body} oninput={(e) => setField('body', (e.currentTarget as HTMLTextAreaElement).value)}
          placeholder={draft.body_mode === 'json' ? '{ "name": "value" }' : draft.body_mode === 'graphql' ? 'query { viewer { id } }' : 'Body text'} spellcheck="false"></textarea>
      {:else if draft.body_mode === 'graphql'}
        <div class="gql-bar">
          <span class="sub-label">Query</span>
          <span class="grow"></span>
          <button class="btn ghost small" onclick={() => apiClient.graphqlIntrospect()} disabled={apiClient.graphqlIntrospecting}>
            {apiClient.graphqlIntrospecting ? 'Loading schema…' : 'Load schema from server'}
          </button>
        </div>
        <div class="body-editor gql-query"><CodeEditor path="query.graphql" content={draft.body} root={ws.current?.root_path ?? ''} language="" readOnly={false} onchange={(v) => setField('body', v)} /></div>
        <div class="gql-bar"><span class="sub-label">Variables (JSON)</span></div>
        <div class="body-editor gql-vars"><CodeEditor path="variables.json" content={draft.graphql_variables ?? ''} root={ws.current?.root_path ?? ''} language="json" readOnly={false} onchange={(v) => setField('graphql_variables', v)} /></div>
        {#if apiClient.graphqlSchema}
          <div class="gql-schema">
            <div class="sub-label">Schema: {apiClient.graphqlSchema.length} types</div>
            {#each apiClient.graphqlSchema as t (t.name)}
              <details class="gql-type">
                <summary>{t.name} <span class="gql-kind">{t.kind.toLowerCase()}</span></summary>
                <div class="gql-fields mono">{t.fields.join(', ') || '—'}</div>
              </details>
            {/each}
          </div>
        {/if}
      {:else}
        <div class="body-editor">
          <CodeEditor path={editorPath} content={draft.body} root={ws.current?.root_path ?? ''} language={editorLang} readOnly={false} onchange={(v) => setField('body', v)} />
        </div>
      {/if}
    {:else if tab === 'auth'}
      <div class="auth">
        <label class="inline-field">
          <span>Type</span>
          <select class="input" value={draft.auth.type} aria-label="Auth type" onchange={(e) => setAuthType((e.currentTarget as HTMLSelectElement).value as ApiAuth['type'])}>
            {#each AUTH_TYPES as t (t.id)}<option value={t.id}>{t.label}</option>{/each}
          </select>
        </label>
        <p class="tab-help">{AUTH_TYPES.find((t) => t.id === draft.auth.type)?.help}</p>
        {#if draft.auth.type === 'bearer'}
          <div class="field-row">
            <label for="auth-token">Token</label>
            <input id="auth-token" class="input mono grow" value={secretValue(draft.auth.token)} oninput={(e) => setAuth({ token: (e.currentTarget as HTMLInputElement).value })} placeholder={secretPlaceholder(draft.auth.token, 'Paste a token, or {{api_token}}')} />
          </div>
        {:else if draft.auth.type === 'basic'}
          <div class="field-row">
            <label for="auth-user">Username</label>
            <input id="auth-user" class="input grow" value={draft.auth.username} oninput={(e) => setAuth({ username: (e.currentTarget as HTMLInputElement).value })} />
          </div>
          <div class="field-row">
            <label for="auth-pass">Password</label>
            <input id="auth-pass" class="input grow" type="password" value={secretValue(draft.auth.password)} oninput={(e) => setAuth({ password: (e.currentTarget as HTMLInputElement).value })} placeholder={secretPlaceholder(draft.auth.password, '')} />
          </div>
        {:else if draft.auth.type === 'api_key'}
          <div class="field-row">
            <label for="auth-key">Key name</label>
            <input id="auth-key" class="input mono grow" value={draft.auth.key} oninput={(e) => setAuth({ key: (e.currentTarget as HTMLInputElement).value })} placeholder="X-Api-Key" />
          </div>
          <div class="field-row">
            <label for="auth-value">Value</label>
            <input id="auth-value" class="input mono grow" value={secretValue(draft.auth.value)} oninput={(e) => setAuth({ value: (e.currentTarget as HTMLInputElement).value })} placeholder={secretPlaceholder(draft.auth.value, 'The key, or {{api_key}}')} />
          </div>
          <div class="field-row">
            <label for="auth-in">Send as</label>
            <select id="auth-in" class="input" value={draft.auth.in} onchange={(e) => setAuth({ in: (e.currentTarget as HTMLSelectElement).value as 'header' | 'query' })}>
              <option value="header">A header</option>
              <option value="query">A query parameter</option>
            </select>
          </div>
        {:else if draft.auth.type === 'oauth2'}
          <div class="field-row">
            <label for="auth-grant">How to sign in</label>
            <select id="auth-grant" class="input" value={draft.auth.grant} onchange={(e) => setAuth({ grant: (e.currentTarget as HTMLSelectElement).value as 'client_credentials' | 'password' | 'refresh_token' | 'authorization_code' })}>
              <option value="authorization_code">In the browser (authorization code + PKCE)</option>
              <option value="client_credentials">Client credentials (app-to-app)</option>
              <option value="password">Username and password</option>
              <option value="refresh_token">Refresh token</option>
            </select>
          </div>
          {#if draft.auth.grant === 'authorization_code'}
            <div class="field-row"><label for="auth-aurl">Authorization URL</label><input id="auth-aurl" class="input mono grow" value={draft.auth.authorization_url ?? ''} oninput={(e) => setAuth({ authorization_url: e.currentTarget.value })} placeholder="https://auth.example.com/oauth/authorize" /></div>
            <p class="tab-help">Register this callback URL with your provider: <code>{baseUrl().replace(/\/$/, '')}/api/v1/api-client/oauth2/callback</code>. “Get token” saves the request, opens your browser and stores the tokens in the Keychain.</p>
          {/if}
          <div class="field-row">
            <label for="auth-turl">Token URL</label>
            <input id="auth-turl" class="input mono grow" value={draft.auth.token_url} oninput={(e) => setAuth({ token_url: (e.currentTarget as HTMLInputElement).value })} placeholder="https://auth.example.com/oauth/token" />
          </div>
          <div class="field-row">
            <label for="auth-cid">Client ID</label>
            <input id="auth-cid" class="input mono grow" value={draft.auth.client_id} oninput={(e) => setAuth({ client_id: (e.currentTarget as HTMLInputElement).value })} />
          </div>
          <div class="field-row">
            <label for="auth-csec">Client secret</label>
            <input id="auth-csec" class="input mono grow" type="password" value={secretValue(draft.auth.client_secret)} oninput={(e) => setAuth({ client_secret: (e.currentTarget as HTMLInputElement).value })} placeholder={secretPlaceholder(draft.auth.client_secret, '')} />
          </div>
          {#if draft.auth.grant === 'password'}
            <div class="field-row">
              <label for="auth-ouser">Username</label>
              <input id="auth-ouser" class="input grow" value={draft.auth.username} oninput={(e) => setAuth({ username: (e.currentTarget as HTMLInputElement).value })} />
            </div>
            <div class="field-row">
              <label for="auth-opass">Password</label>
              <input id="auth-opass" class="input grow" type="password" value={secretValue(draft.auth.password)} oninput={(e) => setAuth({ password: (e.currentTarget as HTMLInputElement).value })} placeholder={secretPlaceholder(draft.auth.password, '')} />
            </div>
          {/if}
          {#if draft.auth.grant === 'refresh_token'}
            <div class="field-row">
              <label for="auth-rt">Refresh token</label>
              <input id="auth-rt" class="input mono grow" value={secretValue(draft.auth.refresh_token)} oninput={(e) => setAuth({ refresh_token: (e.currentTarget as HTMLInputElement).value })} placeholder={secretPlaceholder(draft.auth.refresh_token, '')} />
            </div>
          {/if}
          <div class="field-row">
            <label for="auth-scope">Scope</label>
            <input id="auth-scope" class="input mono grow" value={draft.auth.scope} oninput={(e) => setAuth({ scope: (e.currentTarget as HTMLInputElement).value })} placeholder="read write" />
          </div>
          <div class="field-row">
            <span class="field-spacer"></span>
            <button class="btn small" onclick={fetchOAuthToken} disabled={fetchingToken}>
              <Icon name="key" size={12} />{fetchingToken ? 'Getting token…' : 'Get token'}
            </button>
            {#if isSecretRef(draft.auth.access_token) || draft.auth.access_token}
              <span class="chip ok">{isSecretRef(draft.auth.access_token) ? 'Access token stored in Keychain' : 'Access token received'}</span>
            {/if}
          </div>
        {/if}
        {#if draft.auth.type !== 'none'}
          <p class="keychain-note"><Icon name="lock" size={12} /><span>When you save, tokens and passwords move to the macOS Keychain. They’re never written to disk, shown again, or included in exports. To share one secret across requests, put it in an environment and use a <code>{'{{variable}}'}</code>.</span></p>
        {/if}
      </div>
    {:else if tab === 'scripts'}
      <div class="scripts-pane">
        <p class="tab-help">Optional JavaScript with a Postman-style <code>pm</code> object. Scripts run in a sandboxed worker for up to 5 seconds and are saved with the request.</p>
        <div class="script-block">
          <div class="script-head">
            <span class="sub-label">Before sending</span>
            <span class="script-hint mono">pm.environment.set('id', '42') · pm.request.headers.upsert(…)</span>
          </div>
          <div class="script-editor">
            <CodeEditor path="pre.js" content={draft.pre_request_script ?? ''} root={ws.current?.root_path ?? ''} language="js" readOnly={false} onchange={(v) => setField('pre_request_script', v)} />
          </div>
        </div>
        <div class="script-block">
          <div class="script-head">
            <span class="sub-label">After the response (tests)</span>
            <span class="script-hint mono">pm.test('is 200', () =&gt; pm.expect(pm.response.code).toBe(200))</span>
          </div>
          <div class="script-editor">
            <CodeEditor path="post.js" content={draft.post_response_script ?? ''} root={ws.current?.root_path ?? ''} language="js" readOnly={false} onchange={(v) => setField('post_response_script', v)} />
          </div>
        </div>
      </div>
    {:else if tab === 'docs'}
      <p class="tab-help">Notes for whoever uses this request next, in Markdown. Saved with the request and exported to OpenAPI and Postman.</p>
      <div class="docs-pane">
        <div class="docs-edit">
          <CodeEditor path="docs.md" content={draft.docs ?? ''} root={ws.current?.root_path ?? ''} language="md" readOnly={false} onchange={(v) => setField('docs', v)} />
        </div>
        {#if !compact}
          <div class="docs-preview md-body" aria-label="Docs preview">
            {#if draft.docs?.trim()}{@html docsHtml}{:else}<p class="dim-text">The preview appears here.</p>{/if}
          </div>
        {/if}
      </div>
    {:else if tab === 'settings'}
      <div class="settings-pane">
        <label class="set-row">
          <span class="set-label">Timeout</span>
          <span class="set-control">
            <input class="input set-num mono" type="number" min="0" step="1000" placeholder="Default"
              value={settings.timeout_ms ?? ''}
              oninput={(e) => { const v = (e.currentTarget as HTMLInputElement).value; setSetting('timeout_ms', v === '' ? null : Number(v)); }} />
            <span class="set-unit">milliseconds · empty = 60 seconds</span>
          </span>
        </label>
        <label class="set-row toggle">
          <input type="checkbox" checked={settings.follow_redirects} onchange={(e) => setSetting('follow_redirects', (e.currentTarget as HTMLInputElement).checked)} />
          <span class="set-label">Follow redirects</span>
          <span class="set-unit">Up to 10; each hop is checked like the original URL.</span>
        </label>
        <label class="set-row toggle">
          <input type="checkbox" checked={settings.verify_ssl} onchange={(e) => setSetting('verify_ssl', (e.currentTarget as HTMLInputElement).checked)} />
          <span class="set-label">Verify TLS certificates</span>
          <span class="set-unit">Turn off only for self-signed development servers.</span>
        </label>
        {#if draft.kind === 'http'}
          <label class="set-row">
            <span class="set-label">Send through</span>
            <span class="set-control">
              <select class="input set-select" value={draft.ssh_connection_id ?? ''}
                onchange={(e) => { const v = (e.currentTarget as HTMLSelectElement).value; apiClient.draft = { ...draft, ssh_connection_id: v === '' ? null : v }; }}>
                <option value="">Directly from this Mac</option>
                {#each apiClient.sshConnections as c (c.id)}<option value={c.id}>SSH tunnel: {c.name}</option>{/each}
              </select>
              <span class="set-unit">
                {apiClient.sshConnections.length === 0 ? 'Add an SSH connection on the Connections page to call IP-restricted APIs from a bastion.' : 'For APIs that only accept calls from a whitelisted IP.'}
              </span>
            </span>
          </label>
        {/if}
        <div class="ws-setting">
          <div class="section-title">Workspace</div>
          <label class="set-row toggle">
            <input type="checkbox" checked={ws.apiAllowLocal} onchange={(e) => { const on = (e.currentTarget as HTMLInputElement).checked; void ws.setApiAllowLocal(on).catch(() => { (e.currentTarget as HTMLInputElement).checked = !on; toasts.error('Couldn’t change the setting', 'Only a workspace admin can change it.'); }); }} />
            <span class="set-label">Allow private addresses</span>
            <span class="set-unit">Lets API requests reach localhost and private networks (10.x, 192.168.x…). Off by default; admins only; applies to everyone in this workspace.</span>
          </label>
        </div>
      </div>
    {/if}
  </div>
</div>

{#snippet kvEditor(which: 'headers' | 'query', rows: ApiKeyVal[])}
  <div class="kv-list">
    {#if which === 'headers'}
      <datalist id="hdr-keys">{#each COMMON_HEADERS as h}<option value={h}></option>{/each}</datalist>
    {/if}
    {#if rows.length > 0}
      <div class="kv-head" aria-hidden="true"><span class="kv-check-h"></span><span>{which === 'query' ? 'Parameter' : 'Header'}</span><span>Value</span></div>
    {/if}
    {#each rows as row, i (i)}
      <div class="kv-row" class:off={row.enabled === false}>
        <input class="kv-check" type="checkbox" checked={row.enabled !== false} aria-label="Send {row.key || 'this row'}" title={row.enabled === false ? 'Skipped: tick to send' : 'Sent: untick to skip'}
          onchange={(e) => updateRow(which, i, { enabled: (e.currentTarget as HTMLInputElement).checked })} />
        <input class="input kv-key mono" placeholder="key" aria-label="{which === 'query' ? 'Parameter' : 'Header'} name" value={row.key}
          list={which === 'headers' ? 'hdr-keys' : undefined} autocomplete="off"
          oninput={(e) => updateRow(which, i, { key: (e.currentTarget as HTMLInputElement).value })} />
        <input class="input kv-val mono" placeholder="value" aria-label="Value" value={row.value}
          list={which === 'headers' && headerValues(row.key).length > 0 ? `hdr-vals-${i}` : undefined} autocomplete="off"
          oninput={(e) => updateRow(which, i, { value: (e.currentTarget as HTMLInputElement).value })} />
        {#if which === 'headers' && headerValues(row.key).length > 0}
          <datalist id={`hdr-vals-${i}`}>{#each headerValues(row.key) as v}<option value={v}></option>{/each}</datalist>
        {/if}
        <button class="icon-btn" title="Remove row" aria-label="Remove row" onclick={() => removeRow(which, i)}><Icon name="x" size={12} /></button>
      </div>
    {/each}
    <button class="btn small ghost add-row" onclick={() => addRow(which)}>
      <Icon name="plus" size={12} />Add {which === 'query' ? 'param' : 'header'}
    </button>
  </div>
{/snippet}

{#if saveOpen}
  <SaveRequestDialog
    initialName={draft.name?.trim() || apiClient.tabLabel({ ...draft, name: '' })}
    initialCollection={saved?.collection_id ?? draft.collectionHint ?? null}
    onclose={() => (saveOpen = false)}
    onsave={async (name, collectionId) => !!(await apiClient.saveDraft(name, collectionId))}
  />
{/if}

{#if importOpen}
  <ImportDialog initial="curl" onclose={() => (importOpen = false)} />
{/if}

{#if codeOpen}
  <Modal title="Generate code" width={640} onclose={() => (codeOpen = false)}>
    <div class="code-head">
      <p class="tab-help">This request as code you can paste into a script. Stored secrets appear as <code>***</code>.</p>
      <select class="input" value={codeLang} onchange={(e) => (codeLang = (e.currentTarget as HTMLSelectElement).value as CodeLang)} aria-label="Language">
        {#each CODE_LANGS as l (l.id)}<option value={l.id}>{l.label}</option>{/each}
      </select>
    </div>
    <pre class="code-snippet mono" dir="ltr">{codeSnippet}</pre>
    {#snippet footer()}
      <button class="btn" onclick={() => (codeOpen = false)}>Close</button>
      <button class="btn primary" onclick={() => void copyText(codeSnippet, 'code')}><Icon name="copy" size={13} />Copy code</button>
    {/snippet}
  </Modal>
{/if}

{#if cookiesOpen}
  <Modal title="Cookie jar" width={620} onclose={() => (cookiesOpen = false)}>
    <p class="tab-help">Cookies that responses in this workspace set. Otto sends them back on later requests to the same site, like a browser. They’re kept in memory until the daemon restarts.</p>
    {#if apiClient.cookies.length === 0}
      <p class="dim-text">No cookies yet.</p>
    {:else}
      <table class="cookie-table">
        <thead><tr><th>Site</th><th>Name</th><th>Value</th></tr></thead>
        <tbody>
          {#each apiClient.cookies as c (c.domain + c.name)}
            <tr><td class="mono">{c.domain}</td><td class="mono">{c.name}</td><td class="mono cookie-val" title={c.value}>{c.value}</td></tr>
          {/each}
        </tbody>
      </table>
    {/if}
    {#snippet footer()}
      <button class="btn danger" onclick={clearCookies} disabled={apiClient.cookies.length === 0}>Clear cookies…</button>
      <span class="grow"></span>
      <button class="btn" onclick={() => void apiClient.loadCookies()}>Refresh</button>
      <button class="btn primary" onclick={() => (cookiesOpen = false)}>Done</button>
    {/snippet}
  </Modal>
{/if}

{#if varsOpen}
  <Modal title="Session variables" width={560} onclose={() => (varsOpen = false)}>
    <p class="tab-help">Temporary values for this session only. They override the environment’s values of the same name and are cleared when Otto restarts. Scripts set them with <code>pm.environment.set()</code>.</p>
    <div class="kv-list">
      {#each varEntries as [k, v] (k)}
        <div class="kv-row">
          <input class="input kv-key mono" aria-label="Variable name" value={k} onchange={(e) => apiClient.renameRuntimeVar(k, (e.currentTarget as HTMLInputElement).value)} />
          <input class="input kv-val mono" aria-label="Value of {k}" value={v} oninput={(e) => apiClient.setRuntimeVar(k, (e.currentTarget as HTMLInputElement).value)} />
          <button class="icon-btn" title="Remove variable" aria-label="Remove variable" onclick={() => apiClient.removeRuntimeVar(k)}><Icon name="x" size={12} /></button>
        </div>
      {/each}
      <div class="kv-row">
        <input class="input kv-key mono" placeholder="name" aria-label="New variable name" bind:value={newVarKey} onkeydown={(e) => { if (e.key === 'Enter') addVar(); }} />
        <input class="input kv-val mono" placeholder="value" aria-label="New variable value" bind:value={newVarVal} onkeydown={(e) => { if (e.key === 'Enter') addVar(); }} />
        <button class="icon-btn" title="Add variable" aria-label="Add variable" onclick={addVar}><Icon name="plus" size={12} /></button>
      </div>
    </div>
    {#snippet footer()}
      <button class="btn primary" onclick={() => (varsOpen = false)}>Done</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .builder {
    display: flex;
    flex-direction: column;
    min-height: 0;
    gap: 10px;
  }
  .name-row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .name-input {
    flex: 0 1 auto;
    min-width: 80px;
    max-width: 60%;
    height: 26px;
    padding: 0 6px;
    margin-inline-start: -6px;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .compact .name-input {
    font-size: var(--fs-m);
    width: auto;
    flex: 1;
  }
  .name-input:hover {
    border-color: var(--border);
  }
  .name-input:focus {
    outline: none;
    border-color: var(--accent);
    background: var(--surface-2);
  }
  .name-input::placeholder {
    color: var(--text-dim);
  }
  .where {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .edited {
    color: var(--warning);
  }
  .urlbar {
    display: flex;
    gap: 6px;
    align-items: stretch;
  }
  .kind,
  .method {
    height: 32px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-s);
    font-weight: 600;
    padding: 0 6px;
    cursor: pointer;
    flex-shrink: 0;
  }
  .kind {
    font-weight: 500;
  }
  .method {
    font-family: var(--font-mono);
  }
  .url-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
  }
  /* Input and highlight share font, size, padding and border so every
     character sits in the same place (the old layer drifted). */
  .url-layer {
    height: 32px;
    line-height: 30px;
    padding: 0 10px;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    letter-spacing: 0;
    white-space: pre;
    overflow: hidden;
    border: 1px solid transparent;
    border-radius: var(--radius-s);
    box-sizing: border-box;
  }
  .url-input {
    width: 100%;
    border-color: var(--border);
    background: transparent;
    color: var(--text);
    position: relative;
    z-index: 1;
  }
  .url-input:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .url-input::placeholder {
    color: var(--text-dim);
  }
  .url-highlight {
    position: absolute;
    inset: 0;
    z-index: 0;
    color: transparent;
    background: var(--surface-2);
    pointer-events: none;
  }
  .url-highlight .var {
    background: var(--accent-soft);
    border-radius: var(--radius-s);
  }
  .url-highlight .var.missing {
    background: var(--warning-soft);
  }
  .send {
    height: 32px;
    padding: 0 16px;
    flex-shrink: 0;
  }
  .stop {
    height: 32px;
    flex-shrink: 0;
    color: var(--danger);
  }
  .vars {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--fs-xs);
    min-width: 0;
  }
  .vars-label {
    color: var(--text-dim);
  }
  .var-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 320px;
    height: 20px;
    padding: 0 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    cursor: help;
  }
  .vn {
    font-size: var(--fs-xs);
    color: var(--text);
  }
  .vv {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .var-chip.missing {
    border-color: color-mix(in srgb, var(--warning) 40%, transparent);
    background: var(--warning-soft);
  }
  .var-chip.missing .vv {
    color: var(--warning);
  }
  .vars-warn {
    color: var(--warning);
  }
  .tip,
  .tab-help {
    margin: 0;
    font-size: var(--fs-s);
    line-height: 1.45;
    color: var(--text-dim);
  }
  .tab-help.inline {
    display: inline;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    color: var(--text);
  }
  .dim-text {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .grpc-panel {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .grpc-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .grpc-method {
    flex: 1;
    min-width: 160px;
  }
  .file-btn {
    cursor: pointer;
    min-width: 0;
  }
  .file-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tabstrip {
    display: flex;
    gap: 2px;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
    scrollbar-width: none;
    flex-shrink: 0;
  }
  .tab {
    position: relative;
    height: 30px;
    padding: 0 10px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-m);
    font-weight: 500;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
    white-space: nowrap;
  }
  .tab:hover {
    color: var(--text);
  }
  .tab.active {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .count {
    margin-inline-start: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-weight: 500;
  }
  .tabbody {
    min-height: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .kv-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .kv-head,
  .kv-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .kv-head {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  .kv-head > span:nth-child(2) {
    flex: 0 1 38%;
  }
  .kv-check-h {
    width: 13px;
  }
  .kv-type-h {
    width: 72px;
  }
  .kv-row.off .kv-key,
  .kv-row.off .kv-val {
    color: var(--text-dim);
    text-decoration: line-through;
  }
  .kv-check {
    flex-shrink: 0;
    accent-color: var(--accent);
    margin: 0;
  }
  .kv-key {
    flex: 0 1 38%;
    min-width: 0;
    font-size: var(--fs-s);
  }
  .kv-val {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-s);
  }
  .row-type {
    width: 72px;
    flex-shrink: 0;
  }
  .add-row {
    align-self: flex-start;
  }
  .quick {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-wrap: wrap;
  }
  .bodybar {
    display: flex;
    align-items: center;
    gap: 12px;
    flex-wrap: wrap;
  }
  .inline-field {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .body-area {
    width: 100%;
    min-height: 120px;
    font-size: var(--fs-s);
  }
  .body-editor {
    height: 240px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .compact .body-editor {
    height: 160px;
  }
  .auth {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .field-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .field-row > label,
  .field-spacer {
    width: 120px;
    flex-shrink: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .keychain-note {
    display: flex;
    align-items: flex-start;
    gap: 6px;
    margin: 4px 0 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .keychain-note :global(svg) {
    margin-top: 2px;
    flex-shrink: 0;
  }
  .sub-label {
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .gql-bar {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .gql-query { height: 150px; }
  .gql-vars { height: 90px; }
  .gql-schema {
    max-height: 200px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .gql-type > summary {
    cursor: pointer;
    font-size: var(--fs-s);
    padding: 3px 4px;
  }
  .gql-kind {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .gql-fields {
    color: var(--text-dim);
    padding: 2px 14px 6px;
    word-break: break-word;
  }
  .docs-pane {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    gap: 12px;
  }
  .compact .docs-pane {
    grid-template-columns: minmax(0, 1fr);
  }
  .docs-edit {
    height: 200px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .docs-preview {
    height: 200px;
    overflow: auto;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
  }
  .scripts-pane {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .script-head {
    display: flex;
    align-items: baseline;
    gap: 10px;
    margin-bottom: 4px;
    min-width: 0;
  }
  .script-hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .script-editor {
    height: 140px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .settings-pane {
    display: flex;
    flex-direction: column;
    gap: 12px;
    padding: 2px;
  }
  .set-row {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: var(--fs-m);
    color: var(--text);
    flex-wrap: wrap;
  }
  .set-row.toggle {
    cursor: pointer;
  }
  .set-row.toggle input {
    accent-color: var(--accent);
    flex-shrink: 0;
    margin: 0;
  }
  .set-label {
    min-width: 150px;
  }
  .set-control {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .set-num {
    width: 110px;
  }
  .set-select {
    min-width: 200px;
    max-width: 300px;
  }
  .set-unit {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ws-setting .section-title {
    margin: 6px 0 8px;
  }
  .code-head {
    display: flex;
    align-items: flex-start;
    gap: 12px;
    margin-bottom: 8px;
  }
  .code-head .tab-help {
    flex: 1;
  }
  .code-snippet {
    margin: 0;
    max-height: 360px;
    overflow: auto;
    white-space: pre;
    line-height: 1.5;
    padding: 10px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    user-select: text;
  }
  .cookie-table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
    margin-top: 8px;
  }
  .cookie-table th {
    text-align: start;
    color: var(--text-dim);
    font-weight: 600;
    font-size: var(--fs-xs);
    padding: 4px 6px;
    border-bottom: 1px solid var(--border);
  }
  .cookie-table td {
    padding: 4px 6px;
    border-bottom: 1px solid var(--border);
    max-width: 200px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  @media (max-width: 640px) {
    .urlbar {
      flex-wrap: wrap;
    }
    .url-wrap {
      flex: 1 1 100%;
      order: 3;
    }
    .send {
      margin-inline-start: auto;
    }
    .kv-row {
      flex-wrap: wrap;
    }
    .kv-key,
    .kv-val {
      flex: 1 1 calc(50% - 24px);
    }
    .field-row {
      flex-wrap: wrap;
    }
    .field-row > label,
    .field-spacer {
      width: 100%;
    }
    .docs-pane {
      grid-template-columns: minmax(0, 1fr);
    }
    .name-input {
      width: auto;
      flex: 1;
    }
  }
</style>
