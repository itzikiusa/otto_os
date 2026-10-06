<script lang="ts" module>
  import { plural } from '../../lib/plural';
  /** When the previous drawer instance mounted. The workspace re-keys the
   *  drawer per target, so a mount right after another one is j/k navigation. */
  let lastMountAt = 0;
</script>

<script lang="ts">
  import DockedDrawer from '../../lib/components/DockedDrawer.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import { actionOperation } from './permissions';
  // Detail drawer for the selected row: Overview (normalized fields + action
  // buttons) / Manifest (YAML, read-only CodeEditor; secrets already redacted
  // server-side) / Describe / Events, and for pods Logs / Terminal / Metrics.
  // Detail + container list are fetched once per selection (aborted when the
  // selection moves on).
  import { untrack } from 'svelte';
  import { stringify as toYaml } from 'yaml';
  import Icon from '../../lib/components/Icon.svelte';
  import Tabs from '../../lib/components/Tabs.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import { copyText } from '../../lib/clipboard';
  import { isAbortError } from '../../lib/api/client';
  import { k8sApi } from '../../lib/api/k8s';
  import type { K8sDrawerTab } from '../../lib/stores/k8s.svelte';
  import type { K8sContainer, K8sResourceDetail, K8sResourceKind, K8sRow } from '../../lib/api/types';
  import type { ActionDef } from './actions';
  import { actionsFor } from './actions';
  import { clipLongScalars, formatAge, formatBytes, formatMillicores, healthTone, kindDef, podContainers, rowAge } from './k8s-util';
  import LogsView from './LogsView.svelte';
  import ExecView from './ExecView.svelte';
  import MetricsView from './MetricsView.svelte';
  import WorkloadPods from './WorkloadPods.svelte';
  import PodHttpPanel from './PodHttpPanel.svelte';
  import { workloadKindFor } from './podHttp';

  interface Props {
    modal?: boolean;
    /** Desktop column width (the workspace's splitter owns it). */
    width?: string;
    clusterId: string;
    kind: K8sResourceKind;
    ns: string;
    name: string;
    /** The table row when it’s loaded (null while the list is still loading
     *  or the object vanished — the drawer then leans on the manifest). */
    row: K8sRow | null;
    tab: K8sDrawerTab;
    canEdit: boolean;
    /** Open the shell straight away (`s` shortcut). */
    autoExec?: boolean;
    /** The Terminal tab acted on `autoExec` — the owner clears it (one-shot
     *  per open, not per row: re-showing the tab must not open a new shell). */
    onautoexec?: () => void;
    ontab: (t: K8sDrawerTab) => void;
    onclose: () => void;
    onaction: (def: ActionDef, row: K8sRow) => void;
    /** Workloads: jump to one of this object’s pods (its own drawer). */
    onopenpod?: (ns: string, pod: string, tab?: K8sDrawerTab) => void;
    /** Bumped by the workspace after an action on this object (KS-4): the
     *  drawer re-reads its detail quietly. */
    reloadNonce?: number;
    /** Open this workload’s row in the Monitor view (K-2). */
    onmonitor?: (ns: string, workload: string) => void;
  }
  let { modal = false, width, clusterId, kind, ns, name, row, tab, canEdit, autoExec = false, onautoexec, ontab, onclose, onaction, onopenpod, reloadNonce = 0, onmonitor }: Props = $props();

  $effect(() => {
    void resourceAccess.load('k8s_cluster', clusterId);
    if (ns) void resourceAccess.load('k8s_cluster', clusterId, `namespace:${ns}`);
  });
  function canOperation(op: string): boolean {
    return resourceAccess.can('k8s_cluster', clusterId, op, 'kubernetes',
      ['exec', 'apply', 'scale', 'restart', 'delete'].includes(op) ? 'edit' : 'view', ns ? `namespace:${ns}` : undefined);
  }
  const canLogs = $derived(canOperation('logs'));
  const canExec = $derived(canOperation('exec'));
  const isPod = $derived(kind === 'pods');
  const def = $derived(kindDef(kind));
  /** The pod-http workload this drawer is, or — for a pod — the workload that
   *  owns it (ReplicaSet → its Deployment by the pod-template-hash suffix). */
  const httpWorkload = $derived.by((): { kind: string; name: string } | null => {
    const wk = workloadKindFor(kind);
    if (wk) return { kind: wk, name };
    if (!isPod) return null;
    const owner = (detail?.manifest as { metadata?: { ownerReferences?: { kind: string; name: string }[] } } | null)?.metadata?.ownerReferences?.[0];
    if (!owner) return null;
    if (owner.kind === 'ReplicaSet') {
      const hash = (detail?.manifest as { metadata?: { labels?: Record<string, string> } }).metadata?.labels?.['pod-template-hash'];
      return hash && owner.name.endsWith(`-${hash}`) ? { kind: 'deployment', name: owner.name.slice(0, -hash.length - 1) } : { kind: 'replicaset', name: owner.name };
    }
    const k = owner.kind.toLowerCase();
    return ['statefulset', 'daemonset', 'job'].includes(k) ? { kind: k, name: owner.name } : null;
  });
  const canHttp = $derived((isPod || !!workloadKindFor(kind)) && canOperation('workloads_view'));
  /** `spec.selector` of a workload (row extra, or the manifest when the row
   *  is gone) — unlocks the Pods + Logs tabs. */
  const selector = $derived.by((): string => {
    if (isPod) return '';
    if (row?.extra?.selector) return row.extra.selector;
    const m = (detail?.manifest as { spec?: { selector?: { matchLabels?: Record<string, string> } } } | null)?.spec?.selector?.matchLabels;
    return m ? Object.entries(m).map(([k, v]) => `${k}=${v}`).join(',') : '';
  });
  const TABS = $derived<{ id: K8sDrawerTab; label: string }[]>([
    { id: 'overview' as const, label: 'Overview' },
    ...(selector
      ? [
          { id: 'pods' as const, label: 'Pods' },
          { id: 'logs' as const, label: 'Logs' },
        ]
      : []),
    { id: 'manifest' as const, label: 'Manifest' },
    { id: 'describe' as const, label: 'Describe' },
    { id: 'events' as const, label: 'Events' },
    ...(isPod
      ? [
          { id: 'logs' as const, label: 'Logs' },
          { id: 'terminal' as const, label: 'Terminal' },
          { id: 'metrics' as const, label: 'Metrics' },
        ]
      : []),
    ...(canHttp ? [{ id: 'http' as const, label: 'HTTP' }] : []),
  ].filter(t => t.id === 'logs' ? canLogs : t.id === 'terminal' ? canExec : t.id === 'metrics' ? canOperation('metrics') : true));
  /** Container names across the workload’s pod template (Logs container filter). */
  const templateContainers = $derived.by((): K8sContainer[] => {
    if (isPod || !detail) return [];
    const spec = (detail.manifest as { spec?: { template?: { spec?: { containers?: { name: string }[]; initContainers?: { name: string }[] } } } }).spec?.template?.spec;
    const out: K8sContainer[] = [];
    for (const c of spec?.initContainers ?? []) out.push({ name: c.name, init: true } as K8sContainer);
    for (const c of spec?.containers ?? []) out.push({ name: c.name, init: false } as K8sContainer);
    return out;
  });

  let current: AbortController | null = null;
  let detail = $state<K8sResourceDetail | null>(null);
  let detailError = $state('');
  let detailLoading = $state(false);
  // Derived from the pod manifest `/resource` already returns (was a second
  // `/containers` call → another `kubectl get pod -o json` per open).
  const containers = $derived<K8sContainer[]>(isPod && detail ? podContainers(detail.manifest) : []);

  async function load(quiet = false): Promise<void> {
    const ac = new AbortController();
    current = ac;
    const sig = ac.signal;
    if (!quiet || !detail) {
      detail = null;
      detailLoading = true;
    }
    detailError = '';
    const cid = clusterId;
    const k = kind;
    const n = ns;
    const nm = name;
    await k8sApi
      .resource(cid, k, n, nm, sig)
      .then((d) => {
        if (!sig.aborted) detail = d;
      })
      .catch((e) => {
        if (!sig.aborted && !isAbortError(e)) detailError = loadErrorText(e);
      })
      .finally(() => {
        if (!sig.aborted) detailLoading = false;
      });
  }

  function retry(): void {
    current?.abort();
    void load();
  }

  /** Header refresh button / `reloadNonce`: re-read without blanking the tab. */
  let refreshing = $state(false);
  async function refresh(): Promise<void> {
    current?.abort();
    refreshing = true;
    try {
      await load(true);
    } finally {
      refreshing = false;
    }
  }
  let seenNonce = untrack(() => reloadNonce);
  $effect(() => {
    const n = reloadNonce;
    if (n === seenNonce) return;
    seenNonce = n;
    untrack(() => void refresh());
  });

  // Rapid target changes (j/k with the drawer open — ~30 Hz on key repeat,
  // each re-keying this drawer) settle for 150 ms before the get + describe +
  // events kubectl calls go out; the overview renders from the row meanwhile.
  // A deliberate open (nothing mounted just before) loads at once.
  $effect(() => {
    void clusterId;
    void kind;
    void ns;
    void name;
    let timer: ReturnType<typeof setTimeout> | undefined;
    untrack(() => {
      current?.abort();
      current = null;
      const now = performance.now();
      const rapid = now - lastMountAt < 400;
      lastMountAt = now;
      if (!rapid) {
        void load();
        return;
      }
      detail = null;
      detailError = '';
      detailLoading = true;
      timer = setTimeout(() => void load(), 150);
    });
    return () => {
      if (timer) clearTimeout(timer);
      current?.abort();
    };
  });

  function manifestText(manifest: unknown): string {
    try {
      return toYaml(manifest, { lineWidth: 0 });
    } catch {
      return JSON.stringify(manifest, null, 2);
    }
  }
  // The view shows scalars clipped at 64 KiB (SC-19: a 1 MB one-line value
  // is one enormous highlighted line) and soft-wraps; Copy re-serializes the
  // full manifest on demand.
  const clippedManifest = $derived(detail ? clipLongScalars(detail.manifest) : { value: null, clipped: 0 });
  const yaml = $derived(detail ? manifestText(clippedManifest.value) : '');
  function copyManifest(): void {
    if (!detail) return;
    void copyText(clippedManifest.clipped ? manifestText(detail.manifest) : yaml);
  }

  const actions = $derived(row ? actionsFor(kind, row).filter(a => a.id.startsWith('argocd_') ? resourceAccess.can('k8s_cluster', clusterId, actionOperation(a.id), 'kubernetes', 'edit') : canOperation(actionOperation(a.id))) : []);

  /** Overview facts: normalized row fields first, then kind-specific extras. */
  const facts = $derived.by(() => {
    const out: [string, string][] = [];
    if (!row) return out;
    out.push(['Status', row.status]);
    if (row.ready) out.push(['Ready', row.ready]);
    if (row.restarts != null) out.push(['Restarts', String(row.restarts)]);
    out.push(['Age', formatAge(rowAge(row))]);
    if (row.node) out.push(['Node', row.node]);
    if (row.ip) out.push(['IP', row.ip]);
    if (row.cpu != null) out.push(['CPU', formatMillicores(row.cpu)]);
    if (row.mem != null) out.push(['Memory', formatBytes(row.mem)]);
    // Extras a fixed fact / section already covers.
    const skip = new Set(['ready', 'selector', 'phase', 'key_count']);
    for (const [k, v] of Object.entries(row.extra ?? {})) if (v && !skip.has(k)) out.push([k.replace(/_/g, ' '), v]);
    return out;
  });

  const statusTone = $derived(row ? healthTone(row.health, row.status) : 'neutral');

