///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { toastError } from '../../lib/toastError';
import { plural } from '../../lib/plural';
// Response viewer shared by the page and the compact panel: status / time /
// size chips, then Body (Pretty · Raw · Tree · Preview) · Headers · Cookies ·
// Timeline · Tests. A failed send shows inline with the reason and what to
// do; SSE / WebSocket requests get a live message console instead.
import Icon from '../../lib/components/Icon.svelte';
import Tabs from '../../lib/components/Tabs.svelte';
import CodeEditor from '../../lib/components/CodeEditor.svelte';
import VirtualList from '../../lib/components/VirtualList.svelte';
import ContextPacketDialog from '../../lib/components/ContextPacketDialog.svelte';
import StatusChip from './StatusChip.svelte';
import JsonTree from './JsonTree.svelte';
import { looksBinary, mimeOf } from './responseKind';
import { apiClient } from '../../lib/stores/apiClient.svelte';
import { apiStream } from '../../lib/stores/apiStream.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
import { formatBytes, formatSeconds } from '../../lib/metric-format';
import { parseSetCookies } from '../../lib/api/apiVars';
import { authedBlobUrl } from '../../lib/api/client';
;

  interface Props {
    compact?: boolean;
    /** Open the request's Settings tab (from the error hints). */
    onsettings?: () => void;
  };;

  type BodyView = 'pretty' | 'raw' | 'tree' | 'preview';;;

  // ── Tabs ────────────────────────────────────────────────────────────────────
  type Tab = 'body' | 'headers' | 'cookies' | 'trace' | 'tests';;function $$render() {
 const historyBanner/*Ωignore_positionΩ*/ = (h: { at: string; truncated: boolean })/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", {   "class":`history-banner`,"role":`note`,});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});}
     { svelteHTML.createElement("span", {}); { svelteHTML.createElement("strong", {});  }  historyWhen(h.at);      if(h.truncated){          } }
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { compact = false, onsettings }: Props = $props();

  let sendToAgentOpen = $state(false);
  const resp = $derived(apiClient.lastResponse);
  const failure = $derived(apiClient.lastError);
  /** Set when this tab shows a history entry's stored response (not a send). */
  const fromHistory = $derived(apiClient.responseFromHistory);
  function historyWhen(at: string): string {
    const d = new Date(at);
    return Number.isNaN(d.getTime()) ? at : d.toLocaleString();
  }

  // ── Pretty-print: memoized + size-gated ────────────────────────────────────
  // Bodies over 256 KB are shown raw (re-parsing would block the main thread).
  const PRETTY_SIZE_LIMIT = 256 * 1024;
  const STREAM_RING_MAX = 500;

  function isJson(ct: string | null, body: string): boolean {
    if (ct && /\bjson\b/i.test(ct)) return true;
    const t = body.trim();
    return (t.startsWith('{') && t.endsWith('}')) || (t.startsWith('[') && t.endsWith(']'));
  }
  function respExt(ct: string | null, body: string): string {
    const c = (ct ?? '').toLowerCase();
    const t = body.trim();
    if (c.includes('json') || t.startsWith('{') || t.startsWith('[')) return 'json';
    if (c.includes('html')) return 'html';
    if (c.includes('xml')) return 'xml';
    if (c.includes('javascript')) return 'js';
    if (c.includes('css')) return 'css';
    return 'txt';
  }
  const isJsonResp = $derived(!!resp && isJson(resp.content_type, resp.body));
  const isHtml = $derived(!!resp && /html/i.test(resp.content_type ?? ''));
  const isImage = $derived(!!resp?.content_type && /^image\//i.test(resp.content_type));
  // Bytes the text editor can't show: binary payloads, and images too big for
  // the inline preview (no `body_base64`). Shown as a download notice instead.
  const isBinary = $derived(
    !!resp && !resp.too_large && (isImage ? !resp.body_base64 && mimeOf(resp.content_type) !== 'image/svg+xml' : looksBinary(resp.content_type, resp.body)),
  );
  const respPath = $derived(resp ? `response.${respExt(resp.content_type, resp.body)}` : 'response.txt');
  const respLang = $derived(resp ? respExt(resp.content_type, resp.body) : '');

  // Memoize the parsed JSON so filters / the tree don't re-parse each time.
  let _parsedCache: { body: string; parsed: unknown } | null = null;
  function getCachedParsed(body: string): unknown {
    if (_parsedCache?.body !== body) {
      try { _parsedCache = { body, parsed: JSON.parse(body) }; }
      catch { _parsedCache = null; }
    }
    return _parsedCache?.parsed;
  }
  const parsed = $derived(resp && isJsonResp && resp.body.length <= PRETTY_SIZE_LIMIT ? getCachedParsed(resp.body) : undefined);

  function evalJsonPath(root: unknown, path: string): unknown {
    let p = path.trim();
    if (p === '' || p === '$') return root;
    p = p.replace(/^\$\.?/, '');
    const tokens = p.match(/[^.[\]'"]+/g) ?? [];
    let cur: unknown = root;
    for (const t of tokens) {
      if (cur == null || typeof cur !== 'object') return undefined;
      const key = /^\d+$/.test(t) ? Number(t) : t;
      cur = (cur as Record<string | number, unknown>)[key];
    }
    return cur;
  }

  let bodyView: BodyView = $state('pretty');
  // Keep the view valid for the current response.
  $effect(() => {
    if (bodyView === 'tree' && parsed === undefined) bodyView = 'pretty';
    if (bodyView === 'preview' && !isHtml && !isImage) bodyView = 'pretty';
    if (isImage && bodyView !== 'preview') bodyView = 'preview';
  });

  // Filter (JSONPath in Pretty) / search (Tree) — debounced.
  let filter = $state('');
  let filterDebounced = $state('');
  let _filterTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const v = filter;
    clearTimeout(_filterTimer);
    _filterTimer = setTimeout(() => { filterDebounced = v; }, 150);
  });

  const prettyBody = $derived.by(() => {
    if (!resp) return '';
    if (resp.body.length > PRETTY_SIZE_LIMIT) return resp.body;
    if (parsed !== undefined) {
      try { return JSON.stringify(parsed, null, 2); } catch { /* fall through */ }
    }
    return resp.body;
  });
  const displayBody = $derived.by(() => {
    if (!resp) return '';
    const base = bodyView === 'raw' ? resp.body : prettyBody;
    const f = filterDebounced.trim();
    if (bodyView === 'pretty' && f && parsed !== undefined) {
      const result = evalJsonPath(parsed, f);
      return result === undefined ? `// Nothing at ${f}` : JSON.stringify(result, null, 2);
    }
    return base;
  });

  // The text editor exists whenever the body is showable as text; it is only
  // VISIBLE on the Body tab in Pretty/Raw (see the template).
  const bodyEmpty = $derived(!resp || resp.body.trim() === '');
  const editorEligible = $derived(!!resp && !resp.too_large && !bodyEmpty && !isBinary);
  const showEditor = $derived.by(
    () =>
      editorEligible &&
      tab === 'body' &&
      !(bodyView === 'preview' && ((isImage && !!previewUrl) || isHtml)) &&
      !(bodyView === 'tree' && parsed !== undefined),
  );

  // Soft-wrap whenever the shown text may be one huge line (raw / minified /
  // over the pretty limit / not JSON): an unwrapped multi-hundred-KB line makes
  // every layout pass measure it. Pretty-printed JSON reads better unwrapped.
  // `.by` (a closure): a bare `$derived(expr)` is type-checked where `bodyView`
  // is still narrowed to its `'pretty'` initializer.
  const respWrap = $derived.by(
    () => !!resp && (respLang !== 'json' || bodyView === 'raw' || parsed === undefined || resp.body.length > PRETTY_SIZE_LIMIT),
  );

  // ── Streams ─────────────────────────────────────────────────────────────────
  const streamItems = $derived(
    apiStream.items.length > STREAM_RING_MAX ? apiStream.items.slice(apiStream.items.length - STREAM_RING_MAX) : apiStream.items,
  );
  const streamKind = $derived(apiClient.draft.kind);
  const isStream = $derived(streamKind === 'sse' || streamKind === 'websocket');
  let wsSend = $state('');
  function sendWs(): void {
    if (!wsSend.trim()) return;
    apiStream.send(wsSend);
    wsSend = '';
  }
  const STREAM_STATUS: Record<string, string> = { idle: 'Not connected', connecting: 'Connecting…', open: 'Connected', closed: 'Disconnected', error: 'Connection error' };

  let tab: Tab = $state('body');
  const tests = $derived(apiClient.testResults);
  const scriptLogs = $derived(apiClient.scriptLogs);
  const hasTests = $derived(tests.length > 0 || scriptLogs.length > 0);
  const testsPassed = $derived(tests.filter((t) => t.passed).length);
  const cookies = $derived(resp ? parseSetCookies(resp.headers) : []);
  const tabs = $derived.by(() => {
    const out: { id: Tab; label: string; count?: string }[] = [{ id: 'body', label: 'Body' }];
    if (!resp) return out;
    out.push({ id: 'headers', label: 'Headers', count: String(resp.headers.length) });
    if (cookies.length) out.push({ id: 'cookies', label: 'Cookies', count: String(cookies.length) });
    if (resp.trace?.length) out.push({ id: 'trace', label: 'Timeline' });
    if (hasTests) out.push({ id: 'tests', label: 'Tests', count: tests.length ? `${testsPassed}/${tests.length}` : undefined });
    return out;
  });
  $effect(() => {
    if (!tabs.some((t) => t.id === tab)) tab = 'body';
  });

  const previewUrl = $derived(resp && isImage && resp.body_base64 ? `data:${resp.content_type};base64,${resp.body_base64}` : '');

  // ── Actions ─────────────────────────────────────────────────────────────────
  function extForCt(ct: string | null): string {
    if (!ct) return 'bin';
    const c = ct.toLowerCase();
    for (const [needle, ext] of [['json', 'json'], ['html', 'html'], ['xml', 'xml'], ['png', 'png'], ['jpeg', 'jpg'], ['jpg', 'jpg'], ['gif', 'gif'], ['webp', 'webp'], ['svg', 'svg'], ['pdf', 'pdf'], ['csv', 'csv'], ['javascript', 'js'], ['zip', 'zip'], ['octet-stream', 'bin'], ['text/', 'txt']] as const) {
      if (c.includes(needle)) return ext;
    }
    return 'bin';
  }
  function fileName(): string {
    const cd = resp?.headers.find((h) => h.key.toLowerCase() === 'content-disposition')?.value ?? '';
    const m = /filename\*?=(?:UTF-8'')?["']?([^"';\n]+)/i.exec(cd);
    if (m) { try { return decodeURIComponent(m[1].trim()); } catch { return m[1].trim(); } }
    return `response.${extForCt(resp?.content_type ?? null)}`;
  }
  function base64ToBlob(b64: string, ct: string): Blob {
    const bin = atob(b64);
    const arr = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) arr[i] = bin.charCodeAt(i);
    return new Blob([arr], { type: ct || 'application/octet-stream' });
  }
  async function saveToDisk(): Promise<void> {
    if (!resp) return;
    const r = resp;
    let url = '';
    try {
      if (r.body_id && ws.currentId) {
        // Truncated / non-UTF-8 body: the full bytes wait in the daemon's
        // short-lived response cache (never inlined as base64 any more).
        url = await authedBlobUrl(`/workspaces/${ws.currentId}/api-client/responses/${encodeURIComponent(r.body_id)}/raw`);
      } else {
        const blob = r.body_base64
          ? base64ToBlob(r.body_base64, r.content_type ?? '')
          : new Blob([r.body], { type: r.content_type ?? 'text/plain' });
        url = URL.createObjectURL(blob);
      }
      const a = document.createElement('a');
      a.href = url;
      a.download = fileName();
      document.body.appendChild(a);
      a.click();
      a.remove();
      toasts.success('Response downloaded', a.download);
    } catch (e) {
      if (r.body_id) toastError('Couldn’t save the response', e);
      else toasts.error('Couldn’t save the response', 'The file couldn’t be written.');
    } finally {
      const u = url;
      if (u) setTimeout(() => URL.revokeObjectURL(u), 1500);
    }
  }
  const canDownload = $derived(!!resp && (!!resp.body_id || !!resp.body_base64 || (!!resp.body && !resp.too_large)));

  async function copyBody(): Promise<void> {
    if (!resp) return;
    try {
      await copyTextOrThrow(bodyView === 'raw' ? resp.body : displayBody);
      toasts.success('Response body copied');
    } catch {
      toasts.error('Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.');
    }
  }

  /** Append this response to the request's Docs as an example (a draft edit;
   *  it is kept once the request is saved). */
  function addAsExample(): void {
    if (!resp) return;
    const MAX = 20_000;
    const body = prettyBody.length > MAX ? `${prettyBody.slice(0, MAX)}\n… (truncated)` : prettyBody;
    const lang = respExt(resp.content_type, resp.body);
    const block = `\n\n## Example response\n\n\`${resp.status}${resp.status_text ? ` ${resp.status_text}` : ''}\`${resp.content_type ? ` · ${resp.content_type}` : ''}\n\n\`\`\`${lang === 'txt' ? '' : lang}\n${body}\n\`\`\`\n`;
    const d = apiClient.draft;
    apiClient.draft = { ...d, docs: `${(d.docs ?? '').trimEnd()}${block}`.trimStart() };
    toasts.success('Added to the request’s Docs', 'Save the request (⌘S) to keep it.');
  }

  function moreMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'Add to Docs as example', icon: 'note', action: addAsExample, disabled: !resp || resp.too_large || isImage },
      { label: 'Download response…', icon: 'download', action: () => void saveToDisk(), disabled: !canDownload },
    ];
    if (ws.current) items.push({ separator: true }, { label: 'Send to an agent…', icon: 'send', action: () => (sendToAgentOpen = true) });
    ctxMenu.show(e, items);
  }

  // ── Failure hints ───────────────────────────────────────────────────────────
  const blockedPrivate = $derived(!!failure && /ssrf|blocked address|private|loopback/i.test(failure));
  const timedOut = $derived(!!failure && /timed? ?out/i.test(failure));
  const failureTitle = $derived(
    blockedPrivate ? 'Blocked: this address is on a private network'
      : timedOut ? 'The request timed out'
      : failure && /script/i.test(failure) ? 'A script failed, so nothing was sent'
      : 'The request didn’t get a response',
  );
  async function allowPrivate(): Promise<void> {
    if (!(await confirmer.ask(
      'API requests in this workspace will be able to reach localhost and private-network addresses (10.x, 192.168.x, …). Use this for local development servers. Everyone in the workspace is affected; you can block them again from a request’s Settings tab.',
      { title: 'Allow private addresses?', confirmLabel: 'Allow private addresses', danger: false },
    ))) return;
    try {
      await apiClient.setAllowLocal(true);
      toasts.success('Private addresses allowed', 'Send the request again.');
    } catch (e) {
      toasts.error('Couldn’t allow private addresses', e instanceof Error ? loadErrorText(e) : 'Only a workspace admin can change it.');
    }
  }
