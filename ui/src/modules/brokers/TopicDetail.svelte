<script lang="ts">
  import { untrack } from 'svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { pollWhileVisible } from '../../lib/poll';
  import { copyAsJson, downloadJson, exportCsv } from '../../lib/components/exporters';
  import type {
    BrokerCluster,
    ConsumeReq,
    ConsumeResp,
    KafkaMessage,
    MessageHeader,
    PartitionRange,
    ProduceReq,
    StartPosition,
    TopicDetail,
    ValueFormat,
  } from '../../lib/api/types';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { brokersTopicPort } from '../../lib/uiCommands/brokers';
  import ContextPacketDialog from '../../lib/components/ContextPacketDialog.svelte';

  // ── Send-to-agent dialog (B2a) ─────────────────────────────────────────────
  let sendToAgentOpen = $state(false);

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
  let keyFromBeginning = $state(false);
  let valueFilter = $state('');
  /** When true, message key/value/headers are redacted server-side before returning. */
  let maskPayloads = $state(false);
  let consuming = $state(false);
  // Raw (not deep-proxied): a peek is up to 5000 messages / 16 MiB of payload,
  // replaced wholesale on every peek or tail tick.
  let result = $state.raw<ConsumeResp | null>(null);
  let selected = $state.raw<KafkaMessage | null>(null);
  let rawView = $state(false);
  const selKey = $derived(selected ? `${selected.partition}-${selected.offset}` : '');
  /** The request behind `result` (exports re-read it in full when previews were cut). */
  let lastReq: ConsumeReq | null = null;
  /** Full-value fetch for a selected message whose list preview was truncated. */
  let fullLoading = $state(false);
  let fullErr = $state<string | null>(null);
  let fullCtl: AbortController | null = null;
  const consumeUrl = () => `/brokers/clusters/${cluster.id}/topics/${encodeURIComponent(topic)}/consume`;

  /** Open a message; fetch its full value when the list only holds a preview. */
  function selectMessage(m: KafkaMessage) {
    selected = m;
    rawView = false;
    fullErr = null;
    fullCtl?.abort();
    fullCtl = null;
    fullLoading = false;
    if (!m.value?.truncated) return;
    const ctl = new AbortController();
    fullCtl = ctl;
    fullLoading = true;
    void fetchFull(m, ctl.signal)
      .then((full) => {
        if (ctl.signal.aborted || selected !== m) return;
        if (full) selected = full;
        else fullErr = 'The message is no longer available (retention or compaction).';
      })
      .catch((e) => {
        if (!ctl.signal.aborted) fullErr = loadErrorText(e);
      })
      .finally(() => {
        if (fullCtl === ctl) {
          fullLoading = false;
          fullCtl = null;
        }
      });
  }

  /** One message at (partition, offset), decoded in full with the peek's options. */
  async function fetchFull(m: KafkaMessage, signal?: AbortSignal): Promise<KafkaMessage | null> {
    const r = await api.post<ConsumeResp>(
      consumeUrl(),
      {
        partition: m.partition,
        start: { type: 'offset', offset: m.offset },
        limit: 1,
        decode: lastReq?.decode ?? decode,
        ...(result?.masked ? { mask: true } : {}),
      } satisfies ConsumeReq,
      signal,
    );
    return r.messages.find((x) => x.partition === m.partition && x.offset === m.offset) ?? null;
  }

  // Keep the full row as a pointer target without changing table semantics.
  // The offset button supplies the same action for keyboard/assistive input.
  function inspectRow(event: PointerEvent) {
    if (event.button !== 0 || !(event.target instanceof Element) || event.target.closest('button')) return;
    // Finishing a text drag should leave the selection available to copy.
    if (window.getSelection()?.isCollapsed === false) return;
    const row = event.target.closest<HTMLTableRowElement>('tr[data-message-key]');
    const message = result?.messages.find((m) => `${m.partition}-${m.offset}` === row?.dataset.messageKey);
    if (message && message !== selected) selectMessage(message);
  }

  // ---- incremental live-tail state ----
  // Tracks the max offset seen per partition so each poll only fetches new messages.
  let tailOffsets = $state<Map<number, number>>(new Map());
  // Capped ring buffer so the in-memory list does not grow unbounded.
  const TAIL_CAP = 500;
  // Auto-refresh on an interval (live-tail). The interval re-uses the offsets
  // accumulated so far; toggling off clears them.
  let autoPoll = $state(false);
  const POLL_MS = 60_000;
  /** Messages the last tail tick appended (inline badge — no per-tick toast). */
  let tailAdded = $state(0);

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
  /** Inline field error for the produce form (shown under Value, not a toast). */
  let pValueErr = $state<string | null>(null);
  let produceError = $state<string | null>(null);

  // Agent UI control (lib/uiCommands/brokers.ts): the agent's peek runs in
  // THIS form (the user sees the options and the grid), and a produce is
  // prefilled here while the user confirms it.
  $effect(() =>
    brokersTopicPort.bind({
      clusterId: cluster.id,
      topic,
      async peek(o) {
        tab = 'messages';
        startMode = o.start ?? 'latest';
        partition = o.partition ?? '';
        limit = o.limit ?? 50;
        keyFilter = o.key_filter ?? '';
        valueFilter = o.value_filter ?? '';
        result = null;
        // The agent reads the values it asked for — full, not list previews.
        await consume(false);
        return result;
      },
      showProduce(o) {
        tab = 'produce';
        pKey = o.key ?? '';
        pValue = o.value;
        pPartition = o.partition ?? '';
        pTombstone = false;
      },
      produced() {
        pValue = '';
        pKey = '';
        loadDetail(true);
      },
    }),
  );

  // ---- config editing ----
  let cfgName = $state('');
  let cfgValue = $state('');
  let cfgSaving = $state(false);

  /** `keep` refreshes in place (after a produce) instead of blanking the
   *  header counts, the partitions table and the partition pickers. */
  function loadDetail(keep = false) {
    if (!keep) detail = null;
    detailErr = null;
    void api
      .get<TopicDetail>(`/brokers/clusters/${cluster.id}/topics/${encodeURIComponent(topic)}`)
      .then((d) => (detail = d))
      .catch((e) => (detailErr = loadErrorText(e)));
  }

  $effect(() => {
    // re-run when topic changes
    void topic;
    result = null;
    selected = null;
    fullCtl?.abort();
    lastReq = null;
    tab = 'messages';
    autoPoll = false;
    tailOffsets = new Map();
    loadDetail();
  });

  // Incremental live-tail: on each tick, request only messages past the max
  // offset seen per partition (forward consume). New messages are appended to
  // the existing result; the ring buffer is capped at TAIL_CAP so memory is
  // bounded. The first tick after enabling autoPoll does a normal peek to seed
  // the offsets; subsequent ticks send `start: { type: 'offset', offset: … }`
  // per partition (one call per partition that has a known cursor, falling back
  // to a single all-partition call when no cursor is set yet).
  $effect(() => {
    if (!autoPoll) return;
    // Seed on enable, then chained ticks (paused while hidden, stopped on leave).
    untrack(() => void consumeWithTail(false));
    const poller = pollWhileVisible(
      () => (consuming ? undefined : untrack(() => consumeWithTail(true))),
      { ms: POLL_MS, immediate: false },
    );
    return () => poller.stop();
  });

  function fmtTs(ms: number | null): string {
    if (ms === null) return '—';
    return new Date(ms).toLocaleString();
  }

  const consumeError = $derived(
    !Number.isSafeInteger(limit) || limit < 1 || limit > 5000 ? 'Choose a whole message limit from 1 to 5000.'
      : startMode === 'offset' && (!Number.isSafeInteger(startOffset) || startOffset < 0) ? 'Enter a nonnegative whole offset.'
      : startMode === 'timestamp' && (!startTs || !Number.isFinite(new Date(startTs).getTime())) ? 'Choose a valid start time.'
      : '',
  );

  /** Build a start position appropriate for a fresh (non-tail) peek. */
  function buildStart(): StartPosition {
    if (startMode === 'beginning') return { type: 'beginning' };
    if (startMode === 'offset') return { type: 'offset', offset: Number(startOffset) };
    if (startMode === 'timestamp')
      return { type: 'timestamp', timestamp_ms: new Date(startTs).getTime() };
    return { type: 'latest' };
  }

  /** Update tailOffsets from a batch of returned messages. */
  function updateTailOffsets(msgs: KafkaMessage[]) {
    for (const m of msgs) {
      const prev = tailOffsets.get(m.partition) ?? -1;
      if (m.offset > prev) tailOffsets.set(m.partition, m.offset);
    }
    // Trigger reactivity: reassign the map reference.
    tailOffsets = new Map(tailOffsets);
  }

  /** Cap the messages ring at TAIL_CAP (drop oldest). */
  function capRing(msgs: KafkaMessage[]): KafkaMessage[] {
    return msgs.length > TAIL_CAP ? msgs.slice(msgs.length - TAIL_CAP) : msgs;
  }

  /**
   * Consume messages.
   *
   * When `incremental` is true and tailOffsets has entries, issue per-partition
   * requests from (maxOffset + 1) and append new messages instead of replacing.
   * When tailOffsets is empty (first tick) or incremental is false, do a
   * normal full peek and seed the offsets.
   */
  async function consumeWithTail(incremental: boolean) {
    const hasCursors = tailOffsets.size > 0;

    if (!incremental || !hasCursors) {
      // Seed pass: normal peek, then seed offsets from what comes back.
      await consume();
      if (result) updateTailOffsets(result.messages);
      return;
    }

    // Incremental pass: ONE request for every partition with a known cursor,
    // each resuming just past its last seen offset (was one sequential request
    // per partition — 100 partitions × a peek each per tick).
    consuming = true;
    try {
      const parts = partition !== '' ? [Number(partition)] : [...tailOffsets.keys()];
      const starts = parts.flatMap((p) => {
        const cursor = tailOffsets.get(p);
        return cursor === undefined ? [] : [{ partition: p, offset: cursor + 1 }];
      });
      if (starts.length === 0) return;
      const req: ConsumeReq = {
        start_offsets: starts,
        limit: Math.min(5000, 50 * starts.length),
        decode,
        preview: true,
        ...(maskPayloads ? { mask: true } : {}),
        // No filters on tail increments — they were applied on the seed.
      };
      let r: ConsumeResp;
      try {
        r = await api.post<ConsumeResp>(consumeUrl(), req);
      } catch {
        return; // a failed tick leaves the buffer as is; the next tick retries
      }
      tailAdded = r.messages.length;
      if (r.messages.length > 0) {
        updateTailOffsets(r.messages);
        const prev = result;
        result = {
          messages: capRing([...(prev?.messages ?? []), ...r.messages]),
          partitions: mergePartitionRanges(prev?.partitions ?? [], r.partitions),
          truncated: prev?.truncated ?? false,
          // A mixed buffer must never claim every payload was masked.
          masked: (prev?.masked ?? r.masked) === true && r.masked === true,
        };
      }
    } finally {
      consuming = false;
    }
  }

  /** Merge two PartitionRange arrays, keeping the widest [low, high] per partition. */
  function mergePartitionRanges(a: PartitionRange[], b: PartitionRange[]): PartitionRange[] {
    const map = new Map<number, PartitionRange>();
    for (const r of a) map.set(r.partition, { ...r });
    for (const r of b) {
      const existing = map.get(r.partition);
      if (!existing) {
        map.set(r.partition, { ...r });
      } else {
        map.set(r.partition, {
          partition: r.partition,
          low: Math.min(existing.low, r.low),
          high: Math.max(existing.high, r.high),
        });
      }
    }
    return [...map.values()].sort((a, b) => a.partition - b.partition);
  }

  async function consume(preview = true) {
    if (consuming || consumeError) return;
    const req: ConsumeReq = {
      partition: partition === '' ? null : Number(partition),
      start: buildStart(),
      limit: Number(limit),
      decode,
      key_filter: keyFilter.trim() || null,
      find_from_beginning: keyFilter.trim() ? keyFromBeginning : false,
      value_filter: valueFilter.trim() || null,
      // Server-side PII masking: redacts key/value/headers before they leave
      // the server. Only sent when the toggle is explicitly on.
      ...(maskPayloads ? { mask: true } : {}),
      // The list renders one-line snippets; big values are fetched on open.
      ...(preview ? { preview: true } : {}),
    };
    consuming = true;
    selected = null;
    fullCtl?.abort();
    tailAdded = 0;
    try {
      result = await api.post<ConsumeResp>(consumeUrl(), req);
      lastReq = req;
      // Reset tail cursors: a manual peek replaces the view and reseeds offsets.
      tailOffsets = new Map();
      // An empty range renders inline ("No messages in the selected range.");
      // no toast for what is already on screen.
    } catch (e) {
      toasts.error("Couldn't read messages", e instanceof Error ? e.message : String(e));
    } finally {
      consuming = false;
    }
  }

  /**
   * Compute the 0–100% position of `offset` within a partition's [low, high]
   * watermark range. Returns null when the watermark data is unavailable or the
   * range is empty (high === low).
   */
  function offsetPct(msg: KafkaMessage, partitions: PartitionRange[]): number | null {
    const range = partitions.find((r) => r.partition === msg.partition);
    if (!range) return null;
    const span = range.high - range.low;
    if (span <= 0) return null;
    return Math.min(100, Math.max(0, ((msg.offset - range.low) / span) * 100));
  }

  function addHeader() {
    pHeaders = [...pHeaders, { key: '', value: '' }];
  }
  function removeHeader(i: number) {
    pHeaders = pHeaders.filter((_, idx) => idx !== i);
  }

  async function produce() {
    if (producing) return;
    produceError = null;
    if (!pTombstone && !pValue) {
      pValueErr = 'Enter a value, or tick Tombstone to send a null value.';
      return;
    }
    pValueErr = null;
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
      loadDetail(true);
    } catch (e) {
      produceError = e instanceof Error ? e.message : String(e);
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
      toasts.error("Couldn't update the config", e instanceof Error ? e.message : String(e));
    } finally {
      cfgSaving = false;
    }
  }

  async function deleteTopic() {
    const typed = await confirmer.promptText(
      `Type the topic name to confirm deletion. This is irreversible.`,
      { title: `Delete topic "${topic}"`, confirmLabel: 'Delete', placeholder: topic, danger: true },
    );
    if (typed === null) return;
    if (typed !== topic) {
      // A mistyped name must not look like a silent no-op.
      toasts.warn('Topic not deleted', `The name you typed didn't match "${topic}".`);
      return;
    }
    try {
      await api.del(
        `/brokers/clusters/${cluster.id}/topics/${encodeURIComponent(topic)}?confirm=${guarded}`,
      );
      toasts.success(`Deleted ${topic}`);
      ondeleted();
    } catch (e) {
      toasts.error("Couldn't delete the topic", e instanceof Error ? e.message : String(e));
    }
  }

  // ---- export helpers -------------------------------------------------------

  let exporting = $state(false);
  async function exportMessages(fmt: 'json' | 'csv') {
    if (!result || exporting) return;
    let messages = result.messages;
    // The list holds 2 KiB previews for big values — export the real ones by
    // re-reading the same peek in full (bounded server-side at 16 MiB).
    if (lastReq && messages.some((m) => m.value?.truncated)) {
      exporting = true;
      try {
        const full: ConsumeReq = { ...lastReq, preview: false };
        messages = (await api.post<ConsumeResp>(consumeUrl(), full)).messages;
      } catch (e) {
        toasts.error("Couldn't export the messages", e instanceof Error ? e.message : String(e));
        return;
      } finally {
        exporting = false;
      }
    }
    const rows = messages.map((m) => ({
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
    if (!selected || selected.value?.truncated) return;
    try {
      await copyAsJson({
        partition: selected.partition,
        offset: selected.offset,
        timestamp_ms: selected.timestamp_ms,
        key: selected.key,
        value: selected.value,
        headers: selected.headers,
      });
      toasts.success('Copied to clipboard');
    } catch {
      toasts.error('Copy failed', 'The browser blocked the clipboard write.');
    }
  }
</script>

<div class="td">
  <header>
    <div class="title">
      <Icon name="box" size={15} />
      <span class="name" title={topic}>{topic}</span>
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
    <div class="err" role="alert">
      <Icon name="warning" size={13} />
      <span class="err-text">Couldn't load topic details. <span class="muted">{detailErr}</span></span>
      <button class="btn small" onclick={() => loadDetail(detail !== null)}>
        <Icon name="refresh" size={12} /> Retry
      </button>
    </div>
  {/if}

  {#if tab === 'messages'}
    <div class="consume-bar">
      <select bind:value={startMode} aria-label="Start position" title="Start position">
        <option value="latest">Latest</option>
        <option value="beginning">From beginning</option>
        <option value="offset">From offset</option>
        <option value="timestamp">From time</option>
      </select>
      {#if startMode === 'offset'}
        <input class="sm" type="number" bind:value={startOffset} placeholder="offset" aria-label="Start offset" />
      {/if}
      {#if startMode === 'timestamp'}
        <input class="sm" type="datetime-local" bind:value={startTs} aria-label="Start time" />
      {/if}
      <select bind:value={partition} aria-label="Partition" title="Partition">
        <option value="">All partitions</option>
        {#each detail?.partitions ?? [] as p (p.id)}
          <option value={p.id}>P{p.id}</option>
        {/each}
      </select>
      <input class="sm" type="number" bind:value={limit} min="1" max="5000" title="Max messages" aria-label="Max messages" />
      <select bind:value={decode} title="Decode value as" aria-label="Decode value as">
        <option value="auto">Auto</option>
        <option value="json">JSON</option>
        <option value="utf8">UTF-8</option>
        <option value="protobuf">Protobuf</option>
        <option value="avro">Avro</option>
        <option value="hex">Hex</option>
        <option value="base64">Base64</option>
      </select>
      <div class="filter-group">
        <input class="grow" bind:value={keyFilter} placeholder="filter key…" aria-label="Filter by key" title="Server-side key filter (case-insensitive substring)" />
        {#if keyFilter.trim()}
          <label class="chk-small" title="Scan from beginning to find older matching messages">
            <input type="checkbox" bind:checked={keyFromBeginning} /> From start
          </label>
        {/if}
      </div>
      <input class="grow" bind:value={valueFilter} placeholder="filter value…" aria-label="Filter by value" />
      <label class="auto" class:on={autoPoll} title="Append new messages every minute (incremental, capped at {TAIL_CAP})">
        <input type="checkbox" bind:checked={autoPoll} disabled={!!consumeError && !autoPoll} /> Live · 1m
      </label>
      <label
        class="auto"
        class:on={maskPayloads}
        title="Mask PII/prod — server redacts sensitive values (emails, tokens, keys) before returning messages"
      >
        <input type="checkbox" bind:checked={maskPayloads} />
        <Icon name="lock" size={12} /> Mask
      </label>
      <button class="btn primary small" onclick={() => consume()} disabled={consuming || !!consumeError}>
        {consuming ? 'Reading…' : 'Peek'}
      </button>
      {#if result && result.messages.length > 0}
        <button class="btn small" onclick={() => exportMessages('json')} disabled={exporting} title="Export all as JSON">
          <Icon name="download" size={12} /> JSON
        </button>
        <button class="btn small" onclick={() => exportMessages('csv')} disabled={exporting} title="Export all as CSV">
          <Icon name="download" size={12} /> CSV
        </button>
      {/if}
    </div>

    {#if consumeError}<p class="field-err" role="status">{consumeError}</p>{/if}

    <div class="msg-layout">
      <div class="msg-list">
        <table>
          <thead>
            <tr><th>P</th><th>Offset</th><th>Pos</th><th>Key</th><th>Time</th><th>Size</th></tr>
          </thead>
          <tbody onpointerup={inspectRow}>
            {#each result?.messages ?? [] as m (m.partition + '-' + m.offset)}
              {@const pct = result ? offsetPct(m, result.partitions) : null}
              <tr class:sel={selKey === `${m.partition}-${m.offset}`} data-message-key={`${m.partition}-${m.offset}`}>
                <td>{m.partition}</td>
                <td class="mono"><button class="message-open" aria-label={`Inspect partition ${m.partition} offset ${m.offset}`} title={`Inspect partition ${m.partition} offset ${m.offset}`} aria-pressed={selKey === `${m.partition}-${m.offset}`} onclick={() => selectMessage(m)}>{m.offset}</button></td>
                <td class="pos-cell">
                  {#if pct !== null}
                    <div class="pos-bar-wrap" title="offset {m.offset} — {pct.toFixed(1)}% through partition">
                      <div class="pos-bar" style="width:{pct}%"></div>
                    </div>
                  {:else}
                    <span class="muted">—</span>
                  {/if}
                </td>
                <td class="key" title={m.key?.text ?? undefined}>{m.key?.text ?? '∅'}</td>
                <td class="muted nowrap">{fmtTs(m.timestamp_ms)}</td>
                <td class="muted">{m.size_bytes}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if !result && !consuming}
          <p class="muted pad">Pick a start position and press <strong>Peek</strong> to read messages.</p>
        {:else if result && result.messages.length === 0}
          <p class="muted pad">No messages in the selected range.</p>
        {/if}
        {#if result?.truncated}
          <p class="muted pad small">Showing first {result.messages.length} — increase the limit for more.</p>
        {/if}
        {#if result?.masked}
          <p class="masked-badge pad small">PII masked — sensitive values were redacted server-side.</p>
        {/if}
        {#if autoPoll && tailOffsets.size > 0}
          <p class="muted pad small tail-note" role="status">
            Live tail active — appending new messages (cap {TAIL_CAP}){tailAdded > 0 ? ` · +${tailAdded} new` : ''}.
          </p>
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
            {#if result}
              {@const pct = offsetPct(selected, result.partitions)}
              {#if pct !== null}
                <span class="badge pos-badge" title="Offset position within partition watermarks">
                  {pct.toFixed(1)}%
                </span>
              {/if}
            {/if}
            {#if selected.value?.raw_base64}
              <button class="btn small" onclick={() => (rawView = !rawView)}>
                {rawView ? 'Decoded' : 'Raw'}
              </button>
            {/if}
            <button class="btn small" onclick={copySelectedAsJson} disabled={!!selected.value?.truncated} title="Copy message as JSON">
              <Icon name="copy" size={12} /> Copy
            </button>
            {#if ws.current}
              <button class="btn small" onclick={() => (sendToAgentOpen = true)} disabled={!!selected.value?.truncated} title="Send message to a running agent (redacted preview)">
                <Icon name="send" size={12} /> To agent
              </button>
            {/if}
          </div>
          {#if selected.key}
            <h5>Key <span class="muted">({selected.key.format})</span></h5>
            <pre class="payload key">{selected.key.text || '∅'}</pre>
          {/if}
          <h5>Value</h5>
          {#if selected.value?.truncated}
            <p class="muted small" role="status">
              {#if fullLoading}Loading the full value… (showing the first 2 KiB){:else if fullErr}{fullErr} Showing the first 2 KiB.
                <button class="btn small" onclick={() => selected && selectMessage(selected)}>Retry</button>{:else}Showing the first 2 KiB.{/if}
            </p>
          {/if}
          <pre class="payload">{rawView
              ? (selected.value?.raw_base64 ?? '')
              : (selected.value?.text ?? '∅')}</pre>
          {#if selected.headers.length > 0}
            <h5>Headers</h5>
            <table class="headers">
              <tbody>
                {#each selected.headers as h, i (i)}
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
    {#if !detail && !detailErr}
      <p class="muted pad">Loading partitions…</p>
    {/if}
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
      <input class="grow" bind:value={cfgName} placeholder="config name (e.g. retention.ms)" aria-label="Config name" />
      <input class="grow" bind:value={cfgValue} placeholder="value" aria-label="Config value" />
      <button
        class="btn small"
        onclick={setConfig}
        disabled={cfgSaving || !cfgName.trim()}
        title={cfgName.trim() ? `Set ${cfgName.trim()}` : 'Enter a config name first'}
      >{cfgSaving ? 'Setting…' : 'Set'}</button>
    </div>
    {#if !detail && !detailErr}
      <p class="muted pad">Loading config…</p>
    {/if}
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
          <textarea
            bind:value={pValue}
            rows="6"
            placeholder={pValueBase64 ? 'base64-encoded bytes' : '{ "hello": "world" }'}
            aria-invalid={pValueErr ? 'true' : undefined}
            oninput={() => (pValueErr = null)}
          ></textarea>
          {#if pValueErr}<span class="field-err">{pValueErr}</span>{/if}
        </label>
      {/if}
      <div class="headers-section">
        <div class="headers-title">
          <span class="dim-label">Headers</span>
          <button class="btn small" onclick={addHeader}><Icon name="plus" size={12} /> Add</button>
        </div>
        {#each pHeaders as h, i (i)}
          <div class="header-row">
            <input bind:value={h.key} placeholder="key" class="header-key" aria-label="Header {i + 1} key" />
            <input bind:value={h.value} placeholder="value" class="header-val" aria-label="Header {i + 1} value" />
            <button
              class="icon-btn danger-tiny"
              onclick={() => removeHeader(i)}
              aria-label="Remove header {i + 1}"
              title="Remove header"
            >
              <Icon name="x" size={12} />
            </button>
          </div>
        {/each}
      </div>
      {#if produceError}<p class="field-err" role="alert">{produceError}</p>{/if}
      <button class="btn primary" onclick={produce} disabled={producing}>
        {producing ? 'Producing…' : pTombstone ? 'Produce tombstone' : 'Produce message'}
      </button>
    </div>
  {/if}
</div>

{#if sendToAgentOpen && selected && ws.current}
  <ContextPacketDialog
    workspaceId={ws.current.id}
    sessionId={ws.targetAgentId}
    kind="broker"
    payload={{
      cluster: cluster.name,
      topic,
      partition: selected.partition,
      offset: selected.offset,
      key: selected.key?.text ?? null,
      value: selected.value?.text ?? null,
      headers: selected.headers,
      timestamp_ms: selected.timestamp_ms,
    }}
    onclose={() => (sendToAgentOpen = false)}
  />
{/if}

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
  header {
    gap: 12px;
  }
  header .title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    /* Long topic names truncate instead of shoving "Delete topic" off-screen. */
    min-width: 0;
  }
  header .title > :global(svg),
  header .title > .muted {
    flex: none;
    white-space: nowrap;
  }
  header .name {
    font-weight: 600;
    font-family: var(--font-mono);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  header > .btn {
    flex: none;
  }
  .subtabs {
    display: flex;
    gap: 2px;
    padding: 6px 12px 0;
    border-bottom: 1px solid var(--border);
    /* Scroll the four subtabs horizontally rather than letting the last one
       (Produce) jut off the right edge on the narrowest phones (~320px). */
    overflow-x: auto;
    flex-wrap: nowrap;
    -webkit-overflow-scrolling: touch;
  }
  .subtabs button {
    border: none;
    background: transparent;
    color: var(--text-dim);
    padding: 6px 12px;
    border-radius: var(--radius-s) var(--radius-s) 0 0;
    cursor: pointer;
    font-size: var(--fs-m);
    white-space: nowrap;
    flex: none;
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
    font-size: var(--fs-s);
  }
  .sm {
    width: 110px;
  }
  /* key filter + "from start" checkbox grouped inline */
  .filter-group {
    display: flex;
    align-items: center;
    gap: 5px;
    flex: 1;
    min-width: 100px;
  }
  .filter-group input:not([type='checkbox']) {
    flex: 1;
  }
  .chk-small {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    cursor: pointer;
  }
  .auto {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    cursor: pointer;
  }
  .auto.on {
    color: var(--accent-text);
  }
  /* Server-side PII masking active badge — shown below the message list. */
  .masked-badge {
    color: var(--accent-text);
    font-weight: 600;
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
    border-inline-end: 1px solid var(--border);
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
    font-size: var(--fs-m);
  }
  th {
    text-align: start;
    font-weight: 500;
    color: var(--text-dim);
    font-size: var(--fs-xs);
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
  .message-open {
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--accent-text);
    font: inherit;
    padding: 2px 4px;
    cursor: pointer;
    text-decoration: underline;
    text-underline-offset: 3px;
  }
  .msg-list tbody tr { cursor: pointer; }
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
  /* Offset-position bar cell */
  .pos-cell {
    width: 56px;
    padding: 5px 8px;
  }
  .pos-bar-wrap {
    width: 48px;
    height: 6px;
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    border-radius: 3px;
    overflow: hidden;
  }
  .pos-bar {
    height: 100%;
    background: var(--accent);
    border-radius: 3px;
    min-width: 2px;
  }
  /* Offset-position badge in the detail pane */
  .pos-badge {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text-dim);
  }
  .tail-note {
    padding-top: 4px;
    padding-bottom: 6px;
    color: var(--accent-text);
    opacity: 0.75;
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
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 2px 6px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--accent) 20%, transparent);
    color: var(--accent-text);
  }
  h5 {
    margin: 12px 0 4px;
    font-size: var(--fs-xs);
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-s);
  }
  .header-val {
    flex: 1;
    padding: 5px 7px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .danger-tiny {
    color: var(--danger);
  }
  .field .field-err {
    font-size: var(--fs-xs);
    color: var(--danger);
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-m);
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
    font-size: var(--fs-xs);
  }
  .pad {
    padding: 14px;
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 14px;
    font-size: var(--fs-m);
    color: var(--text);
    border-bottom: 1px solid var(--border);
  }
  .err > :global(svg) {
    color: var(--danger);
  }
  .err-text {
    flex: 1;
    min-width: 0;
  }

  /* Phone (≤640px): the message list + detail are side-by-side on desktop, which
     can't fit a ~375–430px viewport (detail has a 320px min-width). Stack them
     vertically, drop the min-width, and give each a real scrollable height so
     neither collapses to a sliver on short screens. The consume-bar already
     wraps; widen its controls so they don't jut off the edge. */
  @media (max-width: 640px) {
    .message-open { min-width: 36px; min-height: 36px; }
    /* Let the detail grow past the viewport so the produce form / message rows
       scroll into view (the brokers tab-body scrolls on phones) instead of being
       compressed behind the sticky header + bottom nav. */
    .td {
      height: auto;
      min-height: 100%;
    }
    .msg-layout {
      flex-direction: column;
      flex: none;
    }
    .msg-list {
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
      min-height: 200px;
      flex: 1 1 auto;
    }
    .msg-detail {
      width: 100%;
      min-width: 0;
      max-height: 50vh;
    }
    .consume-bar select,
    .consume-bar .sm {
      flex: 1 1 auto;
      min-width: 0;
    }
    .key {
      max-width: 120px;
    }
    .cfg-set {
      flex-wrap: wrap;
    }
  }

  /* Short viewports (phones in landscape): the message layout stays side-by-side
     (wide enough), but the panes get a guaranteed height so the message list
     doesn't shrink to a sliver whose rows fall behind the sticky chrome. The
     detail container is allowed to grow past the viewport (the brokers tab-body
     scrolls on short viewports) so the produce form / message rows scroll into
     view rather than being compressed behind the header + status bar. */
  @media (max-height: 600px) {
    .td {
      height: auto;
      min-height: 100%;
    }
    .msg-layout {
      flex: none;
    }
    .msg-list {
      min-height: 180px;
    }
    .msg-detail {
      min-height: 180px;
    }
    .produce {
      overflow: visible;
    }
  }
</style>
