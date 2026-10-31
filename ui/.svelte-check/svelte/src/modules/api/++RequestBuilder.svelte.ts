///<reference types="svelte" />
;// The request editor, shared by the full API page and the compact right
// panel. Top to bottom: name + where it's saved (Save ⌘S, ⋯ for everything
// else) · type + method + URL + Send · the {{variables}} this request uses
// and what they resolve to · Params / Headers / Body / Auth / Scripts / Docs
// / Settings, each with a one-line explanation.

import { announceShown } from '../../lib/announce.svelte';
import { openExternal } from '../../lib/external';
import { toastError } from '../../lib/toastError';
import { onTabKey } from '../../lib/tabKeys';
import Icon from '../../lib/components/Icon.svelte';
import { focusAfterRowRemoval, rowListOf } from '../../lib/rowFocus';
import LoadState from '../../lib/components/LoadState.svelte';
import Modal from '../../lib/components/Modal.svelte';
import CodeEditor from '../../lib/components/CodeEditor.svelte';
import MethodTag from './MethodTag.svelte';
import SaveRequestDialog from './SaveRequestDialog.svelte';
import ImportDialog from './ImportDialog.svelte';
import { apiClient, HTTP_METHODS, defaultSettings, confirmNewHost, draftSecrets, type ApiDraft, type ApiRequestKind, type ApiSettings } from '../../lib/stores/apiClient.svelte';
import { apiStream } from '../../lib/stores/apiStream.svelte';
import { api, baseUrl, newHostConfirmHost } from '../../lib/api/client';
import { generateCode, CODE_LANGS, type CodeLang } from '../../lib/api/codegen';
import { collectionPaths, fullRequestUrl, requestVarNames, resolveVar, splitVars, varNames } from '../../lib/api/apiVars';
import { renderMarkdownGfm } from '../../lib/md';
import type { ApiAuth, ApiBodyMode, ApiKeyVal, ApiSecretable, GrpcReflectReq } from '../../lib/api/types';
import { isSecretRef } from '../../lib/api/types';
import { ws } from '../../lib/stores/workspace.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
;

  type Tab = 'params' | 'headers' | 'body' | 'auth' | 'settings' | 'scripts' | 'docs';;;

  interface Props {
    /** Compact mode (right panel): tighter, textareas instead of code editors. */
    compact?: boolean;
    /** The open editor tab (bindable, so the response pane can jump to Settings). */
    tab?: Tab;
  };;

  // ── Body types ─────────────────────────────────────────────────────────────
  // One plain-language picker over the backend modes: "Text" covers 'raw' (with
  // a sub-type for the Content-Type), "JSON" is 'json'.
  type RawType = 'Text' | 'JavaScript' | 'JSON' | 'HTML' | 'XML';;;
  type BodyChoice = 'none' | 'json' | 'text' | 'form' | 'multipart' | 'graphql';;;

  // ── form-body rows ─────────────────────────────────────────────────────────
  // Kept in LOCAL state (not derived from the encoded body) so that *empty* rows
  // persist in the editor — the encoders drop empty-key pairs when sending, so a
  // derived approach would make "Add field" appear to do nothing.
  //   • x-www-form-urlencoded ('form')  → `k=v&…`
  //   • multipart/form-data ('multipart') → JSON `[{key,type,value,filename}]`,
  //     so a row can be a File (value = base64 of the file's bytes).
  type FieldType = 'text' | 'file';;;
  interface FormRow { key: string; value: string; type: FieldType; filename?: string; missing_content?: boolean; size?: number; };;

  // ── gRPC state ──────────────────────────────────────────────────────────────
  interface GrpcMethod { name: string; full: string; input_type: string; output_type: string; input_schema: string; client_streaming: boolean; server_streaming: boolean; };;
  interface GrpcService { name: string; methods: GrpcMethod[]; };function $$render() {
 const kvEditor/*Ωignore_positionΩ*/ = (which: 'headers' | 'query', rows: ApiKeyVal[])/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", { "class":`kv-list`,});
    if(which === 'headers'){
       { svelteHTML.createElement("datalist", { "id":`hdr-keys`,});  for(let h of __sveltets_2_ensureArray(COMMON_HEADERS)){ { svelteHTML.createElement("option", { "value":h,}); }} }
    }
    if(rows.length > 0){
       { svelteHTML.createElement("div", {   "class":`kv-head`,"aria-hidden":`true`,}); { svelteHTML.createElement("span", { "class":`kv-check-h`,}); } { svelteHTML.createElement("span", {});which === 'query' ? 'Parameter' : 'Header'; } { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("span", { "class":`kv-x-h`,}); } }
    }
        for(let row of __sveltets_2_ensureArray(rows)){let i = 1;i;
       { svelteHTML.createElement("div", {  "class":`kv-row`,});row.enabled === false;
         { svelteHTML.createElement("input", {            "class":`kv-check`,"type":`checkbox`,"checked":row.enabled !== false,"aria-label":`Send ${row.key || 'this row'}`,"title":row.enabled === false ? 'Skipped: tick to send' : 'Sent: untick to skip',"onchange":(e) => updateRow(which, i, { enabled: (e.currentTarget as HTMLInputElement).checked }),});}
         { svelteHTML.createElement("input", {                "dir":`ltr`,"class":`input kv-key mono`,"placeholder":which === 'query' ? 'page' : 'Accept',"aria-label":`${which === 'query' ? 'Parameter' : 'Header'} name`,"value":row.key,"list":which === 'headers' ? 'hdr-keys' : undefined,"autocomplete":`off`,"oninput":(e) => updateRow(which, i, { key: (e.currentTarget as HTMLInputElement).value }),});}
         { svelteHTML.createElement("input", {                "dir":`ltr`,"class":`input kv-val mono`,"placeholder":which === 'query' ? '2' : 'application/json',"aria-label":`Value`,"value":row.value,"list":which === 'headers' && headerValues(row.key).length > 0 ? `hdr-vals-${i}` : undefined,"autocomplete":`off`,"oninput":(e) => updateRow(which, i, { value: (e.currentTarget as HTMLInputElement).value }),});}
        if(which === 'headers' && headerValues(row.key).length > 0){
           { svelteHTML.createElement("datalist", { "id":`hdr-vals-${i}`,});  for(let v of __sveltets_2_ensureArray(headerValues(row.key))){ { svelteHTML.createElement("option", { "value":v,}); }} }
        }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,...__sveltets_2_empty({"data-remove-row":true}),"title":`Remove ${which === 'query' ? 'parameter' : 'header'} ${row.key || i + 1}`,"aria-label":`Remove ${which === 'query' ? 'parameter' : 'header'} ${row.key || i + 1}`,"onclick":(e) => removeRow(which, i, e),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
       }
    }
     { svelteHTML.createElement("button", {   "class":`btn small ghost add-row`,"onclick":() => addRow(which),}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} which === 'query' ? 'param' : 'header'; }
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  


  let { compact = false, tab = $bindable('params') }: Props = $props()/*Ωignore_startΩ*/;tab;/*Ωignore_endΩ*/;

  // The draft lives in the store so the page + panel share one editing target.
  const draft = $derived(apiClient.draft);
  const canEdit = $derived(ws.myRole !== 'viewer');

  /** Body editors stop colouring while a line is longer than this: a pasted
   *  minified JSON is one wrapped line of tens of thousands of highlight spans
   *  that every typed character re-laid out (see CodeEditor's prop). */
  const LONG_LINE_PLAIN = 20_000;

  const TEXT_TYPES: RawType[] = ['Text', 'JavaScript', 'HTML', 'XML'];

  const BODY_CHOICES: { id: BodyChoice; label: string; help: string }[] = [
    { id: 'none', label: 'No body', help: 'Most GET and DELETE requests don’t send a body.' },
    { id: 'json', label: 'JSON', help: 'Sent as application/json. Use {{variables}} anywhere in it.' },
    { id: 'text', label: 'Text / XML / HTML', help: 'Sent as-is with the Content-Type you pick.' },
    { id: 'form', label: 'Form (URL-encoded)', help: 'key=value pairs, like an HTML form (application/x-www-form-urlencoded).' },
    { id: 'multipart', label: 'Multipart form (files)', help: 'Fields and file uploads (multipart/form-data).' },
    { id: 'graphql', label: 'GraphQL', help: 'A query plus JSON variables, sent as {"query","variables"}.' },
  ];
  const bodyChoice = $derived<BodyChoice>(draft.body_mode === 'raw' ? 'text' : (draft.body_mode as BodyChoice));
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
  // The body's `{{vars}}` are scanned per keystroke only while it is small;
  // above 64 KB the scan (a regex over the whole body) runs 300 ms after the
  // last edit. URL / query / headers / auth stay live.
  const BODY_VAR_SCAN_SYNC_MAX = 64 * 1024;
  let deferredBodyVars = $state.raw<string[]>([]);
  $effect(() => {
    const body = draft.body;
    if (body.length <= BODY_VAR_SCAN_SYNC_MAX) return;
    const t = setTimeout(() => { deferredBodyVars = varNames(body); }, 300);
    return () => clearTimeout(t);
  });
  const bodyVars = $derived(draft.body.length <= BODY_VAR_SCAN_SYNC_MAX ? varNames(draft.body) : deferredBodyVars);
  const usedVars = $derived(
    requestVarNames(draft, bodyVars).map((n) => resolveVar(n, apiClient.runtimeVars, envForVars)),
  );
  const missingVars = $derived(usedVars.filter((v) => v.kind === 'missing').length);
  // Params tab: the URL as sent — enabled rows appended and encoded, known
  // variables filled in (secrets / built-ins / missing ones stay {{name}}).
  const sentUrl = $derived(fullRequestUrl(draft.url, draft.query, apiClient.runtimeVars, envForVars, draft.auth));
  const sentUrlSegments = $derived(splitVars(sentUrl));

  // ── key/value rows ────────────────────────────────────────────────────────
  function addRow(which: 'headers' | 'query', key = '', value = ''): void {
    apiClient.draft = { ...draft, [which]: [...draft[which], { key, value, enabled: true }] };
  }
  function updateRow(which: 'headers' | 'query', i: number, patch: Partial<ApiKeyVal>): void {
    const rows = draft[which].map((r, idx) => (idx === i ? { ...r, ...patch } : r));
    apiClient.draft = { ...draft, [which]: rows };
  }
  function removeRow(which: 'headers' | 'query', i: number, e?: Event): void {
    const list = e ? rowListOf(e, '.kv-list') : null;
    apiClient.draft = { ...draft, [which]: draft[which].filter((_, idx) => idx !== i) };
    void focusAfterRowRemoval(list, i, '.add-row');
  }


  function parseForm(s: string, mode: ApiBodyMode): FormRow[] {
    if (mode === 'multipart' && s.trim().startsWith('[')) {
      try {
        const arr = JSON.parse(s) as Partial<FormRow>[];
        return arr.map((r) => ({ key: r.key ?? '', value: r.value ?? '', type: r.type === 'file' ? 'file' : 'text', filename: r.filename, missing_content: r.missing_content || (r.size !== undefined && !r.value) }));
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
        ? { key: r.key, type: 'file', value: r.value, filename: r.filename ?? '', ...(r.missing_content ? {missing_content:true} : {}) }
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
    const owner = apiClient.draft;
    const wid = ws.currentId;
    const row = formRows[i];
    const current = () => ws.currentId === wid && apiClient.draft === owner && formRows[i] === row;
    try {
      const value = await fileToBase64(file);
      if (current()) updateFormRow(i, { value, filename: file.name, missing_content: false, size: undefined });
    } catch {
      if (current()) toasts.error('Couldn’t read the file', file.name);
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
  // upserts — untrusted markdown. lib/md's GFM renderer runs the allowlist
  // sanitizer before the `{@html}` sink (and never throws).
  const docsHtml = $derived(renderMarkdownGfm(draft.docs ?? '', { breaks: true }));
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
  /** Inline field error under Token URL (never a toast for a field error). */
  let tokenUrlError = $state('');
  async function fetchOAuthToken(): Promise<void> {
    if (draft.auth.type !== 'oauth2') return;
    const wid = ws.currentId;
    if (!wid) return;
    const a = draft.auth;
    const tabId = draft.tabId;
    if (!a.token_url.trim()) { tokenUrlError = 'Enter the OAuth token endpoint first.'; return; }
    tokenUrlError = '';
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
            const request = await api
              .get<import('../../lib/api/types').ApiRequest>(`/workspaces/${wid}/api-client/requests/${encodeURIComponent(saved.id)}`)
              .catch(() => null);
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
      toastError('Couldn’t get a token', e);
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


  let grpcServices = $state<GrpcService[]>([]);
  let grpcParsing = $state(false);

  async function onProtoFile(input: HTMLInputElement): Promise<void> {
    const f = input.files?.[0];
    if (!f) return;
    const owner = apiClient.draft;
    const wid = ws.currentId;
    const current = () => ws.currentId === wid && apiClient.draft === owner;
    try {
      const text = await f.text();
      if (!current()) return;
      apiClient.draft = { ...owner, proto: text, grpc_method: '' };
      grpcServices = [];
      await parseProto();
    } catch {
      if (current()) toasts.error('Couldn’t read the file', f.name);
    }
  }
  async function parseProto(): Promise<void> {
    const wid = ws.currentId;
    if (!wid) return;
    if (!draft.proto?.trim()) { toasts.error('No .proto file', 'Upload a .proto file first.'); return; }
    const owner = apiClient.draft;
    const current = () => ws.currentId === wid && apiClient.draft === owner;
    grpcParsing = true;
    try {
      const res = await api.post<{ services: GrpcService[] }>(`/workspaces/${wid}/api-client/grpc/describe`, { proto: draft.proto });
      if (!current()) return;
      grpcServices = res.services;
      const first = res.services.find((s) => s.methods.length);
      if (first && !draft.grpc_method) selectGrpcMethod(first.methods[0]);
      toasts.success('Read the .proto file', `${res.services.length} service(s)`);
    } catch (e) {
      if (current()) toastError('Couldn’t read the .proto file', e);
    } finally {
      grpcParsing = false;
    }
  }
  let grpcReflecting = $state(false);
  async function reflectGrpc(): Promise<void> {
    const wid = ws.currentId;
    if (!wid) return;
    if (!draft.url.trim()) { toasts.error('Enter the server URL first', 'Reflection asks the gRPC server for its services.'); return; }
    // Clear the uploaded schema before capturing the draft that owns this load.
    apiClient.draft = { ...apiClient.draft, proto: '' };
    const owner = apiClient.draft;
    const current = () => ws.currentId === wid && apiClient.draft === owner;
    grpcReflecting = true;
    try {
      const env = apiClient.activeEnv;
      const body: GrpcReflectReq = {url:owner.url, headers:owner.headers.filter(h => h.enabled !== false && h.key.trim() !== ''),
        environment_id:env?.id ?? null,vars:{...apiClient.runtimeVars}};
      let res: {services:GrpcService[]};
      try { res = await api.long.post<{services:GrpcService[]}>(`/workspaces/${wid}/api-client/grpc/reflect`, body); }
      catch (e) {
        const host = newHostConfirmHost(e);
        if (host === null || !current()) throw e;
        const ok = await confirmNewHost(host, {method:'gRPC reflection',url:body.url,secrets:draftSecrets(owner,env)});
        if (!ok || !current()) return;
        res = await api.long.post<{services:GrpcService[]}>(`/workspaces/${wid}/api-client/grpc/reflect`, {...body,confirm_new_host:true});
      }
      if (!current()) return;
      grpcServices = res.services;
      const first = res.services.find((s) => s.methods.length);
      if (first) selectGrpcMethod(first.methods[0]);
      toasts.success('Loaded services from the server', `${res.services.length} service(s)`);
    } catch (e) {
      if (current()) toastError('Couldn’t read server reflection', e);
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

  $effect(() => {
    apiStream.retainOwner(ws.currentId, draft.tabId);
  });

  // ── actions ────────────────────────────────────────────────────────────────
  function send(): void {
    switch (draft.kind) {
      case 'http': void apiClient.execute(); break;
      case 'grpc': void apiClient.executeGrpc(); break;
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
        }, draft.tabId);
        break;
    }
  }
  const sendLabel = $derived(draft.kind === 'grpc' ? 'Invoke' : isStreaming ? (apiStream.active ? 'Disconnect' : 'Connect') : 'Send');
  const sendBusy = $derived(apiClient.sending || apiStream.status === 'connecting');

  function onUrlKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') { e.preventDefault(); send(); }
  }

  // ⌘↵ send · ⌘S save (⌘T / ⌘D are claimed by the page via keyContext).
  // Window-level, but SCOPED: only while focus is inside this builder's own
  // surface — the API page for the full builder (or nothing focused at all),
  // the right panel for the compact one. Otherwise ⌘S in a vault note or ⌘↵
  // in a DB dialog would also save / SEND the API request, and with the page
  // and the panel both mounted one keypress would send twice.
  let rootEl = $state<HTMLDivElement>();
  function ownsShortcut(): boolean {
    if (!rootEl) return false;
    const active = document.activeElement;
    if (!active || active === document.body) return !compact;
    const scope = (compact ? rootEl.closest('.panel') : rootEl.closest('.api-root')) ?? rootEl;
    return scope.contains(active);
  }
  function onDocKeydown(e: KeyboardEvent): void {
    if (!(e.metaKey || e.ctrlKey) || e.altKey || e.shiftKey || ui.overlayOpen) return;
    if (e.defaultPrevented || (e.key !== 'Enter' && e.key !== 's') || !ownsShortcut()) return;
    if (e.key === 'Enter') { e.preventDefault(); send(); }
    else { e.preventDefault(); void save(); }
  }

  function stopRequest(): void {
    if (draft.kind === 'http' || draft.kind === 'grpc') apiClient.cancelExecute();
    else apiStream.disconnect();
  }
  const canStop = $derived(((draft.kind === 'http' || draft.kind === 'grpc') && apiClient.sending) || (isStreaming && apiStream.active));

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
  /** Settings → "Allow private addresses". Turning it ON lifts the SSRF guard
   *  for everyone in the workspace, so it asks first — the same confirm as the
   *  response pane's "Allow private addresses" button. A failed save puts the
   *  box back and says why (not always "admins only"). */
  async function setAllowLocal(input: HTMLInputElement): Promise<void> {
    const on = input.checked;
    if (on && !(await confirmer.ask(
      'API requests in this workspace will be able to reach localhost and private-network addresses (10.x, 192.168.x, …). Use this for local development servers. Everyone in the workspace is affected; you can block them again here.',
      { title: 'Allow private addresses?', confirmLabel: 'Allow private addresses', danger: false },
    ))) {
      input.checked = false;
      return;
    }
    try {
      await apiClient.setAllowLocal(on);
    } catch (e) {
      input.checked = !on;
      toasts.error('Couldn’t change the setting', e instanceof Error && e.message ? e.message : 'Only a workspace admin can change it.');
    }
  }
  async function clearCookies(): Promise<void> {
    if (!(await confirmer.ask('Delete every cookie Otto has collected for this workspace? Later requests won’t send them.', { title: 'Clear cookies', confirmLabel: 'Clear cookies' }))) return;
    await apiClient.clearCookies();
  }

  let codeOpen = $state(false);
  let codeLang = $state<CodeLang>('curl');
  const codeSnippet = $derived(codeOpen ? generateCode(draft, codeLang) : '');
  async function copyNotify(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${what}`);
    } catch {
      toasts.error('Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.');
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
      { label: 'Copy as curl', icon: 'terminal', action: () => void copyNotify(apiClient.toCurl(), 'as curl') },
      { label: 'Generate code…', icon: 'function', action: () => (codeOpen = true) },
      { label: 'Import from curl…', icon: 'download', action: () => (importOpen = true) },
      { separator: true },
      { label: 'Session variables…', icon: 'key', action: () => (varsOpen = true) },
      { label: 'Cookie jar…', icon: 'archive', action: openCookies },
    ];
    if (saved && canEdit) items.push({ separator: true }, { label: 'Delete request…', icon: 'trash', danger: true, action: () => void deleteSaved() });
    ctxMenu.show(e, items);
  }
;
async () => {

 { svelteHTML.createElement("svelte:window", {  "onkeydown":onDocKeydown,});}

 { const $$_div0 = svelteHTML.createElement("div", {   "class":`builder`,});compact;rootEl = $$_div0;
   { svelteHTML.createElement("div", { "class":`name-row`,});
       { svelteHTML.createElement("input", {               "class":`name-input`,"size":Math.min(48, Math.max(14, (draft.name || apiClient.tabLabel({ ...draft, name: '' })).length + 2)),"value":draft.name,"placeholder":apiClient.tabLabel({ ...draft, name: '' }),"aria-label":`Request name`,"oninput":(e) => setField('name', (e.currentTarget as HTMLInputElement).value),"spellcheck":`false`,});}
       { svelteHTML.createElement("span", {   "class":`where`,"title":location ?? 'Not saved yet',});
        if(location){ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});} { svelteHTML.createElement("span", { "class":`trunc`,});location; }}else{ }
        if(dirty && saved){ { svelteHTML.createElement("span", { "class":`edited`,});  }}
       }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void save(),"title":canEdit ? 'Save request (⌘S)' : 'Viewers can’t save requests',"disabled":!canEdit,});  }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":moreMenu,"aria-label":`More request actions`,"title":`More request actions`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
   }

  if(apiClient.replayIssue()){
     {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(apiClient.replayIssue() ?? '')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`tab-help`,...__sveltets_2_empty({"data-testid":`incomplete-history`}),});
       { svelteHTML.createElement("p", {});apiClient.replayIssue(); }
      if(draft.historyReplay?.requestId){
         { svelteHTML.createElement("button", {     "type":`button`,"class":`btn small`,"onclick":() => void apiClient.openHistorySavedRequest(),});    }
      }
     }}
  }

  
   { svelteHTML.createElement("div", { "class":`urlbar`,});
     { svelteHTML.createElement("select", {         "class":`kind`,"value":draft.kind,"aria-label":`Request type`,"title":REQUEST_KINDS.find((k) => k.id === draft.kind)?.title,"onchange":(e) => setKind((e.currentTarget as HTMLSelectElement).value as ApiRequestKind),});
         for(let k of __sveltets_2_ensureArray(REQUEST_KINDS)){k.id; { svelteHTML.createElement("option", {   "value":k.id,"title":k.title,});k.label; }}
     }
    if(draft.kind === 'http' || draft.kind === 'sse'){
       { svelteHTML.createElement("select", {       "class":`method`,"value":draft.method,"aria-label":`HTTP method`,"onchange":(e) => setField('method', (e.currentTarget as HTMLSelectElement).value),});
           for(let m of __sveltets_2_ensureArray(HTTP_METHODS)){m; { svelteHTML.createElement("option", { "value":m,});m; }}
       }
    }
     { svelteHTML.createElement("div", { "class":`url-wrap`,});
      
       { svelteHTML.createElement("div", {   "class":`url-layer url-highlight`,"aria-hidden":`true`,});
            for(let seg of __sveltets_2_ensureArray(urlSegments)){let i = 1;i; { svelteHTML.createElement("span", {  });seg.isVar;seg.isVar && usedVars.find((v) => v.name === seg.name)?.kind === 'missing';seg.text; }}
       }
       { svelteHTML.createElement("input", {                     "class":`url-layer url-input`,"placeholder":draft.kind === 'websocket' ? 'wss://echo.example.com/socket' : draft.kind === 'grpc' ? 'https://grpc.example.com:443' : 'https://api.example.com/v1/users  or  {{base_url}}/users',"value":draft.url,"oninput":(e) => setField('url', (e.currentTarget as HTMLInputElement).value),"onkeydown":onUrlKeydown,"onpaste":onUrlPaste,"spellcheck":`false`,"autocomplete":`off`,"aria-label":`Request URL`,"dir":`ltr`,});}
     }
     { svelteHTML.createElement("button", {       "class":`btn primary send`,"onclick":send,"disabled":sendBusy,"title":`${sendLabel} (⌘↵)`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`send`,"size":13,}});}sendBusy ? 'Sending…' : sendLabel;
     }
    if(canStop){
       { svelteHTML.createElement("button", {     "class":`btn stop`,"onclick":stopRequest,"title":`Stop the in-flight request`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}  }
    }
   }

  
  if(usedVars.length > 0){
     { svelteHTML.createElement("div", {   "class":`vars`,"aria-label":`Variables used by this request`,});
       { svelteHTML.createElement("span", { "class":`vars-label`,}); envForVars ? ` from ${envForVars.name}` : ''; }
         for(let v of __sveltets_2_ensureArray(usedVars)){v.name;
         { svelteHTML.createElement("span", {   "class":`var-chip ${v.kind}`,"title":v.detail,});
           { svelteHTML.createElement("span", { "class":`vn mono`,});v.name; }
           { svelteHTML.createElement("span", { "class":`vv mono`,});
            if(v.kind === 'secret'){ { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});} } else if (v.kind === 'missing'){ } else if (v.kind === 'dynamic'){ }else{ { svelteHTML.createElement("span", { "class":`trunc`,});v.value === '' ? '(empty)' : v.value; }}
           }
         }
      }
      if(missingVars){ { svelteHTML.createElement("span", { "class":`vars-warn`,});missingVars === 1 ? '1 variable isn’t set' : `${missingVars} variables aren’t set`; envForVars ? 'add it to the environment' : 'pick an environment';      }}
     }
  } else if (!compact && !draft.url.trim()){
     { svelteHTML.createElement("p", { "class":`tip`,});          { svelteHTML.createElement("code", {});  }    { svelteHTML.createElement("code", {});'{{name}}'; }         }
  }

  if(draft.kind === 'grpc'){
     { svelteHTML.createElement("div", { "class":`grpc-panel`,});
       { svelteHTML.createElement("p", { "class":`tab-help`,});        { svelteHTML.createElement("code", {});  }            }
       { svelteHTML.createElement("div", { "class":`grpc-row`,});
         { svelteHTML.createElement("label", { "class":`btn small file-btn`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}draft.proto?.trim() ? 'Replace .proto…' : 'Upload .proto…';
           { svelteHTML.createElement("input", {      "type":`file`,"accept":`.proto`,"hidden":true,"onchange":(e) => onProtoFile(e.currentTarget as HTMLInputElement),});}
         }
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":reflectGrpc,"disabled":grpcReflecting,"title":`List services via server reflection (no .proto needed)`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}grpcReflecting ? 'Loading methods…' : 'Load from server';
         }
        if(draft.proto?.trim()){
           { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":parseProto,"disabled":grpcParsing,});grpcParsing ? 'Reading…' : 'Re-read .proto'; }
        }
        if(grpcServices.length > 0){
           { svelteHTML.createElement("select", {       "class":`input grpc-method`,"value":draft.grpc_method,"onchange":(e) => onGrpcMethodChange((e.currentTarget as HTMLSelectElement).value),"aria-label":`gRPC method`,});
               for(let svc of __sveltets_2_ensureArray(grpcServices)){svc.name;
               { svelteHTML.createElement("optgroup", { "label":svc.name,});
                   for(let m of __sveltets_2_ensureArray(svc.methods)){m.full; { svelteHTML.createElement("option", { "value":m.full,});m.name;m.client_streaming || m.server_streaming ? ' (streaming)' : ''; }}
               }
            }
           }
        } else if (draft.grpc_method){
           { svelteHTML.createElement("span", { "class":`dim-text`,});  { svelteHTML.createElement("code", {});draft.grpc_method; } }
        }
       }
     }
  }

  if(draft.kind === 'websocket'){ { svelteHTML.createElement("p", { "class":`tab-help`,});                          }}
  if(isStreaming && scriptCount){ { svelteHTML.createElement("p", { "class":`tab-help`,});           }}

   { svelteHTML.createElement("div", {     "class":`tabstrip`,"role":`tablist`,"aria-label":`Request parts`,});
       for(let t of __sveltets_2_ensureArray(tabs)){t.id;
       { svelteHTML.createElement("button", {                "class":`tab`,"role":`tab`,"aria-selected":tab === t.id,"tabindex":tab === t.id ? 0 : -1,"id":`api-req-tab-${t.id}`,"aria-controls":`api-req-panel-${t.id}`,"onclick":() => (tab = t.id),"onkeydown":onTabKey,});tab === t.id;
        t.label;if(t.count){ { svelteHTML.createElement("span", {   "class":`count`,"aria-hidden":`true`,});t.count; }}
       }
    }
   }

   { svelteHTML.createElement("div", {       "class":`tabbody`,"role":`tabpanel`,"id":`api-req-panel-${tab}`,"aria-labelledby":`api-req-tab-${tab}`,});
    if(tab === 'params'){
       { svelteHTML.createElement("p", { "class":`tab-help`,});      { svelteHTML.createElement("code", {});  }       }
      ;__sveltets_2_ensureSnippet(kvEditor('query', draft.query));
      if(draft.url.trim()){
         { svelteHTML.createElement("div", { "class":`full-url`,});
           { svelteHTML.createElement("div", { "class":`full-url-head`,});
             { svelteHTML.createElement("span", {   "class":`full-url-label`,"id":`api-full-url-label`,});  }
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Copy the full URL`,"aria-label":`Copy the full URL`,"onclick":() => void copyNotify(sentUrl, 'URL'),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} }
           }
          
           { svelteHTML.createElement("div", {         "class":`full-url-text mono`,"dir":`ltr`,"role":`group`,"aria-labelledby":`api-full-url-label`,...__sveltets_2_empty({"data-testid":`api-full-url`}),});    for(let seg of __sveltets_2_ensureArray(sentUrlSegments)){let i = 1;i;if(seg.isVar){ { svelteHTML.createElement("span", {   "class":`full-url-var`,"title":`Filled in when sent`,});seg.text; }}else{seg.text;}} }
         }
      }
    } else if (tab === 'headers'){
       { svelteHTML.createElement("p", { "class":`tab-help`,});draft.kind === 'grpc' ? 'gRPC metadata sent with the call.' : 'Sent with the request. Content-Type follows the Body type unless you set it yourself.'; }
      ;__sveltets_2_ensureSnippet(kvEditor('headers', draft.headers));
      if(draft.kind !== 'grpc'){
         { svelteHTML.createElement("div", { "class":`quick`,});
           { svelteHTML.createElement("span", { "class":`dim-text`,});  }
             for(let [k, v] of __sveltets_2_ensureArray(QUICK_HEADERS)){k;
            if(!draft.headers.some((h) => h.key.toLowerCase() === k.toLowerCase())){
               { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":() => addRow('headers', k, v),});k; }
            }
          }
         }
      }
    } else if (tab === 'body' && draft.kind === 'grpc'){
       { svelteHTML.createElement("div", { "class":`bodybar`,});
         { svelteHTML.createElement("span", { "class":`tab-help inline`,});     }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":beautify,});  }
       }
      if(compact){
         { svelteHTML.createElement("textarea", {             "dir":`ltr`,"class":`input body-area mono`,"aria-label":`Request message`,"value":draft.body,"oninput":(e) => setField('body', (e.currentTarget as HTMLTextAreaElement).value),"placeholder":'{ }',"spellcheck":`false`,}); }
      }else{
         { svelteHTML.createElement("div", { "class":`body-editor`,}); { const $$_rotidEedoC3C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC3C({ target: __sveltets_2_any(), props: {                "lsp":false,"wrap":true,"highlightLineLimit":LONG_LINE_PLAIN,"path":`message.json`,"content":draft.body,"root":ws.current?.root_path ?? '',"language":`json`,"readOnly":false,"onchange":(v) => setField('body', v),}});} }
      }
    } else if (tab === 'body'){
       { svelteHTML.createElement("div", { "class":`bodybar`,});
         { svelteHTML.createElement("label", { "class":`inline-field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", {       "class":`input`,"value":bodyChoice,"onchange":(e) => onBodyChoice((e.currentTarget as HTMLSelectElement).value as BodyChoice),"aria-label":`Body type`,});
               for(let c of __sveltets_2_ensureArray(BODY_CHOICES)){c.id; { svelteHTML.createElement("option", { "value":c.id,});c.label; }}
           }
         }
        if(bodyChoice === 'text'){
           { svelteHTML.createElement("label", { "class":`inline-field`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("select", {       "class":`input`,"value":rawType,"onchange":(e) => onRawType((e.currentTarget as HTMLSelectElement).value as RawType),"aria-label":`Text format`,});
                 for(let t of __sveltets_2_ensureArray(TEXT_TYPES)){t; { svelteHTML.createElement("option", { "value":t,});t === 'Text' ? 'Plain text' : t; }}
             }
           }
        }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(rawActive && rawType !== 'Text' && rawType !== 'JavaScript'){
           { svelteHTML.createElement("button", {     "class":`btn ghost small`,"onclick":beautify,"title":`Pretty-print the body`,}); rawType; }
        }
       }
       { svelteHTML.createElement("p", { "class":`tab-help`,});BODY_CHOICES.find((c) => c.id === bodyChoice)?.help; }
      if(draft.body_mode === 'none'){
        
      } else if (formActive){
         { svelteHTML.createElement("div", { "class":`kv-list`,});
           { svelteHTML.createElement("div", {   "class":`kv-head`,"aria-hidden":`true`,}); { svelteHTML.createElement("span", {});  }if(multipartActive){ { svelteHTML.createElement("span", { "class":`kv-type-h`,});  }} { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("span", { "class":`kv-x-h`,}); } }
              for(let row of __sveltets_2_ensureArray(formRows)){let i = 1;i;
             { svelteHTML.createElement("div", { "class":`kv-row`,});
               { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input kv-key mono`,"placeholder":`email`,"aria-label":`Field name`,"value":row.key,"oninput":(e) => updateFormRow(i, { key: (e.currentTarget as HTMLInputElement).value }),});}
              if(multipartActive){
                 { svelteHTML.createElement("select", {       "class":`input row-type`,"value":row.type,"onchange":(e) => setRowType(i, (e.currentTarget as HTMLSelectElement).value as FieldType),"aria-label":`Field type`,});
                   { svelteHTML.createElement("option", { "value":`text`,});  }
                   { svelteHTML.createElement("option", { "value":`file`,});  }
                 }
              }
              if(multipartActive && row.type === 'file'){
                 { svelteHTML.createElement("label", {   "class":`btn small file-btn kv-val`,"title":row.filename ?? 'Choose a file',});
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});} { svelteHTML.createElement("span", { "class":`file-name`,});row.filename || 'Choose file…'; }
                   { svelteHTML.createElement("input", {    "type":`file`,"hidden":true,"onchange":(e) => pickFile(i, e.currentTarget as HTMLInputElement),});}
                 }
              }else{
                 { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input kv-val mono`,"placeholder":`ada@example.com`,"aria-label":`Field value`,"value":row.value,"oninput":(e) => updateFormRow(i, { value: (e.currentTarget as HTMLInputElement).value }),});}
              }
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Remove field`,"aria-label":`Remove field`,"onclick":() => removeFormRow(i),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
             }
          }
           { svelteHTML.createElement("button", {   "class":`btn small ghost add-row`,"onclick":addFormRow,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
         }
      } else if (compact){
         { svelteHTML.createElement("textarea", {             "dir":`ltr`,"class":`input body-area mono`,"aria-label":`Request body`,"value":draft.body,"oninput":(e) => setField('body', (e.currentTarget as HTMLTextAreaElement).value),"placeholder":draft.body_mode === 'json' ? '{ "name": "value" }' : draft.body_mode === 'graphql' ? 'query { viewer { id } }' : 'Hello, world',"spellcheck":`false`,}); }
      } else if (draft.body_mode === 'graphql'){
         { svelteHTML.createElement("div", { "class":`gql-bar`,});
           { svelteHTML.createElement("span", { "class":`sub-label`,});  }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {     "class":`btn ghost small`,"onclick":() => apiClient.graphqlIntrospect(),"disabled":apiClient.graphqlIntrospecting,});
            apiClient.graphqlIntrospecting ? 'Loading schema…' : 'Load schema from server';
           }
         }
         { svelteHTML.createElement("div", { "class":`body-editor gql-query`,}); { const $$_rotidEedoC3C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC3C({ target: __sveltets_2_any(), props: {            "lsp":false,"path":`query.graphql`,"content":draft.body,"root":ws.current?.root_path ?? '',"language":"","readOnly":false,"onchange":(v) => setField('body', v),}});} }
         { svelteHTML.createElement("div", { "class":`gql-bar`,}); { svelteHTML.createElement("span", { "class":`sub-label`,});  } }
         { svelteHTML.createElement("div", { "class":`body-editor gql-vars`,}); { const $$_rotidEedoC3C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC3C({ target: __sveltets_2_any(), props: {                "lsp":false,"wrap":true,"highlightLineLimit":LONG_LINE_PLAIN,"path":`variables.json`,"content":draft.graphql_variables ?? '',"root":ws.current?.root_path ?? '',"language":`json`,"readOnly":false,"onchange":(v) => setField('graphql_variables', v),}});} }
        if(apiClient.graphqlSchema){
           { svelteHTML.createElement("div", { "class":`gql-schema`,});
             { svelteHTML.createElement("div", { "class":`sub-label`,}); apiClient.graphqlSchema.length;  }
               for(let t of __sveltets_2_ensureArray(apiClient.graphqlSchema)){t.name;
               { svelteHTML.createElement("details", { "class":`gql-type`,});
                 { svelteHTML.createElement("summary", {});t.name;  { svelteHTML.createElement("span", { "class":`gql-kind`,});t.kind.toLowerCase(); } }
                 { svelteHTML.createElement("div", { "class":`gql-fields mono`,});t.fields.join(', ') || '—'; }
               }
            }
           }
        }
      }else{
         { svelteHTML.createElement("div", { "class":`body-editor`,});
           { const $$_rotidEedoC3C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC3C({ target: __sveltets_2_any(), props: {                "lsp":false,"wrap":true,"highlightLineLimit":LONG_LINE_PLAIN,"path":editorPath,"content":draft.body,"root":ws.current?.root_path ?? '',"language":editorLang,"readOnly":false,"onchange":(v) => setField('body', v),}});}
         }
      }
    } else if (tab === 'auth'){
       { svelteHTML.createElement("div", { "class":`auth`,});
         { svelteHTML.createElement("label", { "class":`inline-field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", {       "class":`input`,"value":draft.auth.type,"aria-label":`Auth type`,"onchange":(e) => setAuthType((e.currentTarget as HTMLSelectElement).value as ApiAuth['type']),});
               for(let t of __sveltets_2_ensureArray(AUTH_TYPES)){t.id; { svelteHTML.createElement("option", { "value":t.id,});t.label; }}
           }
         }
         { svelteHTML.createElement("p", { "class":`tab-help`,});AUTH_TYPES.find((t) => t.id === draft.auth.type)?.help; }
        if(draft.auth.type === 'bearer'){
           { svelteHTML.createElement("div", { "class":`field-row`,});
             { svelteHTML.createElement("label", { "for":`auth-token`,});  }
             { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`auth-token`,"class":`input mono grow`,"value":secretValue(draft.auth.token),"oninput":(e) => setAuth({ token: (e.currentTarget as HTMLInputElement).value }),"placeholder":secretPlaceholder(draft.auth.token, '{{api_token}}'),});}
           }
        } else if (draft.auth.type === 'basic'){
           { svelteHTML.createElement("div", { "class":`field-row`,});
             { svelteHTML.createElement("label", { "for":`auth-user`,});  }
             { svelteHTML.createElement("input", {          "dir":`auto`,"id":`auth-user`,"class":`input grow`,"value":draft.auth.username,"oninput":(e) => setAuth({ username: (e.currentTarget as HTMLInputElement).value }),});}
           }
           { svelteHTML.createElement("div", { "class":`field-row`,});
             { svelteHTML.createElement("label", { "for":`auth-pass`,});  }
             { svelteHTML.createElement("input", {            "id":`auth-pass`,"class":`input grow`,"type":`password`,"value":secretValue(draft.auth.password),"oninput":(e) => setAuth({ password: (e.currentTarget as HTMLInputElement).value }),"placeholder":secretPlaceholder(draft.auth.password, ''),});}
           }
        } else if (draft.auth.type === 'api_key'){
           { svelteHTML.createElement("div", { "class":`field-row`,});
             { svelteHTML.createElement("label", { "for":`auth-key`,});  }
             { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`auth-key`,"class":`input mono grow`,"value":draft.auth.key,"oninput":(e) => setAuth({ key: (e.currentTarget as HTMLInputElement).value }),"placeholder":`X-Api-Key`,});}
           }
           { svelteHTML.createElement("div", { "class":`field-row`,});
             { svelteHTML.createElement("label", { "for":`auth-value`,});  }
             { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`auth-value`,"class":`input mono grow`,"value":secretValue(draft.auth.value),"oninput":(e) => setAuth({ value: (e.currentTarget as HTMLInputElement).value }),"placeholder":secretPlaceholder(draft.auth.value, '{{api_key}}'),});}
           }
           { svelteHTML.createElement("div", { "class":`field-row`,});
             { svelteHTML.createElement("label", { "for":`auth-in`,});  }
             { svelteHTML.createElement("select", {       "id":`auth-in`,"class":`input`,"value":draft.auth.in,"onchange":(e) => setAuth({ in: (e.currentTarget as HTMLSelectElement).value as 'header' | 'query' }),});
               { svelteHTML.createElement("option", { "value":`header`,});  }
               { svelteHTML.createElement("option", { "value":`query`,});   }
             }
           }
        } else if (draft.auth.type === 'oauth2'){
           { svelteHTML.createElement("div", { "class":`field-row`,});
             { svelteHTML.createElement("label", { "for":`auth-grant`,});    }
             { svelteHTML.createElement("select", {       "id":`auth-grant`,"class":`input`,"value":draft.auth.grant,"onchange":(e) => setAuth({ grant: (e.currentTarget as HTMLSelectElement).value as 'client_credentials' | 'password' | 'refresh_token' | 'authorization_code' }),});
               { svelteHTML.createElement("option", { "value":`authorization_code`,});       }
               { svelteHTML.createElement("option", { "value":`client_credentials`,});   }
               { svelteHTML.createElement("option", { "value":`password`,});   }
               { svelteHTML.createElement("option", { "value":`refresh_token`,});  }
             }
           }
          if(draft.auth.grant === 'authorization_code'){
             { svelteHTML.createElement("div", { "class":`field-row`,}); { svelteHTML.createElement("label", { "for":`auth-aurl`,});  } { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`auth-aurl`,"class":`input mono grow`,"value":draft.auth.authorization_url ?? '',"oninput":(e) => setAuth({ authorization_url: e.currentTarget.value }),"placeholder":`https://auth.example.com/oauth/authorize`,});} }
             { svelteHTML.createElement("p", { "class":`tab-help`,});        { svelteHTML.createElement("code", {});baseUrl().replace(/\/$/, '');  }                }
          }
           { svelteHTML.createElement("div", { "class":`field-row`,});
             { svelteHTML.createElement("label", { "for":`auth-turl`,});  }
             { svelteHTML.createElement("input", {                "dir":`ltr`,"id":`auth-turl`,"class":`input mono grow`,"value":draft.auth.token_url,"aria-invalid":tokenUrlError ? 'true' : undefined,"aria-describedby":tokenUrlError ? 'auth-turl-err' : undefined,"oninput":(e) => { tokenUrlError = ''; setAuth({ token_url: (e.currentTarget as HTMLInputElement).value }); },"placeholder":`https://auth.example.com/oauth/token`,});}
           }
          if(tokenUrlError){ { svelteHTML.createElement("p", {   "class":`field-error`,"id":`auth-turl-err`,});tokenUrlError; }}
           { svelteHTML.createElement("div", { "class":`field-row`,});
             { svelteHTML.createElement("label", { "for":`auth-cid`,});  }
             { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`auth-cid`,"class":`input mono grow`,"value":draft.auth.client_id,"oninput":(e) => setAuth({ client_id: (e.currentTarget as HTMLInputElement).value }),});}
           }
           { svelteHTML.createElement("div", { "class":`field-row`,});
             { svelteHTML.createElement("label", { "for":`auth-csec`,});  }
             { svelteHTML.createElement("input", {            "id":`auth-csec`,"class":`input mono grow`,"type":`password`,"value":secretValue(draft.auth.client_secret),"oninput":(e) => setAuth({ client_secret: (e.currentTarget as HTMLInputElement).value }),"placeholder":secretPlaceholder(draft.auth.client_secret, ''),});}
           }
          if(draft.auth.grant === 'password'){
             { svelteHTML.createElement("div", { "class":`field-row`,});
               { svelteHTML.createElement("label", { "for":`auth-ouser`,});  }
               { svelteHTML.createElement("input", {          "dir":`auto`,"id":`auth-ouser`,"class":`input grow`,"value":draft.auth.username,"oninput":(e) => setAuth({ username: (e.currentTarget as HTMLInputElement).value }),});}
             }
             { svelteHTML.createElement("div", { "class":`field-row`,});
               { svelteHTML.createElement("label", { "for":`auth-opass`,});  }
               { svelteHTML.createElement("input", {            "id":`auth-opass`,"class":`input grow`,"type":`password`,"value":secretValue(draft.auth.password),"oninput":(e) => setAuth({ password: (e.currentTarget as HTMLInputElement).value }),"placeholder":secretPlaceholder(draft.auth.password, ''),});}
             }
          }
          if(draft.auth.grant === 'refresh_token'){
             { svelteHTML.createElement("div", { "class":`field-row`,});
               { svelteHTML.createElement("label", { "for":`auth-rt`,});  }
               { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`auth-rt`,"class":`input mono grow`,"value":secretValue(draft.auth.refresh_token),"oninput":(e) => setAuth({ refresh_token: (e.currentTarget as HTMLInputElement).value }),"placeholder":secretPlaceholder(draft.auth.refresh_token, ''),});}
             }
          }
           { svelteHTML.createElement("div", { "class":`field-row`,});
             { svelteHTML.createElement("label", { "for":`auth-scope`,});  }
             { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`auth-scope`,"class":`input mono grow`,"value":draft.auth.scope,"oninput":(e) => setAuth({ scope: (e.currentTarget as HTMLInputElement).value }),"placeholder":`read write`,});}
           }
           { svelteHTML.createElement("div", { "class":`field-row`,});
             { svelteHTML.createElement("span", { "class":`field-spacer`,}); }
             { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":fetchOAuthToken,"disabled":fetchingToken,});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}fetchingToken ? 'Getting token…' : 'Get token';
             }
            if(isSecretRef(draft.auth.access_token) || draft.auth.access_token){
               { svelteHTML.createElement("span", { "class":`chip ok`,});isSecretRef(draft.auth.access_token) ? 'Access token stored in Keychain' : 'Access token received'; }
            }
           }
        }
        if(draft.auth.type !== 'none'){
           { svelteHTML.createElement("p", { "class":`keychain-note`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});} { svelteHTML.createElement("span", {});                                     { svelteHTML.createElement("code", {});'{{variable}}'; }  } }
        }
       }
    } else if (tab === 'scripts'){
       { svelteHTML.createElement("div", { "class":`scripts-pane`,});
         { svelteHTML.createElement("p", { "class":`tab-help`,});      { svelteHTML.createElement("code", {});  }                   }
         { svelteHTML.createElement("div", { "class":`script-block`,});
           { svelteHTML.createElement("div", { "class":`script-head`,});
             { svelteHTML.createElement("span", { "class":`sub-label`,});  }
             { svelteHTML.createElement("span", { "class":`script-hint mono`,});    }
           }
           { svelteHTML.createElement("div", { "class":`script-editor`,});
             { const $$_rotidEedoC5C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC5C({ target: __sveltets_2_any(), props: {              "lsp":false,"path":`pre.js`,"content":draft.pre_request_script ?? '',"root":ws.current?.root_path ?? '',"language":`js`,"readOnly":false,"onchange":(v) => setField('pre_request_script', v),}});}
           }
         }
         { svelteHTML.createElement("div", { "class":`script-block`,});
           { svelteHTML.createElement("div", { "class":`script-head`,});
             { svelteHTML.createElement("span", { "class":`sub-label`,});    }
             { svelteHTML.createElement("span", { "class":`script-hint mono`,});     }
           }
           { svelteHTML.createElement("div", { "class":`script-editor`,});
             { const $$_rotidEedoC5C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC5C({ target: __sveltets_2_any(), props: {              "lsp":false,"path":`post.js`,"content":draft.post_response_script ?? '',"root":ws.current?.root_path ?? '',"language":`js`,"readOnly":false,"onchange":(v) => setField('post_response_script', v),}});}
           }
         }
       }
    } else if (tab === 'docs'){
       { svelteHTML.createElement("p", { "class":`tab-help`,});                   }
       { svelteHTML.createElement("div", { "class":`docs-pane`,});
         { svelteHTML.createElement("div", { "class":`docs-edit`,});
           { const $$_rotidEedoC4C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC4C({ target: __sveltets_2_any(), props: {              "lsp":false,"wrap":true,"path":`docs.md`,"content":draft.docs ?? '',"root":ws.current?.root_path ?? '',"language":`md`,"readOnly":false,"onchange":(v) => setField('docs', v),}});}
         }
        if(!compact){
           { svelteHTML.createElement("div", {   "class":`docs-preview md-body`,"aria-label":`Docs preview`,});
            if(draft.docs?.trim()){ docsHtml;}else{ { svelteHTML.createElement("p", { "class":`dim-text`,});    }}
           }
        }
       }
    } else if (tab === 'settings'){
       { svelteHTML.createElement("div", { "class":`settings-pane`,});
         { svelteHTML.createElement("label", { "class":`set-row`,});
           { svelteHTML.createElement("span", { "class":`set-label`,});  }
           { svelteHTML.createElement("span", { "class":`set-control`,});
             { svelteHTML.createElement("input", {              "class":`input set-num mono`,"type":`number`,"min":`0`,"step":`1000`,"placeholder":`Default`,"value":settings.timeout_ms ?? '',"oninput":(e) => { const v = (e.currentTarget as HTMLInputElement).value; setSetting('timeout_ms', v === '' ? null : Number(v)); },});}
             { svelteHTML.createElement("span", { "class":`set-unit`,});      }
           }
         }
         { svelteHTML.createElement("label", { "class":`set-row toggle`,});
           { svelteHTML.createElement("input", {      "type":`checkbox`,"checked":settings.follow_redirects,"onchange":(e) => setSetting('follow_redirects', (e.currentTarget as HTMLInputElement).checked),});}
           { svelteHTML.createElement("span", { "class":`set-label`,});  }
           { svelteHTML.createElement("span", { "class":`set-unit`,});           }
         }
         { svelteHTML.createElement("label", { "class":`set-row toggle`,});
           { svelteHTML.createElement("input", {      "type":`checkbox`,"checked":settings.verify_ssl,"onchange":(e) => setSetting('verify_ssl', (e.currentTarget as HTMLInputElement).checked),});}
           { svelteHTML.createElement("span", { "class":`set-label`,});   }
           { svelteHTML.createElement("span", { "class":`set-unit`,});settings.verify_ssl ? 'Turn off only for self-signed development servers.' : 'This request trusts any TLS certificate. Turn verification back on when you finish testing.'; }
         }
        if(draft.kind === 'http'){
           { svelteHTML.createElement("label", { "class":`set-row`,});
             { svelteHTML.createElement("span", { "class":`set-label`,});  }
             { svelteHTML.createElement("span", { "class":`set-control`,});
               { svelteHTML.createElement("select", {     "class":`input set-select`,"value":draft.ssh_connection_id ?? '',"onchange":(e) => { const v = (e.currentTarget as HTMLSelectElement).value; apiClient.draft = { ...draft, ssh_connection_id: v === '' ? null : v }; },});
                 { svelteHTML.createElement("option", {"value":"",});    }
                   for(let c of __sveltets_2_ensureArray(apiClient.sshConnections)){c.id; { svelteHTML.createElement("option", { "value":c.id,});  c.name; }}
               }
               { svelteHTML.createElement("span", { "class":`set-unit`,});
                apiClient.sshConnections.length === 0 ? 'Add an SSH connection on the Connections page to call IP-restricted APIs from a bastion.' : 'For APIs that only accept calls from a whitelisted IP.';
               }
             }
           }
        }
         { svelteHTML.createElement("div", { "class":`ws-setting`,});
           { svelteHTML.createElement("div", { "class":`section-title`,});  }
           { svelteHTML.createElement("label", { "class":`set-row toggle`,});
             { svelteHTML.createElement("input", {      "type":`checkbox`,"checked":apiClient.allowLocal,"onchange":(e) => void setAllowLocal(e.currentTarget),});}
             { svelteHTML.createElement("span", { "class":`set-label`,});   }
             { svelteHTML.createElement("span", { "class":`set-unit`,});                     }
           }
         }
       }
    }
   }
 }



