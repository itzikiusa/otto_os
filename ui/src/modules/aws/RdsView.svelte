<script lang="ts">
  // RDS (read-only): DB instances table (status pill, identifier, engine,
  // class, AZ / Multi-AZ, storage, endpoint, created) with a region switcher
  // and a right-side drawer (AwsDrawer) — Overview (key fields + tags),
  // Metrics (CloudWatch via MetricsPanel), Raw JSON. No start/stop/reboot by
  // design — the daemon has no RDS mutations either.
  import { untrack } from 'svelte';
  import { aws } from '../../lib/stores/aws.svelte';
  import { awsApi, isLoginRequired } from '../../lib/api/aws';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import JsonTree from '../database/JsonTree.svelte';
  import ViewToolbar from './ViewToolbar.svelte';
  import AwsDrawer from './AwsDrawer.svelte';
  import MetricsPanel from './MetricsPanel.svelte';
  import { fmtAgo, fmtDate } from './util';
  import type { AwsAccount, RdsInstance, RdsInstanceDetail } from '../../lib/api/types';

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  }
  let { account, onsignin }: Props = $props();

  // Remounted per account (AwsPage {#key}s on account+service).
  // svelte-ignore state_referenced_locally
  let region = $state(account.region);
  let filter = $state('');
  let auto = $state(false);
  let loading = $state(false);
  let error = $state('');
  const instances = $derived(aws.rds[`${account.id}:${region}`] ?? null);

  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    const list = instances ?? [];
    if (!q) return list;
    return list.filter((i) =>
      [i.identifier, i.engine, i.engine_version, i.class, i.az, i.endpoint, i.db_name, i.status, ...Object.values(i.tags ?? {})]
        .filter(Boolean)
        .some((v) => String(v).toLowerCase().includes(q)),
    );
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      await aws.loadRds(account.id, region);
      error = '';
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void region;
    untrack(() => {
      void aws.loadRegions();
      void load();
    });
  });

  // Detail drawer.
  type DrawerTab = 'overview' | 'metrics' | 'raw';
  const DRAWER_TABS: { id: DrawerTab; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    { id: 'metrics', label: 'Metrics' },
    { id: 'raw', label: 'Raw JSON' },
  ];
  let drawerTab = $state<DrawerTab>('overview');
  let detail = $state<{ inst: RdsInstance; full: RdsInstanceDetail | null; error: string } | null>(null);
  const drawerInst = $derived.by(() => {
    const d = detail;
    if (!d) return null;
    return instances?.find((i) => i.identifier === d.inst.identifier) ?? d.inst;
  });

  function pillClass(status: string): string {
    if (status === 'available') return 'ok';
    if (status === 'failed' || status === 'inaccessible-encryption-credentials' || status === 'storage-full') return 'bad';
    if (status === 'stopped' || status === 'deleting') return '';
    return 'warn';
  }

  async function openDetail(i: RdsInstance): Promise<void> {
    if (detail?.inst.identifier !== i.identifier) drawerTab = 'overview';
    detail = { inst: i, full: null, error: '' };
    try {
      const d = await awsApi.rdsInstance(account.id, i.identifier, region);
      if (detail?.inst.identifier === i.identifier) detail = { inst: i, full: d, error: '' };
    } catch (e) {
      if (detail?.inst.identifier === i.identifier)
        detail = { inst: i, full: null, error: e instanceof Error ? e.message : String(e) };
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

  function endpointOf(i: RdsInstance): string {
    if (!i.endpoint) return '';
    return i.port ? `${i.endpoint}:${i.port}` : i.endpoint;
  }

  function menu(e: MouseEvent | KeyboardEvent, i: RdsInstance): void {
    ctxMenu.show(e, [
      { label: 'Details', icon: 'info', action: () => void openDetail(i) },
      { label: 'Metrics', icon: 'chart', action: () => { void openDetail(i); drawerTab = 'metrics'; } },
      { label: 'Copy identifier', icon: 'copy', action: () => void copy(i.identifier, 'identifier') },
      ...(i.endpoint ? [{ label: `Copy endpoint ${endpointOf(i)}`, icon: 'copy', action: () => void copy(endpointOf(i), 'endpoint') }] : []),
    ]);
  }

  const loginNeeded = $derived(isLoginRequired(new Error(error)));
  const counts = $derived.by(() => {
    const c: Record<string, number> = {};
    for (const i of instances ?? []) c[i.status] = (c[i.status] ?? 0) + 1;
    return c;
  });
</script>

<ViewToolbar
  title="RDS"
  subtitle={instances ? `${instances.length} instances · ${counts.available ?? 0} available` : ''}
  bind:filter
  filterPlaceholder="Filter identifier, engine, endpoint, tag…"
  {loading}
  bind:auto
  onrefresh={() => void load()}
>
  <label class="sel">
    <span class="lbl">Region</span>
    {#if aws.regions.length}
      <select bind:value={region} aria-label="Region">
        {#each aws.regions as r (r.code)}<option value={r.code}>{r.code}</option>{/each}
      </select>
    {:else}
      <input class="mono" bind:value={region} aria-label="Region" size={12} />
    {/if}
  </label>
</ViewToolbar>

<div class="body">
<div class="tbl-wrap">
  {#if loading && !instances}
    <div class="pad"><Skeleton rows={8} /></div>
  {:else if error}
    <EmptyState icon="cloud" title="Couldn't list DB instances" body={error} actionLabel={loginNeeded ? 'Sign in' : 'Retry'} onaction={loginNeeded ? onsignin : () => void load()} />
  {:else if shown.length === 0}
    <EmptyState icon="db" title={filter ? 'No matching instances' : `No DB instances in ${region}`} />
  {:else}
    <table class="tbl">
      <thead>
        <tr>
          <th>Status</th><th>Identifier</th><th class="hide-sm">Engine</th><th class="hide-sm">Class</th><th class="hide-md">AZ</th>
          <th class="hide-md num">Storage</th><th class="hide-md">Endpoint</th><th class="hide-md">Created</th><th class="act"></th>
        </tr>
      </thead>
      <tbody>
        {#each shown as i (i.identifier)}
          <tr
            class="trow"
            class:sel={detail?.inst.identifier === i.identifier}
            tabindex="0"
            onclick={() => void openDetail(i)}
            onkeydown={(e) => { if (e.key === 'Enter') void openDetail(i); }}
            oncontextmenu={(e) => menu(e, i)}
          >
            <td><span class="pill {pillClass(i.status)}">{i.status}</span></td>
            <td class="strong mono" title={i.identifier}>{i.identifier}</td>
            <td class="hide-sm">{i.engine ?? '—'}{#if i.engine_version}<span class="dim"> {i.engine_version}</span>{/if}</td>
            <td class="mono hide-sm">{i.class ?? '—'}</td>
            <td class="mono hide-md">{i.az ?? '—'}{#if i.multi_az}<span class="tag" title="Multi-AZ">MAZ</span>{/if}</td>
            <td class="mono hide-md num">{i.storage_gb != null ? `${i.storage_gb} GB` : '—'}</td>
            <td class="mono hide-md" title={endpointOf(i)}>{endpointOf(i) || '—'}</td>
            <td class="dim hide-md" title={fmtDate(i.created)}>{fmtAgo(i.created)}</td>
            <td class="act">
              <button class="icon-btn" onclick={(e) => menu(e, i)} aria-label={`Actions for ${i.identifier}`} title="Actions">⋯</button>
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

{#if detail && drawerInst}
  {@const d = detail}
  {@const inst = drawerInst}
  <AwsDrawer
    kind="db instance"
    name={inst.identifier}
    id={endpointOf(inst)}
    status={inst.status}
    statusClass={pillClass(inst.status)}
    tabs={DRAWER_TABS}
    tab={drawerTab}
    ontab={(t) => (drawerTab = t as DrawerTab)}
    onclose={() => (detail = null)}
  >
    {#if drawerTab === 'overview'}
      <div class="dt">
        <dl class="kv">
          <dt>Engine</dt><dd>{inst.engine ?? '—'} {inst.engine_version ?? ''}</dd>
          <dt>Class</dt><dd class="mono">{inst.class ?? '—'}</dd>
          <dt>AZ</dt><dd class="mono">{inst.az ?? '—'}{#if inst.multi_az} <span class="tag" title="Multi-AZ">Multi-AZ</span>{/if}</dd>
          <dt>Storage</dt><dd>{inst.storage_gb != null ? `${inst.storage_gb} GB` : '—'}{#if inst.storage_type} <span class="dim">({inst.storage_type})</span>{/if}</dd>
          <dt>Endpoint</dt><dd class="mono">{endpointOf(inst) || '—'}</dd>
          <dt>DB name</dt><dd class="mono">{inst.db_name ?? '—'}</dd>
          <dt>Master user</dt><dd class="mono">{inst.master_username ?? '—'}</dd>
          <dt>Public</dt><dd>{inst.publicly_accessible ? 'yes' : 'no'}</dd>
          <dt>Created</dt><dd>{fmtDate(inst.created)} <span class="dim">({fmtAgo(inst.created)})</span></dd>
        </dl>
        <h3>Tags</h3>
        {#if Object.keys(inst.tags ?? {}).length === 0}
          <p class="dim">No tags.</p>
        {:else}
          <div class="tags">
            {#each Object.entries(inst.tags).sort(([a], [b]) => a.localeCompare(b)) as [k, v] (k)}
              <span class="tag pl" title={`${k}=${v}`}><strong>{k}</strong>={v}</span>
            {/each}
          </div>
        {/if}
      </div>
    {:else if drawerTab === 'metrics'}
      {#key `${account.id}/${region}/${inst.identifier}`}
        <MetricsPanel accountId={account.id} namespace="AWS/RDS" dimValue={inst.identifier} {region} {onsignin} />
      {/key}
    {:else}
      <div class="dt">
        {#if d.error}
          <p class="err">{d.error}</p>
        {:else if !d.full}
          <Skeleton rows={5} />
        {:else}
          <div class="raw mono"><JsonTree value={d.full.raw} /></div>
        {/if}
      </div>
    {/if}
  </AwsDrawer>
{/if}
</div>

<style>
  .pad {
    padding: 12px;
  }
  .sel {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: 12px;
  }
  .lbl {
    color: var(--text-dim);
  }
  .sel select,
  .sel input {
    height: 26px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: 12px;
    padding: 0 4px;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    overflow: hidden;
  }
  .tbl-wrap {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: auto;
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
    white-space: nowrap;
  }
  .tbl td {
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 260px;
  }
  .tbl .num {
    text-align: right;
  }
  .tbl .act {
    width: 32px;
    text-align: right;
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
  .strong {
    font-weight: 500;
  }
  .dim {
    color: var(--text-dim);
  }
  .err {
    color: var(--status-exited);
  }
  .pill {
    display: inline-block;
    font-size: 10.5px;
    font-weight: 600;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    text-transform: lowercase;
  }
  .pill.ok {
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
  }
  .pill.warn {
    color: var(--status-warn);
    background: color-mix(in srgb, var(--status-warn) 16%, transparent);
  }
  .pill.bad {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 14%, transparent);
  }
  .tag {
    margin-left: 6px;
    font-size: 9.5px;
    font-weight: 700;
    padding: 0 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
    letter-spacing: 0.04em;
  }
  .tag.pl {
    margin: 0;
    font-size: 11.5px;
    font-weight: 400;
    padding: 2px 8px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    letter-spacing: 0;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .icon-btn {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    width: 24px;
    height: 24px;
    line-height: 1;
  }
  .dt {
    display: flex;
    flex-direction: column;
    gap: 10px;
    font-size: 12.5px;
    padding: 12px 14px;
  }
  .kv {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 4px 12px;
    margin: 0;
  }
  .kv dt {
    color: var(--text-dim);
  }
  .kv dd {
    margin: 0;
    word-break: break-all;
  }
  h3 {
    margin: 6px 0 0;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .raw {
    font-size: 12px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 8px;
    background: var(--bg);
  }
  @media (max-width: 1024px) {
    .hide-md {
      display: none;
    }
  }
  @media (max-width: 640px) {
    .hide-sm {
      display: none;
    }
  }
</style>
