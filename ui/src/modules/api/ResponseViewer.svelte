<script lang="ts">
  // Shared response viewer: status pill (colored by class) + duration + size +
  // content-type, then Body / Headers tabs. Pretty-prints JSON bodies.
  import Icon from '../../lib/components/Icon.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';

  interface Props {
    compact?: boolean;
  }
  let { compact = false }: Props = $props();

  import { toasts } from '../../lib/toast.svelte';
  import { apiStream } from '../../lib/stores/apiStream.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import VirtualList from '../../lib/components/VirtualList.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import ContextPacketDialog from '../../lib/components/ContextPacketDialog.svelte';

  // ── Send-to-agent dialog ────────────────────────────────────────────────────
  let sendToAgentOpen = $state(false);

  const resp = $derived(apiClient.lastResponse);

  // ── Pretty-print: memoized + size-gated ────────────────────────────────────
  // Bodies over 256 KB are shown raw (re-parsing would block the main thread).
  const PRETTY_SIZE_LIMIT = 256 * 1024;
  const STREAM_RING_MAX = 500;

  // JSONPath-ish filter ($.a.b[0].c) applied to JSON bodies.
  let jsonFilter = $state('');
  // Debounced version applied to $derived so the parser doesn't run every keystroke.
  let jsonFilterDebounced = $state('');
  let _filterTimer: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    const v = jsonFilter;
    clearTimeout(_filterTimer);
    _filterTimer = setTimeout(() => { jsonFilterDebounced = v; }, 150);
  });

  const isJsonResp = $derived(!!resp && isJson(resp.content_type, resp.body));
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
  const respPath = $derived(resp ? `response.${respExt(resp.content_type, resp.body)}` : 'response.txt');
  const respLang = $derived(resp ? respExt(resp.content_type, resp.body) : '');
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

  // Memoize the parsed JSON object so JSONPath queries don't re-parse each time.
  // Key is the body string; cleared when the response changes.
  let _parsedCache: { body: string; parsed: unknown } | null = null;
  function getCachedParsed(body: string): unknown {
    if (_parsedCache?.body !== body) {
      try { _parsedCache = { body, parsed: JSON.parse(body) }; }
      catch { _parsedCache = null; }
    }
    return _parsedCache?.parsed;
  }

  const prettyBody = $derived.by(() => {
    if (!resp) return '';
    if (resp.body.length > PRETTY_SIZE_LIMIT) return resp.body; // size-gate
    if (isJson(resp.content_type, resp.body)) {
      const parsed = getCachedParsed(resp.body);
      if (parsed !== undefined) {
        try { return JSON.stringify(parsed, null, 2); } catch { /* fall through */ }
      }
    }
    return resp.body;
  });

  const displayBody = $derived.by(() => {
    if (!resp) return '';
    const base = bodyView === 'pretty' ? prettyBody : resp.body;
    const f = jsonFilterDebounced.trim(); // use debounced value
    if (f && isJsonResp && resp.body.length <= PRETTY_SIZE_LIMIT) {
      try {
        const parsed = getCachedParsed(resp.body);
        const result = evalJsonPath(parsed, f);
        return result === undefined ? `// no match for ${f}` : JSON.stringify(result, null, 2);
      } catch {
        return base;
      }
    }
    return base;
  });

  // ── Capped stream ring-buffer ───────────────────────────────────────────────
  // Cap visible stream items so the DOM never grows unbounded (T2).
  const streamItems = $derived(
    apiStream.items.length > STREAM_RING_MAX
      ? apiStream.items.slice(apiStream.items.length - STREAM_RING_MAX)
      : apiStream.items,
  );

  const streamKind = $derived(apiClient.draft.kind);
  const isStream = $derived(streamKind === 'sse' || streamKind === 'websocket');
  let wsSend = $state('');
  function sendWs(): void {
    if (!wsSend.trim()) return;
    apiStream.send(wsSend);
    wsSend = '';
  }

  type Tab = 'body' | 'headers' | 'trace' | 'tests';
  let tab: Tab = $state('body');
  const tests = $derived(apiClient.testResults);
  const scriptLogs = $derived(apiClient.scriptLogs);
  const hasTests = $derived(tests.length > 0 || scriptLogs.length > 0);
  const testsPassed = $derived(tests.filter((t) => t.passed).length);
  type BodyView = 'pretty' | 'raw';
  let bodyView: BodyView = $state('pretty');

  const isImage = $derived(!!resp?.content_type && /^image\//i.test(resp.content_type));
  const previewUrl = $derived(
    resp && isImage && resp.body_base64
      ? `data:${resp.content_type};base64,${resp.body_base64}`
      : '',
  );

  // ── Save response to disk ──────────────────────────────────────────────────
  function extForCt(ct: string | null): string {
    if (!ct) return 'bin';
    const c = ct.toLowerCase();
    if (c.includes('json')) return 'json';
    if (c.includes('html')) return 'html';
    if (c.includes('xml')) return 'xml';
    if (c.includes('png')) return 'png';
    if (c.includes('jpeg') || c.includes('jpg')) return 'jpg';
    if (c.includes('gif')) return 'gif';
    if (c.includes('webp')) return 'webp';
    if (c.includes('svg')) return 'svg';
    if (c.includes('pdf')) return 'pdf';
    if (c.includes('csv')) return 'csv';
    if (c.includes('javascript')) return 'js';
    if (c.includes('zip')) return 'zip';
    if (c.includes('octet-stream')) return 'bin';
    if (c.includes('text/')) return 'txt';
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
      toasts.success('Saved response', a.download);
    } catch {
      toasts.error('Save failed', 'Could not write the response to disk.');
    }
  }
  const canSave = $derived(!!resp && (!!resp.body_base64 || (!!resp.body && !resp.too_large)));

  function statusClass(status: number): string {
    if (status >= 200 && status < 300) return 'ok';
    if (status >= 300 && status < 400) return 'redirect';
    if (status >= 400 && status < 500) return 'client';
    if (status >= 500) return 'server';
    return 'none';
  }

  function isJson(ct: string | null, body: string): boolean {
    if (ct && /\bjson\b/i.test(ct)) return true;
    const t = body.trim();
    return (t.startsWith('{') && t.endsWith('}')) || (t.startsWith('[') && t.endsWith(']'));
  }

  function fmtSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  }