if(saveOpen){
   { const $$_golaiDtseuqeRevaS0C = __sveltets_2_ensureComponent(SaveRequestDialog); new $$_golaiDtseuqeRevaS0C({ target: __sveltets_2_any(), props: {         "initialName":draft.name?.trim() || apiClient.tabLabel({ ...draft, name: '' }),"initialCollection":saved?.collection_id ?? draft.collectionHint ?? null,"onclose":() => (saveOpen = false),"onsave":async (name, collectionId) => !!(await apiClient.saveDraft(name, collectionId)),}});}
}

if(importOpen){
   { const $$_golaiDtropmI0C = __sveltets_2_ensureComponent(ImportDialog); new $$_golaiDtropmI0C({ target: __sveltets_2_any(), props: {    "initial":`curl`,"onclose":() => (importOpen = false),}});}
}

if(codeOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Generate code`,"width":640,"onclose":() => (codeOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (codeOpen = false),});  }
       { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => void copyNotify(codeSnippet, 'code'),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":13,}});}  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("div", { "class":`code-head`,});
       { svelteHTML.createElement("p", { "class":`tab-help`,});               { svelteHTML.createElement("code", {});  }  }
       { svelteHTML.createElement("select", {       "class":`input`,"value":codeLang,"onchange":(e) => (codeLang = (e.currentTarget as HTMLSelectElement).value as CodeLang),"aria-label":`Language`,});
           for(let l of __sveltets_2_ensureArray(CODE_LANGS)){l.id; { svelteHTML.createElement("option", { "value":l.id,});l.label; }}
       }
     }
     { svelteHTML.createElement("pre", {   "class":`code-snippet mono`,"dir":`ltr`,});codeSnippet; }
    
   Modal}
}

if(cookiesOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Cookie jar`,"width":620,"onclose":() => (cookiesOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {     "class":`btn danger`,"onclick":clearCookies,"disabled":apiClient.cookies.length === 0,});  }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => void apiClient.loadCookies(),});  }
       { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => (cookiesOpen = false),});  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("p", { "class":`tab-help`,});                             }
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`cookies`,"variant":`compact`,"loading":apiClient.cookiesLoading,"error":apiClient.cookiesError,"empty":apiClient.cookies.length === 0,"onretry":() => void apiClient.loadCookies(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("p", { "class":`dim-text`,});   }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
      
       { svelteHTML.createElement("table", { "class":`cookie-table`,});
         { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } } }
         { svelteHTML.createElement("tbody", {});
             for(let c of __sveltets_2_ensureArray(apiClient.cookies)){c.domain + c.name;
             { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("td", { "class":`mono`,});c.domain; } { svelteHTML.createElement("td", { "class":`mono`,});c.name; } { svelteHTML.createElement("td", {   "class":`mono cookie-val`,"title":c.value,});c.value; } }
          }
         }
       }
     LoadState}
    
   Modal}
}

