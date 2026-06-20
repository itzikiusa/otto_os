<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { copyAsJson, downloadJson, exportCsv } from '../../lib/components/exporters';
  import type {
    BrokerCluster,
    ConsumeReq,
    ConsumeResp,
    KafkaMessage,
    MessageHeader,
    ProduceReq,
    StartPosition,
    TopicDetail,
    ValueFormat,
  } from '../../lib/api/types';

  interface Props {
    cluster: BrokerCluster;
    topic: string;
    ondeleted: () => void;
  }
  let { cluster, topic, ondeleted }: Props = $props();

  const guarded = $derived(cluster.read_only || cluster.environment === 'prod');

  type Tab = 'messages' | 'partitions' | 'config' | 'produce';
  let tab = $state<Tab>('messages');

  let detail = $state<TopicDetail | null>(null);
  let detailErr = $state<string | null>(null);

  // ---- consume state ----
  let startMode = $state<'latest' | 'beginning' | 'offset' | 'timestamp'>('latest');
  let startOffset = $state(0);
  let startTs = $state('');
  let partition = $state<number | ''>('');
  let limit = $state(50);
  let decode = $state<ValueFormat>('auto');
  let keyFilter = $state('');
  let valueFilter = $state('');
  let consuming = $state(false);
  let result = $state<ConsumeResp | null>(null);
  let selected = $state<KafkaMessage | null>(null);
  let rawView = $state(false);
  // Auto-refresh the peek on an interval (live-tail), with an off switch.
  let autoPoll = $state(false);
  const POLL_MS = 60_000;

  // ---- produce state ----
  let pKey = $state('');
  let pValue = $state('');
  let pPartition = $state<number | ''>('');
  let pTombstone = $state(false);
  let pKeyBase64 = $state(false);
  let pValueBase64 = $state(false);
  // Extra headers for produce: list of {key, value} pairs.
  let pHeaders = $state<{ key: string; value: string }[]>([]);
  let producing = $state(false);

  // ---- config editing ----
  let cfgName = $state('');
  let cfgValue = $state('');
  let cfgSaving = $state(false);

  function loadDetail() {
    detail = null;
    detailErr = null;
    void api
      .get<TopicDetail>(`/brokers/clusters/${cluster.id}/topics/${encodeURIComponent(topic)}`)
      .then((d) => (detail = d))
      .catch((e) => (detailErr = String(e)));
  }

  $effect(() => {
    // re-run when topic changes
    void topic;
    result = null;
    selected = null;
    tab = 'messages';
    autoPoll = false;
    loadDetail();
  });

  // Auto-refresh the peek every minute while enabled. Only `autoPoll` is a
  // dependency (untrack the consume call so editing the filters doesn't reset
  // the timer); the immediate run gives instant feedback when toggled on.
  $effect(() => {
    if (!autoPoll) return;
    untrack(() => void consume());
    const timer = setInterval(() => {
      if (!consuming) untrack(() => void consume());
    }, POLL_MS);
    return () => clearInterval(timer);
  });

  function fmtTs(ms: number | null): string {
    if (ms === null) return '—';
    return new Date(ms).toLocaleString();
  }

  async function consume() {
    let start: StartPosition;
    if (startMode === 'beginning') start = { type: 'beginning' };
    else if (startMode === 'offset') start = { type: 'offset', offset: Number(startOffset) };
    else if (startMode === 'timestamp')
      start = { type: 'timestamp', timestamp_ms: new Date(startTs).getTime() || Date.now() };
    else start = { type: 'latest' };

    const req: ConsumeReq = {
      partition: partition === '' ? null : Number(partition),
      start,
      limit: Number(limit),
      decode,
      key_filter: keyFilter.trim() || null,
      value_filter: valueFilter.trim() || null,
    };
    consuming = true;
    selected = null;
    try {
      result = await api.post<ConsumeResp>(
        `/brokers/clusters/${cluster.id}/topics/${encodeURIComponent(topic)}/consume`,
        req,
      );
      if (result.messages.length === 0) toasts.info('No messages in the selected range');
    } catch (e) {
      toasts.error('Consume failed', String(e));
    } finally {
      consuming = false;
    }
  }

  function addHeader() {
    pHeaders = [...pHeaders, { key: '', value: '' }];
  }
  function removeHeader(i: number) {
    pHeaders = pHeaders.filter((_, idx) => idx !== i);
  }

  async function produce() {
    if (!pTombstone && !pValue) {
      toasts.error('Value is required (or enable Tombstone)');
      return;
    }
    if (guarded) {
      const ok = await confirmer.ask(
        `Produce to "${topic}" on guarded cluster "${cluster.name}"?`,
        { title: 'Produce to guarded cluster', confirmLabel: 'Produce', danger: true },
      );
      if (!ok) return;
    }
    const headers: MessageHeader[] = pHeaders.filter((h) => h.key.trim());
    const req: ProduceReq = {
      partition: pPartition === '' ? null : Number(pPartition),
      key: pKey || null,
      value: pTombstone ? '' : pValue,
      headers: headers.length ? headers : undefined,
      key_base64: pKeyBase64,
      value_base64: pTombstone ? false : pValueBase64,
      confirm: guarded,
    };
    producing = true;
    try {
      const r = await api.post<{ partition: number; offset: number }>(
        `/brokers/clusters/${cluster.id}/topics/${encodeURIComponent(topic)}/produce`,
        req,
      );
      toasts.success(`Produced to partition ${r.partition} @ offset ${r.offset}`);
      pValue = '';
      pKey = '';
      pTombstone = false;
      pHeaders = [];
      loadDetail();
    } catch (e) {
      toasts.error('Produce failed', String(e));
    } finally {
      producing = false;
    }
  }

  async function setConfig() {
    if (!cfgName.trim()) return;
    if (guarded) {
      const ok = await confirmer.ask(
        `Change config on guarded cluster "${cluster.name}"?`,
        { title: 'Alter config on guarded cluster', confirmLabel: 'Apply', danger: true },
      );
      if (!ok) return;
    }
    cfgSaving = true;
    try {
      detail = {
        ...detail!,
        configs: await api.put(
          `/brokers/clusters/${cluster.id}/topics/${encodeURIComponent(topic)}/configs`,
          { configs: [{ name: cfgName.trim(), value: cfgValue }], confirm: guarded },
        ),
      };
      toasts.success(`Set ${cfgName.trim()}`);
      cfgName = '';
      cfgValue = '';
    } catch (e) {
      toasts.error('Config update failed', String(e));
    } finally {
      cfgSaving = false;
    }
  }

  async function deleteTopic() {
    const typed = await confirmer.promptText(
      `Type the topic name to confirm deletion. This is irreversible.`,
      { title: `Delete topic "${topic}"`, confirmLabel: 'Delete', placeholder: topic },
    );
    if (typed !== topic) return;
    try {
      await api.del(
        `/brokers/clusters/${cluster.id}/topics/${encodeURIComponent(topic)}?confirm=${guarded}`,
      );
      toasts.success(`Deleted ${topic}`);
      ondeleted();
    } catch (e) {
      toasts.error('Delete failed', String(e));
    }
  }

  // ---- export helpers -------------------------------------------------------

  function exportMessages(fmt: 'json' | 'csv') {
    if (!result) return;
    const rows = result.messages.map((m) => ({
      partition: m.partition,
      offset: m.offset,
      timestamp_ms: m.timestamp_ms,
      key: m.key?.text ?? null,
      value: m.value?.text ?? null,
      size_bytes: m.size_bytes,
      headers: m.headers.length ? JSON.stringify(m.headers) : null,
    }));
    if (fmt === 'json') {
      downloadJson(rows, `${topic}-peek.json`);
    } else {
      exportCsv(rows, `${topic}-peek.csv`);
    }
  }

  async function copySelectedAsJson() {
    if (!selected) return;
    await copyAsJson({
      partition: selected.partition,
      offset: selected.offset,
      timestamp_ms: selected.timestamp_ms,
      key: selected.key,
      value: selected.value,
      headers: selected.headers,
    });
    toasts.success('Copied to clipboard');
  }
