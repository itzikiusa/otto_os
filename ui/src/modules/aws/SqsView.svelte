<script lang="ts">
  import { rowMenu } from '../../lib/rowMenu';
  import Badge from '../../lib/components/Badge.svelte';
  import { plural } from '../../lib/plural';
  import { toastError } from '../../lib/toastError';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import { mapLimit } from '../../lib/poll';
  // SQS: queue list with approximate counts → queue detail tabs: Messages
  // (Peek N, JSON-pretty body viewer, delete-message per row [Edit]), Send
  // (body + attributes, FIFO fields only for `.fifo`) [Edit], Attributes,
  // Metrics (CloudWatch via MetricsPanel), Redrive (DLQ → source) [Edit].
  // Purge lives in the ⋯ menu [Edit, typed].
  import { untrack } from 'svelte';
  import { aws } from '../../lib/stores/aws.svelte';
  import { awsApi, isLoginRequired } from '../../lib/api/aws';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmProd } from '../../lib/confirmProd';
  import { toasts } from '../../lib/toast.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import JsonTree from '../database/JsonTree.svelte';
  import ViewToolbar from './ViewToolbar.svelte';
  import MetricsPanel from './MetricsPanel.svelte';
  import RegionPicker from './RegionPicker.svelte';
  import { prettyJson, awsErrorText, serviceTabKey } from './util';
  import type { AwsAccount, SqsMessage, SqsQueue } from '../../lib/api/types';

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  }
  let { account, onsignin }: Props = $props();

  $effect(() => { void resourceAccess.load('aws_account', account.id); });
  const canSend = $derived(resourceAccess.can('aws_account', account.id, 'sqs_send', 'aws_sqs', 'edit'));
  const canDelete = $derived(resourceAccess.can('aws_account', account.id, 'sqs_delete', 'aws_sqs', 'edit'));
  const canPurge = $derived(resourceAccess.can('aws_account', account.id, 'sqs_purge', 'aws_sqs', 'edit'));
  const canRedrive = $derived(resourceAccess.can('aws_account', account.id, 'sqs_redrive', 'aws_sqs', 'edit'));
  const canReceive = $derived(resourceAccess.can('aws_account', account.id, 'sqs_receive', 'aws_sqs', 'view'));
  // A-1: queues live per region; the picker restores the last one used.
  // svelte-ignore state_referenced_locally
  let region = $state(account.region);
  /** `''` = the account default (the daemon resolves it), else an override. */
  const rq = $derived(region === account.region ? '' : region);
  const queues = $derived(aws.sqsQueues[`${account.id}:${rq}`] ?? null);
  let loading = $state(false);
  let error = $state('');
  let filter = $state('');
  let auto = $state(false);
  let selectedUrl = $state<string | null>(null);
  const selected = $derived(queues?.find((q) => q.url === selectedUrl) ?? null);
  const attrs = $derived(selectedUrl ? (aws.sqsAttrs[selectedUrl] ?? null) : null);
  type Tab = 'messages' | 'send' | 'attributes' | 'metrics' | 'redrive';
  let tab = $state<Tab>('messages');
  $effect(() => {
    if (!resourceAccess.get('aws_account', account.id)) return;
    if ((tab === 'messages' && !canReceive) || (tab === 'send' && !canSend) || (tab === 'redrive' && !canRedrive)) tab = 'attributes';
  });

  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    const list = queues ?? [];
    return q ? list.filter((x) => x.name.toLowerCase().includes(q)) : list;
  });

  // A list/detail page opens on an item, never on an empty "pick one" pane:
  // the remembered queue, else the first. (On a phone the list IS the first screen.)
  function autoSelect(list: readonly SqsQueue[]): void {
    if (viewport.isMobile || selectedUrl) return;
    const url = initialSelection('aws.sqs', list, (q) => q.url);
    const q = list.find((x) => x.url === url);
    if (q) select(q);
  }

  async function load(): Promise<void> {
    loading = true;
    try {
      const list = await aws.loadSqsQueues(account.id, '', rq);
      error = '';
      // Approximate counts: capped so a 500-queue account doesn't fire 500 CLI
      // calls on open (the rest load when selected), and at most 2 in flight —
      // each is an `aws` process, and 40 at once took every webview socket to
      // the daemon for seconds. The list is usable while they fill in.
      loading = false;
      autoSelect(list);
      void mapLimit(list.slice(0, 40), 2, (q) =>
        aws.loadSqsAttrs(account.id, q.url, rq).catch(() => undefined),
      );
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  // The open queue vanished (deleted / a reload without it): open the next one.
  $effect(() => {
    if (selected || !queues?.length || viewport.isMobile) return;
    untrack(() => {
      selectedUrl = null;
      autoSelect(queues!);
    });
  });

  // Load on mount and whenever the region changes (`rq` is the only dep).
  $effect(() => {
    void rq;
    untrack(() => {
      selectedUrl = null;
      if (!queues) void load();
      else autoSelect(queues);
    });
  });

  interface QueueDraft {
    body: string;
    delay: number;
    group: string;
    dedup: string;
    attributes: { k: string; v: string }[];
    destination: string;
  }
  const drafts = new Map<string, QueueDraft>();
  function keepDraft(): void {
    if (selectedUrl) drafts.set(selectedUrl, { body: sendBody, delay: sendDelay, group: sendGroup, dedup: sendDedup, attributes: sendAttrs.map(a => ({ ...a })), destination: redriveDest });
  }
  function select(q: SqsQueue): void {
    keepDraft();
    selectedUrl = q.url;
    rememberSelection('aws.sqs', q.url);
    const draft = drafts.get(q.url);
    sendBody = draft?.body ?? '';
    sendDelay = draft?.delay ?? 0;
    sendGroup = draft?.group ?? '';
    sendDedup = draft?.dedup ?? '';
    sendAttrs = draft?.attributes.map(a => ({ ...a })) ?? [];
    redriveDest = draft?.destination ?? '';
    peekVersion++;
    peeking = false;
    openMsg = null;
    messages = [];
    void aws.loadSqsAttrs(account.id, q.url, rq);
  }

  // ── messages ──
  let messages = $state<SqsMessage[]>([]);
  let peekN = $state(10);
  let peeking = $state(false);
  let peekVersion = 0;
  let openMsg = $state<string | null>(null);

  async function peek(): Promise<void> {
    if (!selectedUrl) return;
    peeking = true;
    const version = ++peekVersion;
    try {
      const r = await awsApi.sqsPeek(account.id, { url: selectedUrl, max: peekN, visibility_timeout: 0 }, rq || undefined);
      if (version !== peekVersion) return;
      messages = r.messages;
      if (r.messages.length === 0) toasts.info('No messages visible right now');
    } catch (e) {
      if (version !== peekVersion) return;
      toastError('Couldn’t peek', e);
    } finally {
      if (version === peekVersion) peeking = false;
    }
  }

  /** "SQS queue orders-dlq · prod-account · eu-west-1" — the destination line of every confirm. */
  const queueWhere = (name: string): string => `SQS queue ${name} · ${account.name} · ${rq || account.region}`;

  async function deleteMessage(m: SqsMessage): Promise<void> {
    if (!selected) return;
    const ok = await confirmProd({
      env: account.environment,
      verb: 'Delete message',
      where: queueWhere(selected.name),
      what: `Message ${m.message_id} — this cannot be undone.`,
    });
    if (!ok) return;
    try {
      await awsApi.sqsDeleteMessage(account.id, selected.url, m.receipt_handle, rq || undefined);
      messages = messages.filter((x) => x.message_id !== m.message_id);
      toasts.success('Message deleted');
      void aws.loadSqsAttrs(account.id, selected.url, rq);
    } catch (e) {
      toastError('Couldn’t delete', e);
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
  const validDelay = $derived(Number.isInteger(sendDelay) && sendDelay >= 0 && sendDelay <= 900);

  async function send(): Promise<void> {
    if (!selected || !sendBody.trim() || !validDelay || sending) return;
    const queue = selected;
    sending = true;
    try {
      const message_attributes: Record<string, { DataType: string; StringValue: string }> = {};
      for (const a of sendAttrs) if (a.k.trim()) message_attributes[a.k.trim()] = { DataType: 'String', StringValue: a.v };
      const r = await awsApi.sqsSend(account.id, {
        url: queue.url,
        body: sendBody,
        delay_seconds: sendDelay || undefined,
        group_id: queue.fifo ? sendGroup || undefined : undefined,
        dedup_id: queue.fifo ? sendDedup || undefined : undefined,
        message_attributes: Object.keys(message_attributes).length ? message_attributes : undefined,
      }, rq || undefined);
      toasts.success('Message sent', r.message_id);
      void aws.loadSqsAttrs(account.id, queue.url, rq);
    } catch (e) {
      toastError('Couldn’t send', e);
    } finally {
      sending = false;
    }
  }

  // ── purge / redrive ──
  async function purge(q: SqsQueue): Promise<void> {
    // Always typed (purge empties the whole queue), prod or not.
    const ok = await confirmProd({
      env: account.environment,
      verb: 'Purge',
      title: 'Purge queue',
      where: queueWhere(q.name),
      what: 'Deletes ALL messages in the queue. This cannot be undone.',
      typed: q.name,
      danger: true,
    });
    if (!ok) return;
    try {
      await awsApi.sqsPurge(account.id, q.url, q.name, rq || undefined);
      toasts.success('Purge started', 'SQS empties the queue over the next ~60 s');
      void aws.loadSqsAttrs(account.id, q.url, rq);
    } catch (e) {
      toastError('Couldn’t purge', e);
    }
  }

  let redriveDest = $state('');
  let redriving = $state(false);
  async function redrive(): Promise<void> {
    const src = attrs?.attributes.QueueArn;
    if (!src || !selected) return;
    const ok = await confirmProd({
      env: account.environment,
      verb: 'Start redrive',
      where: queueWhere(selected.name),
      what: `Move every message back to ${redriveDest.trim() || 'its original source queue(s)'}.`,
    });
    if (!ok) return;
    redriving = true;
    try {
      const r = await awsApi.sqsRedrive(account.id, { source_arn: src, destination_arn: redriveDest.trim() || undefined }, rq || undefined);
      toasts.success('Redrive started', r.task_handle);
    } catch (e) {
      toastError('Couldn’t redrive', e);
    } finally {
      redriving = false;
    }
  }

  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${what}`);
    } catch (e) {
      toastError('Couldn’t copy', e);
    }
  }

  function queueMenu(e: MouseEvent | KeyboardEvent, q: SqsQueue): void {
    ctxMenu.show(e, [
      { label: 'Open', icon: 'send', action: () => select(q) },
      { label: 'Refresh counts', icon: 'refresh', action: () => void aws.loadSqsAttrs(account.id, q.url, rq) },
      { label: 'Copy URL', icon: 'copy', action: () => void copy(q.url, 'queue URL') },
      ...(canPurge ? [{ separator: true }, { label: 'Purge queue…', icon: 'trash', danger: true, action: () => void purge(q) }] : []),
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
  subtitle={queues ? `${plural(queues.length, 'queue')}` : ''}
  bind:filter
  filterPlaceholder="Filter queues…"
  {loading}
  bind:auto
  {region}
  onrefresh={() => void load()}
>
  <RegionPicker {account} service="sqs" bind:region />
</ViewToolbar>

<div class="split" class:mobile={viewport.isMobile}>
  {#if showList}
    <div class="list">
      {#if loading && !queues}
        <div class="pad"><Skeleton rows={8} label="queues" /></div>
      {:else if error}
        <EmptyState actionKind={loginNeeded ? 'primary' : 'secondary'} icon="warning" title="Couldn’t list queues" body={awsErrorText(error)} actionLabel={loginNeeded ? 'Sign in' : 'Retry'} onaction={loginNeeded ? onsignin : () => void load()} />
      {:else if shown.length === 0}
        <EmptyState icon="send" title={filter ? 'No matching queues' : 'No queues'} />
      {:else}
        <table class="tbl">
          <thead><tr><th>Queue</th><th class="num" title="ApproximateNumberOfMessages">Avail</th><th class="num hide-sm" title="NotVisible">In-flight</th><th class="num hide-sm" title="Delayed">Delayed</th></tr></thead>
          <tbody>
            {#each shown as q (q.url)}
              {@const a = aws.sqsAttrs[q.url]}
              <tr use:rowMenu
                class="trow"
                class:sel={q.url === selectedUrl}
                tabindex="0"
                onclick={() => select(q)}
                onkeydown={(e) => { if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); select(q); } }}
                oncontextmenu={(e) => queueMenu(e, q)}
              >
                <td class="name" title={q.url}>
                  <Icon name="send" size={12} />
                  <span class="qn">{q.name}</span>
                  {#if q.fifo}<Badge tone="accent" label="FIFO" />{/if}
                  {#if a?.dlq_target_arn}<Badge label="Has DLQ" title={`Dead-letter queue: ${a.dlq_target_arn}`} />{/if}
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
        <!-- Never a "pick a queue" pane: a queue is auto-opened whenever the
             list has one (see autoSelect + the effect below). -->
        {#if queues && queues.length === 0}
          <EmptyState icon="send" title="No queues here" body="Queues in this region show here — peek messages, send, inspect attributes or redrive." />
        {/if}
      {:else}
        <div class="dhead">
          {#if viewport.isMobile}
            <button class="back" onclick={() => { keepDraft(); selectedUrl = null; }} aria-label="Back to queues" title="Back to queues"><Icon name="chevronLeft" size={14} /></button>
          {/if}
          <strong class="qname" title={selected.url}>{selected.name}</strong>
          {#if selected.fifo}<Badge tone="accent" label="FIFO" />{/if}
          {#if attrs}<span class="dim counts mono">{attrs.approx_messages} avail · {attrs.approx_not_visible} in-flight · {attrs.approx_delayed} delayed</span>{/if}
          <button class="icon-btn more" onclick={(e) => selected && queueMenu(e, selected)} aria-label="Queue actions" title="Actions"><Icon name="more" size={14} /></button>
        </div>
        <div class="tabs" role="tablist" aria-label="Queue details">
          {#each [['messages', 'Messages'], ['send', 'Send'], ['attributes', 'Attributes'], ['metrics', 'Metrics'], ['redrive', 'Redrive']] as const as [id, label] (id)}
            <button role="tab" tabindex={tab === id ? 0 : -1} onkeydown={serviceTabKey} aria-selected={tab === id} class:on={tab === id} onclick={() => (tab = id)} disabled={((id === 'send' && !canSend) || (id === 'redrive' && !canRedrive) || (id === 'messages' && !canReceive))} title={((id === 'send' && !canSend) || (id === 'redrive' && !canRedrive) || (id === 'messages' && !canReceive)) ? 'Needs Edit on SQS' : ''}>{label}</button>
          {/each}
        </div>

        <div class="tab-body">
          {#if tab === 'messages'}
            <div class="bar">
              <label>Peek <select bind:value={peekN}>{#each [1, 2, 5, 10] as n (n)}<option value={n}>{n}</option>{/each}</select></label>
              <button class="btn primary small" onclick={() => void peek()} disabled={!canReceive || peeking} title={canReceive ? undefined : 'Needs Edit on SQS'}>{peeking ? 'Peeking…' : 'Peek'}</button>
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
                      {#if m.attributes.ApproximateReceiveCount}<Badge title="Approximate receive count" label={`Received ${m.attributes.ApproximateReceiveCount}×`} />{/if}
                      <button class="icon-btn" onclick={() => void copy(m.body, 'body')} title="Copy body" aria-label="Copy body"><Icon name="copy" size={12} /></button>
                      {#if canDelete}
                        <button class="icon-btn msg-delete" onclick={() => void deleteMessage(m)} title="Delete message" aria-label="Delete message"><Icon name="trash" size={12} /></button>
                      {/if}
                    </div>
                    {#if !open}
                      <pre dir="ltr" class="msg-preview">{m.body.slice(0, 200)}{m.body.length > 200 ? '…' : ''}</pre>
                    {:else}
                      {#if parsed !== undefined}
                        <div dir="ltr" class="msg-body mono"><JsonTree value={parsed} /></div>
                      {:else}
                        <pre dir="ltr" class="msg-body">{m.body}</pre>
                      {/if}
                      {#if Object.keys(m.message_attributes ?? {}).length}
                        <div dir="ltr" class="msg-body mono"><JsonTree value={m.message_attributes} label="message_attributes" /></div>
                      {/if}
                    {/if}
                  </li>
                {/each}
              </ul>
            {/if}
          {:else if tab === 'send'}
            <div class="form">
              <label class="field"><span>Body</span><textarea dir="ltr" bind:value={sendBody} rows={8} placeholder={'{"event": "…"}'} spellcheck="false"></textarea></label>
              <div class="row3">
                <label class="field"><span>Delay (s)</span><input type="number" min="0" max="900" bind:value={sendDelay} aria-invalid={!validDelay} />{#if !validDelay}<span class="err">Enter a whole number from 0 to 900.</span>{/if}</label>
                {#if selected.fifo}
                  <label class="field"><span>Message group ID</span><input dir="auto" bind:value={sendGroup} required /></label>
                  <label class="field"><span>Dedup ID <em>(optional)</em></span><input dir="auto" bind:value={sendDedup} /></label>
                {/if}
              </div>
              <div class="field">
                <span>Message attributes (String)</span>
                {#each sendAttrs as a, i (i)}
                  <div class="kv">
                    <input dir="auto" aria-label="Attribute {i + 1} name" placeholder="name" bind:value={a.k} />
                    <input dir="auto" aria-label="Attribute {i + 1} value" placeholder="value" bind:value={a.v} />
                    <button class="icon-btn" onclick={() => (sendAttrs = sendAttrs.filter((_, j) => j !== i))} aria-label="Remove attribute" title="Remove attribute"><Icon name="x" size={12} /></button>
                  </div>
                {/each}
                <button class="btn small self" onclick={() => (sendAttrs = [...sendAttrs, { k: '', v: '' }])}><Icon name="plus" size={12} /> Attribute</button>
              </div>
              <div class="bar">
                <button class="btn small" onclick={() => (sendBody = prettyJson(sendBody))}>Pretty JSON</button>
                <button class="btn primary small" onclick={() => void send()} disabled={!canSend || sending || !validDelay || !sendBody.trim() || (selected.fifo && !sendGroup.trim())}>{sending ? 'Sending…' : 'Send message'}</button>
              </div>
            </div>
          {:else if tab === 'attributes'}
            {#if !attrs}
              <div class="pad"><Skeleton rows={6} label="queue attributes" /></div>
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
              <MetricsPanel accountId={account.id} namespace="AWS/SQS" dimValue={selected.name} region={rq || undefined} {onsignin} />
            {/key}
          {:else}
            <div class="form">
              <p class="dim">
                Move messages from this queue (typically a dead-letter queue) back to their source. Uses
                <code>start-message-move-task</code>; SQS enforces the DLQ relationship.
              </p>
              <label class="field"><span>Source ARN</span><input dir="ltr" class="mono" value={attrs?.attributes.QueueArn ?? '…'} readonly /></label>
              {#if dlqSource.length}<p class="dim">Known source queues: {dlqSource.join(', ')}</p>{/if}
              <label class="field"><span>Destination ARN <em>(blank = original source)</em></span><input dir="ltr" class="mono" bind:value={redriveDest} placeholder="arn:aws:sqs:…" /></label>
              <div class="bar">
                <button class="btn primary small" onclick={() => void redrive()} disabled={!canRedrive || redriving || !attrs}>{redriving ? 'Starting…' : 'Start redrive'}</button>
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
    border-inline-end: 1px solid var(--border);
    overflow: auto;
    min-height: 0;
  }
  .split.mobile .list {
    border-inline-end: 0;
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
    font-size: var(--fs-m);
  }
  .tbl th {
    position: sticky;
    top: 0;
    z-index: 1;
    background: var(--surface);
    text-align: start;
    font-weight: 600;
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
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
    text-align: end;
    width: 64px;
  }
  .kvt th {
    position: static;
    text-transform: none;
    letter-spacing: 0;
    font-size: var(--fs-s);
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
  .trow:hover {
    background: var(--hover);
  }
  .trow:focus-visible {
    background: var(--surface-2);
    outline: none;
    box-shadow: inset 0 0 0 2px var(--accent-text);
  }
  .trow.sel {
    background: var(--accent-soft);
  }
  .name :global(svg) {
    vertical-align: -2px;
    margin-inline-end: 6px;
  }
  .qn {
    overflow: hidden;
    text-overflow: ellipsis;
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
    font-size: var(--fs-m);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .counts {
    font-size: var(--fs-s);
  }
  .back {
    border: 0;
    background: transparent;
    color: var(--accent-text);
    cursor: pointer;
    padding: 2px;
  }
  .more {
    margin-inline-start: auto;
  }
  .tabs {
    display: flex;
    gap: 2px;
    padding: 0 8px;
    border-bottom: 1px solid var(--border);
  }
  .tabs button {
    padding: 6px 10px;
    border: 0;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: var(--fs-m);
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
    font-size: var(--fs-m);
  }
  .bar select {
    margin-inline-start: 4px;
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-m);
    padding: 6px 8px;
  }
  .field input,
  .field textarea {
    min-width: 0;
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
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr) auto;
    gap: 6px;
  }
  .self {
    align-self: flex-start;
  }
  code {
    font-family: var(--font-mono);
  }
  .msg-delete {
    color: var(--danger);
  }
  @media (max-width: 640px) {
    .hide-sm {
      display: none;
    }
  }
</style>
