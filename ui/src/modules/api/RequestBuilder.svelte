<script lang="ts">
  // Shared request builder used by both the full API page and the compact
  // right-panel. Method + URL + Send, plus a Params/Headers/Body/Auth tab strip,
  // an "Import curl" paste box, "Copy as curl", and "Save".
  import Icon from '../../lib/components/Icon.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import { apiClient, HTTP_METHODS, type ApiDraft } from '../../lib/stores/apiClient.svelte';
  import type { ApiAuth, ApiBodyMode, ApiKeyVal } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';

  interface Props {
    /** Compact mode trims spacing + uses a textarea instead of CodeEditor. */
    compact?: boolean;
  }
  let { compact = false }: Props = $props();

  type Tab = 'params' | 'headers' | 'body' | 'auth';
  let tab: Tab = $state('params');

  // The draft lives in the store so the page + panel share one editing target.
  const draft = $derived(apiClient.draft);

  const bodyModes: ApiBodyMode[] = ['none', 'json', 'raw', 'form', 'graphql'];
  const authTypes: ApiAuth['type'][] = ['none', 'bearer', 'basic', 'api_key'];

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
  // persist in the editor — encodeForm() drops empty-key pairs when sending, so a
  // derived approach would make "Add field" appear to do nothing.
  interface FormRow { key: string; value: string; }
  function parseForm(s: string): FormRow[] {
    if (!s) return [];
    return s.split('&').map((pair) => {
      const eq = pair.indexOf('=');
      if (eq < 0) return { key: decode(pair), value: '' };
      return { key: decode(pair.slice(0, eq)), value: decode(pair.slice(eq + 1)) };
    });
  }
  function encodeForm(rows: FormRow[]): string {
    return rows
      .filter((r) => r.key.trim() !== '')
      .map((r) => `${encodeURIComponent(r.key)}=${encodeURIComponent(r.value)}`)
      .join('&');
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
    if (draft.body_mode === 'form' && body !== lastFormEncoded) {
      formRows = parseForm(body);
      lastFormEncoded = body;
    }
  });
  function syncFormBody(): void {
    const enc = encodeForm(formRows);
    lastFormEncoded = enc;
    apiClient.draft = { ...apiClient.draft, body: enc };
  }
  function addFormRow(): void {
    formRows = [...formRows, { key: '', value: '' }];
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
  function setAuth(patch: Partial<ApiAuth>): void {
    apiClient.draft = { ...draft, auth: { ...draft.auth, ...patch } as ApiAuth };
  }
  function setAuthType(type: ApiAuth['type']): void {
    let auth: ApiAuth;
    switch (type) {
      case 'bearer': auth = { type, token: '' }; break;
      case 'basic': auth = { type, username: '', password: '' }; break;
      case 'api_key': auth = { type, key: '', value: '', in: 'header' }; break;
      default: auth = { type: 'none' };
    }
    apiClient.draft = { ...draft, auth };
  }
  function setBodyMode(mode: ApiBodyMode): void {
    apiClient.draft = { ...draft, body_mode: mode };
  }

  // ── actions ────────────────────────────────────────────────────────────────

  function send(): void {
    void apiClient.execute();
  }

  function onUrlKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      send();
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
  function save(): void {
    if (ws.myRole === 'viewer') {
      toasts.error('Read-only', 'You have viewer access to this workspace');
      return;
    }
    const name = prompt('Request name', draft.name || `${draft.method} ${draft.url}`)?.trim();
    if (!name) return;
    let collectionId: string | null = draft.requestId
      ? (apiClient.requests.find((r) => r.id === draft.requestId)?.collection_id ?? null)
      : null;
    if (apiClient.collections.length > 0) {
      const list = apiClient.collections.map((c, i) => `${i + 1}. ${c.name}`).join('\n');
      const pick = prompt(
        `Save into which collection? (number, blank = none)\n${list}`,
        '',
      )?.trim();
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

<div class="builder" class:compact>
  <!-- URL bar -->
  <div class="urlbar">
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
    <div class="url-wrap">
      <!-- Highlight layer mirrors the input value; `{{var}}` tokens get accent. -->
      <div class="url-highlight" aria-hidden="true">
        {#each urlSegments as seg, i (i)}
          <span class:var={seg.isVar}>{seg.text}</span>
        {/each}
      </div>
      <input
        class="url-input mono"
        placeholder="https://api.example.com/path  ·  use {'{{var}}'}"
        value={draft.url}
        oninput={(e) => setField('url', (e.currentTarget as HTMLInputElement).value)}
        onkeydown={onUrlKeydown}
        spellcheck="false"
        aria-label="Request URL"
      />
    </div>
    <button class="btn primary send" onclick={send} disabled={apiClient.sending}>
      {#if apiClient.sending}
        <Icon name="refresh" size={12} />Sending…
      {:else}
        <Icon name="send" size={12} />Send
      {/if}
    </button>
  </div>

  <!-- toolbar: curl in/out + save -->
  <div class="toolbar">
    <button class="btn small ghost" onclick={() => (curlOpen = !curlOpen)}>
      <Icon name="external" size={11} />Import curl
    </button>
    <button class="btn small ghost" onclick={copyAsCurl}>
      <Icon name="link" size={11} />Copy as curl
    </button>
    <span class="grow"></span>
    {#if apiClient.activeEnv}
      <span class="chip accent" title="Active environment">{apiClient.activeEnv.name}</span>
    {/if}
    <button class="btn small" onclick={save}>
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

  <!-- tab strip -->
  <div class="tabstrip" role="tablist">
    {#each [['params', 'Params', draft.query.length], ['headers', 'Headers', draft.headers.length], ['body', 'Body', draft.body_mode !== 'none' ? 1 : 0], ['auth', 'Auth', draft.auth.type !== 'none' ? 1 : 0]] as [id, label, count] (id)}
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
    {:else if tab === 'body'}
      <div class="bodymode">
        {#each bodyModes as m (m)}
          <button class="seg" class:active={draft.body_mode === m} onclick={() => setBodyMode(m)}>{m}</button>
        {/each}
      </div>
      {#if draft.body_mode === 'none'}
        <div class="empty-mini">No body for this request.</div>
      {:else if draft.body_mode === 'form'}
        <div class="kv-list">
          {#each formRows as row, i (i)}
            <div class="kv-row">
              <input class="input kv-key mono" placeholder="key" value={row.key} oninput={(e) => updateFormRow(i, { key: (e.currentTarget as HTMLInputElement).value })} />
              <input class="input kv-val mono" placeholder="value" value={row.value} oninput={(e) => updateFormRow(i, { value: (e.currentTarget as HTMLInputElement).value })} />
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
      {:else}
        <div class="body-editor">
          <CodeEditor
            path={draft.body_mode === 'json' ? 'body.json' : 'body.txt'}
            content={draft.body}
            root={ws.current?.root_path ?? ''}
            language={draft.body_mode === 'json' ? 'json' : ''}
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
        {:else}
          <div class="empty-mini">No authentication.</div>
        {/if}
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
  .toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
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
  .add-row {
    align-self: flex-start;
  }
  .bodymode,
  .auth-type {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
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
  .field-row > label {
    width: 80px;
    flex-shrink: 0;
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .empty-mini {
    font-size: 12px;
    color: var(--text-dim);
    padding: 8px 2px;
  }
</style>