</script>

<!-- The chrome (desktop column / phone sheet, ✕, Esc, modal registration,
     focus) is the shared DockedDrawer; the host workspace owns the width. -->
<DockedDrawer open title="{def.singular} details" {onclose} {modal} {width} testid="k8s-drawer">
  {#snippet head()}
    <div class="dr-headrow">
      <div class="dr-title">
        <span class="dr-kind">{def.singular}</span>
        <span class="dr-name mono" title={name}>{name}</span>
        {#if ns}<span class="dr-ns mono">{ns}</span>{/if}
        {#if row}<Badge tone={statusTone} label={row.status} dot live={statusTone === 'info'} />{/if}
      </div>
      <button class="icon-btn" onclick={() => void refresh()} disabled={refreshing} aria-label="Refresh details" title="Refresh details" data-testid="k8s-drawer-refresh"><Icon name="refresh" size={14} /></button>
    </div>
  {/snippet}

  <Tabs label="Detail tabs" idBase="k8s" tabs={TABS} value={tab} onchange={ontab} />

  <div class="dr-body" role="tabpanel" id="k8s-panel-{tab}" aria-labelledby="k8s-tab-{tab}">
    {#if tab === 'overview'}
      <div class="ov">
        {#if row}
          {#if isPod || selector || (canEdit && actions.length)}
            <div class="ov-actions">
              {#if isPod}
                <button class="btn small" disabled={!canLogs} title={canLogs ? undefined : "You don’t have permission to read logs in this namespace"} onclick={() => ontab('logs')}><Icon name="file" size={12} /> Logs</button>
                {#if canExec}<button class="btn small" onclick={() => ontab('terminal')}><Icon name="terminal" size={12} /> Shell</button>{/if}
              {:else if selector}
                <button class="btn small" onclick={() => ontab('pods')}><Icon name="box" size={12} /> Pods</button>
                <button class="btn small" disabled={!canLogs} title={canLogs ? undefined : "You don’t have permission to read logs in this namespace"} onclick={() => ontab('logs')}><Icon name="file" size={12} /> Logs</button>
              {/if}
              {#if onmonitor && httpWorkload}
                <button class="btn small" onclick={() => onmonitor(ns, httpWorkload.name)} title="Open {httpWorkload.name} in the Monitor (history, restarts, req/s)" data-testid="k8s-drawer-monitor"><Icon name="gauge" size={12} /> Monitor</button>
              {/if}
              {#if canEdit}
                {#each actions as a (a.id + a.label)}
                  <button class="btn small" class:danger={a.danger} onclick={() => onaction(a, row)}>
                    {#if a.icon}<Icon name={a.icon} size={12} />{/if}{a.label}
                  </button>
                {/each}
              {/if}
            </div>
          {/if}
          <dl class="facts">
            {#each facts as [k, v] (k)}
              <dt>{k}</dt>
              <dd class:mono={/ip|node|revision|repo|path|version|image/i.test(k)} title={v}>{v}</dd>
            {/each}
          </dl>
          {#if row.images?.length}
            <div class="sec">
              <div class="sec-title">Images</div>
              {#each row.images as im (im)}<div class="mono small ell" title={im}>{im}</div>{/each}
            </div>
          {/if}
          {#if selector}
            <div class="sec">
              <div class="sec-title">Selector</div>
              <div class="labels">
                {#each selector.split(',') as kv (kv)}<span class="chip mono" title={kv}>{kv}</span>{/each}
              </div>
            </div>
          {/if}
          {#if Object.keys(row.labels ?? {}).length}
            <div class="sec">
              <div class="sec-title">Labels</div>
              <div class="labels">
                {#each Object.entries(row.labels) as [k, v] (k)}<span class="chip mono" title="{k}={v}">{k}={v}</span>{/each}
              </div>
            </div>
          {/if}
        {:else if detailLoading}
          <Skeleton rows={4} height={22} />
        {:else if detailError}
          <LoadState what="this object" variant="compact" error={detailError} empty={true} onretry={retry} />
        {:else}
          <div class="dim">This object isn’t in the current list any more. The manifest / describe tabs show its last known state, if the API still has it.</div>
        {/if}
      </div>
    {:else if tab === 'manifest'}
      {#if detailLoading}<div class="pad"><Skeleton rows={8} height={16} /></div>
      {:else if detailError}<div class="pad"><LoadState what="this object" variant="compact" error={detailError} empty={true} onretry={retry} /></div>
      {:else}
        <div class="code-tools">
          <span class="dim">{kind === 'secrets' ? 'Secret values are redacted by the daemon.' : 'managedFields stripped.'}{clippedManifest.clipped ? ` ${plural(clippedManifest.clipped, 'long value')} shortened — Copy has the full manifest.` : ''}</span>
          <button class="btn small" onclick={copyManifest}><Icon name="copy" size={12} /> Copy</button>
        </div>
        <div class="code">
          {#key `${clusterId}/${kind}/${ns}/${name}`}
            <CodeEditor path="manifest.yaml" root="" content={yaml} readOnly minimal wrap />
          {/key}
        </div>
      {/if}
    {:else if tab === 'describe'}
      {#if detailLoading}<div class="pad"><Skeleton rows={8} height={16} /></div>
      {:else if detailError}<div class="pad"><LoadState what="this object" variant="compact" error={detailError} empty={true} onretry={retry} /></div>
      {:else}
        <div class="code-tools">
          <span class="dim mono">kubectl describe {def.singular.toLowerCase()} {name}</span>
          <button class="btn small" onclick={() => void copyText(detail?.describe ?? '')}><Icon name="copy" size={12} /> Copy</button>
        </div>
        <pre class="describe mono">{detail?.describe ?? ''}</pre>
      {/if}
    {:else if tab === 'events'}
      {#if detailLoading}<div class="pad"><Skeleton rows={4} height={22} /></div>
      {:else if detailError}<div class="pad"><LoadState what="this object" variant="compact" error={detailError} empty={true} onretry={retry} /></div>
      {:else if !detail?.events.length}<div class="dim pad">No events for this object.</div>
      {:else}
        <table class="events">
          <thead><tr><th>Type</th><th>Reason</th><th class="num">Count</th><th>Last seen</th><th>Message</th></tr></thead>
          <tbody>
            {#each detail.events as ev, i (i)}
              <tr class:warn={ev.type !== 'Normal'}>
                <td>{ev.type}</td>
                <td class="mono">{ev.reason}</td>
                <td class="num mono">{ev.count}</td>
                <td class="mono nowrap">{ev.last_seen}</td>
                <td class="msg">{ev.message}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
    {:else if tab === 'pods'}
      <WorkloadPods {clusterId} {ns} {selector} canEdit={canExec} onopenpod={(pod, t) => onopenpod?.(ns, pod, t)} />
    {:else if tab === 'logs' && !isPod && canLogs}
      <LogsView {clusterId} {ns} {selector} title={name} containers={templateContainers} onopenpod={(pod) => onopenpod?.(ns, pod, 'logs')} />
    {:else if tab === 'logs' && canLogs}
      <LogsView {clusterId} {ns} pod={name} {containers} />
    {:else if tab === 'terminal' && canExec}
      <ExecView {clusterId} {ns} pod={name} {containers} autoOpen={autoExec} onautoopened={onautoexec} />
    {:else if tab === 'metrics' && canOperation('metrics')}
      <MetricsView {clusterId} {ns} pod={name} workload={httpWorkload?.name ?? ''} />
    {:else if tab === 'http' && canHttp}
      {#if detailLoading && !detail}<div class="pad"><Skeleton rows={4} height={22} /></div>
      {:else}
        <PodHttpPanel {clusterId} {ns} pod={isPod ? name : undefined} workload={httpWorkload} manifest={detail?.manifest ?? null} canMutate={canEdit && canExec} />
      {/if}
    {/if}
  </div>
</DockedDrawer>

<style>
  .dr-headrow {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dr-title {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--fs-m);
  }
  .dr-kind {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .dr-name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .dr-ns {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .dr-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
  }
  .ov {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .ov-actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .facts {
    display: grid;
    grid-template-columns: minmax(80px, auto) 1fr;
    gap: 4px 14px;
    margin: 0;
    font-size: var(--fs-m);
  }
  .facts dt {
    color: var(--text-dim);
    text-transform: capitalize;
  }
  .facts dd {
    margin: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .sec-title {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    margin-bottom: 4px;
  }
  .labels {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  /* The global .chip is inline-flex, where text-overflow never applies to its
     bare text — long label values were cut mid-character. Inline-block lets the
     ellipsis render (line-height matches the chip's 20px box less borders). */
  .labels .chip {
    display: inline-block;
    line-height: 18px;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .code-tools {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-s);
  }
  .code {
    flex: 1;
    min-height: 240px;
  }
  .describe {
    margin: 0;
    padding: 10px 12px;
    font-size: var(--fs-s);
    line-height: 1.5;
    white-space: pre;
    overflow: auto;
    flex: 1;
  }
  .events {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  .events th {
    position: sticky;
    top: 0;
    background: var(--surface);
    text-align: start;
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  .events td {
    padding: 4px 8px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
    vertical-align: top;
  }
  .events tr.warn td:first-child {
    color: var(--danger);
  }
  .events .msg {
    white-space: pre-wrap;
    word-break: break-word;
  }
  .num {
    text-align: end;
  }
  .nowrap {
    white-space: nowrap;
  }
  .pad {
    padding: 12px 14px;
  }
  .small {
    font-size: var(--fs-s);
  }
  .ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
    line-height: 1.5;
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