if(varsOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Session variables`,"width":560,"onclose":() => (varsOpen = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":() => (varsOpen = false),});  }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("p", { "class":`tab-help`,});                          { svelteHTML.createElement("code", {});  }  }
     { svelteHTML.createElement("div", { "class":`kv-list`,});
         for(let [k, v] of __sveltets_2_ensureArray(varEntries)){k;
         { svelteHTML.createElement("div", { "class":`kv-row`,});
           { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`input kv-key mono`,"aria-label":`Variable name`,"value":k,"onchange":(e) => apiClient.renameRuntimeVar(k, (e.currentTarget as HTMLInputElement).value),});}
           { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`input kv-val mono`,"aria-label":`Value of ${k}`,"value":v,"oninput":(e) => apiClient.setRuntimeVar(k, (e.currentTarget as HTMLInputElement).value),});}
           { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Remove variable`,"aria-label":`Remove variable`,"onclick":() => apiClient.removeRuntimeVar(k),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
         }
      }
       { svelteHTML.createElement("div", { "class":`kv-row`,});
         { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input kv-key mono`,"placeholder":`base_url`,"aria-label":`New variable name`,"bind:value":newVarKey,"onkeydown":(e) => { if (e.key === 'Enter') addVar(); },});/*Ωignore_startΩ*/() => newVarKey = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input kv-val mono`,"placeholder":`https://api.example.com`,"aria-label":`New variable value`,"bind:value":newVarVal,"onkeydown":(e) => { if (e.key === 'Enter') addVar(); },});/*Ωignore_startΩ*/() => newVarVal = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Add variable`,"aria-label":`Add variable`,"onclick":addVar,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} }
       }
     }
    
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('tab'), slots: {}, events: {} }}
const RequestBuilder__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RequestBuilder__SvelteComponent_ = ReturnType<typeof RequestBuilder__SvelteComponent_>;
/*Ωignore_endΩ*/export default RequestBuilder__SvelteComponent_;