<script lang="ts">
  // Detail drawer for the selected row: Overview (normalized fields + action
  // buttons) / Manifest (YAML, read-only CodeEditor; secrets already redacted
  // server-side) / Describe / Events, and for pods Logs / Terminal / Metrics.
  // Detail + container list are fetched once per selection (aborted when the
  // selection moves on).
  import { untrack } from 'svelte';
  import { stringify as toYaml } from 'yaml';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import { copyText } from '../../lib/clipboard';
  import { isAbortError } from '../../lib/api/client';
  import { k8sApi } from '../../lib/api/k8s';
  import type { K8sDrawerTab } from '../../lib/stores/k8s.svelte';
  import type { K8sContainer, K8sResourceDetail, K8sResourceKind, K8sRow } from '../../lib/api/types';
  import type { ActionDef } from './actions';
  import { actionsFor } from './actions';
  import { formatAge, formatBytes, formatMillicores, healthClass, kindDef } from './k8s-util';
  import LogsView from './LogsView.svelte';
  import ExecView from './ExecView.svelte';
  import MetricsView from './MetricsView.svelte';
  import WorkloadPods from './WorkloadPods.svelte';

  interface Props {
    clusterId: string;
    kind: K8sResourceKind;
    ns: string;
    name: string;
    /** The table row when it's loaded (null while the list is still loading
     *  or the object vanished — the drawer then leans on the manifest). */
    row: K8sRow | null;
    tab: K8sDrawerTab;
    canEdit: boolean;
    /** Open the shell straight away (`s` shortcut). */
    autoExec?: boolean;
    ontab: (t: K8sDrawerTab) => void;
    onclose: () => void;
    onaction: (def: ActionDef, row: K8sRow) => void;
    /** Workloads: jump to one of this object's pods (its own drawer). */
    onopenpod?: (ns: string, pod: string, tab?: K8sDrawerTab) => void;
  }
  let { clusterId, kind, ns, name, row, tab, canEdit, autoExec = false, ontab, onclose, onaction, onopenpod }: Props = $props();

  const isPod = $derived(kind === 'pods');
  const def = $derived(kindDef(kind));
  /** `spec.selector` of a workload (row extra, or the manifest when the row
   *  is gone) — unlocks the Pods + Logs tabs. */
  const selector = $derived.by((): string => {
    if (isPod) return '';
    if (row?.extra?.selector) return row.extra.selector;
    const m = (detail?.manifest as { spec?: { selector?: { matchLabels?: Record<string, string> } } } | null)?.spec?.selector?.matchLabels;
    return m ? Object.entries(m).map(([k, v]) => `${k}=${v}`).join(',') : '';
  });
  const TABS = $derived<{ id: K8sDrawerTab; label: string }[]>([
    { id: 'overview', label: 'Overview' },
    ...(selector
      ? [
          { id: 'pods' as const, label: 'Pods' },
          { id: 'logs' as const, label: 'Logs' },
        ]
      : []),
    { id: 'manifest', label: 'Manifest' },
    { id: 'describe', label: 'Describe' },
    { id: 'events', label: 'Events' },
    ...(isPod
      ? [
          { id: 'logs' as const, label: 'Logs' },
          { id: 'terminal' as const, label: 'Terminal' },
          { id: 'metrics' as const, label: 'Metrics' },
        ]
      : []),
  ]);
  /** Container names across the workload's pod template (Logs container filter). */
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
  let containers: K8sContainer[] = $state([]);

  async function load(): Promise<void> {
    const ac = new AbortController();
    current = ac;
    const sig = ac.signal;
    detail = null;
    detailError = '';
    detailLoading = true;
    containers = [];
    const cid = clusterId;
    const k = kind;
    const n = ns;
    const nm = name;
    const detailP = k8sApi
      .resource(cid, k, n, nm, sig)
      .then((d) => {
        if (!sig.aborted) detail = d;
      })
      .catch((e) => {
        if (!sig.aborted && !isAbortError(e)) detailError = e instanceof Error ? e.message : String(e);
      })
      .finally(() => {
        if (!sig.aborted) detailLoading = false;
      });
    const ctrP = isPod
      ? k8sApi
          .containers(cid, n, nm, sig)
          .then((r) => {
            if (!sig.aborted) containers = r.containers;
          })
          .catch(() => {
            /* logs/exec fall back to kubectl's default container */
          })
      : Promise.resolve();
    await Promise.all([detailP, ctrP]);
  }

  $effect(() => {
    void clusterId;
    void kind;
    void ns;
    void name;
    untrack(() => {
      current?.abort();
      void load();
    });
    return () => current?.abort();
  });

  const yaml = $derived.by(() => {
    if (!detail) return '';
    try {
      return toYaml(detail.manifest, { lineWidth: 0 });
    } catch {
      return JSON.stringify(detail.manifest, null, 2);
    }
  });

  const actions = $derived(row ? actionsFor(kind, row) : []);

  /** Overview facts: normalized row fields first, then kind-specific extras. */
  const facts = $derived.by(() => {
    const out: [string, string][] = [];
    if (!row) return out;
    out.push(['Status', row.status]);
    if (row.ready) out.push(['Ready', row.ready]);
    if (row.restarts != null) out.push(['Restarts', String(row.restarts)]);
    out.push(['Age', formatAge(row.age_seconds)]);
    if (row.node) out.push(['Node', row.node]);
    if (row.ip) out.push(['IP', row.ip]);
    if (row.cpu != null) out.push(['CPU', formatMillicores(row.cpu)]);
    if (row.mem != null) out.push(['Memory', formatBytes(row.mem)]);
    // Extras a fixed fact / section already covers.
    const skip = new Set(['ready', 'selector', 'phase', 'key_count']);
    for (const [k, v] of Object.entries(row.extra ?? {})) if (v && !skip.has(k)) out.push([k.replace(/_/g, ' '), v]);
    return out;
  });

  function tabKey(e: KeyboardEvent, i: number): void {
    if (e.key !== 'ArrowRight' && e.key !== 'ArrowLeft') return;
    e.preventDefault();
    const j = (i + (e.key === 'ArrowRight' ? 1 : -1) + TABS.length) % TABS.length;
    ontab(TABS[j].id);
  }
