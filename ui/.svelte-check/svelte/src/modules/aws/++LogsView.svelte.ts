///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
// CloudWatch Logs: log groups (server-side prefix search, paged) on the left;
// on the right either the Events view — stream picker, time-range presets,
// filter pattern, a windowed event table with a JSON-aware detail pane and a
// live tail (poll every 2 s while events keep arriving, backing off to 10 s
// when idle, only while visible; forward from the newest timestamp seen,
// deduped by event id, ring buffer of 5 000) — or Logs Insights: a query over
// one or more groups, polled on a 1,1,2,2,3,5 s backoff until done, rendered with
// the DB Explorer ResultsGrid; saved queries live in localStorage per account.
// Deep link: `#/aws/<id>/logs/<group>/<region>` (a trailing `/` on the group
// means "prefix": the list is filtered and the first match selected).
import { onDestroy, untrack } from 'svelte';
import { aws } from '../../lib/stores/aws.svelte';
import { awsApi, isLoginRequired } from '../../lib/api/aws';
import { router } from '../../lib/router.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
import { initialSelection, rememberSelection } from '../../lib/lastSelection';
import { pollWhileVisible, type Poller } from '../../lib/poll';
import { adaptiveCadence, statusPollMs } from '../../lib/pollBackoff';
import { TableWindow } from '../../lib/tableWindow.svelte';
import { exportCsv } from '../../lib/components/exporters';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Icon from '../../lib/components/Icon.svelte';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import ResultsGrid from '../database/ResultsGrid.svelte';
import ViewToolbar from './ViewToolbar.svelte';
import { awsErrorText, fmtBytes, logsRoute } from './util';
import { awsLoadText } from './loadText';
import type {
    AwsAccount,
    AwsLogEvent,
    AwsLogGroup,
    AwsLogStream,
    AwsLogsInsightsResults,
  } from '../../lib/api/types';
