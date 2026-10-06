<script lang="ts">
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
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
  import ResultsGrid from '../database/ResultsGrid.svelte';
  import ViewToolbar from './ViewToolbar.svelte';
  import { awsErrorText, fmtBytes, logsRoute } from './util';
  import type {
    AwsAccount,
    AwsLogEvent,
    AwsLogGroup,
    AwsLogStream,
    AwsLogsInsightsResults,
  } from '../../lib/api/types';

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  }
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
      groupsError = e instanceof Error ? e.message : String(e);
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
  let events = $state<AwsLogEvent[]>([]);
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
      eventsError = e instanceof Error ? e.message : String(e);
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
    } catch (e) {
      toastError('Couldn’t copy', e);
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
  interface SavedQuery {
    name: string;
    query: string;
  }
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

  async function pollResults(id: string): Promise<void> {
    try {
      const r = await awsApi.logsInsightsResults(account.id, id, iRegion);
      if (qid !== id) return;
      iResult = r;
      if (r.done) {
        iRunning = false;
        if (r.status !== 'Complete') iError = `The query ended with status ${r.status}.`;
        return;
      }
    } catch (e) {
      if (qid !== id) return;
      iError = e instanceof Error ? e.message : String(e);
      iRunning = false;
      return;
    }
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
      iError = e instanceof Error ? e.message : String(e);
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
</script>

<ViewToolbar
  title="CloudWatch Logs"
  subtitle={groups ? `${groups.length}${groupsToken ? '+' : ''} log groups` : ''}
  bind:filter={groupPrefix}
  filterPlaceholder="Log group prefix, e.g. /aws/lambda/"
  loading={groupsLoading || eventsLoading}
  onrefresh={() => (tab === 'events' && selected ? loadEvents() : loadGroups())}
>
  <label class="sel">
    <span class="lbl">Region</span>
    {#if aws.regions.length}
      <select bind:value={region} aria-label="Region">
        {#each aws.regions as r (r.code)}<option value={r.code}>{r.code}</option>{/each}
      </select>
    {:else}
      <!-- Commit on change (blur / Enter), not per keystroke: every region
           value reloads the groups. -->
      <input dir="ltr" class="mono" value={region} aria-label="Region" size={12} onchange={(e) => { const v = e.currentTarget.value.trim(); if (v) region = v; }} />
    {/if}
  </label>
  <Tabs label="Logs view" size="s" tabs={VIEW_TABS} value={tab} onchange={(id) => (tab = id)} />
</ViewToolbar>

<div class="logs">
  <aside class="groups" aria-label="Log groups" style:--groups-w="{groupsW}px">
    {#if groupsLoading && !groups}
      <div class="pad"><Skeleton rows={8} label="log groups" /></div>
    {:else if groupsError && !groups}
      <EmptyState
        actionKind={groupsLogin ? 'primary' : 'secondary'}
        icon="warning"
        title="Couldn’t list log groups"
        body={awsErrorText(groupsError)}
        actionLabel={groupsLogin ? 'Sign in' : 'Retry'}
        onaction={groupsLogin ? onsignin : () => void loadGroups()}
      />
    {:else if groups && groups.length === 0}
      <EmptyState icon="file" title={groupPrefix.trim() ? `No log groups starting with “${groupPrefix.trim()}”` : `No log groups in ${region}`} body="Group names are case-sensitive prefixes." />
    {:else if groups}
      <ul class="glist">
        {#each groups as g (g.name)}
          <li>
            {#if tab === 'insights'}
              <label class="gi" title={g.name}>
                <input
                  type="checkbox"
                  checked={insightGroups.includes(g.name)}
                  onchange={(e) => {
                    const on = (e.currentTarget as HTMLInputElement).checked;
                    insightGroups = on ? [...insightGroups, g.name] : insightGroups.filter((x) => x !== g.name);
                  }}
                />
                <span class="gname">{g.name}</span>
              </label>
            {:else}
              <button class="gi" class:on={selected === g.name} onclick={() => pickGroup(g.name)} title={g.name}>
                <span class="gname">{g.name}</span>
                <span class="gmeta">{g.stored_bytes != null ? fmtBytes(g.stored_bytes) : ''}{g.retention_days ? ` · ${g.retention_days}d` : ''}</span>
              </button>
            {/if}
          </li>
        {/each}
      </ul>
      {#if groupsToken}
        <button class="btn small more" onclick={() => void loadGroups(true)} disabled={groupsLoading}>Load more</button>
      {/if}
    {/if}
  </aside>
  <PaneDivider bind:width={groupsW} storageKey="aws.logs.groupsW" label="Resize log groups list" />

  <section class="main">
    {#if tab === 'events'}
      {#if !selected}
        <!-- A group opens automatically when the list has one; this is only
             the nothing-to-read case. -->
        <EmptyState icon="file" title="No log group open" body="Log groups in this region list on the left; their events show here." />
      {:else}
        <div class="bar">
          <span class="gtitle mono" title={selected}>{selected}</span>
          <label class="sel">
            <span class="lbl">Stream</span>
            <select bind:value={stream} aria-label="Log stream">
              <option value="">All streams</option>
              {#each streams as s (s.name)}<option value={s.name}>{s.name}</option>{/each}
            </select>
          </label>
          <div class="seg" role="group" aria-label="Time range">
            {#each RANGES as r (r.id)}
              <button class:on={rangeId === r.id} aria-pressed={rangeId === r.id} onclick={() => (rangeId = r.id)}>{r.label}</button>
            {/each}
          </div>
          <form class="pat" onsubmit={(e) => { e.preventDefault(); applyPattern(); }}>
            <input dir="ltr"
              class="mono"
              bind:value={pattern}
              placeholder={'Filter pattern: ERROR, { $.level = "error" }'}
              aria-label="Filter pattern"
            />
            <button class="btn small" type="submit">Apply</button>
          </form>
          <button class="btn small" class:primary={tail} aria-pressed={tail} onclick={() => (tail = !tail)} title="Poll for new events every 2 s (backing off to 10 s while nothing arrives) while this window is visible">
            <Icon name={tail ? 'pause' : 'play'} size={12} /> {tail ? 'Tailing' : 'Live tail'}
          </button>
          <button class="icon-btn" onclick={exportEvents} disabled={!events.length} aria-label="Export events as CSV" title="Export events as CSV"><Icon name="download" size={13} /></button>
        </div>
        <div class="ev-body">
          <div class="tbl-wrap" class:windowed={tw.active(events.length)} bind:this={listEl} bind:clientHeight={tw.viewH} onscroll={tw.onscroll}>
            {#if eventsLoading && !events.length}
              <div class="pad"><Skeleton rows={8} label="events" /></div>
            {:else if eventsError && !events.length}
              <EmptyState
                actionKind={eventsLogin ? 'primary' : 'secondary'}
                icon="warning"
                title="Couldn’t read log events"
                body={awsErrorText(eventsError)}
                actionLabel={eventsLogin ? 'Sign in' : 'Retry'}
                onaction={eventsLogin ? onsignin : () => void loadEvents()}
              />
            {:else if !events.length}
              <EmptyState
                icon="file"
                title="No events in this range"
                body={appliedPattern ? `Nothing matched “${appliedPattern}” in the last ${rangeId}. Widen the range or change the pattern.` : `Nothing was logged in the last ${rangeId}. Widen the range or turn on Live tail.`}
              />
            {:else}
              <table class="tbl">
                <thead><tr><th class="ts">Time</th><th class="hide-sm">Stream</th><th>Message</th></tr></thead>
                <tbody>
                  {#if win.top}<tr class="tw-spacer" aria-hidden="true"><td colspan="3" style="height:{win.top}px"></td></tr>{/if}
                  {#each events.slice(win.start, win.end) as e (e.id)}
                    <tr class="trow" class:sel={detail?.id === e.id} tabindex="0" onclick={() => (detail = e)} onkeydown={(k) => { if (k.target === k.currentTarget && (k.key === 'Enter' || k.key === ' ')) { k.preventDefault(); detail = e; } }}>
                      <td class="mono dim ts">{fmtTs(e.timestamp)}</td>
                      <td class="mono dim hide-sm" title={e.stream}>{e.stream}</td>
                      <td class="mono msg" title={e.message}>{e.message}</td>
                    </tr>
                  {/each}
                  {#if win.bottom}<tr class="tw-spacer" aria-hidden="true"><td colspan="3" style="height:{win.bottom}px"></td></tr>{/if}
                </tbody>
              </table>
              {#if eventsToken && !tail}
                <div class="more-row"><button class="btn small" onclick={() => void loadEvents(true)} disabled={eventsLoading}>Load more</button></div>
              {/if}
              {#if events.length >= RING}<p class="note">Showing the newest {RING.toLocaleString()} events.</p>{/if}
            {/if}
          </div>
          {#if detail}
            {@const p = pretty(detail.message)}
            <div class="detail" aria-label="Event detail">
              <div class="d-head">
                <span class="mono dim">{fmtTs(detail.timestamp)}</span>
                <span class="mono dim d-stream" title={detail.stream}>{detail.stream}</span>
                <span class="spacer"></span>
                <button class="icon-btn" onclick={() => void copy(detail?.message ?? '', 'message')} aria-label="Copy message" title="Copy message"><Icon name="copy" size={13} /></button>
                <button class="icon-btn" onclick={() => (detail = null)} aria-label="Close event detail" title="Close"><Icon name="x" size={13} /></button>
              </div>
              <pre class="mono d-body" class:json={p.json}>{p.text}</pre>
            </div>
          {/if}
        </div>
      {/if}
    {:else}
      <div class="ins">
        <div class="bar">
          <span class="lbl">{insightGroups.length ? `${plural(insightGroups.length, 'group')}` : 'Tick log groups on the left'}</span>
          <div class="seg" role="group" aria-label="Time range">
            {#each RANGES as r (r.id)}
              <button class:on={rangeId === r.id} aria-pressed={rangeId === r.id} onclick={() => (rangeId = r.id)}>{r.label}</button>
            {/each}
          </div>
          <span class="spacer"></span>
          {#if saved.length}
            <select
              aria-label="Saved queries"
              value=""
              onchange={(e) => {
                const s = saved.find((x) => x.name === (e.currentTarget as HTMLSelectElement).value);
                if (s) query = s.query;
                (e.currentTarget as HTMLSelectElement).value = '';
              }}
            >
              <option value="">Saved queries…</option>
              {#each saved as s (s.name)}<option value={s.name}>{s.name}</option>{/each}
            </select>
          {/if}
          <button class="btn small" onclick={() => void saveQuery()} disabled={!query.trim()}>Save</button>
          {#if iRunning}
            <button class="btn small" onclick={() => void stopInsights()} title="Stop the Insights query"><Icon name="stop" size={12} /> Stop</button>
          {:else}
            <button class="btn small primary" onclick={() => void runInsights()} disabled={!insightGroups.length || !query.trim()} title={!insightGroups.length ? 'Tick at least one log group' : 'Run the query (billed per GB scanned)'}><Icon name="play" size={12} /> Run</button>
          {/if}
          <button class="icon-btn" onclick={exportInsights} disabled={!iResult?.result.rows.length} aria-label="Export results as CSV" title="Export results as CSV"><Icon name="download" size={13} /></button>
        </div>
        <textarea dir="ltr" class="mono q" bind:value={query} rows="4" spellcheck="false" aria-label="Logs Insights query"></textarea>
        {#if saved.length}
          <div class="saved">
            {#each saved as s (s.name)}
              <span class="sq-chip">
                <button class="chip-l" onclick={() => (query = s.query)} title={s.query}>{s.name}</button>
                <button class="chip-x" onclick={() => void deleteSaved(s)} aria-label={`Delete saved query ${s.name}`} title="Delete"><Icon name="x" size={12} /></button>
              </span>
            {/each}
          </div>
        {/if}
        <div class="ins-res">
          {#if iError && !iResult}
            <EmptyState
              actionKind={iLogin ? 'primary' : 'secondary'}
              icon="warning"
              title="The Insights query failed"
              body={awsErrorText(iError)}
              actionLabel={iLogin ? 'Sign in' : 'Retry'}
              onaction={iLogin ? onsignin : () => void runInsights()}
            />
          {:else if !iResult && !iRunning}
            <EmptyState icon="search" title="Run a Logs Insights query" body="Results over the selected groups and time range appear here. Insights bills per GB scanned." />
          {:else}
            {#if iResult}
              <p class="stats dim">
                {iResult.status}{iResult.records_matched != null ? ` · ${Math.round(iResult.records_matched).toLocaleString()} matched` : ''}{iResult.records_scanned != null ? ` · ${Math.round(iResult.records_scanned).toLocaleString()} scanned` : ''}{iResult.bytes_scanned != null ? ` · ${fmtBytes(iResult.bytes_scanned)}` : ''}
              </p>
            {/if}
            <ResultsGrid result={iResult?.result ?? null} error={iError || null} statement={ranQuery} connectionId={null} running={iRunning} oncancel={() => void stopInsights()} />
          {/if}
        </div>
      </div>
    {/if}
  </section>
</div>

<style>
  .pad {
    padding: 12px;
  }
  .sel {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-s);
  }
  .lbl {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .sel select,
  .sel input,
  .bar select,
  .pat input {
    height: 26px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    padding: 0 4px;
    max-inline-size: 220px;
  }
  .logs {
    flex: 1;
    min-height: 0;
    display: flex;
    overflow: hidden;
  }
  .groups {
    flex: 0 0 var(--groups-w);
    border-inline-end: 1px solid var(--border);
    overflow-y: auto;
    min-height: 0;
  }
  .glist {
    list-style: none;
    margin: 0;
    padding: 4px;
  }
  .gi {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    inline-size: 100%;
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    text-align: start;
    padding: 4px 8px;
    border-radius: var(--radius-m);
    cursor: pointer;
  }
  label.gi {
    flex-direction: row;
    align-items: center;
    gap: 6px;
  }
  .gi:hover,
  .gi:focus-visible {
    background: var(--hover);
  }
  .gi.on {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .gname {
    font-size: var(--fs-s);
    max-inline-size: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
  .gmeta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .more {
    margin: 6px 8px 10px;
  }
  .main {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
  }
  .gtitle {
    font-size: var(--fs-s);
    font-weight: 600;
    max-inline-size: 280px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pat {
    display: inline-flex;
    gap: 4px;
    flex: 1;
    min-inline-size: 180px;
  }
  .pat input {
    flex: 1;
    max-inline-size: none;
  }
  .ev-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .tbl-wrap {
    flex: 1;
    min-height: 0;
    overflow: auto;
  }
  .tbl {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
    table-layout: fixed;
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
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
  }
  .tbl th.ts {
    inline-size: 190px;
  }
  .tbl th.hide-sm {
    inline-size: 200px;
  }
  .tbl td {
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tw-spacer td {
    padding: 0;
    border: 0;
  }
  .trow {
    cursor: pointer;
  }
  .trow:hover {
    background: var(--hover);
  }
  .trow:focus-visible {
    background: var(--hover);
    outline: none;
    box-shadow: inset 0 0 0 2px var(--accent-text);
  }
  .trow.sel {
    background: var(--accent-soft);
  }
  .dim {
    color: var(--text-dim);
  }
  .more-row {
    display: flex;
    justify-content: center;
    padding: 8px;
  }
  .note {
    margin: 6px 12px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .detail {
    border-top: 1px solid var(--border);
    max-block-size: 40%;
    display: flex;
    flex-direction: column;
    min-height: 0;
    background: var(--surface);
  }
  .d-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    font-size: var(--fs-s);
  }
  .d-stream {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-inline-size: 0;
  }
  .d-body {
    margin: 0;
    padding: 8px 12px 12px;
    overflow: auto;
    font-size: var(--fs-s);
    white-space: pre-wrap;
    word-break: break-word;
  }
  .spacer {
    flex: 1;
  }
  .ins {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .q {
    margin: 8px 12px 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font-size: var(--fs-s);
    padding: 6px 8px;
    resize: vertical;
    min-block-size: 70px;
  }
  .saved {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    padding: 6px 12px 0;
  }
  .sq-chip {
    display: inline-flex;
    align-items: center;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .sq-chip button {
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .chip-l {
    padding-block: 2px; padding-inline: 8px 4px;
  }
  .chip-x {
    padding-block: 2px; padding-inline: 2px 6px;
    color: var(--text-dim);
  }
  .ins-res {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
    padding-block-start: 8px;
  }
  .stats {
    margin: 0 12px 6px;
    font-size: var(--fs-s);
  }
  @media (max-width: 640px) {
    .logs {
      flex-direction: column;
    }
    .groups {
      flex: 0 0 30%;
      min-block-size: 120px;
      border-inline-end: 0;
      border-bottom: 1px solid var(--border);
    }
    .hide-sm {
      display: none;
    }
    /* Stacked on phone: nothing to resize sideways. */
    .logs > :global(.pane-divider) {
      display: none;
    }
  }
</style>