;
async () => {



 { svelteHTML.createElement("div", {  "class":`viewer`,});compact;
  if(isStream){
     { svelteHTML.createElement("div", { "class":`stream-console`,});
       { svelteHTML.createElement("div", { "class":`head`,});
         { svelteHTML.createElement("span", {   "class":`chip`,});apiStream.status === 'open';apiStream.status === 'error';STREAM_STATUS[apiStream.status] ?? apiStream.status; }
         { svelteHTML.createElement("span", { "class":`meta`,});plural(apiStream.items.length, 'message');apiStream.dropped ? ` · ${apiStream.dropped} older discarded` : ''; }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => apiStream.clear(),"disabled":apiStream.items.length === 0,});  }
       }
       { svelteHTML.createElement("div", { "class":`stream-log mono`,});
        if(apiStream.items.length === 0){
           { svelteHTML.createElement("p", { "class":`empty-line`,});streamKind === 'sse' ? 'Connect to start receiving server-sent events.' : 'Connect, then send and receive WebSocket messages here.'; }
        }else{
          if(apiStream.items.length > STREAM_RING_MAX){
             { svelteHTML.createElement("div", { "class":`ring-note`,});   STREAM_RING_MAX;  apiStream.items.length;  }
          }
           { const $$_tsiLlautriV3C = __sveltets_2_ensureComponent(VirtualList); const $$_tsiLlautriV3 = new $$_tsiLlautriV3C({ target: __sveltets_2_any(), props: {         "items":streamItems,"estimateHeight":28,"class":`stream-vlist`,"findText":(it) => `${it.kind === 'event' ? it.event || 'event' : it.kind === 'message' ? (it.dir === 'out' ? 'Sent' : 'Received') : it.kind}\n${it.data}`,row:(it) => { async ()/*Ωignore_positionΩ*/ => {
               { svelteHTML.createElement("div", { "class":`stream-item ${it.kind} ${it.dir ?? ''}`,});
                 { svelteHTML.createElement("span", { "class":`si-tag`,});
                  if(it.kind === 'event'){it.event || 'event';
                  } else if (it.kind === 'message'){it.dir === 'out' ? 'Sent' : 'Received';
                  }else{it.kind;}
                 }
                 { svelteHTML.createElement("span", { "class":`si-data`,});it.data; }
               }
            };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {row} = $$_tsiLlautriV3.$$prop_def;/*Ωignore_endΩ*/
            
           VirtualList}
        }
       }
      if(streamKind === 'websocket'){
         { svelteHTML.createElement("div", { "class":`ws-send`,});
           { svelteHTML.createElement("input", {              "dir":`ltr`,"class":`input mono grow`,"aria-label":`Message to send`,"placeholder":apiStream.status === 'open' ? '{"type":"ping"}' : 'Connect first',"bind:value":wsSend,"disabled":apiStream.status !== 'open',"onkeydown":(e) => { if (e.key === 'Enter') sendWs(); },});/*Ωignore_startΩ*/() => wsSend = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":sendWs,"disabled":apiStream.status !== 'open' || !wsSend.trim(),});  }
         }
      }
     }
  } else if (failure){
    if(fromHistory){;__sveltets_2_ensureSnippet(historyBanner(fromHistory));}
     {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: `${failureTitle}. ${failure}`, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`failure`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":16,}});}
       { svelteHTML.createElement("div", { "class":`f-body`,});
         { svelteHTML.createElement("div", { "class":`f-title`,});failureTitle; }
         { svelteHTML.createElement("p", { "class":`f-detail mono`,});failure; }
        if(blockedPrivate){
           { svelteHTML.createElement("p", { "class":`f-hint`,});                 }
           { svelteHTML.createElement("div", { "class":`f-actions`,}); { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":allowPrivate,});   } }
        } else if (timedOut){
           { svelteHTML.createElement("p", { "class":`f-hint`,});           }
          if(onsettings){ { svelteHTML.createElement("div", { "class":`f-actions`,}); { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":onsettings,});   } }}
        }else{
           { svelteHTML.createElement("p", { "class":`f-hint`,});                }
        }
       }
     }}
  } else if (!resp){
     { svelteHTML.createElement("div", { "class":`empty`,});
      if(apiClient.sending){
         {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),('Sending…')));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`empty-title`,});  }}
      }else{
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`send`,"size":compact ? 24 : 24,}});}
         { svelteHTML.createElement("p", { "class":`empty-title`,});        }
        if(!compact){ { svelteHTML.createElement("p", { "class":`empty-sub`,});  { svelteHTML.createElement("kbd", {});  } { svelteHTML.createElement("kbd", {});  }            }}
      }
     }
  }else{
    if(fromHistory){;__sveltets_2_ensureSnippet(historyBanner(fromHistory));}
     { svelteHTML.createElement("div", { "class":`head`,});
       { const $$_pihCsutatS2C = __sveltets_2_ensureComponent(StatusChip); new $$_pihCsutatS2C({ target: __sveltets_2_any(), props: {    "status":resp.status,"text":resp.status_text,}});}
       { svelteHTML.createElement("span", {   "class":`chip`,"title":`Total time`,});formatSeconds(resp.duration_ms / 1000); }
       { svelteHTML.createElement("span", {   "class":`chip`,"title":`Response size`,});formatBytes(resp.size_bytes); }
      if(resp.content_type){ { svelteHTML.createElement("span", {   "class":`meta mono ct`,"title":resp.content_type,});resp.content_type; }}
      if(apiClient.sending){ { svelteHTML.createElement("span", { "class":`meta`,});  }}
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":copyBody,"disabled":resp.too_large || isImage,"title":resp.too_large ? 'Too large to copy — download it from More response actions' : isImage ? 'Images can’t be copied as text — download it from More response actions' : 'Copy the body as shown',});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}
       }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":moreMenu,"aria-label":`More response actions`,"title":`More response actions`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
     }

    
     { const $$_sbaT1C = __sveltets_2_ensureComponent(Tabs); const $$_sbaT1 = new $$_sbaT1C({ target: __sveltets_2_any(), props: {        "label":`Response`,tabs,"value":tab,"onchange":(id) => (tab = id),"idBase":`api-resp`,trailing:() => { async ()/*Ωignore_positionΩ*/ => {
        if(tab === 'body' && resp && !resp.too_large && !isBinary){
           { svelteHTML.createElement("div", {     "class":`segmented view`,"role":`group`,"aria-label":`Body view`,});
            if(!isImage){
               { svelteHTML.createElement("button", {     "aria-pressed":bodyView === 'pretty',"onclick":() => (bodyView = 'pretty'),});bodyView === 'pretty';  }
               { svelteHTML.createElement("button", {     "aria-pressed":bodyView === 'raw',"onclick":() => (bodyView = 'raw'),});bodyView === 'raw';  }
            }
            if(parsed !== undefined){
               { svelteHTML.createElement("button", {     "aria-pressed":bodyView === 'tree',"onclick":() => (bodyView = 'tree'),});bodyView === 'tree';  }
            }
            if(isHtml || isImage){
               { svelteHTML.createElement("button", {     "aria-pressed":bodyView === 'preview',"onclick":() => (bodyView = 'preview'),});bodyView === 'preview';  }
            }
           }
        }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {trailing} = $$_sbaT1.$$prop_def;/*Ωignore_endΩ*/
      
     Tabs}

     { svelteHTML.createElement("div", {       "class":`rbody`,"role":`tabpanel`,"id":`api-resp-panel-${tab}`,"aria-labelledby":`api-resp-tab-${tab}`,});
      if(tab === 'body'){
        if(resp.too_large){
           { svelteHTML.createElement("div", { "class":`notice`,});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`box`,"size":16,}});}
             { svelteHTML.createElement("div", {});
               { svelteHTML.createElement("div", { "class":`n-title`,});   formatBytes(resp.size_bytes);     }
               { svelteHTML.createElement("div", { "class":`n-sub`,});            }
             }
           }
        } else if (isBinary){
           { svelteHTML.createElement("div", { "class":`notice`,});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`box`,"size":16,}});}
             { svelteHTML.createElement("div", { "class":`n-body`,});
               { svelteHTML.createElement("div", { "class":`n-title`,});   formatBytes(resp.size_bytes); }
               { svelteHTML.createElement("div", { "class":`n-sub`,});
                isImage ? 'This image is too large to preview here' : `${mimeOf(resp.content_type) || 'Unknown content type'} can’t be shown as text`;canDownload ? '. Download it to open it in another app.' : '. Send the request again to download it.';
               }
              if(canDownload){
                 { svelteHTML.createElement("button", {   "class":`btn small n-action`,"onclick":() => void saveToDisk(),});
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} 
                 }
              }
             }
           }
        } else if (bodyView === 'preview' && isImage && previewUrl){
           { svelteHTML.createElement("div", { "class":`img-wrap`,}); { svelteHTML.createElement("img", {      "class":`img-preview`,"src":previewUrl,"alt":`Response preview`,});} }
        } else if (bodyView === 'preview' && isHtml){
          
           { svelteHTML.createElement("iframe", {     "class":`html-preview`,"title":`HTML preview of the response`,"sandbox":"","srcdoc":resp.body,}); }
        } else if (bodyEmpty){
           { svelteHTML.createElement("p", { "class":`empty-line`,});     }
        }else{
          if(resp.truncated){
             { svelteHTML.createElement("div", { "class":`notice warn`,});
               { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":14,}});}
               { svelteHTML.createElement("span", {});      formatBytes(resp.size_bytes);           }
             }
          }
          if(parsed !== undefined && (bodyView === 'pretty' || bodyView === 'tree')){
             { svelteHTML.createElement("label", { "class":`filter`,});
               { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
               { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`mono`,"placeholder":bodyView === 'tree' ? 'Find a key or value…' : '$.data[0].id',"aria-label":bodyView === 'tree' ? 'Find in the response' : 'JSONPath filter',"bind:value":filter,"spellcheck":`false`,});/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
              if(filter){ { svelteHTML.createElement("button", {       "class":`icon-btn clear`,"onclick":() => (filter = ''),"aria-label":`Clear`,"title":`Clear`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }}
             }
          }
          if(bodyView === 'tree' && parsed !== undefined){
             { svelteHTML.createElement("div", { "class":`tree-wrap`,}); { const $$_eerTnosJ3C = __sveltets_2_ensureComponent(JsonTree); new $$_eerTnosJ3C({ target: __sveltets_2_any(), props: {    "value":parsed,"query":filterDebounced,}});} }
          }
        }
      } else if (tab === 'headers'){
         { svelteHTML.createElement("table", { "class":`htable`,});
           { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } } }
           { svelteHTML.createElement("tbody", {});
                for(let h of __sveltets_2_ensureArray(resp.headers)){let i = 1;i;
               { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("td", { "class":`hkey mono`,});h.key; } { svelteHTML.createElement("td", { "class":`hval mono`,});h.value; } }
            }
           }
         }
      } else if (tab === 'cookies'){
         { svelteHTML.createElement("p", { "class":`tab-lead`,});                      }
         { svelteHTML.createElement("table", { "class":`htable`,});
           { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } } }
           { svelteHTML.createElement("tbody", {});
                for(let c of __sveltets_2_ensureArray(cookies)){let i = 1;i;
               { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("td", { "class":`hkey mono`,});c.name; } { svelteHTML.createElement("td", { "class":`hval mono`,});c.value; } { svelteHTML.createElement("td", { "class":`hval dim`,});c.attributes || '—'; } }
            }
           }
         }
      } else if (tab === 'trace'){
         { svelteHTML.createElement("ol", { "class":`trace`,});
              for(let step of __sveltets_2_ensureArray(resp.trace)){let i = 1;i;
             { svelteHTML.createElement("li", { "class":`trace-step ${step.level}`,});
               { svelteHTML.createElement("span", {   "class":`trace-dot`,"aria-hidden":`true`,}); }
               { svelteHTML.createElement("span", { "class":`trace-label`,});step.label; }
               { svelteHTML.createElement("span", { "class":`trace-detail mono`,});step.detail; }
              if(step.ms != null){ { svelteHTML.createElement("span", { "class":`trace-ms`,});step.ms;  }}
             }
          }
         }
      }else{
         { svelteHTML.createElement("div", { "class":`tests-pane`,});
          if(tests.length === 0){
             { svelteHTML.createElement("p", { "class":`empty-line`,});     { svelteHTML.createElement("code", {});  }       }
          }else{
             { svelteHTML.createElement("ul", { "class":`test-list`,});
                  for(let t of __sveltets_2_ensureArray(tests)){let i = 1;i;
                 { svelteHTML.createElement("li", { "class":`test-item ${t.passed ? 'pass' : 'fail'}`,});
                   { svelteHTML.createElement("span", { "class":`test-word`,});t.passed ? 'Passed' : 'Failed'; }
                   { svelteHTML.createElement("span", { "class":`test-name`,});t.name; }
                  if(!t.passed && t.error){ { svelteHTML.createElement("span", { "class":`test-err mono`,});t.error; }}
                 }
              }
             }
          }
          if(scriptLogs.length > 0){
             { svelteHTML.createElement("div", { "class":`section-title`,});  }
             { svelteHTML.createElement("pre", { "class":`console-log mono`,});scriptLogs.join('\n'); }
          }
         }
      }
      
      if(editorEligible){
         { svelteHTML.createElement("div", {  "class":`resp-editor`,});__sveltets_2_ensureType(String, Number, showEditor ? null : 'none');
           { const $$_rotidEedoC3C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC3C({ target: __sveltets_2_any(), props: {              "lsp":false,"wrap":respWrap,"path":respPath,"content":displayBody,"root":ws.current?.root_path ?? '',"language":respLang,"readOnly":true,}});}
         }
      }
     }
  }
 }

if(sendToAgentOpen && resp && ws.current){
   { const $$_golaiDtekcaPtxetnoC0C = __sveltets_2_ensureComponent(ContextPacketDialog); new $$_golaiDtekcaPtxetnoC0C({ target: __sveltets_2_any(), props: {           "workspaceId":ws.current.id,"sessionId":ws.activeSessionId,"kind":`api`,"payload":{ status: resp.status, content_type: resp.content_type, body: resp.body },"onclose":() => (sendToAgentOpen = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ResponseViewer__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ResponseViewer__SvelteComponent_ = ReturnType<typeof ResponseViewer__SvelteComponent_>;
/*Ωignore_endΩ*/export default ResponseViewer__SvelteComponent_;