</script>

<div class="td">
  <header>
    <div class="title">
      <Icon name="box" size={15} />
      <span class="name">{topic}</span>
      {#if detail}<span class="muted">· {detail.partitions.length}p · {detail.message_count.toLocaleString()} msgs</span>{/if}
    </div>
    <button class="btn small danger" onclick={deleteTopic}>Delete topic</button>
  </header>

  <nav class="subtabs">
    <button class:on={tab === 'messages'} onclick={() => (tab = 'messages')}>Messages</button>
    <button class:on={tab === 'partitions'} onclick={() => (tab = 'partitions')}>Partitions</button>
    <button class:on={tab === 'config'} onclick={() => (tab = 'config')}>Config</button>
    <button class:on={tab === 'produce'} onclick={() => (tab = 'produce')}>Produce</button>
  </nav>

  {#if detailErr}
    <p class="err">{detailErr}</p>
  {/if}

  {#if tab === 'messages'}
    <div class="consume-bar">
      <select bind:value={startMode}>
        <option value="latest">Latest</option>
        <option value="beginning">From beginning</option>
        <option value="offset">From offset</option>
        <option value="timestamp">From time</option>
      </select>
      {#if startMode === 'offset'}
        <input class="sm" type="number" bind:value={startOffset} placeholder="offset" />
      {/if}
      {#if startMode === 'timestamp'}
        <input class="sm" type="datetime-local" bind:value={startTs} />
      {/if}
      <select bind:value={partition}>
        <option value="">All partitions</option>
        {#each detail?.partitions ?? [] as p (p.id)}
          <option value={p.id}>P{p.id}</option>
        {/each}
      </select>
      <input class="sm" type="number" bind:value={limit} min="1" max="5000" title="Max messages" />
      <select bind:value={decode} title="Decode value as">
        <option value="auto">Auto</option>
        <option value="json">JSON</option>
        <option value="utf8">UTF-8</option>
        <option value="protobuf">Protobuf</option>
        <option value="avro">Avro</option>
        <option value="hex">Hex</option>
        <option value="base64">Base64</option>
      </select>
      <input class="grow" bind:value={valueFilter} placeholder="filter value…" />
      <label class="auto" class:on={autoPoll} title="Re-peek every minute">
        <input type="checkbox" bind:checked={autoPoll} /> Auto · 1m
      </label>
      <button class="btn primary small" onclick={consume} disabled={consuming}>
        {consuming ? 'Reading…' : 'Peek'}
      </button>
      {#if result && result.messages.length > 0}
        <button class="btn small" onclick={() => exportMessages('json')} title="Export all as JSON">
          <Icon name="download" size={12} /> JSON
        </button>
        <button class="btn small" onclick={() => exportMessages('csv')} title="Export all as CSV">
          <Icon name="download" size={12} /> CSV
        </button>
      {/if}
    </div>

    <div class="msg-layout">
      <div class="msg-list">
        <table>
          <thead>
            <tr><th>P</th><th>Offset</th><th>Key</th><th>Time</th><th>Size</th></tr>
          </thead>
          <tbody>
            {#each result?.messages ?? [] as m (m.partition + '-' + m.offset)}
              <tr
                class:sel={selected === m}
                onclick={() => {
                  selected = m;
                  rawView = false;
                }}
              >
                <td>{m.partition}</td>
                <td class="mono">{m.offset}</td>
                <td class="key">{m.key?.text ?? '∅'}</td>
                <td class="muted nowrap">{fmtTs(m.timestamp_ms)}</td>
                <td class="muted">{m.size_bytes}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if result && result.messages.length === 0}
          <p class="muted pad">No messages.</p>
        {/if}
        {#if result?.truncated}
          <p class="muted pad small">Showing first {result.messages.length} — increase the limit for more.</p>
        {/if}
      </div>

      <div class="msg-detail">
        {#if selected}
          <div class="md-head">
            <span class="mono">P{selected.partition} · offset {selected.offset}</span>
            <span class="muted">{fmtTs(selected.timestamp_ms)}</span>
            {#if selected.value?.format}
              <span class="badge">{selected.value.format}{selected.value.schema_id != null ? ` #${selected.value.schema_id}` : ''}</span>
            {/if}
            {#if selected.headers.length > 0}
              <span class="badge muted-badge">{selected.headers.length} header{selected.headers.length === 1 ? '' : 's'}</span>
            {/if}
            {#if selected.value?.raw_base64}
              <button class="btn tiny" onclick={() => (rawView = !rawView)}>
                {rawView ? 'Decoded' : 'Raw'}
              </button>
            {/if}
            <button class="btn tiny" onclick={copySelectedAsJson} title="Copy message as JSON">
              <Icon name="copy" size={11} /> Copy
            </button>
          </div>
          {#if selected.key}
            <h5>Key <span class="muted">({selected.key.format})</span></h5>
            <pre class="payload key">{selected.key.text || '∅'}</pre>
          {/if}
          <h5>Value</h5>
          <pre class="payload">{rawView
              ? (selected.value?.raw_base64 ?? '')
              : (selected.value?.text ?? '∅')}</pre>
          {#if selected.headers.length > 0}
            <h5>Headers</h5>
            <table class="headers">
              <tbody>
                {#each selected.headers as h (h.key)}
                  <tr><td class="mono">{h.key}</td><td>{h.value}</td></tr>
                {/each}
              </tbody>
            </table>
          {/if}
        {:else}
          <p class="muted pad">Select a message to inspect its key, value, and headers.</p>
        {/if}
      </div>
    </div>
  {:else if tab === 'partitions'}
    <table class="grid">
      <thead>
        <tr><th>Partition</th><th>Leader</th><th>Replicas</th><th>ISR</th><th>Low</th><th>High</th><th>Messages</th></tr>
      </thead>
      <tbody>
        {#each detail?.partitions ?? [] as p (p.id)}
          <tr>
            <td>{p.id}</td>
            <td>{p.leader}</td>
            <td class="mono">{p.replicas.join(', ')}</td>
            <td class="mono">{p.isr.join(', ')}</td>
            <td class="muted">{p.low}</td>
            <td class="muted">{p.high}</td>
            <td>{p.message_count.toLocaleString()}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {:else if tab === 'config'}
    <div class="cfg-set">
      <input class="grow" bind:value={cfgName} placeholder="config name (e.g. retention.ms)" />
      <input class="grow" bind:value={cfgValue} placeholder="value" />
      <button class="btn small" onclick={setConfig} disabled={cfgSaving}>Set</button>
    </div>
    <table class="grid">
      <thead><tr><th>Name</th><th>Value</th><th>Source</th></tr></thead>
      <tbody>
        {#each detail?.configs ?? [] as c (c.name)}
          <tr class:overridden={!c.is_default}>
            <td class="mono">{c.name}</td>
            <td>{c.is_sensitive ? '••••••' : (c.value ?? '')}</td>
            <td class="muted small">{c.source}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  {:else if tab === 'produce'}
    <div class="produce">
      <div class="produce-opts">
        <label class="chk-opt"><input type="checkbox" bind:checked={pTombstone} /> Tombstone (null value)</label>
        <label class="chk-opt"><input type="checkbox" bind:checked={pKeyBase64} /> Key is Base64</label>
        {#if !pTombstone}
          <label class="chk-opt"><input type="checkbox" bind:checked={pValueBase64} /> Value is Base64</label>
        {/if}
      </div>
      <label class="field">
        <span>Key (optional){pKeyBase64 ? ' — base64' : ''}</span>
        <input bind:value={pKey} placeholder={pKeyBase64 ? 'base64-encoded bytes' : 'string key'} />
      </label>
      <label class="field">
        <span>Partition (optional)</span>
        <select bind:value={pPartition}>
          <option value="">Auto</option>
          {#each detail?.partitions ?? [] as p (p.id)}<option value={p.id}>P{p.id}</option>{/each}
        </select>
      </label>
      {#if !pTombstone}
        <label class="field grow">
          <span>Value{pValueBase64 ? ' — base64' : ''}</span>
          <textarea bind:value={pValue} rows="6" placeholder={pValueBase64 ? 'base64-encoded bytes' : '{ "hello": "world" }'}></textarea>
        </label>
      {/if}
      <div class="headers-section">
        <div class="headers-title">
          <span class="dim-label">Headers</span>
          <button class="btn tiny" onclick={addHeader}>+ Add</button>
        </div>
        {#each pHeaders as h, i (i)}
          <div class="header-row">
            <input bind:value={h.key} placeholder="key" class="header-key" />
            <input bind:value={h.value} placeholder="value" class="header-val" />
            <button class="btn tiny danger-tiny" onclick={() => removeHeader(i)} title="Remove header">×</button>
          </div>
        {/each}
      </div>
      <button class="btn primary" onclick={produce} disabled={producing}>
        {producing ? 'Producing…' : pTombstone ? 'Produce tombstone' : 'Produce message'}
      </button>
    </div>
  {/if}
</div>

<style>
  .td {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  header .title {
    display: flex;
    align-items: baseline;
    gap: 8px;
  }
  header .name {
    font-weight: 600;
    font-family: var(--font-mono);
  }
  .subtabs {
    display: flex;
    gap: 2px;
    padding: 6px 12px 0;
    border-bottom: 1px solid var(--border);
  }
  .subtabs button {
    border: none;
    background: transparent;
    color: var(--text-dim);
    padding: 6px 12px;
    border-radius: var(--radius-s) var(--radius-s) 0 0;
    cursor: pointer;
    font-size: 13px;
  }
  .subtabs button.on {
    color: var(--text);
    border-bottom: 2px solid var(--accent);
  }
  .consume-bar {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 10px 12px;
    align-items: center;
    border-bottom: 1px solid var(--border);
  }
  .consume-bar select,
  .consume-bar input {
    padding: 5px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
  }
  .sm {
    width: 110px;
  }
  .auto {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    color: var(--text-dim);
    white-space: nowrap;
    cursor: pointer;
  }
  .auto.on {
    color: var(--accent);
  }
  .grow {
    flex: 1;
    min-width: 100px;
  }
  .msg-layout {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .msg-list {
    flex: 1;
    overflow: auto;
    border-right: 1px solid var(--border);
  }
  .msg-detail {
    width: 45%;
    min-width: 320px;
    overflow: auto;
    padding: 12px 14px;
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
  th {
    text-align: left;
    font-weight: 500;
    color: var(--text-dim);
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    padding: 6px 10px;
    position: sticky;
    top: 0;
    background: var(--surface);
  }
  tbody td {
    padding: 5px 10px;
    border-top: 1px solid var(--border);
  }
  .msg-list tbody tr {
    cursor: pointer;
  }
  .msg-list tbody tr:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .msg-list tbody tr.sel {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
  }
  .key {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
  .nowrap {
    white-space: nowrap;
  }
  .grid.grid,
  table.grid {
    margin: 0;
  }
  table.grid tr.overridden td {
    color: var(--text);
  }
  table.grid {
    overflow: auto;
    display: block;
  }
  .md-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    margin-bottom: 8px;
  }
  .badge {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 2px 6px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent);
  }
  h5 {
    margin: 12px 0 4px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.03em;
    color: var(--text-dim);
  }
  .payload {
    margin: 0;
    padding: 10px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    font-family: var(--font-mono);
    font-size: 12px;
    white-space: pre-wrap;
    word-break: break-word;
    max-height: 40vh;
    overflow: auto;
  }
  .payload.key {
    max-height: 120px;
  }
  table.headers td {
    border: none;
    padding: 2px 8px 2px 0;
    font-size: 12px;
  }
  .cfg-set {
    display: flex;
    gap: 6px;
    padding: 10px 12px;
    border-bottom: 1px solid var(--border);
  }
  .cfg-set input {
    padding: 5px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
  }
  .produce {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 14px;
    overflow: auto;
  }
  .produce-opts {
    display: flex;
    gap: 14px;
    flex-wrap: wrap;
    padding: 6px 0;
  }
  .chk-opt {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
    color: var(--text-dim);
    cursor: pointer;
    white-space: nowrap;
  }
  .headers-section {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .headers-title {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dim-label {
    font-size: 12px;
    color: var(--text-dim);
  }
  .header-row {
    display: flex;
    gap: 5px;
    align-items: center;
  }
  .header-key {
    width: 140px;
    padding: 5px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
  }
  .header-val {
    flex: 1;
    padding: 5px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 12px;
  }
  .danger-tiny {
    color: var(--status-exited, #ff5f57);
    padding: 2px 7px;
    font-size: 14px;
    line-height: 1;
  }
  .muted-badge {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text-dim);
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .field span {
    font-size: 12px;
    color: var(--text-dim);
  }
  .field input,
  .field select,
  .field textarea {
    padding: 7px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: 13px;
    font-family: inherit;
  }
  .field textarea {
    font-family: var(--font-mono);
    resize: vertical;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .muted {
    color: var(--text-dim);
  }
  .small {
    font-size: 11px;
  }
  .pad {
    padding: 14px;
  }
  .err {
    color: var(--status-exited, #ff5f57);
    padding: 10px 14px;
    font-size: 13px;
  }
  .btn.danger {
    color: var(--status-exited, #ff5f57);
  }
  .btn.tiny {
    padding: 2px 8px;
    font-size: 11px;
  }
</style>
