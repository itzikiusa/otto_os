<script lang="ts">
  // SQS: queue list with approximate counts → queue detail tabs: Messages
  // (Peek N, JSON-pretty body viewer, delete-message per row [Edit]), Send
  // (body + attributes, FIFO fields only for `.fifo`) [Edit], Attributes,
  // Metrics (CloudWatch via MetricsPanel), Redrive (DLQ → source) [Edit].
  // Purge lives in the ⋯ menu [Edit, typed].
  import { untrack } from 'svelte';
  import { aws } from '../../lib/stores/aws.svelte';
  import { awsApi, isLoginRequired } from '../../lib/api/aws';
  import { auth } from '../../lib/stores/auth.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import JsonTree from '../database/JsonTree.svelte';
  import ViewToolbar from './ViewToolbar.svelte';
  import MetricsPanel from './MetricsPanel.svelte';
  import { prettyJson } from './util';
  import type { AwsAccount, SqsMessage, SqsQueue } from '../../lib/api/types';

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  }
  let { account, onsignin }: Props = $props();

  const canEdit = $derived(auth.can('aws_sqs', 'edit'));
  const queues = $derived(aws.sqsQueues[account.id] ?? null);
  let loading = $state(false);
  let error = $state('');
  let filter = $state('');
  let auto = $state(false);
  let selectedUrl = $state<string | null>(null);
  const selected = $derived(queues?.find((q) => q.url === selectedUrl) ?? null);
  const attrs = $derived(selectedUrl ? (aws.sqsAttrs[selectedUrl] ?? null) : null);
  type Tab = 'messages' | 'send' | 'attributes' | 'metrics' | 'redrive';
  let tab = $state<Tab>('messages');

  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    const list = queues ?? [];
    return q ? list.filter((x) => x.name.toLowerCase().includes(q)) : list;
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      const list = await aws.loadSqsQueues(account.id);
      error = '';
      // Approximate counts: fan out, but cap so a 500-queue account doesn't
      // fire 500 CLI calls on open (the rest load when selected).
      await Promise.all(list.slice(0, 40).map((q) => aws.loadSqsAttrs(account.id, q.url)));
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    untrack(() => {
      if (!queues) void load();
    });
  });

  function select(q: SqsQueue): void {
    selectedUrl = q.url;
    messages = [];
    void aws.loadSqsAttrs(account.id, q.url);
  }

  // ── messages ──
  let messages = $state<SqsMessage[]>([]);
  let peekN = $state(10);
  let peeking = $state(false);
  let openMsg = $state<string | null>(null);

  async function peek(): Promise<void> {
    if (!selectedUrl) return;
    peeking = true;
    try {
      const r = await awsApi.sqsPeek(account.id, { url: selectedUrl, max: peekN, visibility_timeout: 0 });
      messages = r.messages;
      if (r.messages.length === 0) toasts.info('No messages visible right now');
    } catch (e) {
      toasts.error('Peek failed', e instanceof Error ? e.message : String(e));
    } finally {
      peeking = false;
    }
  }

  async function deleteMessage(m: SqsMessage): Promise<void> {
    if (!selected) return;
    const ok = await confirmer.ask(`Delete message ${m.message_id} from “${selected.name}”? This cannot be undone.`, {
      title: 'Delete message',
    });
    if (!ok) return;
    try {
      await awsApi.sqsDeleteMessage(account.id, selected.url, m.receipt_handle);
      messages = messages.filter((x) => x.message_id !== m.message_id);
      toasts.success('Message deleted');
      void aws.loadSqsAttrs(account.id, selected.url);
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  function parsedBody(m: SqsMessage): unknown {
    try {
      return JSON.parse(m.body);
    } catch {
      return undefined;
    }
  }

  // ── send ──
  let sendBody = $state('');
  let sendDelay = $state(0);
  let sendGroup = $state('');
  let sendDedup = $state('');
  let sendAttrs = $state<{ k: string; v: string }[]>([]);
  let sending = $state(false);

  async function send(): Promise<void> {
    if (!selected || !sendBody.trim()) return;
    sending = true;
    try {
      const message_attributes: Record<string, { DataType: string; StringValue: string }> = {};
      for (const a of sendAttrs) if (a.k.trim()) message_attributes[a.k.trim()] = { DataType: 'String', StringValue: a.v };
      const r = await awsApi.sqsSend(account.id, {
        url: selected.url,
        body: sendBody,
        delay_seconds: sendDelay || undefined,
        group_id: selected.fifo ? sendGroup || undefined : undefined,
        dedup_id: selected.fifo ? sendDedup || undefined : undefined,
        message_attributes: Object.keys(message_attributes).length ? message_attributes : undefined,
      });
      toasts.success('Message sent', r.message_id);
      void aws.loadSqsAttrs(account.id, selected.url);
    } catch (e) {
      toasts.error('Send failed', e instanceof Error ? e.message : String(e));
    } finally {
      sending = false;
    }
  }

  // ── purge / redrive ──
  async function purge(q: SqsQueue): Promise<void> {
    const typed = await confirmer.promptText(
      `Purge ALL messages from “${q.name}”? Type the queue name to confirm.`,
      { title: 'Purge queue', confirmLabel: 'Purge', placeholder: q.name },
    );
    if (typed === null) return;
    if (typed !== q.name) {
      toasts.warn('Name did not match — purge cancelled');
      return;
    }
    try {
      await awsApi.sqsPurge(account.id, q.url, typed);
      toasts.success('Purge started', 'SQS empties the queue over the next ~60 s');
      void aws.loadSqsAttrs(account.id, q.url);
    } catch (e) {
      toasts.error('Purge failed', e instanceof Error ? e.message : String(e));
    }
  }

  let redriveDest = $state('');
  let redriving = $state(false);
  async function redrive(): Promise<void> {
    const src = attrs?.attributes.QueueArn;
    if (!src || !selected) return;
    const ok = await confirmer.ask(
      `Move every message from “${selected.name}” back to ${redriveDest.trim() || 'its original source queue(s)'}?`,
      { title: 'Start redrive', confirmLabel: 'Start', danger: false },
    );
    if (!ok) return;
    redriving = true;
    try {
      const r = await awsApi.sqsRedrive(account.id, { source_arn: src, destination_arn: redriveDest.trim() || undefined });
      toasts.success('Redrive started', r.task_handle);
    } catch (e) {
      toasts.error('Redrive failed', e instanceof Error ? e.message : String(e));
    } finally {
      redriving = false;
    }
  }

  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${what}`);
    } catch (e) {
      toasts.error('Copy failed', e instanceof Error ? e.message : String(e));
    }
  }

  function queueMenu(e: MouseEvent | KeyboardEvent, q: SqsQueue): void {
    ctxMenu.show(e, [
      { label: 'Open', icon: 'send', action: () => select(q) },
      { label: 'Refresh counts', icon: 'refresh', action: () => void aws.loadSqsAttrs(account.id, q.url) },
      { label: 'Copy URL', icon: 'copy', action: () => void copy(q.url, 'queue URL') },
      ...(canEdit ? [{ separator: true }, { label: 'Purge queue…', icon: 'trash', danger: true, action: () => void purge(q) }] : []),
    ]);
  }

  const dlqSource = $derived.by(() => {
    // Queues whose RedrivePolicy targets the selected queue (so Redrive knows
    // this is a DLQ) — best effort from the cached attributes.
    const arn = attrs?.attributes.QueueArn;
    if (!arn) return [];
    return Object.entries(aws.sqsAttrs)
      .filter(([, a]) => a.dlq_target_arn === arn)
      .map(([url]) => queues?.find((q) => q.url === url)?.name ?? url);
  });

  const showList = $derived(!viewport.isMobile || !selected);
  const showDetail = $derived(!viewport.isMobile || !!selected);
  const loginNeeded = $derived(isLoginRequired(new Error(error)));
</script>

<ViewToolbar
  title="SQS"
  subtitle={`${queues?.length ?? 0} queues`}
  bind:filter
  filterPlaceholder="Filter queues…"
  {loading}
  bind:auto
  onrefresh={() => void load()}
/>

<div class="split" class:mobile={viewport.isMobile}>
  {#if showList}
    <div class="list">
      {#if loading && !queues}
        <div class="pad"><Skeleton rows={8} /></div>
      {:else if error}
        <EmptyState icon="cloud" title="Couldn't list queues" body={error} actionLabel={loginNeeded ? 'Sign in' : 'Retry'} onaction={loginNeeded ? onsignin : () => void load()} />
      {:else if shown.length === 0}
        <EmptyState icon="send" title={filter ? 'No matching queues' : 'No queues'} />
      {:else}
        <table class="tbl">
          <thead><tr><th>Queue</th><th class="num" title="ApproximateNumberOfMessages">Avail</th><th class="num hide-sm" title="NotVisible">In-flight</th><th class="num hide-sm" title="Delayed">Delayed</th></tr></thead>
          <tbody>
            {#each shown as q (q.url)}
              {@const a = aws.sqsAttrs[q.url]}
              <tr
                class="trow"
                class:sel={q.url === selectedUrl}
                tabindex="0"
                onclick={() => select(q)}
                onkeydown={(e) => { if (e.key === 'Enter') select(q); }}
                oncontextmenu={(e) => queueMenu(e, q)}
              >
                <td class="name" title={q.url}>
                  <Icon name="send" size={12} />
                  <span class="qn">{q.name}</span>
                  {#if q.fifo}<span class="tag">FIFO</span>{/if}
                  {#if a?.dlq_target_arn}<span class="tag dim" title={`DLQ: ${a.dlq_target_arn}`}>→DLQ</span>{/if}
                </td>
                <td class="num mono">{a ? a.approx_messages : '…'}</td>
                <td class="num mono hide-sm">{a ? a.approx_not_visible : '…'}</td>
                <td class="num mono hide-sm">{a ? a.approx_delayed : '…'}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    </div>
  {/if}

  {#if showDetail}
    <div class="detail">
      {#if !selected}
        <EmptyState icon="send" title="Pick a queue" body="Peek messages, send, inspect attributes or redrive." />
      {:else}
        <div class="dhead">
          {#if viewport.isMobile}
            <button class="back" onclick={() => (selectedUrl = null)} aria-label="Back to queues"><Icon name="chevronLeft" size={14} /></button>
          {/if}
          <strong class="qname" title={selected.url}>{selected.name}</strong>
          {#if selected.fifo}<span class="tag">FIFO</span>{/if}
          {#if attrs}<span class="dim counts mono">{attrs.approx_messages} avail · {attrs.approx_not_visible} in-flight · {attrs.approx_delayed} delayed</span>{/if}
          <button class="more" onclick={(e) => selected && queueMenu(e, selected)} aria-label="Queue actions" title="Actions">⋯</button>
        </div>
        <div class="tabs" role="tablist">
          {#each [['messages', 'Messages'], ['send', 'Send'], ['attributes', 'Attributes'], ['metrics', 'Metrics'], ['redrive', 'Redrive']] as const as [id, label] (id)}
            <button role="tab" aria-selected={tab === id} class:on={tab === id} onclick={() => (tab = id)} disabled={(id === 'send' || id === 'redrive') && !canEdit} title={(id === 'send' || id === 'redrive') && !canEdit ? 'Needs Edit on SQS' : ''}>{label}</button>
          {/each}
        </div>

        <div class="tab-body">
          {#if tab === 'messages'}
            <div class="bar">
              <label>Peek <select bind:value={peekN}>{#each [1, 2, 5, 10] as n (n)}<option value={n}>{n}</option>{/each}</select></label>
              <button class="primary sm" onclick={() => void peek()} disabled={peeking}>{peeking ? 'Peeking…' : 'Peek'}</button>
              <span class="dim">Non-destructive (visibility timeout 0). Messages may appear in any order.</span>
            </div>
            {#if messages.length === 0}
              <p class="dim pad">No messages loaded — press Peek.</p>
            {:else}
              <ul class="msgs">
                {#each messages as m (m.message_id)}
                  {@const parsed = parsedBody(m)}
                  {@const open = openMsg === m.message_id}
                  <li class="msg">
                    <div class="msg-head">
                      <button class="msg-toggle" onclick={() => (openMsg = open ? null : m.message_id)} aria-expanded={open}>
                        <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
                        <span class="mono mid">{m.message_id}</span>
                      </button>
                      <span class="dim mono">{m.attributes.SentTimestamp ? new Date(Number(m.attributes.SentTimestamp)).toLocaleString() : ''}</span>
                      {#if m.attributes.ApproximateReceiveCount}<span class="tag dim" title="ApproximateReceiveCount">rx {m.attributes.ApproximateReceiveCount}</span>{/if}
                      <button class="icon-btn" onclick={() => void copy(m.body, 'body')} title="Copy body" aria-label="Copy body"><Icon name="copy" size={12} /></button>
                      {#if canEdit}
                        <button class="icon-btn danger" onclick={() => void deleteMessage(m)} title="Delete message" aria-label="Delete message"><Icon name="trash" size={12} /></button>
                      {/if}
                    </div>
                    {#if !open}
                      <pre class="msg-preview">{m.body.slice(0, 200)}{m.body.length > 200 ? '…' : ''}</pre>
                    {:else}
                      {#if parsed !== undefined}
                        <div class="msg-body mono"><JsonTree value={parsed} /></div>
                      {:else}
                        <pre class="msg-body">{m.body}</pre>
                      {/if}
                      {#if Object.keys(m.message_attributes ?? {}).length}
                        <div class="msg-body mono"><JsonTree value={m.message_attributes} label="message_attributes" /></div>
                      {/if}
                    {/if}
                  </li>
                {/each}
              </ul>
            {/if}
          {:else if tab === 'send'}
            <div class="form">
              <label class="field"><span>Body</span><textarea bind:value={sendBody} rows={8} placeholder={'{"event": "…"}'} spellcheck="false"></textarea></label>
              <div class="row3">
                <label class="field"><span>Delay (s)</span><input type="number" min="0" max="900" bind:value={sendDelay} /></label>
                {#if selected.fifo}
                  <label class="field"><span>Message group ID</span><input bind:value={sendGroup} required /></label>
                  <label class="field"><span>Dedup ID <em>(optional)</em></span><input bind:value={sendDedup} /></label>
                {/if}
              </div>
              <div class="field">
                <span>Message attributes (String)</span>
                {#each sendAttrs as a, i (i)}
                  <div class="kv">
                    <input placeholder="name" bind:value={a.k} />
                    <input placeholder="value" bind:value={a.v} />
                    <button class="icon-btn" onclick={() => (sendAttrs = sendAttrs.filter((_, j) => j !== i))} aria-label="Remove attribute"><Icon name="x" size={12} /></button>
                  </div>
                {/each}
                <button class="ghost sm self" onclick={() => (sendAttrs = [...sendAttrs, { k: '', v: '' }])}><Icon name="plus" size={12} /> Attribute</button>
              </div>
              <div class="bar">
                <button class="ghost sm" onclick={() => (sendBody = prettyJson(sendBody))}>Pretty JSON</button>
                <button class="primary sm" onclick={() => void send()} disabled={sending || !sendBody.trim() || (selected.fifo && !sendGroup.trim())}>{sending ? 'Sending…' : 'Send message'}</button>
              </div>
            </div>
          {:else if tab === 'attributes'}
            {#if !attrs}
              <div class="pad"><Skeleton rows={6} /></div>
            {:else}
              <table class="tbl kvt">
                <tbody>
                  {#each Object.entries(attrs.attributes).sort(([a], [b]) => a.localeCompare(b)) as [k, v] (k)}
                    <tr><th>{k}</th><td class="mono wrap">{k === 'Policy' || k === 'RedrivePolicy' ? prettyJson(v) : v}</td></tr>
                  {/each}
                </tbody>
              </table>
            {/if}
          {:else if tab === 'metrics'}
            {#key `${account.id}/${selected.name}`}
              <MetricsPanel accountId={account.id} namespace="AWS/SQS" dimValue={selected.name} {onsignin} />
            {/key}
          {:else}
            <div class="form">
              <p class="dim">
                Move messages from this queue (typically a dead-letter queue) back to their source. Uses
                <code>start-message-move-task</code>; SQS enforces the DLQ relationship.
              </p>
              <label class="field"><span>Source ARN</span><input class="mono" value={attrs?.attributes.QueueArn ?? '…'} readonly /></label>
              {#if dlqSource.length}<p class="dim">Known source queues: {dlqSource.join(', ')}</p>{/if}
              <label class="field"><span>Destination ARN <em>(blank = original source)</em></span><input class="mono" bind:value={redriveDest} placeholder="arn:aws:sqs:…" /></label>
              <div class="bar">
                <button class="primary sm" onclick={() => void redrive()} disabled={redriving || !attrs}>{redriving ? 'Starting…' : 'Start redrive'}</button>
              </div>
            </div>
          {/if}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .split {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(260px, 36%) minmax(0, 1fr);
  }
  .split.mobile {
    grid-template-columns: minmax(0, 1fr);
  }
  .list {
    border-right: 1px solid var(--border);
    overflow: auto;
    min-height: 0;
  }
  .split.mobile .list {
    border-right: 0;
  }
  .detail {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .pad {
    padding: 12px;
  }
  .tbl {
    width: 100%;
    border-collapse: collapse;
    font-size: 12.5px;
  }
  .tbl th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--surface);
    text-align: left;
    font-weight: 600;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
  }
  .tbl td {
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 320px;
  }
  .tbl .num {
    text-align: right;
    width: 64px;
  }
  .kvt th {
    position: static;
    text-transform: none;
    letter-spacing: 0;
    font-size: 12px;
    width: 34%;
    vertical-align: top;
  }
  .kvt td.wrap {
    white-space: pre-wrap;
    word-break: break-all;
    max-width: none;
  }
  .trow {
    cursor: pointer;
  }
  .trow:hover,
  .trow:focus-visible {
    background: var(--surface-2);
    outline: none;
  }
  .trow.sel {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .name :global(svg) {
    vertical-align: -2px;
    margin-right: 6px;
  }
  .qn {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tag {
    font-size: 9.5px;
    font-weight: 700;
    padding: 0 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
    letter-spacing: 0.04em;
  }
  .tag.dim {
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .dim {
    color: var(--text-dim);
  }
  .dhead {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    flex-wrap: wrap;
  }
  .qname {
    font-size: 13.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .counts {
    font-size: 11.5px;
  }
  .back {
    border: 0;
    background: transparent;
    color: var(--accent);
    cursor: pointer;
    padding: 2px;
  }
  .more {
    margin-left: auto;
    border: 0;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 16px;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 0 8px;
    border-bottom: 1px solid var(--border);
  }
  .tabs button {
    padding: 7px 10px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 12.5px;
  }
  .tabs button.on {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .tabs button:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .tab-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 12px;
    flex-wrap: wrap;
    font-size: 12.5px;
  }
  .bar select {
    margin-left: 4px;
  }
  .msgs {
    list-style: none;
    margin: 0;
    padding: 0 12px 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .msg {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    overflow: hidden;
  }
  .msg-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    font-size: 12px;
    flex-wrap: wrap;
  }
  .msg-toggle {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 0;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    padding: 0;
    min-width: 0;
  }
  .mid {
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 260px;
    white-space: nowrap;
  }
  .msg-preview,
  .msg-body {
    margin: 0;
    padding: 6px 10px 8px;
    font-family: var(--font-mono);
    font-size: 11.5px;
    white-space: pre-wrap;
    word-break: break-word;
    border-top: 1px solid var(--border);
    color: var(--text-dim);
  }
  .msg-body {
    color: var(--text);
    max-height: 50vh;
    overflow: auto;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px;
    max-width: 760px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .field em {
    font-style: normal;
  }
  .field input,
  .field textarea {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 12.5px;
    padding: 6px 8px;
  }
  .field textarea {
    font-family: var(--font-mono);
    resize: vertical;
  }
  .row3 {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: 10px;
  }
  .kv {
    display: grid;
    grid-template-columns: 1fr 1fr auto;
    gap: 6px;
  }
  .self {
    align-self: flex-start;
  }
  code {
    font-family: var(--font-mono);
  }
  .primary {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--accent-contrast);
    font-weight: 600;
    cursor: pointer;
  }
  .primary:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .ghost {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 6px 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .sm {
    padding: 4px 10px;
    font-size: 12px;
  }
  .icon-btn {
    display: inline-grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .icon-btn.danger {
    color: var(--status-exited);
  }
  @media (max-width: 640px) {
    .hide-sm {
      display: none;
    }
  }
</style>
