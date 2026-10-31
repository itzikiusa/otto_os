///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import Badge from '../../lib/components/Badge.svelte';
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
import { untrack } from 'svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { api } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { confirmProd } from '../../lib/confirmProd';
import { formatBytes } from '../../lib/metric-format';
import { loadErrorText } from '../../lib/loadError';
import { pollWhileVisible } from '../../lib/poll';
import { adaptiveCadence } from '../../lib/pollBackoff';
import { TableWindow } from '../../lib/tableWindow.svelte';
import { copyAsJson, downloadJson, exportCsv } from '../../lib/components/exporters';
import type { BrokerCluster, ConsumeReq, ConsumeResp, KafkaMessage, MessageHeader,
    PartitionRange, ProduceReq, StartPosition, TopicDetail, ValueFormat } from '../../lib/api/types';
import { ws } from '../../lib/stores/workspace.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { brokersTopicPort } from '../../lib/uiCommands/brokers';
import ContextPacketDialog from '../../lib/components/ContextPacketDialog.svelte';
;

  interface Props {
    cluster: BrokerCluster;
    topic: string;
    ondeleted: () => void;
  };;

  type Tab = 'messages' | 'partitions' | 'config' | 'produce';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ── Send-to-agent dialog (B2a) ─────────────────────────────────────────────
  let sendToAgentOpen = $state(false);

  let { cluster, topic, ondeleted }: Props = $props();

  const guarded = $derived(cluster.read_only || cluster.environment === 'prod');

  const TOPIC_TABS: TabItem<Tab>[] = [
    { id: 'messages', label: 'Messages' },
    { id: 'partitions', label: 'Partitions' },
    { id: 'config', label: 'Config' },
    { id: 'produce', label: 'Produce' },
  ];
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
  // Window the message table (SC-08): a 5000-message peek mounts only the
  // visible slice between two spacer rows. The partition ranges are indexed
  // once per result so each visible row's position bar is an O(1) lookup.
  const messages = $derived(result?.messages ?? []);
  const rangeByPartition = $derived(new Map((result?.partitions ?? []).map((r) => [r.partition, r])));
  const tw = new TableWindow();
  const win = $derived(tw.range(messages.length));
  let msgListEl = $state<HTMLDivElement>();
  $effect(() => {
    void win;
    tw.measure(msgListEl);
  });
  // ⌘F over every peeked message ROW (what the table shows — the value lives
  // in the inspector), not just the mounted slice.
  $effect(() =>
    tw.findRows(
      () => msgListEl,
      () => messages,
      (m) =>
        [
          m.partition,
          m.offset,
          rangePct(m, rangeByPartition.get(m.partition)) === null ? '—' : '',
          m.key?.text ?? '∅',
          fmtTs(m.timestamp_ms),
          m.size_bytes,
        ].join('\n'),
    ),
  );
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
  // Auto-refresh (live-tail). Each tick re-uses the offsets accumulated so far;
  // toggling off clears them. Adaptive cadence: 3 s while ticks keep returning
  // new messages, doubling to 15 s while the topic is idle (each tick is a
  // consume through the daemon, possibly over an SSH tunnel).
  let autoPoll = $state(false);
  const TAIL_MIN_MS = 3000;
  const TAIL_MAX_MS = 15_000;
  /** Messages the last tail tick appended (inline badge — no per-tick toast). */
  let tailAdded = $state(0);
  let tailError = $state<string | null>(null);
  let tailLastSuccess = $state<number | null>(null);
  /** Bumped whenever a tail tick appends messages (drives the adaptive cadence;
   *  `tailAdded` alone can't tell a failed tick from an empty one). */
  let tailSeq = 0;

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
    tailError = null;
    tailLastSuccess = null;
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
    const cadence = adaptiveCadence({ min: TAIL_MIN_MS, max: TAIL_MAX_MS });
    const poller = pollWhileVisible(
      async () => {
        if (consuming) return;
        const before = tailSeq;
        await untrack(() => consumeWithTail(true));
        cadence.record(tailSeq !== before);
      },
      {
        get ms() {
          return cadence.ms;
        },
        immediate: false,
      },
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
    const owner = consumeUrl();
    const hasCursors = tailOffsets.size > 0;

    if (!incremental || !hasCursors) {
      // Seed pass: normal peek, then seed offsets from what comes back.
      await consume();
      if (owner === consumeUrl() && result && !tailError) updateTailOffsets(result.messages);
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
        r = await api.post<ConsumeResp>(owner, req);
      } catch (e) {
        if (owner !== consumeUrl()) return;
        tailError = loadErrorText(e);
        tailAdded = 0;
        return; // Preserve the last good buffer; polling retries automatically.
      }
      if (owner !== consumeUrl()) return;
      tailError = null;
      tailLastSuccess = Date.now();
      tailAdded = r.messages.length;
      if (r.messages.length > 0) {
        tailSeq += 1;
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
    const owner = consumeUrl();
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
    const prev = selected;
    selected = null;
    fullCtl?.abort();
    tailAdded = 0;
    try {
      const next = await api.post<ConsumeResp>(owner, req);
      if (owner !== consumeUrl()) return;
      result = next;
      tailError = null;
      tailLastSuccess = Date.now();
      lastReq = req;
      tw.reset(msgListEl);
      // Reset tail cursors: a manual peek replaces the view and reseeds offsets.
      tailOffsets = new Map();
      // Never land on an empty "Pick a message" pane when there are messages
      // (layout.md): keep the previous message if it is still in range, else
      // open the first. Phones stack list over detail, so they stay on the list.
      if (!viewport.isPhone && next.messages.length) {
        const keep = prev && next.messages.find((m) => m.partition === prev.partition && m.offset === prev.offset);
        selectMessage(keep ?? next.messages[0]);
      }
      // An empty range renders inline ("No messages in the selected range.");
      // no toast for what is already on screen.
    } catch (e) {
      if (owner !== consumeUrl()) return;
      tailError = loadErrorText(e);
      if (!autoPoll) toasts.error("Couldn’t read messages", tailError);
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
    return rangePct(msg, partitions.find((r) => r.partition === msg.partition));
  }
  function rangePct(msg: KafkaMessage, range: PartitionRange | undefined): number | null {
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
      // Key, value size and partition: what a produce actually sends.
      const valueBytes = pTombstone
        ? null
        : pValueBase64
          ? Math.floor((pValue.length * 3) / 4)
          : new TextEncoder().encode(pValue).length;
      const ok = await confirmProd({
        env: cluster.environment,
        verb: 'Produce',
        title: cluster.environment === 'prod' ? 'Produce to production?' : 'Produce to guarded cluster?',
        where: `Topic ${topic} · ${cluster.name}${cluster.read_only ? ' (read-only cluster)' : ''}`,
        what: [
          `Key: ${pKey || '(none)'}`,
          `Value: ${valueBytes === null ? 'null (tombstone)' : formatBytes(valueBytes)}`,
          `Partition: ${pPartition === '' ? 'chosen by the producer' : pPartition}`,
        ].join(' · '),
        danger: true,
      });
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
      toastError("Couldn’t update the config", e);
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
      toasts.warn('Topic not deleted', `The name you typed didn’t match "${topic}".`);
      return;
    }
    try {
      await api.del(
        `/brokers/clusters/${cluster.id}/topics/${encodeURIComponent(topic)}?confirm=${guarded}`,
      );
      toasts.success(`Deleted ${topic}`);
      ondeleted();
    } catch (e) {
      toastError("Couldn’t delete the topic", e);
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
        toastError("Couldn’t export the messages", e);
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
      toasts.error('Couldn’t copy', 'The browser blocked the clipboard write. Select the message text and copy it manually.');
    }
  }
  $effect(() => { if (detailErr) announce(`Couldn’t load topic details. ${detailErr}`, { assertive: true }); });
  $effect(() => { if (consumeError) announce(consumeError, { assertive: true }); });
  $effect(() => { if (produceError) announce(produceError, { assertive: true }); });
  $effect(() => { if (autoPoll && tailError) announce(`Live tail interrupted — retrying. ${tailError}`, { assertive: true }); });
  // A derived flag, so a tail tick (a new offsets map) doesn't re-announce.
  const tailActive = $derived(autoPoll && tailOffsets.size > 0 && !tailError);
  $effect(() => { if (tailActive) announce('Live tail active — appending new messages.'); });
  $effect(() => { if (selected?.value?.truncated && fullErr) announce(`${fullErr} Showing the first 2 KiB.`, { assertive: true }); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`td`,});
   { svelteHTML.createElement("header", {});
     { svelteHTML.createElement("div", { "class":`title`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`box`,"size":14,}});}
       { svelteHTML.createElement("span", {   "class":`name`,"title":topic,});topic; }
      if(detail){ { svelteHTML.createElement("span", { "class":`muted`,}); detail.partitions.length;  detail.message_count.toLocaleString();  }}
     }
     { svelteHTML.createElement("button", {   "class":`btn small danger`,"onclick":deleteTopic,});  }
   }

  
   { svelteHTML.createElement("div", { "class":`subtabs`,});
     { const $$_sbaT2C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT2C({ target: __sveltets_2_any(), props: {          "label":`Topic view`,"idBase":`topic`,"tabs":TOPIC_TABS,"value":tab,"onchange":(id) => (tab = id),}});}
   }

  if(detailErr){
     { svelteHTML.createElement("div", { "class":`err`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});}
       { svelteHTML.createElement("span", { "class":`err-text`,});     { svelteHTML.createElement("span", { "class":`muted`,});detailErr; } }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => loadDetail(detail !== null),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} 
       }
     }
  }

   { svelteHTML.createElement("div", {       "class":`tpanel`,"role":`tabpanel`,"id":`topic-panel-${tab}`,"aria-labelledby":`topic-tab-${tab}`,});
  if(tab === 'messages'){
     { svelteHTML.createElement("div", { "class":`consume-bar`,});
       { svelteHTML.createElement("select", {     "bind:value":startMode,"aria-label":`Start position`,"title":`Start position`,});/*Ωignore_startΩ*/() => startMode = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", { "value":`latest`,});  }
         { svelteHTML.createElement("option", { "value":`beginning`,});  }
         { svelteHTML.createElement("option", { "value":`offset`,});  }
         { svelteHTML.createElement("option", { "value":`timestamp`,});  }
       }
      if(startMode === 'offset'){
         { svelteHTML.createElement("input", {          "class":`sm`,"type":`number`,"bind:value":startOffset,"placeholder":`offset`,"aria-label":`Start offset`,});/*Ωignore_startΩ*/() => startOffset = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      }
      if(startMode === 'timestamp'){
         { svelteHTML.createElement("input", {        "class":`sm`,"type":`datetime-local`,"bind:value":startTs,"aria-label":`Start time`,});/*Ωignore_startΩ*/() => startTs = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      }
       { svelteHTML.createElement("select", {     "bind:value":partition,"aria-label":`Partition`,"title":`Partition`,});/*Ωignore_startΩ*/() => partition = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});  }
           for(let p of __sveltets_2_ensureArray(detail?.partitions ?? [])){p.id;
           { svelteHTML.createElement("option", { "value":p.id,}); p.id; }
        }
       }
       { svelteHTML.createElement("input", {              "class":`sm`,"type":`number`,"bind:value":limit,"min":`1`,"max":`5000`,"title":`Max messages`,"aria-label":`Max messages`,});/*Ωignore_startΩ*/() => limit = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("select", {     "bind:value":decode,"title":`Decode value as`,"aria-label":`Decode value as`,});/*Ωignore_startΩ*/() => decode = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", { "value":`auto`,});  }
         { svelteHTML.createElement("option", { "value":`json`,});  }
         { svelteHTML.createElement("option", { "value":`utf8`,});  }
         { svelteHTML.createElement("option", { "value":`protobuf`,});  }
         { svelteHTML.createElement("option", { "value":`avro`,});  }
         { svelteHTML.createElement("option", { "value":`hex`,});  }
         { svelteHTML.createElement("option", { "value":`base64`,});  }
       }
       { svelteHTML.createElement("div", { "class":`filter-group`,});
         { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`grow`,"bind:value":keyFilter,"placeholder":`filter key…`,"aria-label":`Filter by key`,"title":`Server-side key filter (case-insensitive substring)`,});/*Ωignore_startΩ*/() => keyFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
        if(keyFilter.trim()){
           { svelteHTML.createElement("label", {   "class":`chk-small`,"title":`Scan from beginning to find older matching messages`,});
             { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":keyFromBeginning,});/*Ωignore_startΩ*/() => keyFromBeginning = __sveltets_2_any(null);/*Ωignore_endΩ*/}  
           }
        }
       }
       { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`grow`,"bind:value":valueFilter,"placeholder":`filter value…`,"aria-label":`Filter by value`,});/*Ωignore_startΩ*/() => valueFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("label", {    "class":`auto`,"title":`Append new messages every 3 s, backing off to 15 s while none arrive (incremental, capped at ${TAIL_CAP})`,});autoPoll;
         { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":autoPoll,"disabled":!!consumeError && !autoPoll,});/*Ωignore_startΩ*/() => autoPoll = __sveltets_2_any(null);/*Ωignore_endΩ*/} 
       }
       { svelteHTML.createElement("label", {      "class":`auto`,"title":`Mask PII/prod — server redacts sensitive values (emails, tokens, keys) before returning messages`,});maskPayloads;
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":maskPayloads,});/*Ωignore_startΩ*/() => maskPayloads = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});} 
       }
       { svelteHTML.createElement("button", {     "class":`btn primary small`,"onclick":() => consume(),"disabled":consuming || !!consumeError,});
        consuming ? 'Reading…' : 'Peek';
       }
      if(result && result.messages.length > 0){
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => exportMessages('json'),"disabled":exporting,"title":`Export all as JSON`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} 
         }
         { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => exportMessages('csv'),"disabled":exporting,"title":`Export all as CSV`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} 
         }
      }
     }

    if(consumeError){ { svelteHTML.createElement("p", { "class":`field-err`,});consumeError; }}

     { svelteHTML.createElement("div", { "class":`msg-layout`,});
       { const $$_div3 = svelteHTML.createElement("div", {        "class":`msg-list`,"onscroll":tw.onscroll,});tw.active(messages.length);msgListEl = $$_div3;tw.viewH= $$_div3.clientHeight;
         { svelteHTML.createElement("table", {});
           { svelteHTML.createElement("thead", {});
             { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } }
           }
           { svelteHTML.createElement("tbody", { "onpointerup":inspectRow,});
            if(win.top){ { svelteHTML.createElement("tr", {   "class":`tw-spacer`,"aria-hidden":`true`,}); { svelteHTML.createElement("td", {   "colspan":6,"style":`height:${win.top}px`,}); } }}
               for(let m of __sveltets_2_ensureArray((messages.slice(win.start, win.end)))){m.partition + '-' + m.offset;
              const pct = rangePct(m, rangeByPartition.get(m.partition));
               { svelteHTML.createElement("tr", {   ...__sveltets_2_empty({"data-message-key":`${m.partition}-${m.offset}`}),});selKey === `${m.partition}-${m.offset}`;
                 { svelteHTML.createElement("td", {});m.partition; }
                 { svelteHTML.createElement("td", { "class":`mono`,}); { svelteHTML.createElement("button", {         "class":`message-open`,"aria-label":`Inspect partition ${m.partition} offset ${m.offset}`,"title":`Inspect partition ${m.partition} offset ${m.offset}`,"aria-pressed":selKey === `${m.partition}-${m.offset}`,"onclick":() => selectMessage(m),});m.offset; } }
                 { svelteHTML.createElement("td", { "class":`pos-cell`,});
                  if(pct !== null){
                     { svelteHTML.createElement("div", {   "class":`pos-bar-wrap`,"title":`offset ${m.offset} — ${pct.toFixed(1)}% through partition`,});
                       { svelteHTML.createElement("div", {   "class":`pos-bar`,"style":`width:${pct}%`,}); }
                     }
                  }else{
                     { svelteHTML.createElement("span", { "class":`muted`,});  }
                  }
                 }
                 { svelteHTML.createElement("td", {   "class":`key`,"title":m.key?.text ?? undefined,});m.key?.text ?? '∅'; }
                 { svelteHTML.createElement("td", { "class":`muted nowrap`,});fmtTs(m.timestamp_ms); }
                 { svelteHTML.createElement("td", { "class":`muted`,});m.size_bytes; }
               }
            }
            if(win.bottom){ { svelteHTML.createElement("tr", {   "class":`tw-spacer`,"aria-hidden":`true`,}); { svelteHTML.createElement("td", {   "colspan":6,"style":`height:${win.bottom}px`,}); } }}
           }
         }
        if(!result && !consuming){
           { svelteHTML.createElement("p", { "class":`muted pad`,});       { svelteHTML.createElement("strong", {});  }    }
        } else if (result && result.messages.length === 0){
           { svelteHTML.createElement("p", { "class":`muted pad`,});      }
        }
        if(result?.truncated){
           { svelteHTML.createElement("p", { "class":`muted pad small`,});  result.messages.length;       }
        }
        if(result?.masked){
           { svelteHTML.createElement("p", { "class":`masked-note pad small`,});        }
        }
        if(autoPoll && (tailOffsets.size > 0 || tailError)){
           { svelteHTML.createElement("p", { "class":`muted pad small tail-note`,});
            if(tailError){     tailError;tailLastSuccess ? ` Last received successfully at ${fmtTs(tailLastSuccess)}.` : '';
               { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":consuming,"onclick":() => consumeWithTail(true),});  }
            }else{        TAIL_CAP; tailAdded > 0 ? ` · +${tailAdded} new` : ''; }
           }
        }
       }

       { svelteHTML.createElement("div", { "class":`msg-detail`,});
        if(selected){
           { svelteHTML.createElement("div", { "class":`md-head`,});
             { svelteHTML.createElement("span", { "class":`mono`,}); selected.partition;   selected.offset; }
             { svelteHTML.createElement("span", { "class":`muted`,});fmtTs(selected.timestamp_ms); }
            if(selected.value?.format){
               { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":`${selected.value.format}${selected.value.schema_id != null ? ` #${selected.value.schema_id}` : ''}`,}});}
            }
            if(selected.headers.length > 0){
               { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {  "label":plural(selected.headers.length, 'header'),}});}
            }
            if(result){
              const pct = offsetPct(selected, result.partitions);
              if(pct !== null){
                 { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "label":`${pct.toFixed(1)}%`,"title":`Offset position within partition watermarks`,}});}
              }
            }
            if(selected.value?.raw_base64){
               { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (rawView = !rawView),});
                rawView ? 'Decoded' : 'Raw';
               }
            }
             { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":copySelectedAsJson,"disabled":!!selected.value?.truncated,"title":`Copy message as JSON`,});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} 
             }
            if(ws.current){
               { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => (sendToAgentOpen = true),"disabled":!!selected.value?.truncated,"title":`Send message to a running agent (redacted preview)`,});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`send`,"size":12,}});}  
               }
            }
           }
          if(selected.key){
             { svelteHTML.createElement("h5", {});  { svelteHTML.createElement("span", { "class":`muted`,}); selected.key.format;  } }
             { svelteHTML.createElement("pre", { "class":`payload key`,});selected.key.text || '∅'; }
          }
           { svelteHTML.createElement("h5", {});  }
          if(selected.value?.truncated){
             { svelteHTML.createElement("p", { "class":`muted small`,});
              if(fullLoading){        } else if (fullErr){fullErr;     
                 { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => selected && selectMessage(selected),});  }}else{    }
             }
          }
           { svelteHTML.createElement("pre", { "class":`payload`,});rawView
              ? (selected.value?.raw_base64 ?? '')
              : (selected.value?.text ?? '∅'); }
          if(selected.headers.length > 0){
             { svelteHTML.createElement("h5", {});  }
             { svelteHTML.createElement("table", { "class":`headers`,});
               { svelteHTML.createElement("tbody", {});
                    for(let h of __sveltets_2_ensureArray(selected.headers)){let i = 1;i;
                   { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("td", { "class":`mono`,});h.key; } { svelteHTML.createElement("td", {});h.value; } }
                }
               }
             }
          }
        }else{
           { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {      "icon":`note`,"title":`Pick a message`,"body":`${messages.length} ${messages.length === 1 ? 'message' : 'messages'} loaded. Select one to inspect its key, value and headers.`,}});}
        }
       }
     }
  } else if (tab === 'partitions'){
    if(!detail && !detailErr){
       { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":4,"label":`partitions`,}});} }
    } else if (detail){
     { svelteHTML.createElement("table", { "class":`grid`,});
       { svelteHTML.createElement("thead", {});
         { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } }
       }
       { svelteHTML.createElement("tbody", {});
           for(let p of __sveltets_2_ensureArray(detail?.partitions ?? [])){p.id;
           { svelteHTML.createElement("tr", {});
             { svelteHTML.createElement("td", {});p.id; }
             { svelteHTML.createElement("td", {});p.leader; }
             { svelteHTML.createElement("td", { "class":`mono`,});p.replicas.join(', '); }
             { svelteHTML.createElement("td", { "class":`mono`,});p.isr.join(', '); }
             { svelteHTML.createElement("td", { "class":`muted`,});p.low; }
             { svelteHTML.createElement("td", { "class":`muted`,});p.high; }
             { svelteHTML.createElement("td", {});p.message_count.toLocaleString(); }
           }
        }
       }
     }
    }
  } else if (tab === 'config'){
     { svelteHTML.createElement("div", { "class":`cfg-set`,});
       { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`grow`,"bind:value":cfgName,"placeholder":`retention.ms`,"aria-label":`Config name`,});/*Ωignore_startΩ*/() => cfgName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`grow`,"bind:value":cfgValue,"placeholder":`604800000`,"aria-label":`Config value`,});/*Ωignore_startΩ*/() => cfgValue = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":setConfig,"disabled":cfgSaving || !cfgName.trim(),"title":cfgName.trim() ? `Set ${cfgName.trim()}` : 'Enter a config name first',});cfgSaving ? 'Setting…' : 'Set'; }
     }
    if(!detail && !detailErr){
       { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":6,"label":`config`,}});} }
    } else if (detail){
     { svelteHTML.createElement("table", { "class":`grid`,});
       { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } } }
       { svelteHTML.createElement("tbody", {});
           for(let c of __sveltets_2_ensureArray(detail?.configs ?? [])){c.name;
           { svelteHTML.createElement("tr", { });!c.is_default;
             { svelteHTML.createElement("td", { "class":`mono`,});c.name; }
             { svelteHTML.createElement("td", {});c.is_sensitive ? '••••••' : (c.value ?? ''); }
             { svelteHTML.createElement("td", { "class":`muted small`,});c.source; }
           }
        }
       }
     }
    }
  } else if (tab === 'produce'){
     { svelteHTML.createElement("div", { "class":`produce`,});
       { svelteHTML.createElement("div", { "class":`produce-opts`,});
         { svelteHTML.createElement("label", { "class":`chk-opt`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":pTombstone,});/*Ωignore_startΩ*/() => pTombstone = __sveltets_2_any(null);/*Ωignore_endΩ*/}    }
         { svelteHTML.createElement("label", { "class":`chk-opt`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":pKeyBase64,});/*Ωignore_startΩ*/() => pKeyBase64 = __sveltets_2_any(null);/*Ωignore_endΩ*/}    }
        if(!pTombstone){
           { svelteHTML.createElement("label", { "class":`chk-opt`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":pValueBase64,});/*Ωignore_startΩ*/() => pValueBase64 = __sveltets_2_any(null);/*Ωignore_endΩ*/}    }
        }
       }
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {}); pKeyBase64 ? ' — base64' : ''; }
         { svelteHTML.createElement("input", {      "dir":`auto`,"bind:value":pKey,"placeholder":pKeyBase64 ? 'base64-encoded bytes' : 'string key',});/*Ωignore_startΩ*/() => pKey = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("select", { "bind:value":pPartition,});/*Ωignore_startΩ*/() => pPartition = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", {"value":"",});  }
             for(let p of __sveltets_2_ensureArray(detail?.partitions ?? [])){p.id; { svelteHTML.createElement("option", { "value":p.id,}); p.id; }}
         }
       }
      if(!pTombstone){
         { svelteHTML.createElement("label", { "class":`field grow`,});
           { svelteHTML.createElement("span", {}); pValueBase64 ? ' — base64' : ''; }
           { svelteHTML.createElement("textarea", {            "dir":`ltr`,"bind:value":pValue,"rows":6,"placeholder":pValueBase64 ? 'base64-encoded bytes' : '{ "hello": "world" }',"aria-invalid":pValueErr ? 'true' : undefined,"oninput":() => (pValueErr = null),});/*Ωignore_startΩ*/() => pValue = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
          if(pValueErr){ { svelteHTML.createElement("span", { "class":`field-err`,});pValueErr; }}
         }
      }
       { svelteHTML.createElement("div", { "class":`headers-section`,});
         { svelteHTML.createElement("div", { "class":`headers-title`,});
           { svelteHTML.createElement("span", { "class":`dim-label`,});  }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":addHeader,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
         }
            for(let h of __sveltets_2_ensureArray(pHeaders)){let i = 1;i;
           { svelteHTML.createElement("div", { "class":`header-row`,});
             { svelteHTML.createElement("input", {          "dir":`ltr`,"bind:value":h.key,"placeholder":`key`,"class":`header-key`,"aria-label":`Header ${i + 1} key`,});/*Ωignore_startΩ*/() => h.key = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("input", {          "dir":`ltr`,"bind:value":h.value,"placeholder":`value`,"class":`header-val`,"aria-label":`Header ${i + 1} value`,});/*Ωignore_startΩ*/() => h.value = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("button", {         "class":`icon-btn danger-tiny`,"onclick":() => removeHeader(i),"aria-label":`Remove header ${i + 1}`,"title":`Remove header`,});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
           }
        }
       }
      if(produceError){ { svelteHTML.createElement("p", { "class":`field-err`,});produceError; }}
       { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":produce,"disabled":producing,});
        producing ? 'Producing…' : pTombstone ? 'Produce tombstone' : 'Produce message';
       }
     }
  }
   }
 }

if(sendToAgentOpen && selected && ws.current){
   { const $$_golaiDtekcaPtxetnoC0C = __sveltets_2_ensureComponent(ContextPacketDialog); new $$_golaiDtekcaPtxetnoC0C({ target: __sveltets_2_any(), props: {           "workspaceId":ws.current.id,"sessionId":ws.targetAgentId,"kind":`broker`,"payload":{
      cluster: cluster.name,
      topic,
      partition: selected.partition,
      offset: selected.offset,
      key: selected.key?.text ?? null,
      value: selected.value?.text ?? null,
      headers: selected.headers,
      timestamp_ms: selected.timestamp_ms,
    },"onclose":() => (sendToAgentOpen = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TopicDetail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TopicDetail__SvelteComponent_ = ReturnType<typeof TopicDetail__SvelteComponent_>;
/*Ωignore_endΩ*/export default TopicDetail__SvelteComponent_;