</script>

<aside class="drawer" aria-label="{def.singular} details" data-testid="k8s-drawer">
  <header class="dr-head">
    <div class="dr-title">
      <span class="dr-kind">{def.singular}</span>
      <span class="dr-name mono" title={name}>{name}</span>
      {#if ns}<span class="dr-ns mono">{ns}</span>{/if}
      {#if row}<span class="status-pill {healthClass(row.health, row.status)}"><span class="hdot"></span>{row.status}</span>{/if}
    </div>
    <button class="icon-btn" onclick={onclose} aria-label="Close details" title="Close (Esc)"><Icon name="x" size={14} /></button>
  </header>

  <div class="dr-tabs" role="tablist" aria-label="Detail tabs">
    {#each TABS as t, i (t.id)}
      <button
        role="tab"
        id="k8s-tab-{t.id}"
        aria-selected={tab === t.id}
        aria-controls="k8s-panel-{t.id}"
        tabindex={tab === t.id ? 0 : -1}
        class:active={tab === t.id}
        onclick={() => ontab(t.id)}
        onkeydown={(e) => tabKey(e, i)}
      >{t.label}</button>
    {/each}
  </div>

  <div class="dr-body" role="tabpanel" id="k8s-panel-{tab}" aria-labelledby="k8s-tab-{tab}">
    {#if tab === 'overview'}
      <div class="ov">
        {#if row}
          {#if isPod || selector || (canEdit && actions.length)}
            <div class="ov-actions">
              {#if isPod}
                <button class="btn small" onclick={() => ontab('logs')}><Icon name="file" size={12} /> Logs</button>
                {#if canEdit}<button class="btn small" onclick={() => ontab('terminal')}><Icon name="terminal" size={12} /> Shell</button>{/if}
              {:else if selector}
                <button class="btn small" onclick={() => ontab('pods')}><Icon name="box" size={12} /> Pods</button>
                <button class="btn small" onclick={() => ontab('logs')}><Icon name="file" size={12} /> Logs</button>
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
          <div class="err">{detailError}</div>
        {:else}
          <div class="dim">This object isn't in the current list any more. The manifest / describe tabs show its last known state, if the API still has it.</div>
        {/if}
      </div>
    {:else if tab === 'manifest'}
      {#if detailLoading}<div class="pad"><Skeleton rows={8} height={16} /></div>
      {:else if detailError}<div class="err pad">{detailError}</div>
      {:else}
        <div class="code-tools">
          <span class="dim">{kind === 'secrets' ? 'Secret values are redacted by the daemon.' : 'managedFields stripped.'}</span>
          <button class="btn small" onclick={() => void copyText(yaml)}><Icon name="copy" size={12} /> Copy</button>
        </div>
        <div class="code">
          {#key `${clusterId}/${kind}/${ns}/${name}`}
            <CodeEditor path="manifest.yaml" root="" content={yaml} readOnly minimal />
          {/key}
        </div>
      {/if}
    {:else if tab === 'describe'}
      {#if detailLoading}<div class="pad"><Skeleton rows={8} height={16} /></div>
      {:else if detailError}<div class="err pad">{detailError}</div>
      {:else}
        <div class="code-tools">
          <span class="dim mono">kubectl describe {def.singular.toLowerCase()} {name}</span>
          <button class="btn small" onclick={() => void copyText(detail?.describe ?? '')}><Icon name="copy" size={12} /> Copy</button>
        </div>
        <pre class="describe mono">{detail?.describe ?? ''}</pre>
      {/if}
    {:else if tab === 'events'}
      {#if detailLoading}<div class="pad"><Skeleton rows={4} height={22} /></div>
      {:else if detailError}<div class="err pad">{detailError}</div>
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
      <WorkloadPods {clusterId} {ns} {selector} {canEdit} onopenpod={(pod, t) => onopenpod?.(ns, pod, t)} />
    {:else if tab === 'logs' && !isPod}
      <LogsView {clusterId} {ns} {selector} title={name} containers={templateContainers} onopenpod={(pod) => onopenpod?.(ns, pod, 'logs')} />
    {:else if tab === 'logs'}
      <LogsView {clusterId} {ns} pod={name} {containers} />
    {:else if tab === 'terminal'}
      <ExecView {clusterId} {ns} pod={name} {containers} autoOpen={autoExec} />
    {:else if tab === 'metrics'}
      <MetricsView {clusterId} {ns} pod={name} />
    {/if}
  </div>
</aside>

<style>
  .drawer {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
    background: var(--surface);
  }
  .dr-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px 6px 14px;
    border-bottom: 1px solid var(--border);
  }
  .dr-title {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: 12.5px;
  }
  .dr-kind {
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
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
    font-size: 11.5px;
  }
  .status-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11.5px;
  }
  .hdot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-dim);
  }
  .health-ok {
    color: var(--status-working);
  }
  .health-ok .hdot {
    background: var(--status-working);
  }
  .health-bad {
    color: var(--status-exited);
  }
  .health-bad .hdot {
    background: var(--status-exited);
  }
  .health-progressing {
    color: var(--accent);
  }
  .health-progressing .hdot {
    background: var(--accent);
  }
  .dr-tabs {
    display: flex;
    gap: 2px;
    padding: 4px 8px 0;
    border-bottom: 1px solid var(--border);
    overflow-x: auto;
  }
  .dr-tabs button {
    border: none;
    background: transparent;
    padding: 6px 10px;
    font-size: 12px;
    color: var(--text-dim);
    cursor: pointer;
    border-bottom: 2px solid transparent;
    white-space: nowrap;
  }
  .dr-tabs button.active {
    color: var(--text);
    border-bottom-color: var(--accent);
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
    font-size: 12.5px;
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
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    margin-bottom: 4px;
  }
  .labels {
    display: flex;
    gap: 4px;
    flex-wrap: wrap;
  }
  .labels .chip {
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
    font-size: 11.5px;
  }
  .code {
    flex: 1;
    min-height: 240px;
  }
  .describe {
    margin: 0;
    padding: 10px 12px;
    font-size: 11.5px;
    line-height: 1.5;
    white-space: pre;
    overflow: auto;
    flex: 1;
  }
  .events {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  .events th {
    position: sticky;
    top: 0;
    background: var(--surface);
    text-align: left;
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  .events td {
    padding: 5px 8px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
    vertical-align: top;
  }
  .events tr.warn td:first-child {
    color: var(--status-exited);
  }
  .events .msg {
    white-space: pre-wrap;
    word-break: break-word;
  }
  .num {
    text-align: right;
  }
  .nowrap {
    white-space: nowrap;
  }
  .pad {
    padding: 12px 14px;
  }
  .small {
    font-size: 11.5px;
  }
  .ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .err {
    color: var(--status-exited);
    font-size: 12px;
    white-space: pre-wrap;
  }
  .dim {
    color: var(--text-dim);
    font-size: 12px;
    line-height: 1.5;
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
