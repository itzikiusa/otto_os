<script lang="ts">
  // Response viewer shared by the page and the compact panel: status / time /
  // size chips, then Body (Pretty · Raw · Tree · Preview) · Headers · Cookies ·
  // Timeline · Tests. A failed send shows inline with the reason and what to
  // do; SSE / WebSocket requests get a live message console instead.
  import Icon from '../../lib/components/Icon.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import ContextPacketDialog from '../../lib/components/ContextPacketDialog.svelte';
  import StatusChip from './StatusChip.svelte';
  import JsonTree from './JsonTree.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { apiStream } from '../../lib/stores/apiStream.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { formatBytes, formatSeconds } from '../../lib/metric-format';
  import { parseSetCookies } from '../../lib/api/apiVars';

  interface Props {
    compact?: boolean;
    /** Open the request's Settings tab (from the error hints). */
    onsettings?: () => void;
  }
  let { compact = false, onsettings }: Props = $props();

  let sendToAgentOpen = $state(false);
  const resp = $derived(apiClient.lastResponse);
  const failure = $derived(apiClient.lastError);

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

  type BodyView = 'pretty' | 'raw' | 'tree' | 'preview';
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

  // ── Tabs ────────────────────────────────────────────────────────────────────
  type Tab = 'body' | 'headers' | 'cookies' | 'trace' | 'tests';
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
  function onTabKey(e: KeyboardEvent, i: number): void {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
    e.preventDefault();
    const next = tabs[(i + (e.key === 'ArrowRight' ? 1 : tabs.length - 1)) % tabs.length];
    tab = next.id;
    (e.currentTarget as HTMLElement).parentElement?.querySelectorAll<HTMLElement>('[role=tab]')[tabs.indexOf(next)]?.focus();
  }

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
  function saveToDisk(): void {
    if (!resp) return;
    try {
      const blob = resp.body_base64
        ? base64ToBlob(resp.body_base64, resp.content_type ?? '')
        : new Blob([resp.body], { type: resp.content_type ?? 'text/plain' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = fileName();
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(url), 1500);
      toasts.success('Response downloaded', a.download);
    } catch {
      toasts.error('Couldn’t save the response', 'The file could not be written.');
    }
  }
  const canDownload = $derived(!!resp && (!!resp.body_base64 || (!!resp.body && !resp.too_large)));

  async function copyBody(): Promise<void> {
    if (!resp) return;
    try {
      await copyTextOrThrow(bodyView === 'raw' ? resp.body : displayBody);
      toasts.success('Response body copied');
    } catch {
      toasts.error('Couldn’t copy', 'The clipboard isn’t available.');
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
      { label: 'Download response…', icon: 'download', action: saveToDisk, disabled: !canDownload },
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
      await ws.setApiAllowLocal(true);
      toasts.success('Private addresses allowed', 'Send the request again.');
    } catch (e) {
      toasts.error('Couldn’t change the setting', e instanceof Error ? e.message : 'Only a workspace admin can change it.');
    }
  }
</script>

<div class="viewer" class:compact>
  {#if isStream}
    <div class="stream-console">
      <div class="head">
        <span class="chip" class:ok={apiStream.status === 'open'} class:bad={apiStream.status === 'error'}>{STREAM_STATUS[apiStream.status] ?? apiStream.status}</span>
        <span class="meta">{apiStream.items.length} message{apiStream.items.length === 1 ? '' : 's'}{apiStream.dropped ? ` · ${apiStream.dropped} older discarded` : ''}</span>
        <span class="grow"></span>
        <button class="btn small ghost" onclick={() => apiStream.clear()} disabled={apiStream.items.length === 0}>Clear</button>
      </div>
      <div class="stream-log mono">
        {#if apiStream.items.length === 0}
          <p class="empty-line">{streamKind === 'sse' ? 'Connect to start receiving server-sent events.' : 'Connect, then send and receive WebSocket messages here.'}</p>
        {:else}
          {#if apiStream.items.length > STREAM_RING_MAX}
            <div class="ring-note">Showing the last {STREAM_RING_MAX} of {apiStream.items.length} messages.</div>
          {/if}
          <VirtualList items={streamItems} estimateHeight={28} class="stream-vlist">
            {#snippet row(it)}
              <div class="stream-item {it.kind} {it.dir ?? ''}">
                <span class="si-tag">
                  {#if it.kind === 'event'}{it.event || 'event'}
                  {:else if it.kind === 'message'}{it.dir === 'out' ? 'Sent' : 'Received'}
                  {:else}{it.kind}{/if}
                </span>
                <span class="si-data">{it.data}</span>
              </div>
            {/snippet}
          </VirtualList>
        {/if}
      </div>
      {#if streamKind === 'websocket'}
        <div class="ws-send">
          <input class="input mono grow" aria-label="Message to send" placeholder={apiStream.status === 'open' ? 'Message to send' : 'Connect first'}
            bind:value={wsSend} disabled={apiStream.status !== 'open'} onkeydown={(e) => { if (e.key === 'Enter') sendWs(); }} />
          <button class="btn small primary" onclick={sendWs} disabled={apiStream.status !== 'open' || !wsSend.trim()}>Send message</button>
        </div>
      {/if}
    </div>
  {:else if failure}
    <div class="failure" role="alert">
      <Icon name="warning" size={16} />
      <div class="f-body">
        <div class="f-title">{failureTitle}</div>
        <p class="f-detail mono">{failure}</p>
        {#if blockedPrivate}
          <p class="f-hint">Otto blocks localhost and private-network addresses by default, so a request can’t reach internal services by accident.</p>
          <div class="f-actions"><button class="btn small" onclick={allowPrivate}>Allow private addresses…</button></div>
        {:else if timedOut}
          <p class="f-hint">Requests give up after 60 seconds unless you set a timeout.</p>
          {#if onsettings}<div class="f-actions"><button class="btn small" onclick={onsettings}>Open Settings</button></div>{/if}
        {:else}
          <p class="f-hint">Check the URL and the active environment, then send again. The request is recorded in History.</p>
        {/if}
      </div>
    </div>
  {:else if !resp}
    <div class="empty">
      {#if apiClient.sending}
        <p class="empty-title" role="status">Sending…</p>
      {:else}
        <Icon name="send" size={compact ? 20 : 24} />
        <p class="empty-title">Send the request to see the response here</p>
        {#if !compact}
          <p class="empty-sub">Press <kbd>⌘</kbd><kbd>↵</kbd> or click Send. You’ll see the status, timing, headers and body.</p>
        {/if}
      {/if}
    </div>
  {:else}
    <div class="head">
      <StatusChip status={resp.status} text={resp.status_text} />
      <span class="chip" title="Total time">{formatSeconds(resp.duration_ms / 1000)}</span>
      <span class="chip" title="Response size">{formatBytes(resp.size_bytes)}</span>
      {#if resp.content_type}<span class="meta mono ct" title={resp.content_type}>{resp.content_type}</span>{/if}
      {#if apiClient.sending}<span class="meta" role="status">Sending again…</span>{/if}
      <span class="grow"></span>
      <button class="btn small ghost" onclick={copyBody} disabled={resp.too_large || isImage} title={resp.too_large ? 'Too large to copy — download it from More response actions' : isImage ? 'Images can’t be copied as text — download it from More response actions' : 'Copy the body as shown'}>
        <Icon name="copy" size={12} />Copy
      </button>
      <button class="icon-btn" onclick={moreMenu} aria-label="More response actions" title="More response actions"><Icon name="more" size={14} /></button>
    </div>

    <div class="rtabs" role="tablist" aria-label="Response">
      {#each tabs as t, i (t.id)}
        <button class="rtab" class:active={tab === t.id} role="tab" aria-selected={tab === t.id} tabindex={tab === t.id ? 0 : -1}
          onclick={() => (tab = t.id)} onkeydown={(e) => onTabKey(e, i)}>
          {t.label}{#if t.count}<span class="count" aria-hidden="true">{t.count}</span>{/if}
        </button>
      {/each}
      {#if tab === 'body' && !resp.too_large}
        <span class="grow"></span>
        <div class="segmented view" role="group" aria-label="Body view">
          {#if !isImage}
            <button class:active={bodyView === 'pretty'} aria-pressed={bodyView === 'pretty'} onclick={() => (bodyView = 'pretty')}>Pretty</button>
            <button class:active={bodyView === 'raw'} aria-pressed={bodyView === 'raw'} onclick={() => (bodyView = 'raw')}>Raw</button>
          {/if}
          {#if parsed !== undefined}
            <button class:active={bodyView === 'tree'} aria-pressed={bodyView === 'tree'} onclick={() => (bodyView = 'tree')}>Tree</button>
          {/if}
          {#if isHtml || isImage}
            <button class:active={bodyView === 'preview'} aria-pressed={bodyView === 'preview'} onclick={() => (bodyView = 'preview')}>Preview</button>
          {/if}
        </div>
      {/if}
    </div>

    <div class="rbody">
      {#if tab === 'body'}
        {#if resp.too_large}
          <div class="notice">
            <Icon name="box" size={16} />
            <div>
              <div class="n-title">This response is {formatBytes(resp.size_bytes)}, too large to show</div>
              <div class="n-sub">Bodies over 25 MB aren’t loaded. Try a smaller page of data.</div>
            </div>
          </div>
        {:else if bodyView === 'preview' && isImage && previewUrl}
          <div class="img-wrap"><img class="img-preview" src={previewUrl} alt="Response preview" /></div>
        {:else if bodyView === 'preview' && isHtml}
          <!-- sandbox="" : no scripts, forms, or same-origin access -->
          <iframe class="html-preview" title="HTML preview of the response" sandbox="" srcdoc={resp.body}></iframe>
        {:else if resp.body.trim() === ''}
          <p class="empty-line">The response has no body.</p>
        {:else}
          {#if resp.truncated}
            <div class="notice warn">
              <Icon name="info" size={14} />
              <span>Showing the first 512 KB of {formatBytes(resp.size_bytes)}. Download the response from ⋯ to get all of it.</span>
            </div>
          {/if}
          {#if parsed !== undefined && (bodyView === 'pretty' || bodyView === 'tree')}
            <label class="filter">
              <Icon name="search" size={12} />
              <input
                class="mono"
                placeholder={bodyView === 'tree' ? 'Find a key or value' : 'Filter with a JSONPath, e.g. $.data[0].id'}
                aria-label={bodyView === 'tree' ? 'Find in the response' : 'JSONPath filter'}
                bind:value={filter}
                spellcheck="false"
              />
              {#if filter}<button class="icon-btn clear" onclick={() => (filter = '')} aria-label="Clear" title="Clear"><Icon name="x" size={12} /></button>{/if}
            </label>
          {/if}
          {#if bodyView === 'tree' && parsed !== undefined}
            <div class="tree-wrap"><JsonTree value={parsed} query={filterDebounced} /></div>
          {:else}
            <div class="resp-editor">
              <CodeEditor path={respPath} content={displayBody} root={ws.current?.root_path ?? ''} language={respLang} readOnly={true} />
            </div>
          {/if}
        {/if}
      {:else if tab === 'headers'}
        <table class="htable">
          <thead><tr><th>Name</th><th>Value</th></tr></thead>
          <tbody>
            {#each resp.headers as h, i (i)}
              <tr><td class="hkey mono">{h.key}</td><td class="hval mono">{h.value}</td></tr>
            {/each}
          </tbody>
        </table>
      {:else if tab === 'cookies'}
        <p class="tab-lead">Cookies this response set. Otto keeps them in the workspace’s cookie jar and sends them on later requests to the same site.</p>
        <table class="htable">
          <thead><tr><th>Name</th><th>Value</th><th>Attributes</th></tr></thead>
          <tbody>
            {#each cookies as c, i (i)}
              <tr><td class="hkey mono">{c.name}</td><td class="hval mono">{c.value}</td><td class="hval dim">{c.attributes || '—'}</td></tr>
            {/each}
          </tbody>
        </table>
      {:else if tab === 'trace'}
        <ol class="trace">
          {#each resp.trace as step, i (i)}
            <li class="trace-step {step.level}">
              <span class="trace-dot" aria-hidden="true"></span>
              <span class="trace-label">{step.label}</span>
              <span class="trace-detail mono">{step.detail}</span>
              {#if step.ms != null}<span class="trace-ms">{step.ms} ms</span>{/if}
            </li>
          {/each}
        </ol>
      {:else}
        <div class="tests-pane">
          {#if tests.length === 0}
            <p class="empty-line">No tests ran. Add <code>pm.test(…)</code> calls to the request’s post-response script.</p>
          {:else}
            <ul class="test-list">
              {#each tests as t, i (i)}
                <li class="test-item {t.passed ? 'pass' : 'fail'}">
                  <span class="test-word">{t.passed ? 'Passed' : 'Failed'}</span>
                  <span class="test-name">{t.name}</span>
                  {#if !t.passed && t.error}<span class="test-err mono">{t.error}</span>{/if}
                </li>
              {/each}
            </ul>
          {/if}
          {#if scriptLogs.length > 0}
            <div class="section-title">Console</div>
            <pre class="console-log mono">{scriptLogs.join('\n')}</pre>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

{#if sendToAgentOpen && resp && ws.current}
  <ContextPacketDialog
    workspaceId={ws.current.id}
    sessionId={ws.activeSessionId}
    kind="api"
    payload={{ status: resp.status, content_type: resp.content_type, body: resp.body }}
    onclose={() => (sendToAgentOpen = false)}
  />
{/if}

<style>
  .viewer {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 6px;
    color: var(--text-dim);
    text-align: center;
    padding: 16px;
  }
  .empty-title {
    margin: 0;
    font-size: var(--fs-m);
    color: var(--text);
  }
  .empty-sub {
    margin: 0;
    font-size: var(--fs-s);
  }
  kbd {
    font-family: var(--font-ui);
    font-size: var(--fs-xs);
    padding: 1px 5px;
    border: 1px solid var(--border);
    border-bottom-width: 2px;
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
  }
  .empty-line {
    margin: 0;
    padding: 8px 2px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .failure {
    display: flex;
    gap: 10px;
    padding: 14px;
    margin-top: 4px;
    border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
    border-radius: var(--radius-m);
    background: var(--danger-soft);
  }
  .failure > :global(svg) {
    color: var(--danger);
    flex-shrink: 0;
    margin-top: 1px;
  }
  .f-body {
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .f-title {
    font-weight: 600;
    color: var(--text);
  }
  .f-detail {
    margin: 0;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .f-hint {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .f-actions {
    display: flex;
    gap: 6px;
  }
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 0 8px;
    flex-wrap: wrap;
  }
  .chip.ok {
    color: var(--success);
  }
  .meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  @media (max-width: 640px) {
    .ct {
      display: none;
    }
  }
  .ct {
    max-width: 240px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rtabs {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 2px;
    border-bottom: 1px solid var(--border);
    min-width: 0;
  }
  .rtab {
    height: 28px;
    padding: 0 10px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-s);
    font-weight: 500;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
    white-space: nowrap;
  }
  .rtab:hover {
    color: var(--text);
  }
  .rtab.active {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .count {
    margin-inline-start: 5px;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .segmented.view {
    align-self: center;
    margin-bottom: 3px;
  }
  .rbody {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding-top: 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .filter {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 27px;
    padding: 0 4px 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .filter:focus-within {
    border-color: var(--accent);
  }
  .filter input {
    flex: 1;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    outline: none;
  }
  .clear {
    width: 20px;
    height: 20px;
  }
  .resp-editor,
  .tree-wrap {
    flex: 1;
    min-height: 160px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: auto;
  }
  .resp-editor {
    overflow: hidden;
  }
  .html-preview {
    flex: 1;
    min-height: 240px;
    width: 100%;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
  }
  .htable {
    width: 100%;
    border-collapse: collapse;
    user-select: text;
    font-size: var(--fs-s);
  }
  .htable th {
    position: sticky;
    top: 0;
    text-align: start;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    background: var(--bg);
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
  }
  .htable td {
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    vertical-align: top;
    overflow-wrap: anywhere;
  }
  .hkey {
    width: 32%;
    color: var(--text);
    font-weight: 500;
  }
  .hval {
    color: var(--text);
  }
  .hval.dim {
    color: var(--text-dim);
  }
  .tab-lead {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .notice {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    padding: 10px 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--surface-2);
    font-size: var(--fs-s);
    color: var(--text);
  }
  .notice.warn {
    background: var(--warning-soft);
    border-color: color-mix(in srgb, var(--warning) 30%, transparent);
  }
  .n-title {
    font-weight: 600;
  }
  .n-sub {
    color: var(--text-dim);
  }
  .img-wrap {
    padding: 4px 0;
  }
  .img-preview {
    max-width: 100%;
    max-height: 420px;
    object-fit: contain;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: repeating-conic-gradient(var(--surface-2) 0% 25%, transparent 0% 50%) 50% / 18px 18px;
  }
  .trace {
    list-style: none;
    margin: 0;
    padding: 4px 0 4px 2px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .trace-step {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 8px;
    border-inline-start: 2px solid var(--border);
    margin-inline-start: 5px;
    font-size: var(--fs-s);
  }
  .trace-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-dim);
    margin-inline-start: -12px;
    flex-shrink: 0;
  }
  .trace-step.timing .trace-dot { background: var(--accent); }
  .trace-step.success .trace-dot { background: var(--success); }
  .trace-step.error .trace-dot { background: var(--danger); }
  .trace-step.redirect .trace-dot { background: var(--warning); }
  .trace-label {
    font-weight: 600;
    color: var(--text);
    min-width: 110px;
  }
  .trace-detail {
    flex: 1;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .trace-ms {
    font-variant-numeric: tabular-nums;
    color: var(--text);
    font-weight: 600;
    font-size: var(--fs-xs);
  }
  .tests-pane {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .tests-pane .section-title {
    margin: 4px 0 0;
  }
  .test-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .test-item {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
  }
  .test-item.pass { background: var(--success-soft); }
  .test-item.fail { background: var(--danger-soft); }
  .test-word { font-weight: 600; }
  .test-item.pass .test-word { color: var(--success); }
  .test-item.fail .test-word { color: var(--danger); }
  .test-err { color: var(--danger); }
  .console-log {
    margin: 0;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    padding: 8px;
    max-height: 160px;
    overflow: auto;
    white-space: pre-wrap;
    user-select: text;
  }
  code {
    font-family: var(--font-mono);
  }
  .stream-console {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .stream-log {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: 4px 0;
  }
  :global(.stream-vlist) {
    flex: 1;
    min-height: 0;
    height: 100%;
  }
  .ring-note {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    padding: 2px 8px 4px;
    flex-shrink: 0;
  }
  .stream-item {
    display: flex;
    gap: 8px;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    align-items: baseline;
  }
  .stream-item.event { background: var(--surface-2); }
  .stream-item.message.out { background: var(--accent-soft); }
  .stream-item.message.in { background: var(--surface-2); }
  .stream-item.error { background: var(--danger-soft); }
  .stream-item.open,
  .stream-item.closed { color: var(--text-dim); font-style: italic; }
  .si-tag {
    flex: 0 0 auto;
    min-width: 64px;
    font-weight: 600;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .stream-item.error .si-tag { color: var(--danger); }
  .si-data {
    flex: 1;
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
    color: var(--text);
  }
  .ws-send {
    display: flex;
    gap: 6px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
  }
</style>
