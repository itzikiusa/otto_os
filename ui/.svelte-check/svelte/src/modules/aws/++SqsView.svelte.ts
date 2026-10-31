///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
import Badge from '../../lib/components/Badge.svelte';
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
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
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Tabs from '../../lib/components/Tabs.svelte';
import JsonTree from '../../lib/components/JsonTree.svelte';
import ViewToolbar from './ViewToolbar.svelte';
import MetricsPanel from './MetricsPanel.svelte';
import RegionPicker from './RegionPicker.svelte';
import { prettyJson, awsErrorText } from './util';
import { awsLoadText } from './loadText';
import type { AwsAccount, SqsMessage, SqsQueue } from '../../lib/api/types';
;

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  };;
  type Tab = 'messages' | 'send' | 'attributes' | 'metrics' | 'redrive';;;

  interface QueueDraft {
    body: string;
    delay: number;
    group: string;
    dedup: string;
    attributes: { k: string; v: string }[];
    destination: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { account, onsignin }: Props = $props();

  $effect(() => { void resourceAccess.load('aws_account', account.id); });
  const canSend = $derived(resourceAccess.can('aws_account', account.id, 'sqs_send', 'aws_sqs', 'edit'));
  const canDelete = $derived(resourceAccess.can('aws_account', account.id, 'sqs_delete', 'aws_sqs', 'edit'));
  const canPurge = $derived(resourceAccess.can('aws_account', account.id, 'sqs_purge', 'aws_sqs', 'edit'));
  const canRedrive = $derived(resourceAccess.can('aws_account', account.id, 'sqs_redrive', 'aws_sqs', 'edit'));
  // Peek is a receive: it bumps each message's receive count (and can
  // dead-letter it), so the daemon gates it at Edit on its own `sqs_receive`.
  const canReceive = $derived(resourceAccess.can('aws_account', account.id, 'sqs_receive', 'aws_sqs', 'edit'));
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

  // Region-scoped, latest-wins: a slow region-A reply (or error) that lands
  // after a switch to B must not overwrite B's error, end B's spinner, select
  // an A queue, or fan A's queue URLs out against B's region.
  let loadSeq = 0;
  /** `awaitAttrs` (auto-refresh): resolve only once the attribute fan-out has
   *  settled, so the toolbar's "next tick N s after the previous refresh
   *  SETTLES" gap starts after the last `aws` call — otherwise each tick
   *  restarted the fan-out from queue 0, the CLI never idled and queues past
   *  ~15-20 were never refreshed. */
  async function load(opts: { awaitAttrs?: boolean } = {}): Promise<void> {
    const seq = ++loadSeq;
    const rg = rq;
    loading = true;
    try {
      const list = await aws.loadSqsQueues(account.id, '', rg);
      if (seq !== loadSeq) return;
      error = '';
      // Approximate counts: capped so a 500-queue account doesn't fire 500 CLI
      // calls on open (the rest load when selected), and at most 2 in flight —
      // each is an `aws` process, and 40 at once took every webview socket to
      // the daemon for seconds. The list is usable while they fill in. Each
      // call carries the region captured above and stops once superseded.
      loading = false;
      autoSelect(list);
      const urls = list.slice(0, 40).map((q) => q.url);
      const fan = aws.fanOutSqsAttrs(account.id, urls, rg, () => seq === loadSeq);
      if (opts.awaitAttrs) await fan;
    } catch (e) {
      if (seq !== loadSeq) return;
      error = awsLoadText(e);
    } finally {
      if (seq === loadSeq) loading = false;
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
    const queue = selected;
    const accountId = account.id;
    const region = rq;
    const ok = await confirmProd({
      env: account.environment,
      verb: 'Delete message',
      where: queueWhere(queue.name),
      what: `Message ${m.message_id} — this cannot be undone.`,
    });
    if (!ok) return;
    try {
      await awsApi.sqsDeleteMessage(accountId, queue.url, m.receipt_handle, region || undefined);
      if (selected?.url === queue.url && account.id === accountId && rq === region)
        messages = messages.filter((x) => x.message_id !== m.message_id);
      toasts.success('Message deleted');
      void aws.loadSqsAttrs(accountId, queue.url, region);
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
    // The region the confirm names is the region the send goes to.
    const region = rq;
    const message_attributes: Record<string, { DataType: string; StringValue: string }> = {};
    for (const a of sendAttrs) if (a.k.trim()) message_attributes[a.k.trim()] = { DataType: 'String', StringValue: a.v };
    const attrNames = Object.keys(message_attributes);
    // Outward write: live consumers act on what lands in the queue — say
    // where it goes and what is sent before it leaves (typed-free; prod gets
    // the PRODUCTION line + danger styling).
    sending = true;
    const ok = await confirmProd({
      env: account.environment,
      verb: 'Send message',
      where: queueWhere(queue.name),
      what: [
        sendBody.trim(),
        attrNames.length ? `Attributes: ${attrNames.join(', ')}` : '',
        sendDelay ? `Delay: ${sendDelay}s` : '',
        queue.fifo && sendGroup ? `Group: ${sendGroup}` : '',
      ]
        .filter(Boolean)
        .join(' · '),
    });
    if (!ok || selected?.url !== queue.url) {
      sending = false;
      return;
    }
    try {
      const r = await awsApi.sqsSend(account.id, {
        url: queue.url,
        body: sendBody,
        delay_seconds: sendDelay || undefined,
        group_id: queue.fifo ? sendGroup || undefined : undefined,
        dedup_id: queue.fifo ? sendDedup || undefined : undefined,
        message_attributes: attrNames.length ? message_attributes : undefined,
      }, region || undefined);
      toasts.success('Message sent', r.message_id);
      void aws.loadSqsAttrs(account.id, queue.url, region);
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
    } catch {
      toasts.error('Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.');
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
;
async () => {

  { const $$_rablooTweiV0C = __sveltets_2_ensureComponent(ViewToolbar); const $$_rablooTweiV0 = new $$_rablooTweiV0C({ target: __sveltets_2_any(), props: {            children:() => { return __sveltets_2_any(0); },"title":`SQS`,"subtitle":queues ? `${plural(queues.length, 'queue')}` : '',filter,"filterPlaceholder":`Filter queues…`,loading,auto,region,"onrefresh":() => load({ awaitAttrs: true }),}});/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rablooTweiV0.$$bindings = 'filter';/*Ωignore_startΩ*/() => auto = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rablooTweiV0.$$bindings = 'auto';
    { const $$_rekciPnoigeR1C = __sveltets_2_ensureComponent(RegionPicker); const $$_rekciPnoigeR1 = new $$_rekciPnoigeR1C({ target: __sveltets_2_any(), props: {   account,"service":`sqs`,region,}});/*Ωignore_startΩ*/() => region = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rekciPnoigeR1.$$bindings = 'region';}
 ViewToolbar}

 { svelteHTML.createElement("div", {  "class":`split`,});viewport.isMobile;
  if(showList){
     { svelteHTML.createElement("div", { "class":`list`,});
      if(loading && !queues){
         { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":8,"label":`queues`,}});} }
      } else if (error){
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {     "what":`queues`,"error":awsErrorText(error),"empty":true,"onretry":() => void load(),errorActions:() => { async ()/*Ωignore_positionΩ*/ => {if(loginNeeded){ { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":onsignin,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}   }}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/ LoadState}
      } else if (shown.length === 0){
         { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {    "icon":`send`,"title":filter ? 'No matching queues' : 'No queues',}});}
      }else{
         { svelteHTML.createElement("table", { "class":`tbl`,});
           { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {   "class":`num`,"title":`ApproximateNumberOfMessages`,});  } { svelteHTML.createElement("th", {   "class":`num hide-sm`,"title":`NotVisible`,});  } { svelteHTML.createElement("th", {   "class":`num hide-sm`,"title":`Delayed`,});  } } }
           { svelteHTML.createElement("tbody", {});
               for(let q of __sveltets_2_ensureArray(shown)){q.url;
              const a = aws.sqsAttrs[q.url];
               {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('tr')));{ svelteHTML.createElement("tr", __sveltets_2_union($$action_0), {             "class":`trow`,"tabindex":0,"onclick":() => select(q),"onkeydown":(e) => { if (e.target === e.currentTarget && (e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); select(q); } },"oncontextmenu":(e) => queueMenu(e, q),});q.url === selectedUrl;
                 { svelteHTML.createElement("td", {   "class":`name`,"title":q.url,});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`send`,"size":12,}});}
                   { svelteHTML.createElement("span", { "class":`qn`,});q.name; }
                  if(q.fifo){ { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":`FIFO`,}});}}
                  if(a?.dlq_target_arn){ { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "label":`Has DLQ`,"title":`Dead-letter queue: ${a.dlq_target_arn}`,}});}}
                 }
                 { svelteHTML.createElement("td", { "class":`num mono`,});a ? a.approx_messages : '…'; }
                 { svelteHTML.createElement("td", { "class":`num mono hide-sm`,});a ? a.approx_not_visible : '…'; }
                 { svelteHTML.createElement("td", { "class":`num mono hide-sm`,});a ? a.approx_delayed : '…'; }
               }}
            }
           }
         }
      }
     }
  }

  if(showDetail){
     { svelteHTML.createElement("div", { "class":`detail`,});
      if(!selected){
        
        if(queues && queues.length === 0){
           { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {      "icon":`send`,"title":`No queues here`,"body":`Queues in this region show here — peek messages, send, inspect attributes or redrive.`,}});}
        }
      }else{
         { svelteHTML.createElement("div", { "class":`dhead`,});
          if(viewport.isMobile){
             { svelteHTML.createElement("button", {       "class":`back`,"onclick":() => { keepDraft(); selectedUrl = null; },"aria-label":`Back to queues`,"title":`Back to queues`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":14,}});} }
          }
           { svelteHTML.createElement("strong", {   "class":`qname`,"title":selected.url,});selected.name; }
          if(selected.fifo){ { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":`FIFO`,}});}}
          if(attrs){ { svelteHTML.createElement("span", { "class":`dim counts mono`,});attrs.approx_messages;   attrs.approx_not_visible;   attrs.approx_delayed;  }}
           { svelteHTML.createElement("button", {       "class":`icon-btn more`,"onclick":(e) => selected && queueMenu(e, selected),"aria-label":`Queue actions`,"title":`Actions`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
         }
         { const $$_sbaT2C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT2C({ target: __sveltets_2_any(), props: {           "label":`Queue details`,"idBase":`sqs`,"value":tab,"onchange":(id) => (tab = id),"tabs":([['messages', 'Messages', canReceive], ['send', 'Send', canSend], ['attributes', 'Attributes', true], ['metrics', 'Metrics', true], ['redrive', 'Redrive', canRedrive]] as const).map(([id, label, ok]) => ({ id, label, disabled: !ok, title: ok ? undefined : 'Needs Edit on SQS' })),}});}

         { svelteHTML.createElement("div", {       "class":`tab-body`,"role":`tabpanel`,"id":`sqs-panel-${tab}`,"aria-labelledby":`sqs-tab-${tab}`,});
          if(tab === 'messages'){
             { svelteHTML.createElement("div", { "class":`bar`,});
               { svelteHTML.createElement("label", {});  { svelteHTML.createElement("select", { "bind:value":peekN,});/*Ωignore_startΩ*/() => peekN = __sveltets_2_any(null);/*Ωignore_endΩ*/   for(let n of __sveltets_2_ensureArray(([1, 2, 5, 10]))){n; { svelteHTML.createElement("option", { "value":n,});n; }} } }
               { svelteHTML.createElement("button", {       "class":`btn primary small`,"onclick":() => void peek(),"disabled":!canReceive || peeking,"title":canReceive ? undefined : 'Needs Edit on SQS',});peeking ? 'Peeking…' : 'Peek'; }
               { svelteHTML.createElement("span", { "class":`dim`,});                                  }
             }
            if(messages.length === 0){
               { svelteHTML.createElement("p", { "class":`dim pad`,});      }
            }else{
               { svelteHTML.createElement("ul", { "class":`msgs`,});
                   for(let m of __sveltets_2_ensureArray(messages)){m.message_id;
                  const parsed = parsedBody(m);
                  const open = openMsg === m.message_id;
                   { svelteHTML.createElement("li", { "class":`msg`,});
                     { svelteHTML.createElement("div", { "class":`msg-head`,});
                       { svelteHTML.createElement("button", {     "class":`msg-toggle`,"onclick":() => (openMsg = open ? null : m.message_id),"aria-expanded":open,});
                         { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});}
                         { svelteHTML.createElement("span", { "class":`mono mid`,});m.message_id; }
                       }
                       { svelteHTML.createElement("span", { "class":`dim mono`,});m.attributes.SentTimestamp ? new Date(Number(m.attributes.SentTimestamp)).toLocaleString() : ''; }
                      if(m.attributes.ApproximateReceiveCount){ { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "title":`Approximate receive count`,"label":`Received ${m.attributes.ApproximateReceiveCount}×`,}});}}
                       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void copy(m.body, 'body'),"title":`Copy body`,"aria-label":`Copy body`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} }
                      if(canDelete){
                         { svelteHTML.createElement("button", {       "class":`icon-btn msg-delete`,"onclick":() => void deleteMessage(m),"title":`Delete message`,"aria-label":`Delete message`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} }
                      }
                     }
                    if(!open){
                       { svelteHTML.createElement("pre", {   "dir":`ltr`,"class":`msg-preview`,});m.body.slice(0, 200);m.body.length > 200 ? '…' : ''; }
                    }else{
                      if(parsed !== undefined){
                         { svelteHTML.createElement("div", {   "dir":`ltr`,"class":`msg-body mono`,}); { const $$_eerTnosJ6C = __sveltets_2_ensureComponent(JsonTree); new $$_eerTnosJ6C({ target: __sveltets_2_any(), props: {  "value":parsed,}});} }
                      }else{
                         { svelteHTML.createElement("pre", {   "dir":`ltr`,"class":`msg-body`,});m.body; }
                      }
                      if(Object.keys(m.message_attributes ?? {}).length){
                         { svelteHTML.createElement("div", {   "dir":`ltr`,"class":`msg-body mono`,}); { const $$_eerTnosJ6C = __sveltets_2_ensureComponent(JsonTree); new $$_eerTnosJ6C({ target: __sveltets_2_any(), props: {    "value":m.message_attributes,"label":`message_attributes`,}});} }
                      }
                    }
                   }
                }
               }
            }
          } else if (tab === 'send'){
             { svelteHTML.createElement("div", { "class":`form`,});
               { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("textarea", {         "dir":`ltr`,"bind:value":sendBody,"rows":8,"placeholder":'{"event": "…"}',"spellcheck":`false`,});/*Ωignore_startΩ*/() => sendBody = __sveltets_2_any(null);/*Ωignore_endΩ*/ } }
               { svelteHTML.createElement("div", { "class":`row3`,});
                 { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});  } { svelteHTML.createElement("input", {          "type":`number`,"min":`0`,"max":`900`,"bind:value":sendDelay,"aria-invalid":!validDelay,});/*Ωignore_startΩ*/() => sendDelay = __sveltets_2_any(null);/*Ωignore_endΩ*/}if(!validDelay){ { svelteHTML.createElement("span", { "class":`err`,});        }} }
                if(selected.fifo){
                   { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});   }  { svelteHTML.createElement("input", {    "dir":`auto`,"bind:value":sendGroup,"required":true,});/*Ωignore_startΩ*/() => sendGroup = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
                   { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});   { svelteHTML.createElement("em", {});  } } { svelteHTML.createElement("input", {    "dir":`auto`,"bind:value":sendDedup,});/*Ωignore_startΩ*/() => sendDedup = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
                }
               }
               { svelteHTML.createElement("div", { "class":`field`,});
                 { svelteHTML.createElement("span", {});   }
                    for(let a of __sveltets_2_ensureArray(sendAttrs)){let i = 1;i;
                   { svelteHTML.createElement("div", { "class":`kv`,});
                     { svelteHTML.createElement("input", {        "dir":`auto`,"aria-label":`Attribute ${i + 1} name`,"placeholder":`name`,"bind:value":a.k,});/*Ωignore_startΩ*/() => a.k = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                     { svelteHTML.createElement("input", {        "dir":`auto`,"aria-label":`Attribute ${i + 1} value`,"placeholder":`value`,"bind:value":a.v,});/*Ωignore_startΩ*/() => a.v = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (sendAttrs = sendAttrs.filter((_, j) => j !== i)),"aria-label":`Remove attribute`,"title":`Remove attribute`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
                   }
                }
                 { svelteHTML.createElement("button", {   "class":`btn small self`,"onclick":() => (sendAttrs = [...sendAttrs, { k: '', v: '' }]),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
               }
               { svelteHTML.createElement("div", { "class":`bar`,});
                 { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (sendBody = prettyJson(sendBody)),});  }
                 { svelteHTML.createElement("button", {     "class":`btn primary small`,"onclick":() => void send(),"disabled":!canSend || sending || !validDelay || !sendBody.trim() || (selected.fifo && !sendGroup.trim()),});sending ? 'Sending…' : 'Send message'; }
               }
             }
          } else if (tab === 'attributes'){
            if(!attrs){
               { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {    "rows":6,"label":`queue attributes`,}});} }
            }else{
               { svelteHTML.createElement("table", { "class":`tbl kvt`,});
                 { svelteHTML.createElement("tbody", {});
                     for(let [k, v] of __sveltets_2_ensureArray((Object.entries(attrs.attributes).sort(([a], [b]) => a.localeCompare(b))))){k;
                     { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});k; } { svelteHTML.createElement("td", { "class":`mono wrap`,});k === 'Policy' || k === 'RedrivePolicy' ? prettyJson(v) : v; } }
                  }
                 }
               }
            }
          } else if (tab === 'metrics'){
            `${account.id}/${selected.name}`; {
               { const $$_lenaPscirteM3C = __sveltets_2_ensureComponent(MetricsPanel); new $$_lenaPscirteM3C({ target: __sveltets_2_any(), props: {         "accountId":account.id,"namespace":`AWS/SQS`,"dimValue":selected.name,"region":rq || undefined,onsignin,}});}
            }
          }else{
             { svelteHTML.createElement("div", { "class":`form`,});
               { svelteHTML.createElement("p", { "class":`dim`,});
                             
                 { svelteHTML.createElement("code", {});  }     
               }
               { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});  }  { svelteHTML.createElement("input", {      "dir":`ltr`,"class":`mono`,"value":attrs?.attributes.QueueArn ?? '…',"readonly":true,});} }
              if(dlqSource.length){ { svelteHTML.createElement("p", { "class":`dim`,});   dlqSource.join(', '); }}
               { svelteHTML.createElement("label", { "class":`field`,}); { svelteHTML.createElement("span", {});   { svelteHTML.createElement("em", {});    } } { svelteHTML.createElement("input", {        "dir":`ltr`,"class":`mono`,"bind:value":redriveDest,"placeholder":`arn:aws:sqs:…`,});/*Ωignore_startΩ*/() => redriveDest = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
               { svelteHTML.createElement("div", { "class":`bar`,});
                 { svelteHTML.createElement("button", {     "class":`btn primary small`,"onclick":() => void redrive(),"disabled":!canRedrive || redriving || !attrs,});redriving ? 'Starting…' : 'Start redrive'; }
               }
             }
          }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SqsView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SqsView__SvelteComponent_ = ReturnType<typeof SqsView__SvelteComponent_>;
/*Ωignore_endΩ*/export default SqsView__SvelteComponent_;