</script>

<div class="viewer" class:compact>
  {#if isStream}
    <div class="stream-console">
      <div class="stream-head">
        <span class="status-pill {apiStream.status === 'open' ? 'ok' : apiStream.status === 'error' ? 'server' : 'none'}">{apiStream.status}</span>
        <span class="meta">{apiStream.items.length} message(s)</span>
        <span class="grow"></span>
        <button class="save-btn" onclick={() => apiStream.clear()} title="Clear log">Clear</button>
      </div>
      <div class="stream-log mono">
        {#if apiStream.items.length === 0}
          <div class="empty-mini">{streamKind === 'sse' ? 'Connect to start receiving events.' : 'Connect, then send messages.'}</div>
        {:else}
          {#if apiStream.items.length > STREAM_RING_MAX}
            <div class="ring-note">Showing last {STREAM_RING_MAX} of {apiStream.items.length} messages.</div>
          {/if}
          <VirtualList items={streamItems} estimateHeight={28} class="stream-vlist">
            {#snippet row(it)}
              <div class="stream-item {it.kind} {it.dir ?? ''}">
                <span class="si-tag">
                  {#if it.kind === 'event'}{it.event || 'event'}
                  {:else if it.kind === 'message'}{it.dir === 'out' ? '▲ sent' : '▼ recv'}
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
          <input
            class="input mono grow"
            placeholder={apiStream.status === 'open' ? 'Message to send…' : 'Connect first'}
            bind:value={wsSend}
            disabled={apiStream.status !== 'open'}
            onkeydown={(e) => { if (e.key === 'Enter') sendWs(); }}
          />
          <button class="btn small primary" onclick={sendWs} disabled={apiStream.status !== 'open' || !wsSend.trim()}>Send</button>
        </div>
      {/if}
    </div>
  {:else if !resp}
    <div class="empty">
      <Icon name="send" size={compact ? 20 : 26} />
      <span>Send a request to see the response.</span>
    </div>
  {:else}
    <div class="resp-head">
      <span class="status-pill {statusClass(resp.status)}">
        {resp.status}{#if resp.status_text}&nbsp;{resp.status_text}{/if}
      </span>
      <span class="meta"><Icon name="clock" size={11} />{resp.duration_ms} ms</span>
      <span class="meta"><Icon name="box" size={11} />{fmtSize(resp.size_bytes)}</span>
      {#if resp.content_type}
        <span class="meta mono ellipsis ct" title={resp.content_type}>{resp.content_type}</span>
      {/if}
      <span class="grow"></span>
      {#if canSave}
        <button class="save-btn" onclick={saveToDisk} title="Save response to disk">
          <Icon name="check" size={11} />Save
        </button>
      {/if}
      {#if resp && ws.current}
        <button class="save-btn" onclick={() => (sendToAgentOpen = true)} title="Send response to a running agent">
          <Icon name="send" size={11} />To agent
        </button>
      {/if}
    </div>

    <div class="rtabs" role="tablist">
      <button class="rtab" class:active={tab === 'body'} role="tab" aria-selected={tab === 'body'} onclick={() => (tab = 'body')}>Body</button>
      <button class="rtab" class:active={tab === 'headers'} role="tab" aria-selected={tab === 'headers'} onclick={() => (tab = 'headers')}>
        Headers <span class="hcount">{resp.headers.length}</span>
      </button>
      {#if resp.trace && resp.trace.length > 0}
        <button class="rtab" class:active={tab === 'trace'} role="tab" aria-selected={tab === 'trace'} onclick={() => (tab = 'trace')}>Trace</button>
      {/if}
      {#if hasTests}
        <button class="rtab" class:active={tab === 'tests'} role="tab" aria-selected={tab === 'tests'} onclick={() => (tab = 'tests')}>
          Tests {#if tests.length}<span class="hcount {testsPassed === tests.length ? 'ok-c' : 'fail-c'}">{testsPassed}/{tests.length}</span>{/if}
        </button>
      {/if}
      {#if tab === 'body' && !isImage && !resp.too_large}
        <span class="grow"></span>
        <div class="view-toggle">
          <button class="vt" class:active={bodyView === 'pretty'} onclick={() => (bodyView = 'pretty')}>Pretty</button>
          <button class="vt" class:active={bodyView === 'raw'} onclick={() => (bodyView = 'raw')}>Raw</button>
        </div>
      {/if}
    </div>

    <div class="rbody">
      {#if tab === 'body'}
        {#if resp.too_large}
          <div class="big-body">
            <Icon name="box" size={22} />
            <div class="big-title">Response is {fmtSize(resp.size_bytes)} — too large to display</div>
            <div class="big-sub">Bodies over 25&nbsp;MB aren't loaded inline. Re-run against a smaller payload to inspect it here.</div>
          </div>
        {:else if isImage && previewUrl}
          <div class="img-wrap">
            <img class="img-preview" src={previewUrl} alt="Response preview" />
            <div class="img-meta">{fmtSize(resp.size_bytes)} · {resp.content_type}</div>
          </div>
        {:else if prettyBody.trim() === '' && resp.body.trim() === ''}
          <div class="empty-mini">Empty response body.</div>
        {:else}
          {#if resp.truncated}
            <div class="trunc-banner">
              <Icon name="box" size={12} />
              <span>Showing the first 512&nbsp;KB of {fmtSize(resp.size_bytes)}. Use <strong>Save</strong> to get the full response.</span>
            </div>
          {/if}
          {#if isJsonResp}
            <div class="resp-filter">
              <Icon name="search" size={12} />
              <input class="input mono grow" placeholder="JSONPath filter — e.g. $.data[0].id  (⌘F searches)" bind:value={jsonFilter} spellcheck="false" />
              {#if jsonFilter}<button class="link-clear" onclick={() => (jsonFilter = '')} aria-label="Clear filter">✕</button>{/if}
            </div>
          {/if}
          <div class="resp-editor">
            <CodeEditor path={respPath} content={displayBody} root={ws.current?.root_path ?? ''} language={respLang} readOnly={true} />
          </div>
        {/if}
      {:else if tab === 'headers'}
        <table class="htable mono">
          <tbody>
            {#each resp.headers as h, i (i)}
              <tr>
                <td class="hkey">{h.key}</td>
                <td class="hval">{h.value}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {:else if tab === 'trace'}
        <ol class="trace">
          {#each resp.trace as step, i (i)}
            <li class="trace-step {step.level}">
              <span class="trace-dot"></span>
              <span class="trace-label">{step.label}</span>
              <span class="trace-detail mono">{step.detail}</span>
              {#if step.ms != null}<span class="trace-ms">{step.ms} ms</span>{/if}
            </li>
          {/each}
        </ol>
      {:else}
        <div class="tests-pane">
          {#if tests.length === 0}
            <div class="empty-mini">No tests. Add a post-response script with <code>pm.test(...)</code>.</div>
          {:else}
            <ul class="test-list">
              {#each tests as t, i (i)}
                <li class="test-item {t.passed ? 'pass' : 'fail'}">
                  <span class="test-icon">{t.passed ? '✓' : '✕'}</span>
                  <span class="test-name">{t.name}</span>
                  {#if !t.passed && t.error}<span class="test-err mono">{t.error}</span>{/if}
                </li>
              {/each}
            </ul>
          {/if}
          {#if scriptLogs.length > 0}
            <div class="console-title">Console</div>
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
    gap: 8px;
    color: var(--text-dim);
    font-size: 12.5px;
  }
  .resp-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 2px 8px;
    flex-wrap: wrap;
  }
  .status-pill {
    display: inline-flex;
    align-items: center;
    height: 20px;
    padding: 0 9px;
    border-radius: 999px;
    font-size: 11.5px;
    font-weight: 700;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .status-pill.ok {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
  .status-pill.redirect {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .status-pill.client {
    background: color-mix(in srgb, #d2691e 20%, transparent);
    color: #d2691e;
  }
  .status-pill.server {
    background: color-mix(in srgb, var(--status-exited) 18%, transparent);
    color: var(--status-exited);
  }
  .meta {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 11px;
    color: var(--text-dim);
  }
  .ct {
    max-width: 220px;
  }
  .rtabs {
    display: flex;
    gap: 2px;
    border-bottom: 1px solid var(--border);
  }
  .rtab {
    height: 26px;
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
  .rtab.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  .hcount {
    font-size: 10px;
    color: var(--text-dim);
  }
  .rbody {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding-top: 8px;
    display: flex;
    flex-direction: column;
  }
  .resp-filter {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 0 8px;
    color: var(--text-dim);
  }
  .link-clear {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 12px;
  }
  .resp-editor {
    flex: 1;
    min-height: 180px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .body-pre {
    margin: 0;
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
    font-size: 11.5px;
    line-height: 1.55;
    color: var(--text);
  }
  .empty-mini {
    font-size: 12px;
    color: var(--text-dim);
    padding: 8px 2px;
  }
  .htable {
    width: 100%;
    border-collapse: collapse;
    user-select: text;
  }
  .htable td {
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    font-size: 11.5px;
    vertical-align: top;
    word-break: break-word;
  }
  .hkey {
    color: var(--accent);
    width: 34%;
    font-weight: 600;
  }
  .hval {
    color: var(--text);
  }
  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grow {
    flex: 1;
  }
  .save-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 9px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
  }
  .save-btn:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
  }
  .view-toggle {
    display: inline-flex;
    gap: 2px;
    align-self: center;
    margin-bottom: 2px;
  }
  .vt {
    height: 20px;
    padding: 0 8px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .vt.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .img-wrap {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 4px 0;
  }
  .img-preview {
    max-width: 100%;
    max-height: 420px;
    object-fit: contain;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background:
      repeating-conic-gradient(var(--surface-2) 0% 25%, transparent 0% 50%) 50% / 18px 18px;
  }
  .img-meta {
    font-size: 11px;
    color: var(--text-dim);
  }
  .big-body {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    text-align: center;
    padding: 32px 16px;
    color: var(--text-dim);
  }
  .big-title {
    font-size: 13px;
    font-weight: 600;
    color: var(--text);
  }
  .big-sub {
    font-size: 12px;
    max-width: 360px;
  }
  .trunc-banner {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    margin-bottom: 8px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, #d2691e 14%, transparent);
    color: #d2691e;
    font-size: 11.5px;
  }
  .trunc-banner strong {
    font-weight: 700;
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
    font-size: 12px;
  }
  .trace-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-dim);
    margin-inline-start: -10px;
    flex-shrink: 0;
  }
  .trace-step.timing .trace-dot { background: var(--accent); }
  .trace-step.success { border-inline-start-color: color-mix(in srgb, var(--status-working) 55%, transparent); }
  .trace-step.success .trace-dot { background: var(--status-working); }
  .trace-step.error { border-inline-start-color: color-mix(in srgb, var(--status-exited) 55%, transparent); }
  .trace-step.error .trace-dot { background: var(--status-exited); }
  .trace-step.redirect .trace-dot { background: #d2691e; }
  .trace-label {
    font-weight: 600;
    color: var(--text);
    min-width: 110px;
  }
  .trace-detail {
    flex: 1;
    color: var(--text-dim);
    font-size: 11px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .trace-ms {
    font-variant-numeric: tabular-nums;
    color: var(--accent);
    font-weight: 600;
    font-size: 11px;
  }
  .hcount.ok-c { color: var(--status-working); font-weight: 700; }
  .hcount.fail-c { color: var(--status-exited); font-weight: 700; }
  .tests-pane { display: flex; flex-direction: column; gap: 8px; }
  .test-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 3px; }
  .test-item {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    font-size: 12px;
  }
  .test-item.pass { background: color-mix(in srgb, var(--status-working) 12%, transparent); }
  .test-item.fail { background: color-mix(in srgb, var(--status-exited) 12%, transparent); }
  .test-icon { font-weight: 700; }
  .test-item.pass .test-icon { color: var(--status-working); }
  .test-item.fail .test-icon { color: var(--status-exited); }
  .test-name { flex: 0 0 auto; }
  .test-err { color: var(--status-exited); font-size: 11px; }
  .console-title { font-size: 11px; color: var(--text-dim); font-weight: 600; margin-top: 4px; }
  .console-log {
    margin: 0;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    padding: 8px;
    font-size: 11px;
    max-height: 160px;
    overflow: auto;
    white-space: pre-wrap;
    user-select: text;
  }
  .stream-console {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .stream-head {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 2px 8px;
  }
  .stream-log {
    flex: 1;
    min-height: 0;
    overflow: hidden; /* VirtualList owns the scroll */
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
    font-size: 10.5px;
    color: var(--text-dim);
    font-style: italic;
    padding: 2px 8px 4px;
    flex-shrink: 0;
  }
  .stream-item {
    display: flex;
    gap: 8px;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    font-size: 11.5px;
    align-items: baseline;
  }
  .stream-item.event { background: color-mix(in srgb, var(--accent) 8%, transparent); }
  .stream-item.message.out { background: color-mix(in srgb, var(--accent) 12%, transparent); }
  .stream-item.message.in { background: var(--surface-2); }
  .stream-item.error { background: color-mix(in srgb, var(--status-exited) 14%, transparent); }
  .stream-item.open, .stream-item.closed { color: var(--text-dim); font-style: italic; }
  .si-tag {
    flex: 0 0 auto;
    min-width: 56px;
    font-weight: 700;
    font-size: 10px;
    text-transform: uppercase;
    color: var(--accent);
  }
  .stream-item.error .si-tag { color: var(--status-exited); }
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
