<script lang="ts">
  // Shared request builder used by both the full API page and the compact
  // right-panel. Method + URL + Send, plus a Params/Headers/Body/Auth tab strip,
  // an "Import curl" paste box, "Copy as curl", and "Save".
  import Icon from '../../lib/components/Icon.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import { apiClient, HTTP_METHODS, defaultSettings, type ApiDraft, type ApiRequestKind, type ApiSettings } from '../../lib/stores/apiClient.svelte';
  import { apiStream } from '../../lib/stores/apiStream.svelte';
  import { api } from '../../lib/api/client';
  import { generateCode, CODE_LANGS, type CodeLang } from '../../lib/api/codegen';
  import { marked } from 'marked';
  import type { ApiAuth, ApiBodyMode, ApiKeyVal, ApiResponse } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';

  interface Props {
    /** Compact mode trims spacing + uses a textarea instead of CodeEditor. */
    compact?: boolean;
  }
  let { compact = false }: Props = $props();

  type Tab = 'params' | 'headers' | 'body' | 'auth' | 'settings' | 'scripts' | 'docs';
  let tab: Tab = $state('params');

  // The draft lives in the store so the page + panel share one editing target.
  const draft = $derived(apiClient.draft);

  const authTypes: ApiAuth['type'][] = ['none', 'bearer', 'basic', 'api_key', 'oauth2'];

  // ── Postman-style body modes ───────────────────────────────────────────────
  // The "raw" radio covers both the 'raw' and 'json' backend modes; its precise
  // content-type is chosen by the sub-dropdown (Text/JavaScript/JSON/HTML/XML).
  type RawType = 'Text' | 'JavaScript' | 'JSON' | 'HTML' | 'XML';
  const RAW_TYPES: RawType[] = ['Text', 'JavaScript', 'JSON', 'HTML', 'XML'];

  interface BodyRadio { id: ApiBodyMode | 'binary'; label: string; disabled?: boolean; }
  const BODY_RADIOS: BodyRadio[] = [
    { id: 'none', label: 'none' },
    { id: 'multipart', label: 'form-data' },
    { id: 'form', label: 'x-www-form-urlencoded' },
    { id: 'raw', label: 'raw' },
    { id: 'binary', label: 'binary', disabled: true },
    { id: 'graphql', label: 'GraphQL' },
  ];

  // The "raw" radio is selected for either backend mode.
  const rawActive = $derived(draft.body_mode === 'raw' || draft.body_mode === 'json');
  // Both form-flavoured modes share the key/value editor.
  const formActive = $derived(draft.body_mode === 'form' || draft.body_mode === 'multipart');
  // Beautify only makes sense for the raw text editor (json/raw).
  const showBeautify = $derived(rawActive);

  function isModeActive(id: ApiBodyMode | 'binary'): boolean {
    return id === 'raw' ? rawActive : draft.body_mode === id;
  }

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
    return 'JSON'; // sensible default when entering raw from another mode
  }
  function currentContentType(): string {
    return draft.headers.find((h) => h.key.trim().toLowerCase() === 'content-type')?.value ?? '';
  }

  // Editor language/path follow the raw sub-type (graphql → plain).
  const RAW_PATH: Record<RawType, string> = {
    Text: 'body.txt', JavaScript: 'body.js', JSON: 'body.json', HTML: 'body.html', XML: 'body.xml',
  };
  const RAW_LANG: Record<RawType, string> = {
    Text: '', JavaScript: 'js', JSON: 'json', HTML: 'html', XML: 'xml',
  };
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
      default: return null; // none, binary
    }
  }
  /** Returns headers with the implied Content-Type applied (custom values kept). */
  function withAutoContentType(rows: ApiKeyVal[], ct: string | null): ApiKeyVal[] {
    const headers = rows.map((r) => ({ ...r }));
    const idx = headers.findIndex((h) => h.key.trim().toLowerCase() === 'content-type');
    if (ct === null) {
      if (idx >= 0 && AUTO_CONTENT_TYPES.has(headers[idx].value.trim().toLowerCase())) {
        headers.splice(idx, 1);
      }
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
  function onBodyRadio(id: ApiBodyMode | 'binary'): void {
    if (id === 'binary') return; // not supported yet
    if (id === 'raw') { applyBody('json', 'JSON'); return; } // default raw → JSON
    applyBody(id, 'Text');
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
      if (rawType === 'JSON' || draft.kind === 'grpc') {
        out = JSON.stringify(JSON.parse(body), null, 2);
      } else if (rawType === 'XML' || rawType === 'HTML') {
        out = beautifyXml(body);
      } else {
        toasts.info('Nothing to beautify', 'Choose JSON, XML or HTML.');
        return;
      }
      setField('body', out);
    } catch {
      toasts.error('Beautify failed', rawType === 'JSON' ? 'Body is not valid JSON.' : 'Could not format the body.');
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

  // Split the URL into segments so `{{var}}` tokens can be highlighted behind
  // the (transparent-background) input.
  const urlSegments = $derived(splitVars(draft.url));
  function splitVars(s: string): { text: string; isVar: boolean }[] {
    const out: { text: string; isVar: boolean }[] = [];
    const re = /\{\{[^}]+\}\}/g;
    let last = 0;
    let m: RegExpExecArray | null;
    while ((m = re.exec(s))) {
      if (m.index > last) out.push({ text: s.slice(last, m.index), isVar: false });
      out.push({ text: m[0], isVar: true });
      last = m.index + m[0].length;
    }
    if (last < s.length) out.push({ text: s.slice(last), isVar: false });
    return out;
  }

  // ── key/value rows ────────────────────────────────────────────────────────

  function addRow(which: 'headers' | 'query'): void {
    apiClient.draft = { ...draft, [which]: [...draft[which], { key: '', value: '', enabled: true }] };
  }
  function updateRow(
    which: 'headers' | 'query',
    i: number,
    patch: Partial<ApiKeyVal>,
  ): void {
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
  //
  // Two wire formats share these rows:
  //   • x-www-form-urlencoded ('form')  → `k=v&…` (text only, like before)
  //   • multipart/form-data ('multipart') → JSON `[{key,type,value,filename}]`,
  //     so a row can be a File (value = base64 of the file's bytes).
  type FieldType = 'text' | 'file';
  interface FormRow { key: string; value: string; type: FieldType; filename?: string; }
  function parseForm(s: string, mode: ApiBodyMode): FormRow[] {
    if (mode === 'multipart' && s.trim().startsWith('[')) {
      try {
        const arr = JSON.parse(s) as Partial<FormRow>[];
        return arr.map((r) => ({
          key: r.key ?? '',
          value: r.value ?? '',
          type: r.type === 'file' ? 'file' : 'text',
          filename: r.filename,
        }));
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
      return JSON.stringify(
        live.map((r) =>
          r.type === 'file'
            ? { key: r.key, type: 'file', value: r.value, filename: r.filename ?? '' }
            : { key: r.key, type: 'text', value: r.value },
        ),
      );
    }
    // urlencoded: text only (file rows degrade to their (empty) value).
    return live.map((r) => `${encodeURIComponent(r.key)}=${encodeURIComponent(r.value)}`).join('&');
  }
  function decode(s: string): string {
    try { return decodeURIComponent(s.replace(/\+/g, ' ')); } catch { return s; }
  }

  let formRows = $state<FormRow[]>([]);
  let lastFormEncoded = $state('');
  // Seed (on mount) and re-seed when the body changes EXTERNALLY (import curl /
  // load a saved request) — but NOT from our own encodeForm writes (tracked by
  // lastFormEncoded). Empty rows live only in `formRows`, so "Add field" sticks.
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
  /** True for multipart/form-data, where rows may be Files. */
  const multipartActive = $derived(draft.body_mode === 'multipart');
  /** Read the chosen file into base64 and store it on the row. */
  async function pickFile(i: number, input: HTMLInputElement): Promise<void> {
    const file = input.files?.[0];
    if (!file) return;
    try {
      const b64 = await fileToBase64(file);
      updateFormRow(i, { value: b64, filename: file.name });
    } catch {
      toasts.error('Could not read file', file.name);
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
    // Switching type clears the value (text⇄file are not interchangeable).
    updateFormRow(i, { type, value: '', filename: undefined });
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
    'content-type': [
      'application/json', 'application/x-www-form-urlencoded', 'multipart/form-data',
      'text/plain', 'text/html', 'application/xml', 'text/xml', 'application/octet-stream',
      'application/graphql',
    ],
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

  // ── field setters ──────────────────────────────────────────────────────────

  function setField<K extends keyof ApiDraft>(k: K, v: ApiDraft[K]): void {
    apiClient.draft = { ...draft, [k]: v };
  }
  const docsHtml = $derived.by(() => {
    try { return marked.parse(draft.docs ?? '', { async: false, gfm: true, breaks: true }) as string; }
    catch { return ''; }
  });
  const settings = $derived(draft.settings ?? defaultSettings());
  const settingsBadge = $derived(
    settings.timeout_ms != null || !settings.follow_redirects || !settings.verify_ssl ? 1 : 0,
  );
  function setSetting<K extends keyof ApiSettings>(k: K, v: ApiSettings[K]): void {
    apiClient.draft = { ...draft, settings: { ...settings, [k]: v } };
  }
  function setAuth(patch: Partial<ApiAuth>): void {
    apiClient.draft = { ...draft, auth: { ...draft.auth, ...patch } as ApiAuth };
  }
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
  let fetchingToken = $state(false);
  async function fetchOAuthToken(): Promise<void> {
    if (draft.auth.type !== 'oauth2') return;
    const wid = ws.currentId;
    if (!wid) return;
    const a = draft.auth;
    if (!a.token_url.trim()) { toasts.error('No token URL', 'Set the OAuth2 token URL first.'); return; }
    fetchingToken = true;
    try {
      const res = await api.post<{ access_token: string; token_type?: string; refresh_token?: string; expires_in?: number }>(
        `/workspaces/${wid}/api-client/oauth2/token`,
        { grant: a.grant, token_url: a.token_url, client_id: a.client_id, client_secret: a.client_secret, scope: a.scope, username: a.username, password: a.password, refresh_token: a.refresh_token },
      );
      setAuth({ access_token: res.access_token, token_type: res.token_type || 'Bearer', refresh_token: res.refresh_token || a.refresh_token });
      toasts.success('Token acquired', `${res.token_type || 'Bearer'} · expires in ${res.expires_in ?? '?'}s`);
    } catch (e) {
      toasts.error('Token request failed', e instanceof Error ? e.message : String(e));
    } finally {
      fetchingToken = false;
    }
  }
  // ── request kind (HTTP / SSE / WebSocket / gRPC) ────────────────────────────
  const REQUEST_KINDS: { id: ApiRequestKind; label: string }[] = [
    { id: 'http', label: 'HTTP' },
    { id: 'sse', label: 'SSE' },
    { id: 'websocket', label: 'WebSocket' },
    { id: 'grpc', label: 'gRPC' },
  ];
  const isStreaming = $derived(draft.kind === 'sse' || draft.kind === 'websocket');
  function setKind(k: ApiRequestKind): void {
    if (apiStream.active) apiStream.disconnect();
    apiClient.draft = { ...draft, kind: k };
  }
  // Keep the active tab valid for the current kind's tab strip.
  $effect(() => {
    const allowed =
      draft.kind === 'grpc' ? ['body', 'headers']
      : draft.kind === 'websocket' ? ['headers', 'auth', 'settings']
      : ['params', 'auth', 'headers', 'body', 'scripts', 'docs', 'settings'];
    if (!allowed.includes(tab)) tab = allowed[0] as Tab;
  });

  // ── gRPC state ──────────────────────────────────────────────────────────────
  interface GrpcMethod {
    name: string; full: string; input_type: string; output_type: string;
    input_schema: string; client_streaming: boolean; server_streaming: boolean;
  }
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
    if (!draft.proto?.trim()) { toasts.error('No .proto', 'Upload or paste a .proto first.'); return; }
    grpcParsing = true;
    try {
      const res = await api.post<{ services: GrpcService[] }>(
        `/workspaces/${wid}/api-client/grpc/describe`, { proto: draft.proto });
      grpcServices = res.services;
      const first = res.services.find((s) => s.methods.length);
      if (first && !draft.grpc_method) selectGrpcMethod(first.methods[0]);
      toasts.success('Parsed .proto', `${res.services.length} service(s)`);
    } catch (e) {
      toasts.error('Parse failed', e instanceof Error ? e.message : String(e));
    } finally {
      grpcParsing = false;
    }
  }
  let grpcReflecting = $state(false);
  async function reflectGrpc(): Promise<void> {
    const wid = ws.currentId;
    if (!wid) return;
    if (!draft.url.trim()) { toasts.error('No URL', 'Enter the gRPC server URL first.'); return; }
    grpcReflecting = true;
    try {
      // Reflection-based: clear any uploaded proto so invoke uses reflection too.
      apiClient.draft = { ...apiClient.draft, proto: '' };
      const res = await api.post<{ services: GrpcService[] }>(
        `/workspaces/${wid}/api-client/grpc/reflect`,
        { url: draft.url, headers: draft.headers.filter((h) => h.enabled !== false && h.key.trim() !== '') },
      );
      grpcServices = res.services;
      const first = res.services.find((s) => s.methods.length);
      if (first) selectGrpcMethod(first.methods[0]);
      toasts.success('Reflected schema', `${res.services.length} service(s)`);
    } catch (e) {
      toasts.error('Reflection failed', e instanceof Error ? e.message : String(e));
    } finally {
      grpcReflecting = false;
    }
  }
  function selectGrpcMethod(m: GrpcMethod): void {
    apiClient.draft = {
      ...apiClient.draft,
      grpc_method: m.full,
      body: draft.body?.trim() ? draft.body : m.input_schema,
    };
  }
  function onGrpcMethodChange(full: string): void {
    const m = grpcServices.flatMap((s) => s.methods).find((x) => x.full === full);
    if (m) selectGrpcMethod(m);
  }
  async function invokeGrpc(): Promise<void> {
    const wid = ws.currentId;
    if (!wid) return;
    if (!draft.grpc_method) { toasts.error('No method', 'Pick a gRPC method to call.'); return; }
    grpcInvoking = true;
    try {
      const res = await api.post<ApiResponse>(`/workspaces/${wid}/api-client/grpc/invoke`, {
        url: draft.url,
        proto: draft.proto ?? '',
        method: draft.grpc_method,
        body: draft.body,
        headers: draft.headers.filter((h) => h.enabled !== false && h.key.trim() !== ''),
      });
      apiClient.lastResponse = res;
    } catch (e) {
      toasts.error('gRPC call failed', e instanceof Error ? e.message : String(e));
    } finally {
      grpcInvoking = false;
    }
  }

  // ── actions ────────────────────────────────────────────────────────────────

  function send(): void {
    switch (draft.kind) {
      case 'http': void apiClient.execute(); break;
      case 'grpc': void invokeGrpc(); break;
      case 'sse':
      case 'websocket':
        if (apiStream.active) apiStream.disconnect();
        else apiStream.connect(draft.kind, draft.url, draft.method, draft.headers, draft.body);
        break;
    }
  }
  const sendLabel = $derived(
    draft.kind === 'grpc' ? 'Invoke' : isStreaming ? (apiStream.active ? 'Disconnect' : 'Connect') : 'Send',
  );
  const sendBusy = $derived(apiClient.sending || grpcInvoking || apiStream.status === 'connecting');

  function onUrlKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      send();
    }
  }

  // ── Global keyboard shortcuts ─────────────────────────────────────────────
  // ⌘↵ (or Ctrl+↵ on Windows/Linux) → send/connect
  // ⌘S                              → save
  // ⌘T                              → open new tab
  function onDocKeydown(e: KeyboardEvent): void {
    if (!e.metaKey && !e.ctrlKey) return;
    if (e.key === 'Enter') {
      e.preventDefault();
      send();
    } else if (e.key === 's') {
      e.preventDefault();
      void save();
    } else if (e.key === 't') {
      e.preventDefault();
      apiClient.openTab();
    }
  }

  /** Stop an in-flight HTTP request or streaming connection. */
  function stopRequest(): void {
    if (draft.kind === 'http') {
      apiClient.cancelExecute();
    } else {
      apiStream.disconnect();
    }
  }
  const canStop = $derived(
    (draft.kind === 'http' && apiClient.sending) ||
    ((draft.kind === 'sse' || draft.kind === 'websocket') && apiStream.active),
  );

  /** Paste a curl command straight into the address bar → import it. */
  function onUrlPaste(e: ClipboardEvent): void {
    const text = e.clipboardData?.getData('text') ?? '';
    if (/^\s*curl[\s]/i.test(text)) {
      e.preventDefault();
      void apiClient.importCurl(text);
    }
  }

  // Session/global variables panel.
  let varsOpen = $state(false);
  const varCount = $derived(Object.keys(apiClient.runtimeVars).length);
  const varEntries = $derived(Object.entries(apiClient.runtimeVars));
  let newVarKey = $state('');
  let newVarVal = $state('');
  function addVar(): void {
    if (!newVarKey.trim()) return;
    apiClient.setRuntimeVar(newVarKey.trim(), newVarVal);
    newVarKey = '';
    newVarVal = '';
  }

  // Cookie jar panel.
  let cookiesOpen = $state(false);
  function toggleCookies(): void {
    cookiesOpen = !cookiesOpen;
    if (cookiesOpen) void apiClient.loadCookies();
  }

  // Code snippet generation.
  let codeOpen = $state(false);
  let codeLang = $state<CodeLang>('curl');
  const codeSnippet = $derived(codeOpen ? generateCode(draft, codeLang) : '');
  async function copyCode(): Promise<void> {
    try {
      await navigator.clipboard.writeText(codeSnippet);
      toasts.success('Copied snippet', codeLang);
    } catch {
      toasts.error('Copy failed', 'Clipboard unavailable');
    }
  }

  // Import curl paste box.
  let curlOpen = $state(false);
  let curlText = $state('');
  async function doImportCurl(): Promise<void> {
    const ok = await apiClient.importCurl(curlText);
    if (ok) {
      curlText = '';
      curlOpen = false;
    }
  }

  async function copyAsCurl(): Promise<void> {
    try {
      await navigator.clipboard.writeText(apiClient.toCurl());
      toasts.success('Copied as curl');
    } catch {
      toasts.error('Copy failed', 'Clipboard unavailable');
    }
  }

  // Save into a collection (prompt for name + collection).
  async function save(): Promise<void> {
    if (ws.myRole === 'viewer') {
      toasts.error('Read-only', 'You have viewer access to this workspace');
      return;
    }
    const name = await confirmer.promptText('Request name', {
      title: 'Save request',
      confirmLabel: 'Save',
      initial: draft.name || `${draft.method} ${draft.url}`,
    });
    if (!name) return;
    let collectionId: string | null = draft.requestId
      ? (apiClient.requests.find((r) => r.id === draft.requestId)?.collection_id ?? null)
      : null;
    if (apiClient.collections.length > 0) {
      const list = apiClient.collections.map((c, i) => `${i + 1}. ${c.name}`).join('\n');
      const pick = await confirmer.promptText(`Save into which collection? (number, blank = none)\n${list}`, {
        title: 'Choose collection',
        confirmLabel: 'Save',
      });
      if (pick) {
        const idx = Number(pick) - 1;
        if (idx >= 0 && idx < apiClient.collections.length) {
          collectionId = apiClient.collections[idx].id;
        }
      }
    }
    void apiClient.saveDraft(name, collectionId);
  }
</script>

<svelte:window onkeydown={onDocKeydown} />

<div class="builder" class:compact>
  <!-- request-type selector (HTTP / SSE / WebSocket / gRPC) -->
  <div class="kindbar">
    <select
      class="kind-select"
      value={draft.kind}
      onchange={(e) => setKind((e.currentTarget as HTMLSelectElement).value as ApiRequestKind)}
      aria-label="Request type"
    >
      {#each REQUEST_KINDS as k (k.id)}
        <option value={k.id}>{k.label}</option>
      {/each}
    </select>
    {#if isStreaming}
      <span class="stream-status {apiStream.status}">{apiStream.status}</span>
    {/if}
  </div>

  <!-- URL bar -->
  <div class="urlbar">
    {#if draft.kind === 'http' || draft.kind === 'sse'}
      <select
        class="method"
        value={draft.method}
        onchange={(e) => setField('method', (e.currentTarget as HTMLSelectElement).value)}
        aria-label="HTTP method"
      >
        {#each HTTP_METHODS as m (m)}
          <option value={m}>{m}</option>
        {/each}
      </select>
    {/if}
    <div class="url-wrap">
      <!-- Highlight layer mirrors the input value; `{{var}}` tokens get accent. -->
      <div class="url-highlight" aria-hidden="true">
        {#each urlSegments as seg, i (i)}
          <span class:var={seg.isVar}>{seg.text}</span>
        {/each}
      </div>
      <input
        class="url-input mono"
        placeholder={draft.kind === 'websocket' ? 'wss://echo.example.com/socket' : draft.kind === 'grpc' ? 'https://grpc.example.com:443' : 'https://api.example.com/path  ·  use {{var}}'}
        value={draft.url}
        oninput={(e) => setField('url', (e.currentTarget as HTMLInputElement).value)}
        onkeydown={onUrlKeydown}
        onpaste={onUrlPaste}
        spellcheck="false"
        aria-label="Request URL"
      />
    </div>
    <button class="btn primary send" class:danger={isStreaming && apiStream.active} onclick={send} disabled={sendBusy} title="Send request (⌘↵)">
      {#if sendBusy}
        <Icon name="refresh" size={12} />…
      {:else}
        <Icon name="send" size={12} />{sendLabel}
      {/if}
    </button>
    {#if canStop}
      <button class="btn ghost stop" onclick={stopRequest} title="Cancel in-flight request">
        <Icon name="x" size={12} />Stop
      </button>
    {/if}
  </div>

  <!-- toolbar: curl in/out + save -->
  <div class="toolbar">
    <button class="btn small ghost" onclick={() => (curlOpen = !curlOpen)}>
      <Icon name="external" size={11} />Import curl
    </button>
    <button class="btn small ghost" onclick={copyAsCurl}>
      <Icon name="link" size={11} />Copy as curl
    </button>
    <button class="btn small ghost" onclick={() => (codeOpen = !codeOpen)}>
      <Icon name="external" size={11} />Code
    </button>
    <button class="btn small ghost" onclick={toggleCookies}>
      <Icon name="link" size={11} />Cookies{#if apiClient.cookies.length}<span class="cookie-count">{apiClient.cookies.length}</span>{/if}
    </button>
    <button class="btn small ghost" onclick={() => (varsOpen = !varsOpen)}>
      <Icon name="gear" size={11} />Vars{#if varCount}<span class="cookie-count">{varCount}</span>{/if}
    </button>
    <span class="grow"></span>
    {#if apiClient.activeEnv}
      <span class="chip accent" title="Active environment">{apiClient.activeEnv.name}</span>
    {/if}
    <button class="btn small" onclick={save} title="Save request (⌘S)">
      <Icon name="check" size={11} />Save
    </button>
  </div>

  {#if curlOpen}
    <div class="curl-box">
      <textarea
        class="input curl-area mono"
        bind:value={curlText}
        placeholder="Paste a curl command…"
        spellcheck="false"
        rows={compact ? 2 : 3}
      ></textarea>
      <div class="curl-actions">
        <button class="btn small ghost" onclick={() => (curlOpen = false)}>Cancel</button>
        <button class="btn small primary" onclick={doImportCurl} disabled={!curlText.trim()}>Import</button>
      </div>
    </div>
  {/if}

  {#if varsOpen}
    <div class="code-box">
      <div class="code-head">
        <span class="set-label">Session variables</span>
        <span class="set-unit">override environment · used as {'{{var}}'} and set by scripts</span>
        <span class="grow"></span>
        <button class="link-btn" onclick={() => (varsOpen = false)}>Close</button>
      </div>
      <div class="kv-list">
        {#each varEntries as [k, v] (k)}
          <div class="kv-row">
            <input class="input kv-key mono" value={k} onchange={(e) => apiClient.renameRuntimeVar(k, (e.currentTarget as HTMLInputElement).value)} />
            <input class="input kv-val mono" value={v} oninput={(e) => apiClient.setRuntimeVar(k, (e.currentTarget as HTMLInputElement).value)} />
            <button class="icon-btn" title="Remove" aria-label="Remove variable" onclick={() => apiClient.removeRuntimeVar(k)}><Icon name="x" size={12} /></button>
          </div>
        {/each}
        <div class="kv-row">
          <input class="input kv-key mono" placeholder="new variable" bind:value={newVarKey} onkeydown={(e) => { if (e.key === 'Enter') addVar(); }} />
          <input class="input kv-val mono" placeholder="value" bind:value={newVarVal} onkeydown={(e) => { if (e.key === 'Enter') addVar(); }} />
          <button class="icon-btn" title="Add" aria-label="Add variable" onclick={addVar}><Icon name="plus" size={12} /></button>
        </div>
      </div>
    </div>
  {/if}

  {#if cookiesOpen}
    <div class="code-box">
      <div class="code-head">
        <span class="set-label">Cookie jar ({apiClient.cookies.length})</span>
        <span class="grow"></span>
        <button class="link-btn" onclick={() => apiClient.loadCookies()}>Refresh</button>
        <button class="link-btn" onclick={() => apiClient.clearCookies()}>Clear all</button>
        <button class="link-btn" onclick={() => (cookiesOpen = false)}>Close</button>
      </div>
      {#if apiClient.cookies.length === 0}
        <div class="empty-mini">No cookies yet. They're captured automatically from responses.</div>
      {:else}
        <table class="cookie-table mono">
          <thead><tr><th>Domain</th><th>Name</th><th>Value</th></tr></thead>
          <tbody>
            {#each apiClient.cookies as c (c.domain + c.name)}
              <tr><td>{c.domain}</td><td>{c.name}</td><td class="cookie-val" title={c.value}>{c.value}</td></tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>
  {/if}

  {#if codeOpen}
    <div class="code-box">
      <div class="code-head">
        <select class="rawtype" value={codeLang} onchange={(e) => (codeLang = (e.currentTarget as HTMLSelectElement).value as CodeLang)} aria-label="Snippet language">
          {#each CODE_LANGS as l (l.id)}<option value={l.id}>{l.label}</option>{/each}
        </select>
        <span class="grow"></span>
        <button class="link-btn" onclick={copyCode}>Copy</button>
        <button class="link-btn" onclick={() => (codeOpen = false)}>Close</button>
      </div>
      <pre class="code-snippet mono">{codeSnippet}</pre>
    </div>
  {/if}

  {#if draft.kind === 'grpc'}
    <div class="grpc-panel">
      <div class="grpc-row">
        <label class="file-pick mono grpc-proto">
          <Icon name="external" size={11} />
          <span class="file-name">{draft.proto?.trim() ? 'Replace .proto…' : 'Upload .proto…'}</span>
          <input type="file" accept=".proto" hidden onchange={(e) => onProtoFile(e.currentTarget as HTMLInputElement)} />
        </label>
        <button class="btn small ghost" onclick={reflectGrpc} disabled={grpcReflecting} title="List services via server reflection (no .proto)">
          <Icon name="refresh" size={11} />{grpcReflecting ? 'Reflecting…' : 'Reflect'}
        </button>
        {#if draft.proto?.trim()}
          <button class="btn small ghost" onclick={parseProto} disabled={grpcParsing}>
            <Icon name="refresh" size={11} />{grpcParsing ? 'Parsing…' : 'Re-parse'}
          </button>
        {/if}
        {#if grpcServices.length > 0}
          <select class="grpc-method" value={draft.grpc_method} onchange={(e) => onGrpcMethodChange((e.currentTarget as HTMLSelectElement).value)} aria-label="gRPC method">
            {#each grpcServices as svc (svc.name)}
              <optgroup label={svc.name}>
                {#each svc.methods as m (m.full)}
                  <option value={m.full}>{m.name}{m.client_streaming || m.server_streaming ? ' (streaming)' : ''}</option>
                {/each}
              </optgroup>
            {/each}
          </select>
        {/if}
      </div>
      {#if grpcServices.length === 0}
        <div class="empty-mini">Upload a <code>.proto</code> to list services &amp; methods. Unary calls are invoked through the daemon.</div>
      {/if}
    </div>
  {/if}

  <!-- tab strip -->
  <div class="tabstrip" role="tablist">
    {#each (draft.kind === 'grpc' ? [['body', 'Message', 1], ['headers', 'Metadata', draft.headers.length]] : draft.kind === 'websocket' ? [['headers', 'Headers', draft.headers.length], ['auth', 'Authorization', draft.auth.type !== 'none' ? 1 : 0], ['settings', 'Settings', settingsBadge]] : [['params', 'Params', draft.query.length], ['auth', 'Authorization', draft.auth.type !== 'none' ? 1 : 0], ['headers', 'Headers', draft.headers.length], ['body', 'Body', draft.body_mode !== 'none' ? 1 : 0], ['scripts', 'Scripts', (draft.pre_request_script?.trim() || draft.post_response_script?.trim()) ? 1 : 0], ['docs', 'Docs', draft.docs?.trim() ? 1 : 0], ['settings', 'Settings', settingsBadge]]) as [id, label, count] (id)}
      <button
        class="tab"
        class:active={tab === id}
        role="tab"
        aria-selected={tab === id}
        onclick={() => (tab = id as Tab)}
      >
        {label}{#if Number(count) > 0}<span class="dot-badge"></span>{/if}
      </button>
    {/each}
  </div>

  <!-- tab body -->
  <div class="tabbody">
    {#if tab === 'params'}
      {@render kvEditor('query', draft.query)}
    {:else if tab === 'headers'}
      {@render kvEditor('headers', draft.headers)}
    {:else if tab === 'body' && draft.kind === 'grpc'}
      <div class="bodybar">
        <span class="grpc-msg-label">Request message (JSON)</span>
        <span class="grow"></span>
        <button class="link-btn" onclick={beautify} title="Pretty-print JSON">Beautify</button>
      </div>
      {#if compact}
        <textarea class="input body-area mono" value={draft.body} oninput={(e) => setField('body', (e.currentTarget as HTMLTextAreaElement).value)} placeholder="{'{ }'}" spellcheck="false"></textarea>
      {:else}
        <div class="body-editor">
          <CodeEditor path="message.json" content={draft.body} root={ws.current?.root_path ?? ''} language="json" readOnly={false} onchange={(v) => setField('body', v)} />
        </div>
      {/if}
    {:else if tab === 'body'}
      <div class="bodybar">
        <div class="body-radios">
          {#each BODY_RADIOS as r (r.id)}
            <label class="radio" class:disabled={r.disabled} title={r.disabled ? 'Binary body upload is not supported yet' : undefined}>
              <input
                type="radio"
                name="bodymode{compact ? '-c' : ''}"
                checked={isModeActive(r.id)}
                disabled={r.disabled}
                onchange={() => onBodyRadio(r.id)}
              />
              <span>{r.label}</span>
            </label>
          {/each}
          {#if rawActive}
            <select
              class="rawtype"
              value={rawType}
              onchange={(e) => onRawType((e.currentTarget as HTMLSelectElement).value as RawType)}
              aria-label="Raw content type"
            >
              {#each RAW_TYPES as t (t)}<option value={t}>{t}</option>{/each}
            </select>
          {/if}
        </div>
        <span class="grow"></span>
        {#if showBeautify}
          <button class="link-btn" onclick={beautify} title="Pretty-print the body">Beautify</button>
        {/if}
      </div>
      {#if draft.body_mode === 'none'}
        <div class="empty-mini">This request does not have a body.</div>
      {:else if formActive}
        <div class="kv-list">
          {#each formRows as row, i (i)}
            <div class="kv-row">
              <input class="input kv-key mono" placeholder="key" value={row.key} oninput={(e) => updateFormRow(i, { key: (e.currentTarget as HTMLInputElement).value })} />
              {#if multipartActive}
                <select class="row-type" value={row.type} onchange={(e) => setRowType(i, (e.currentTarget as HTMLSelectElement).value as FieldType)} aria-label="Field type">
                  <option value="text">Text</option>
                  <option value="file">File</option>
                </select>
              {/if}
              {#if multipartActive && row.type === 'file'}
                <label class="file-pick mono" title={row.filename ?? 'Choose a file'}>
                  <Icon name="external" size={11} />
                  <span class="file-name">{row.filename || 'Choose file…'}</span>
                  <input type="file" hidden onchange={(e) => pickFile(i, e.currentTarget as HTMLInputElement)} />
                </label>
              {:else}
                <input class="input kv-val mono" placeholder="value" value={row.value} oninput={(e) => updateFormRow(i, { value: (e.currentTarget as HTMLInputElement).value })} />
              {/if}
              <button class="icon-btn" title="Remove" aria-label="Remove" onclick={() => removeFormRow(i)}><Icon name="x" size={12} /></button>
            </div>
          {/each}
          <button class="btn small ghost add-row" onclick={addFormRow}><Icon name="plus" size={11} />Add field</button>
        </div>
      {:else if compact}
        <textarea
          class="input body-area mono"
          value={draft.body}
          oninput={(e) => setField('body', (e.currentTarget as HTMLTextAreaElement).value)}
          placeholder={draft.body_mode === 'json' ? '{ }' : draft.body_mode === 'graphql' ? 'query { }' : 'raw body'}
          spellcheck="false"
        ></textarea>
      {:else if draft.body_mode === 'graphql'}
        <div class="gql-bar">
          <span class="set-label">Query</span>
          <span class="grow"></span>
          <button class="link-btn" onclick={() => apiClient.graphqlIntrospect()} disabled={apiClient.graphqlIntrospecting}>
            {apiClient.graphqlIntrospecting ? 'Introspecting…' : 'Introspect schema'}
          </button>
        </div>
        <div class="body-editor gql-query">
          <CodeEditor path="query.graphql" content={draft.body} root={ws.current?.root_path ?? ''} language="" readOnly={false} onchange={(v) => setField('body', v)} />
        </div>
        <div class="gql-bar"><span class="set-label">Variables (JSON)</span></div>
        <div class="body-editor gql-vars">
          <CodeEditor path="variables.json" content={draft.graphql_variables ?? ''} root={ws.current?.root_path ?? ''} language="json" readOnly={false} onchange={(v) => setField('graphql_variables', v)} />
        </div>
        {#if apiClient.graphqlSchema}
          <div class="gql-schema">
            <div class="set-label">Schema ({apiClient.graphqlSchema.length} types)</div>
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
          <CodeEditor
            path={editorPath}
            content={draft.body}
            root={ws.current?.root_path ?? ''}
            language={editorLang}
            readOnly={false}
            onchange={(v) => setField('body', v)}
          />
        </div>
      {/if}
    {:else if tab === 'auth'}
      <div class="auth">
        <div class="auth-type">
          {#each authTypes as t (t)}
            <button class="seg" class:active={draft.auth.type === t} onclick={() => setAuthType(t)}>{t === 'api_key' ? 'api key' : t}</button>
          {/each}
        </div>
        {#if draft.auth.type === 'bearer'}
          <div class="field-row">
            <label for="auth-token">Token</label>
            <input id="auth-token" class="input mono grow" value={draft.auth.token} oninput={(e) => setAuth({ token: (e.currentTarget as HTMLInputElement).value })} placeholder="token or {'{{var}}'}" />
          </div>
        {:else if draft.auth.type === 'basic'}
          <div class="field-row">
            <label for="auth-user">Username</label>
            <input id="auth-user" class="input grow" value={draft.auth.username} oninput={(e) => setAuth({ username: (e.currentTarget as HTMLInputElement).value })} />
          </div>
          <div class="field-row">
            <label for="auth-pass">Password</label>
            <input id="auth-pass" class="input grow" type="password" value={draft.auth.password} oninput={(e) => setAuth({ password: (e.currentTarget as HTMLInputElement).value })} />
          </div>
        {:else if draft.auth.type === 'api_key'}
          <div class="field-row">
            <label for="auth-key">Key</label>
            <input id="auth-key" class="input mono grow" value={draft.auth.key} oninput={(e) => setAuth({ key: (e.currentTarget as HTMLInputElement).value })} placeholder="X-Api-Key" />
          </div>
          <div class="field-row">
            <label for="auth-value">Value</label>
            <input id="auth-value" class="input mono grow" value={draft.auth.value} oninput={(e) => setAuth({ value: (e.currentTarget as HTMLInputElement).value })} />
          </div>
          <div class="field-row">
            <label for="auth-in">Add to</label>
            <select id="auth-in" class="input" value={draft.auth.in} onchange={(e) => setAuth({ in: (e.currentTarget as HTMLSelectElement).value as 'header' | 'query' })}>
              <option value="header">Header</option>
              <option value="query">Query param</option>
            </select>
          </div>
        {:else if draft.auth.type === 'oauth2'}
          <div class="field-row">
            <label for="auth-grant">Grant type</label>
            <select id="auth-grant" class="input" value={draft.auth.grant} onchange={(e) => setAuth({ grant: (e.currentTarget as HTMLSelectElement).value as 'client_credentials' | 'password' | 'refresh_token' })}>
              <option value="client_credentials">Client Credentials</option>
              <option value="password">Password</option>
              <option value="refresh_token">Refresh Token</option>
            </select>
          </div>
          <div class="field-row">
            <label for="auth-turl">Token URL</label>
            <input id="auth-turl" class="input mono grow" value={draft.auth.token_url} oninput={(e) => setAuth({ token_url: (e.currentTarget as HTMLInputElement).value })} placeholder="https://auth.example.com/oauth/token" />
          </div>
          <div class="field-row">
            <label for="auth-cid">Client ID</label>
            <input id="auth-cid" class="input mono grow" value={draft.auth.client_id} oninput={(e) => setAuth({ client_id: (e.currentTarget as HTMLInputElement).value })} />
          </div>
          <div class="field-row">
            <label for="auth-csec">Client Secret</label>
            <input id="auth-csec" class="input mono grow" type="password" value={draft.auth.client_secret} oninput={(e) => setAuth({ client_secret: (e.currentTarget as HTMLInputElement).value })} />
          </div>
          {#if draft.auth.grant === 'password'}
            <div class="field-row">
              <label for="auth-ouser">Username</label>
              <input id="auth-ouser" class="input grow" value={draft.auth.username} oninput={(e) => setAuth({ username: (e.currentTarget as HTMLInputElement).value })} />
            </div>
            <div class="field-row">
              <label for="auth-opass">Password</label>
              <input id="auth-opass" class="input grow" type="password" value={draft.auth.password} oninput={(e) => setAuth({ password: (e.currentTarget as HTMLInputElement).value })} />
            </div>
          {/if}
          {#if draft.auth.grant === 'refresh_token'}
            <div class="field-row">
              <label for="auth-rt">Refresh Token</label>
              <input id="auth-rt" class="input mono grow" value={draft.auth.refresh_token} oninput={(e) => setAuth({ refresh_token: (e.currentTarget as HTMLInputElement).value })} />
            </div>
          {/if}
          <div class="field-row">
            <label for="auth-scope">Scope</label>
            <input id="auth-scope" class="input mono grow" value={draft.auth.scope} oninput={(e) => setAuth({ scope: (e.currentTarget as HTMLInputElement).value })} placeholder="read write" />
          </div>
          <div class="field-row">
            <span class="field-spacer"></span>
            <button class="btn small primary" onclick={fetchOAuthToken} disabled={fetchingToken}>
              <Icon name="refresh" size={11} />{fetchingToken ? 'Requesting…' : 'Get New Token'}
            </button>
          </div>
          {#if draft.auth.access_token}
            <div class="field-row">
              <label for="auth-at">Access Token</label>
              <input id="auth-at" class="input mono grow oauth-token" value={draft.auth.access_token} readonly title={draft.auth.access_token} />
            </div>
          {/if}
        {:else}
          <div class="empty-mini">No authentication.</div>
        {/if}
      </div>
    {:else if tab === 'scripts'}
      <div class="scripts-pane">
        <div class="script-block">
          <div class="script-head">
            <span class="script-title">Pre-request Script</span>
            <span class="script-hint mono">pm.environment.set('k', v) · pm.request.headers.upsert(…)</span>
          </div>
          <div class="script-editor">
            <CodeEditor path="pre.js" content={draft.pre_request_script ?? ''} root={ws.current?.root_path ?? ''} language="js" readOnly={false} onchange={(v) => setField('pre_request_script', v)} />
          </div>
        </div>
        <div class="script-block">
          <div class="script-head">
            <span class="script-title">Post-response Script (Tests)</span>
            <span class="script-hint mono">pm.test('ok', () =&gt; pm.expect(pm.response.code).toBe(200))</span>
          </div>
          <div class="script-editor">
            <CodeEditor path="post.js" content={draft.post_response_script ?? ''} root={ws.current?.root_path ?? ''} language="js" readOnly={false} onchange={(v) => setField('post_response_script', v)} />
          </div>
        </div>
      </div>
    {:else if tab === 'docs'}
      <div class="docs-pane">
        <div class="docs-edit">
          <CodeEditor path="docs.md" content={draft.docs ?? ''} root={ws.current?.root_path ?? ''} language="md" readOnly={false} onchange={(v) => setField('docs', v)} />
        </div>
        {#if draft.docs?.trim() && !compact}
          <div class="docs-preview">{@html docsHtml}</div>
        {/if}
      </div>
    {:else if tab === 'settings'}
      <div class="settings-pane">
        <label class="set-row">
          <span class="set-label">Request timeout</span>
          <span class="set-control">
            <input class="input set-num mono" type="number" min="0" step="100" placeholder="60000"
              value={settings.timeout_ms ?? ''}
              oninput={(e) => { const v = (e.currentTarget as HTMLInputElement).value; setSetting('timeout_ms', v === '' ? null : Number(v)); }} />
            <span class="set-unit">ms · blank = default (60s)</span>
          </span>
        </label>
        <label class="set-row toggle">
          <input type="checkbox" checked={settings.follow_redirects} onchange={(e) => setSetting('follow_redirects', (e.currentTarget as HTMLInputElement).checked)} />
          <span class="set-label">Automatically follow redirects</span>
        </label>
        <label class="set-row toggle">
          <input type="checkbox" checked={settings.verify_ssl} onchange={(e) => setSetting('verify_ssl', (e.currentTarget as HTMLInputElement).checked)} />
          <span class="set-label">Verify TLS certificate <span class="set-unit">(off = accept self-signed / invalid certs)</span></span>
        </label>
      </div>
    {/if}
  </div>
</div>

{#snippet kvEditor(which: 'headers' | 'query', rows: ApiKeyVal[])}
  <div class="kv-list">
    {#if which === 'headers'}
      <datalist id="hdr-keys">
        {#each COMMON_HEADERS as h}<option value={h}></option>{/each}
      </datalist>
    {/if}
    {#each rows as row, i (i)}
      <div class="kv-row">
        <input
          class="kv-check"
          type="checkbox"
          checked={row.enabled !== false}
          onchange={(e) => updateRow(which, i, { enabled: (e.currentTarget as HTMLInputElement).checked })}
          title="Enabled"
        />
        <input
          class="input kv-key mono"
          placeholder="key"
          value={row.key}
          list={which === 'headers' ? 'hdr-keys' : undefined}
          autocomplete="off"
          oninput={(e) => updateRow(which, i, { key: (e.currentTarget as HTMLInputElement).value })}
        />
        <input
          class="input kv-val mono"
          placeholder="value"
          value={row.value}
          list={which === 'headers' && headerValues(row.key).length > 0 ? `hdr-vals-${i}` : undefined}
          autocomplete="off"
          oninput={(e) => updateRow(which, i, { value: (e.currentTarget as HTMLInputElement).value })}
        />
        {#if which === 'headers' && headerValues(row.key).length > 0}
          <datalist id={`hdr-vals-${i}`}>
            {#each headerValues(row.key) as v}<option value={v}></option>{/each}
          </datalist>
        {/if}
        <button class="icon-btn" title="Remove" aria-label="Remove row" onclick={() => removeRow(which, i)}><Icon name="x" size={12} /></button>
      </div>
    {/each}
    <button class="btn small ghost add-row" onclick={() => addRow(which)}>
      <Icon name="plus" size={11} />Add {which === 'query' ? 'param' : 'header'}
    </button>
  </div>
{/snippet}

<style>
  .builder {
    display: flex;
    flex-direction: column;
    min-height: 0;
    gap: 8px;
  }
  .kindbar {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .kind-select {
    height: 26px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--accent);
    font-weight: 700;
    font-size: 12px;
    padding: 0 6px;
    cursor: pointer;
  }
  .stream-status {
    font-size: 11px;
    font-weight: 600;
    text-transform: capitalize;
    padding: 1px 8px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .stream-status.open { background: color-mix(in srgb, var(--status-working) 18%, transparent); color: var(--status-working); }
  .stream-status.connecting { background: color-mix(in srgb, var(--accent) 18%, transparent); color: var(--accent); }
  .stream-status.error { background: color-mix(in srgb, var(--status-exited) 18%, transparent); color: var(--status-exited); }
  .send.danger {
    background: var(--status-exited);
    border-color: var(--status-exited);
  }
  .grpc-panel {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .grpc-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .grpc-proto {
    flex: 0 0 auto;
  }
  .grpc-method {
    flex: 1;
    min-width: 160px;
    height: 26px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-1, var(--surface-2));
    color: var(--text);
    font-size: 12px;
    padding: 0 6px;
    cursor: pointer;
  }
  .grpc-msg-label {
    font-size: 12px;
    font-weight: 600;
    color: var(--text-dim);
  }
  .urlbar {
    display: flex;
    gap: 6px;
    align-items: stretch;
  }
  .method {
    height: 30px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--accent);
    font-weight: 700;
    font-size: 12px;
    padding: 0 6px;
    cursor: pointer;
  }
  .url-wrap {
    position: relative;
    flex: 1;
    min-width: 0;
  }
  .url-input,
  .url-highlight {
    height: 30px;
    line-height: 30px;
    padding: 0 9px;
    font-size: 12.5px;
    white-space: pre;
    overflow: hidden;
  }
  .url-input {
    width: 100%;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
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
  .url-highlight {
    position: absolute;
    inset: 0;
    z-index: 0;
    color: transparent;
    border: 1px solid transparent;
    pointer-events: none;
  }
  .url-highlight .var {
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    border-radius: 3px;
  }
  .send {
    height: 30px;
    flex-shrink: 0;
  }
  .stop {
    height: 30px;
    flex-shrink: 0;
    color: var(--status-exited);
    border-color: color-mix(in srgb, var(--status-exited) 40%, transparent);
  }
  .stop:hover {
    background: color-mix(in srgb, var(--status-exited) 14%, transparent);
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .curl-box {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .curl-area {
    width: 100%;
    resize: vertical;
  }
  .curl-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .tabstrip {
    display: flex;
    flex-wrap: wrap;
    gap: 2px;
    border-bottom: 1px solid var(--border);
  }
  .tab {
    position: relative;
    height: 28px;
    padding: 0 12px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
  }
  .tab:hover {
    color: var(--text);
  }
  .tab.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  .dot-badge {
    display: inline-block;
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--accent);
    margin-left: 5px;
    vertical-align: middle;
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
    gap: 5px;
  }
  .kv-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .kv-check {
    flex-shrink: 0;
    accent-color: var(--accent);
  }
  .kv-key {
    flex: 0 1 38%;
    min-width: 0;
  }
  .kv-val {
    flex: 1;
    min-width: 0;
  }
  .row-type {
    flex: 0 0 auto;
    height: 26px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: 11.5px;
    padding: 0 4px;
    cursor: pointer;
  }
  .file-pick {
    flex: 1;
    min-width: 0;
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    padding: 0 8px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--accent);
    font-size: 11.5px;
    cursor: pointer;
    overflow: hidden;
  }
  .file-pick:hover {
    border-color: color-mix(in srgb, var(--accent) 50%, transparent);
  }
  .file-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .add-row {
    align-self: flex-start;
  }
  .auth-type {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  /* Postman-style body bar: radio row + raw-type dropdown + Beautify */
  .bodybar {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }
  .body-radios {
    display: flex;
    align-items: center;
    gap: 14px;
    flex-wrap: wrap;
  }
  .radio {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    color: var(--text-dim);
    cursor: pointer;
    user-select: none;
  }
  .radio:hover:not(.disabled) {
    color: var(--text);
  }
  .radio input {
    accent-color: var(--accent);
    cursor: pointer;
    margin: 0;
  }
  .radio:has(input:checked) {
    color: var(--accent);
    font-weight: 600;
  }
  .radio.disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .radio.disabled input {
    cursor: not-allowed;
  }
  .rawtype {
    height: 22px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--accent);
    font-size: 11.5px;
    font-weight: 600;
    padding: 0 4px;
    cursor: pointer;
  }
  .link-btn {
    border: none;
    background: transparent;
    color: var(--accent);
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    padding: 2px 4px;
  }
  .link-btn:hover {
    text-decoration: underline;
  }
  .seg {
    height: 24px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: 11.5px;
    cursor: pointer;
    text-transform: capitalize;
  }
  .seg.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
  }
  .body-area {
    width: 100%;
    min-height: 120px;
    resize: vertical;
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
    width: 90px;
    flex-shrink: 0;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .oauth-token {
    color: var(--status-working);
  }
  .cookie-count {
    margin-left: 4px;
    font-size: 10px;
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
    border-radius: 999px;
    padding: 0 5px;
  }
  .cookie-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 11px;
  }
  .cookie-table th {
    text-align: left;
    color: var(--text-dim);
    font-weight: 600;
    padding: 2px 6px;
  }
  .cookie-table td {
    padding: 2px 6px;
    border-top: 1px solid var(--border);
    max-width: 160px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .code-box {
    display: flex;
    flex-direction: column;
    gap: 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 8px;
    background: var(--surface-2);
  }
  .code-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .code-snippet {
    margin: 0;
    max-height: 240px;
    overflow: auto;
    white-space: pre;
    font-size: 11.5px;
    line-height: 1.5;
    color: var(--text);
    user-select: text;
  }
  .gql-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 4px 0;
  }
  .gql-query {
    height: 150px;
  }
  .gql-vars {
    height: 90px;
  }
  .gql-schema {
    margin-top: 8px;
    max-height: 200px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .gql-type > summary {
    cursor: pointer;
    font-size: 12px;
    padding: 3px 4px;
  }
  .gql-kind {
    font-size: 10px;
    color: var(--text-dim);
    text-transform: uppercase;
  }
  .gql-fields {
    font-size: 11px;
    color: var(--text-dim);
    padding: 2px 14px 6px;
    word-break: break-word;
  }
  .docs-pane {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .docs-edit {
    height: 160px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .docs-preview {
    font-size: 13px;
    line-height: 1.6;
    color: var(--text);
    border-top: 1px solid var(--border);
    padding-top: 8px;
  }
  .docs-preview :global(h1),
  .docs-preview :global(h2) { font-size: 15px; margin: 8px 0 4px; }
  .docs-preview :global(code) { background: var(--surface-2); padding: 1px 4px; border-radius: 4px; }
  .docs-preview :global(pre) { background: var(--surface-2); padding: 8px; border-radius: 6px; overflow: auto; }
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
  }
  .script-title {
    font-size: 12px;
    font-weight: 600;
    color: var(--text);
  }
  .script-hint {
    font-size: 10.5px;
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
    padding: 4px 2px;
  }
  .set-row {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12.5px;
    color: var(--text);
  }
  .set-row.toggle {
    cursor: pointer;
  }
  .set-row.toggle input {
    accent-color: var(--accent);
    flex-shrink: 0;
  }
  .set-label {
    min-width: 130px;
  }
  .set-control {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .set-num {
    width: 110px;
  }
  .set-unit {
    font-size: 11px;
    color: var(--text-dim);
  }
  .empty-mini {
    font-size: 12px;
    color: var(--text-dim);
    padding: 8px 2px;
  }

  @media (max-width: 640px) {
    .builder {
      min-width: 0;
      max-width: 100%;
    }
    .urlbar {
      flex-wrap: wrap;
      gap: 6px;
    }
    .url-wrap {
      flex: 1 1 100%;
      order: 2;
      min-width: 0;
    }
    .method {
      order: 1;
    }
    .send {
      order: 1;
      flex-shrink: 0;
    }
    .kv-row {
      flex-wrap: wrap;
    }
    .kv-key,
    .kv-val {
      flex: 1 1 calc(50% - 20px);
      min-width: 0;
    }
    .auth-type {
      gap: 4px;
    }
    .field-row {
      flex-wrap: wrap;
    }
    .field-row > label,
    .field-spacer {
      width: auto;
      min-width: 70px;
    }
    .field-row .grow {
      min-width: 0;
    }
    .body-radios {
      gap: 8px;
    }
  }
</style>