;

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  };;
  interface SavedQuery {
    name: string;
    query: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { account, onsignin }: Props = $props();

  const RING = 5000;
  // Each tail tick / Insights status poll spawns an `aws` CLI process on the
  // daemon: the tail runs at 2 s while events arrive and doubles to 10 s when
  // idle; Insights uses the shared 1,1,2,2,3,5 s status backoff (lib/pollBackoff).
  const TAIL_MIN_MS = 2000;
  const TAIL_MAX_MS = 10_000;
  const RANGES: { id: string; label: string; ms: number }[] = [
    { id: '5m', label: '5m', ms: 5 * 60_000 },
    { id: '15m', label: '15m', ms: 15 * 60_000 },
    { id: '1h', label: '1h', ms: 3600_000 },
    { id: '3h', label: '3h', ms: 3 * 3600_000 },
    { id: '24h', label: '24h', ms: 24 * 3600_000 },
    { id: '7d', label: '7d', ms: 7 * 24 * 3600_000 },
  ];

  // Deep link (read once — the view is remounted per account+service).
  // svelte-ignore state_referenced_locally
  const linkGroup = router.parts[3] ?? '';
  // svelte-ignore state_referenced_locally
  const linkRegion = router.parts[4] ?? '';
  const linkIsPrefix = linkGroup.endsWith('/');

  // svelte-ignore state_referenced_locally
  let region = $state(linkRegion || account.region);
  let tab = $state<'events' | 'insights'>('events');
  const VIEW_TABS: TabItem<'events' | 'insights'>[] = [
    { id: 'events', label: 'Events' },
    { id: 'insights', label: 'Insights' },
  ];

  // ── groups ──
  let groupPrefix = $state(linkIsPrefix ? linkGroup : '');
  let groups = $state<AwsLogGroup[] | null>(null);
  let groupsToken = $state<string | null>(null);
  let groupsLoading = $state(false);
  let groupsError = $state('');
  let selected = $state<string>(linkIsPrefix ? '' : linkGroup);
  let groupsW = $state(loadPaneWidth('aws.logs.groupsW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  let groupSeq = 0;

  async function loadGroups(more = false): Promise<void> {
    const seq = ++groupSeq;
    groupsLoading = true;
    try {
      const r = await awsApi.logGroups(account.id, groupPrefix.trim(), more ? groupsToken : null, region);
      if (seq !== groupSeq) return;
      groups = more ? [...(groups ?? []), ...r.groups] : r.groups;
      groupsToken = r.next_token ?? null;
      groupsError = '';
      // Open on a group, never an empty "pick one" pane: the deep-linked prefix's
      // first match, else the last one viewed, else the first.
      if (!selected && groups.length) {
        selected = linkIsPrefix ? groups[0].name : (initialSelection('aws.logs', groups, (g) => g.name) ?? groups[0].name);
      }
    } catch (e) {
      if (seq !== groupSeq) return;
      groupsError = awsLoadText(e);
      if (!more) groups = null;
    } finally {
      if (seq === groupSeq) groupsLoading = false;
    }
  }

  // Region change → regions list + groups; prefix typing → debounced reload.
  // A REAL switch (not the first run) also drops everything that named the
  // old region's groups — the selected group, its streams, the Insights group
  // list and the loaded events — so nothing from region A is queried in B.
  let lastRegion: string | null = null;
  $effect(() => {
    const r = region;
    untrack(() => {
      if (lastRegion !== null && lastRegion !== r) {
        selected = '';
        streams = [];
        stream = '';
        insightGroups = [];
        eventsAbort?.abort();
        resetEvents();
        eventsError = '';
      }
      lastRegion = r;
      void aws.loadRegions();
      void loadGroups();
    });
  });
  let prefixTimer: ReturnType<typeof setTimeout> | null = null;
  let firstPrefix = true;
  $effect(() => {
    void groupPrefix;
    if (firstPrefix) {
      firstPrefix = false;
      return;
    }
    if (prefixTimer) clearTimeout(prefixTimer);
    prefixTimer = setTimeout(() => void loadGroups(), 400);
  });

  function pickGroup(name: string): void {
    if (selected === name) return;
    selected = name;
    rememberSelection('aws.logs', name);
    router.replace(logsRoute(account.id, name, region));
  }

  // ── streams ──
  let streams = $state<AwsLogStream[]>([]);
  let stream = $state(''); // '' = all streams
  $effect(() => {
    const g = selected;
    stream = '';
    streams = [];
    if (!g) return;
    untrack(() => {
      const rg = region;
      awsApi
        .logStreams(account.id, g, '', null, rg)
        .then((r) => {
          if (selected === g && region === rg) streams = r.streams;
        })
        .catch(() => {
          // The stream picker is optional — "All streams" still works.
        });
    });
  });

  // ── events ──
  let rangeId = $state('15m');
  let pattern = $state('');
  let appliedPattern = $state('');
  // Raw: replaced wholesale (pushEvents), never mutated — a deep proxy over a
  // 5000-row ring re-wrapped every row on each 2 s tail append.
  let events = $state.raw<AwsLogEvent[]>([]);
  let eventsToken = $state<string | null>(null);
  let eventsLoading = $state(false);
  let eventsError = $state('');
  let tail = $state(false);
  let detail = $state<AwsLogEvent | null>(null);
  let eventsAbort: AbortController | null = null;
  const seen = new Set<string>();

  function rangeMs(): number {
    return RANGES.find((r) => r.id === rangeId)?.ms ?? 15 * 60_000;
  }

  function resetEvents(): void {
    events = [];
    seen.clear();
    eventsToken = null;
    detail = null;
  }

  /** Append unseen events; returns how many were new. */
  function pushEvents(list: AwsLogEvent[]): number {
    const fresh = list.filter((e) => !seen.has(e.id));
    if (!fresh.length) return 0;
    for (const e of fresh) seen.add(e.id);
    let next = [...events, ...fresh];
    if (next.length > RING) {
      for (const e of next.slice(0, next.length - RING)) seen.delete(e.id);
      next = next.slice(next.length - RING);
    }
    events = next;
    return fresh.length;
  }

  async function loadEvents(more = false): Promise<void> {
    const g = selected;
    if (!g) return;
    eventsAbort?.abort();
    const ctl = new AbortController();
    eventsAbort = ctl;
    if (!more) resetEvents();
    eventsLoading = true;
    try {
      const r = await awsApi.logEvents(
        account.id,
        {
          group: g,
          streams: stream ? [stream] : undefined,
          pattern: appliedPattern,
          start: Date.now() - rangeMs(),
          token: more ? eventsToken : null,
          max: 500,
          region,
        },
        ctl.signal,
      );
      if (ctl.signal.aborted || selected !== g) return;
      pushEvents(r.events);
      eventsToken = r.next_token ?? null;
      eventsError = '';
    } catch (e) {
      if (ctl.signal.aborted) return;
      eventsError = awsLoadText(e);
    } finally {
      if (eventsAbort === ctl) eventsLoading = false;
    }
  }

  // Reload whenever the group / stream / range / applied pattern changes.
  $effect(() => {
    void selected;
    void stream;
    void rangeId;
    void appliedPattern;
    void region;
    if (tab !== 'events') return;
    untrack(() => void loadEvents());
  });

  function applyPattern(): void {
    appliedPattern = pattern.trim();
  }

  // Live tail: forward from the newest timestamp seen; ids dedupe the overlap.
  // Adaptive cadence: the overlap event comes back on every tick, so "got
  // data" means FRESH events after dedupe, not a non-empty response.
  // The filter (stream / pattern / region) is captured — and tracked — here, so
  // changing any of them restarts the tail (aborting the in-flight tick)
  // instead of letting an old-filter tick append after `loadEvents` reset.
  let tailPoller: Poller | null = null;
  $effect(() => {
    if (!tail || !selected || tab !== 'events') return;
    const g = selected;
    const st = stream;
    const pat = appliedPattern;
    const rg = region;
    const cadence = adaptiveCadence({ min: TAIL_MIN_MS, max: TAIL_MAX_MS });
    tailPoller = pollWhileVisible(
      async (signal) => {
        const newest = events.length ? events[events.length - 1].timestamp : Date.now() - 60_000;
        const r = await awsApi.logEvents(
          account.id,
          { group: g, streams: st ? [st] : undefined, pattern: pat, start: newest, max: 500, region: rg },
          signal,
        );
        if (signal.aborted) return;
        if (selected === g && stream === st && appliedPattern === pat && region === rg) {
          cadence.record(pushEvents(r.events) > 0);
        }
      },
      {
        get ms() {
          return cadence.ms;
        },
        immediate: false,
      },
    );
    return () => {
      tailPoller?.stop();
      tailPoller = null;
    };
  });

  // Windowed table + stick-to-bottom while tailing.
  const tw = new TableWindow();
  const win = $derived(tw.range(events.length));
  let listEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    void win;
    tw.measure(listEl);
  });
  $effect(() => {
    void events.length;
    if (tail && listEl) {
      const el = listEl;
      queueMicrotask(() => (el.scrollTop = el.scrollHeight));
    }
  });

  function fmtTs(ms: number): string {
    const d = new Date(ms);
    const p = (n: number, w = 2) => String(n).padStart(w, '0');
    return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
  }
  function pretty(msg: string): { json: boolean; text: string } {
    const t = msg.trim();
    if (t.startsWith('{') || t.startsWith('[')) {
      try {
        return { json: true, text: JSON.stringify(JSON.parse(t), null, 2) };
      } catch {
        // not JSON — show as-is
      }
    }
    return { json: false, text: msg };
  }

  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${what}`);
    } catch {
      toasts.error('Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.');
    }
  }

  function exportEvents(): void {
    const name = (selected || 'logs').replace(/[^A-Za-z0-9._-]+/g, '_').replace(/^_+/, '');
    exportCsv(
      events.map((e) => ({ timestamp: new Date(e.timestamp).toISOString(), stream: e.stream, message: e.message })),
      `${name}.csv`,
    );
  }

  // ── Insights ──
  // svelte-ignore state_referenced_locally
  const SAVED_KEY = `otto.aws.logs.saved.${account.id}`;
  const DEFAULT_QUERY = 'fields @timestamp, @message, @logStream\n| sort @timestamp desc\n| limit 200';

  function readSaved(): SavedQuery[] {
    try {
      const raw = localStorage.getItem(SAVED_KEY);
      const v = raw ? JSON.parse(raw) : [];
      return Array.isArray(v) ? v.filter((x) => x && typeof x.name === 'string' && typeof x.query === 'string') : [];
    } catch {
      return [];
    }
  }
  let saved = $state<SavedQuery[]>(readSaved());
  function writeSaved(list: SavedQuery[]): void {
    saved = list;
    try {
      localStorage.setItem(SAVED_KEY, JSON.stringify(list));
    } catch {
      // storage unavailable — the list still lives for this session
    }
  }
  let query = $state(DEFAULT_QUERY);
  let insightGroups = $state<string[]>([]);
  $effect(() => {
    const g = selected;
    if (g && !untrack(() => insightGroups).includes(g)) insightGroups = [...untrack(() => insightGroups), g];
  });
  let qid = $state<string | null>(null);
  let iRunning = $state(false);
  let iResult = $state<AwsLogsInsightsResults | null>(null);
  let iError = $state('');
  let ranQuery = $state('');
  let iPoll: ReturnType<typeof setTimeout> | null = null;
  /** Region the running Insights query was started in — polls and Stop must
   *  follow it, not the picker (a switch would lose a still-billing query). */
  let iRegion = '';
  /** Status polls issued for the current query (indexes the backoff). */
  let iPollN = 0;

  function stopPolling(): void {
    if (iPoll) clearTimeout(iPoll);
    iPoll = null;
  }

  // Set on unmount: a poll already awaiting its result must not re-arm
  // (`stopPolling` only clears a timer that is pending, not one in flight).
  let disposed = false;

  /** Close the event detail from its ✕ and return focus to the selected
   *  row: the ✕ unmounts with the pane, which otherwise dropped keyboard
   *  focus to <body> and lost the user's place in the table (WCAG 2.4.3). */
  function closeDetail(from: HTMLElement): void {
    const row = from.closest('.detail')?.parentElement?.querySelector<HTMLElement>('tr.trow.sel');
    detail = null;
    if (row?.isConnected) row.focus();
  }

  async function pollResults(id: string): Promise<void> {
    try {
      const r = await awsApi.logsInsightsResults(account.id, id, iRegion);
      if (disposed || qid !== id) return;
      iResult = r;
      if (r.done) {
        iRunning = false;
        if (r.status !== 'Complete') iError = `The query ended with status ${r.status}.`;
        return;
      }
    } catch (e) {
      if (disposed || qid !== id) return;
      iError = awsLoadText(e);
      iRunning = false;
      return;
    }
    if (disposed) return;
    // Same backoff as Athena: 1,1,2,2,3,5 s…, 15 s while the window is hidden
    // (was a fixed 1.5 s — a long query burned a core in `aws` processes).
    iPoll = setTimeout(() => void pollResults(id), statusPollMs(iPollN++, document.visibilityState === 'hidden'));
  }

  async function runInsights(): Promise<void> {
    if (!insightGroups.length || !query.trim()) return;
    stopPolling();
    iError = '';
    iResult = null;
    iRunning = true;
    ranQuery = query;
    const end = Date.now();
    const started = region;
    try {
      const r = await awsApi.logsInsightsStart(
        account.id,
        { groups: insightGroups, query, start: end - rangeMs(), end },
        started,
      );
      iRegion = started;
      qid = r.query_id;
      iPollN = 0;
      void pollResults(r.query_id);
    } catch (e) {
      iRunning = false;
      iError = awsLoadText(e);
    }
  }

  async function stopInsights(): Promise<void> {
    const id = qid;
    stopPolling();
    iRunning = false;
    if (!id) return;
    try {
      await awsApi.logsInsightsStop(account.id, id, iRegion);
      toasts.info('Query stopped');
    } catch (e) {
      toastError('Couldn’t stop', e);
    }
  }

  async function saveQuery(): Promise<void> {
    const name = await confirmer.promptText('Name this Insights query', {
      title: 'Save query',
      confirmLabel: 'Save',
      placeholder: 'Errors by stream',
    });
    if (!name?.trim()) return;
    writeSaved([...saved.filter((s) => s.name !== name.trim()), { name: name.trim(), query }]);
    toasts.success('Query saved', name.trim());
  }
  async function deleteSaved(s: SavedQuery): Promise<void> {
    const ok = await confirmer.ask(`Delete the saved query “${s.name}”?`, { title: 'Delete saved query', confirmLabel: 'Delete' });
    if (ok) writeSaved(saved.filter((x) => x.name !== s.name));
  }

  function exportInsights(): void {
    const r = iResult?.result;
    if (!r) return;
    exportCsv(
      r.rows.map((row) => Object.fromEntries(r.columns.map((c, i) => [c.name, row[i]]))),
      'insights.csv',
    );
  }

  onDestroy(() => {
    disposed = true;
    // Leaving the view stops a still-running Insights query (it would keep
    // scanning — and billing — with nobody polling it).
    if (iRunning && qid) void awsApi.logsInsightsStop(account.id, qid, iRegion).catch(() => {});
    stopPolling();
    eventsAbort?.abort();
    if (prefixTimer) clearTimeout(prefixTimer);
  });

  const groupsLogin = $derived(isLoginRequired(new Error(groupsError)));
  const eventsLogin = $derived(isLoginRequired(new Error(eventsError)));
  const iLogin = $derived(isLoginRequired(new Error(iError)));
;
async () => {

  { const $$_rablooTweiV0C = __sveltets_2_ensureComponent(ViewToolbar); const $$_rablooTweiV0 = new $$_rablooTweiV0C({ target: __sveltets_2_any(), props: {            children:() => { return __sveltets_2_any(0); },"title":`CloudWatch Logs`,"subtitle":groups ? `${groups.length}${groupsToken ? '+' : ''} log groups` : '',filter:groupPrefix,"filterPlaceholder":`Log group prefix, e.g. /aws/lambda/`,"loading":groupsLoading || eventsLoading,"onrefresh":() => (tab === 'events' && selected ? loadEvents() : loadGroups()),}});/*Ωignore_startΩ*/() => groupPrefix = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rablooTweiV0.$$bindings = 'filter';
   { svelteHTML.createElement("label", { "class":`sel`,});
     { svelteHTML.createElement("span", { "class":`lbl`,});  }
    if(aws.regions.length){
       { svelteHTML.createElement("select", {   "bind:value":region,"aria-label":`Region`,});/*Ωignore_startΩ*/() => region = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let r of __sveltets_2_ensureArray(aws.regions)){r.code; { svelteHTML.createElement("option", { "value":r.code,});r.code; }}
       }
    }else{
      
       { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`mono`,"value":region,"aria-label":`Region`,"size":12,"onchange":(e) => { const v = e.currentTarget.value.trim(); if (v) region = v; },});}
    }
   }
   { const $$_sbaT1C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT1C({ target: __sveltets_2_any(), props: {          "label":`Logs view`,"size":`s`,"tabs":VIEW_TABS,"value":tab,"onchange":(id) => (tab = id),}});}
 ViewToolbar}

 { svelteHTML.createElement("div", { "class":`logs`,});
   { svelteHTML.createElement("aside", {    "class":`groups`,"aria-label":`Log groups`,});__sveltets_2_ensureType(String, Number, `${groupsW} `);
    if(groupsLoading && !groups){
       { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":8,"label":`log groups`,}});} }
    } else if (groupsError && !groups){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {     "what":`log groups`,"error":awsErrorText(groupsError),"empty":true,"onretry":() => void loadGroups(),errorActions:() => { async ()/*Ωignore_positionΩ*/ => {if(groupsLogin && onsignin){ { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":onsignin,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}   }}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/ LoadState}
    } else if (groups && groups.length === 0){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {      "icon":`file`,"title":groupPrefix.trim() ? `No log groups starting with “${groupPrefix.trim()}”` : `No log groups in ${region}`,"body":`Group names are case-sensitive prefixes.`,}});}
    } else if (groups){
       { svelteHTML.createElement("ul", { "class":`glist`,});
           for(let g of __sveltets_2_ensureArray(groups)){g.name;
           { svelteHTML.createElement("li", {});
            if(tab === 'insights'){
               { svelteHTML.createElement("label", {   "class":`gi`,"title":g.name,});
                 { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":insightGroups.includes(g.name),"onchange":(e) => {
                    const on = (e.currentTarget as HTMLInputElement).checked;
                    insightGroups = on ? [...insightGroups, g.name] : insightGroups.filter((x) => x !== g.name);
                  },});}
                 { svelteHTML.createElement("span", { "class":`gname`,});g.name; }
               }
            }else{
               { svelteHTML.createElement("button", {      "class":`gi`,"onclick":() => pickGroup(g.name),"title":g.name,});selected === g.name;
                 { svelteHTML.createElement("span", { "class":`gname`,});g.name; }
                 { svelteHTML.createElement("span", { "class":`gmeta`,});g.stored_bytes != null ? fmtBytes(g.stored_bytes) : '';g.retention_days ? ` · ${g.retention_days}d` : ''; }
               }
            }
           }
        }
       }
      if(groupsToken){
         { svelteHTML.createElement("button", {     "class":`btn small more`,"onclick":() => void loadGroups(true),"disabled":groupsLoading,});  }
      }
    }
   }
   { const $$_rediviDenaP1C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP1 = new $$_rediviDenaP1C({ target: __sveltets_2_any(), props: {       width:groupsW,"storageKey":`aws.logs.groupsW`,"label":`Resize log groups list`,}});/*Ωignore_startΩ*/() => groupsW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP1.$$bindings = 'width';}

   { svelteHTML.createElement("section", { "class":`main`,});
    if(tab === 'events'){
      if(!selected){
        
         { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {      "icon":`file`,"title":`No log group open`,"body":`Log groups in this region list on the left; their events show here.`,}});}
      }else{
         { svelteHTML.createElement("div", { "class":`bar`,});
           { svelteHTML.createElement("span", {   "class":`gtitle mono`,"title":selected,});selected; }
           { svelteHTML.createElement("label", { "class":`sel`,});
             { svelteHTML.createElement("span", { "class":`lbl`,});  }
             { svelteHTML.createElement("select", {   "bind:value":stream,"aria-label":`Log stream`,});/*Ωignore_startΩ*/() => stream = __sveltets_2_any(null);/*Ωignore_endΩ*/
               { svelteHTML.createElement("option", {"value":"",});  }
                 for(let s of __sveltets_2_ensureArray(streams)){s.name; { svelteHTML.createElement("option", { "value":s.name,});s.name; }}
             }
           }
           { svelteHTML.createElement("div", {     "class":`seg`,"role":`group`,"aria-label":`Time range`,});
               for(let r of __sveltets_2_ensureArray(RANGES)){r.id;
               { svelteHTML.createElement("button", {     "aria-pressed":rangeId === r.id,"onclick":() => (rangeId = r.id),});rangeId === r.id;r.label; }
            }
           }
           { svelteHTML.createElement("form", {   "class":`pat`,"onsubmit":(e) => { e.preventDefault(); applyPattern(); },});
             { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`mono`,"bind:value":pattern,"placeholder":'Filter pattern: ERROR, { $.level = "error" }',"aria-label":`Filter pattern`,});/*Ωignore_startΩ*/() => pattern = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("button", {   "class":`btn small`,"type":`submit`,});  }
           }
           { svelteHTML.createElement("button", {        "class":`btn small`,"aria-pressed":tail,"onclick":() => (tail = !tail),"title":`Poll for new events every 2 s (backing off to 10 s while nothing arrives) while this window is visible`,});tail;
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":tail ? 'pause' : 'play',"size":12,}});} tail ? 'Tailing' : 'Live tail';
           }
           { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":exportEvents,"disabled":!events.length,"aria-label":`Export events as CSV`,"title":`Export events as CSV`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":13,}});} }
         }
         { svelteHTML.createElement("div", { "class":`ev-body`,});
           { const $$_div3 = svelteHTML.createElement("div", {      "class":`tbl-wrap`,"onscroll":tw.onscroll,});tw.active(events.length);listEl = $$_div3;tw.viewH= $$_div3.clientHeight;
            if(eventsLoading && !events.length){
               { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS5C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS5C({ target: __sveltets_2_any(), props: {    "rows":8,"label":`events`,}});} }
            } else if (eventsError && !events.length){
               { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL4 = new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {     "what":`log events`,"error":awsErrorText(eventsError),"empty":true,"onretry":() => void loadEvents(),errorActions:() => { async ()/*Ωignore_positionΩ*/ => {if(eventsLogin && onsignin){ { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":onsignin,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}   }}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL4.$$prop_def;/*Ωignore_endΩ*/ LoadState}
            } else if (!events.length){
               { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {       "icon":`file`,"title":`No events in this range`,"body":appliedPattern ? `Nothing matched “${appliedPattern}” in the last ${rangeId}. Widen the range or change the pattern.` : `Nothing was logged in the last ${rangeId}. Widen the range or turn on Live tail.`,}});}
            }else{
               { svelteHTML.createElement("table", { "class":`tbl`,});
                 { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "class":`ts`,});  } { svelteHTML.createElement("th", { "class":`hide-sm`,});  } { svelteHTML.createElement("th", {});  } } }
                 { svelteHTML.createElement("tbody", {});
                  if(win.top){ { svelteHTML.createElement("tr", {   "class":`tw-spacer`,"aria-hidden":`true`,}); { svelteHTML.createElement("td", {   "colspan":3,"style":`height:${win.top}px`,}); } }}
                     for(let e of __sveltets_2_ensureArray((events.slice(win.start, win.end)))){e.id;
                     { svelteHTML.createElement("tr", {        "class":`trow`,"tabindex":0,"onclick":() => (detail = e),"onkeydown":(k) => { if (k.target === k.currentTarget && (k.key === 'Enter' || k.key === ' ')) { k.preventDefault(); detail = e; } },});detail?.id === e.id;
                       { svelteHTML.createElement("td", { "class":`mono dim ts`,});fmtTs(e.timestamp); }
                       { svelteHTML.createElement("td", {   "class":`mono dim hide-sm`,"title":e.stream,});e.stream; }
                       { svelteHTML.createElement("td", {   "class":`mono msg`,"title":e.message,});e.message; }
                     }
                  }
                  if(win.bottom){ { svelteHTML.createElement("tr", {   "class":`tw-spacer`,"aria-hidden":`true`,}); { svelteHTML.createElement("td", {   "colspan":3,"style":`height:${win.bottom}px`,}); } }}
                 }
               }
              if(eventsToken && !tail){
                 { svelteHTML.createElement("div", { "class":`more-row`,}); { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void loadEvents(true),"disabled":eventsLoading,});  } }
              }
              if(events.length >= RING){ { svelteHTML.createElement("p", { "class":`note`,});   RING.toLocaleString();  }}
            }
           }
          if(detail){
            const p = pretty(detail.message);
             { svelteHTML.createElement("div", {   "class":`detail`,"aria-label":`Event detail`,});
               { svelteHTML.createElement("div", { "class":`d-head`,});
                 { svelteHTML.createElement("span", { "class":`mono dim`,});fmtTs(detail.timestamp); }
                 { svelteHTML.createElement("span", {   "class":`mono dim d-stream`,"title":detail.stream,});detail.stream; }
                 { svelteHTML.createElement("span", { "class":`spacer`,}); }
                 { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void copy(detail?.message ?? '', 'message'),"aria-label":`Copy message`,"title":`Copy message`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":13,}});} }
                 { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":(ev) => closeDetail(ev.currentTarget),"aria-label":`Close event detail`,"title":`Close`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":13,}});} }
               }
               { svelteHTML.createElement("pre", {  "class":`mono d-body`,});p.json;p.text; }
             }
          }
         }
      }
    }else{
       { svelteHTML.createElement("div", { "class":`ins`,});
         { svelteHTML.createElement("div", { "class":`bar`,});
           { svelteHTML.createElement("span", { "class":`lbl`,});insightGroups.length ? `${plural(insightGroups.length, 'group')}` : 'Tick log groups on the left'; }
           { svelteHTML.createElement("div", {     "class":`seg`,"role":`group`,"aria-label":`Time range`,});
               for(let r of __sveltets_2_ensureArray(RANGES)){r.id;
               { svelteHTML.createElement("button", {     "aria-pressed":rangeId === r.id,"onclick":() => (rangeId = r.id),});rangeId === r.id;r.label; }
            }
           }
           { svelteHTML.createElement("span", { "class":`spacer`,}); }
          if(saved.length){
             { svelteHTML.createElement("select", {      "aria-label":`Saved queries`,"value":"","onchange":(e) => {
                const s = saved.find((x) => x.name === (e.currentTarget as HTMLSelectElement).value);
                if (s) query = s.query;
                (e.currentTarget as HTMLSelectElement).value = '';
              },});
               { svelteHTML.createElement("option", {"value":"",});  }
                 for(let s of __sveltets_2_ensureArray(saved)){s.name; { svelteHTML.createElement("option", { "value":s.name,});s.name; }}
             }
          }
           { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void saveQuery(),"disabled":!query.trim(),});  }
          if(iRunning){
             { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void stopInsights(),"title":`Stop the Insights query`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}  }
          }else{
             { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":() => void runInsights(),"disabled":!insightGroups.length || !query.trim(),"title":!insightGroups.length ? 'Tick at least one log group' : 'Run the query (billed per GB scanned)',}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}  }
          }
           { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":exportInsights,"disabled":!iResult?.result.rows.length,"aria-label":`Export results as CSV`,"title":`Export results as CSV`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":13,}});} }
         }
         { svelteHTML.createElement("textarea", {           "dir":`ltr`,"class":`mono q`,"bind:value":query,"rows":4,"spellcheck":`false`,"aria-label":`Logs Insights query`,});/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
        if(saved.length){
           { svelteHTML.createElement("div", { "class":`saved`,});
               for(let s of __sveltets_2_ensureArray(saved)){s.name;
               { svelteHTML.createElement("span", { "class":`sq-chip`,});
                 { svelteHTML.createElement("button", {     "class":`chip-l`,"onclick":() => (query = s.query),"title":s.query,});s.name; }
                 { svelteHTML.createElement("button", {       "class":`chip-x`,"onclick":() => void deleteSaved(s),"aria-label":`Delete saved query ${s.name}`,"title":`Delete`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
               }
            }
           }
        }
         { svelteHTML.createElement("div", { "class":`ins-res`,});
          if(iError && !iResult){
             { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL4 = new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {     "what":`the Insights results`,"error":awsErrorText(iError),"empty":true,"onretry":() => void runInsights(),errorActions:() => { async ()/*Ωignore_positionΩ*/ => {if(iLogin && onsignin){ { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":onsignin,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}   }}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {errorActions} = $$_etatSdaoL4.$$prop_def;/*Ωignore_endΩ*/ LoadState}
          } else if (!iResult && !iRunning){
             { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {      "icon":`search`,"title":`Run a Logs Insights query`,"body":`Results over the selected groups and time range appear here. Insights bills per GB scanned.`,}});}
          }else{
            if(iResult){
               { svelteHTML.createElement("p", { "class":`stats dim`,});
                iResult.status;iResult.records_matched != null ? ` · ${Math.round(iResult.records_matched).toLocaleString()} matched` : '';iResult.records_scanned != null ? ` · ${Math.round(iResult.records_scanned).toLocaleString()} scanned` : '';iResult.bytes_scanned != null ? ` · ${fmtBytes(iResult.bytes_scanned)}` : '';
               }
            }
             { const $$_dirGstluseR4C = __sveltets_2_ensureComponent(ResultsGrid); new $$_dirGstluseR4C({ target: __sveltets_2_any(), props: {            "result":iResult?.result ?? null,"error":iError || null,"statement":ranQuery,"connectionId":null,"running":iRunning,"oncancel":() => void stopInsights(),}});}
          }
         }
       }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LogsView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LogsView__SvelteComponent_ = ReturnType<typeof LogsView__SvelteComponent_>;
/*Ωignore_endΩ*/export default LogsView__SvelteComponent_;