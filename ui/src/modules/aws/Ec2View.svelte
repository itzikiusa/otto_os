<script lang="ts">
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  // EC2: instances table (state pill, name, id, type, AZ, IPs, launch) with a
  // state filter + region switcher; row actions start/stop/reboot [Edit] — stop
  // and reboot need the instance id typed; a right-side drawer (AwsDrawer,
  // same look as the Kubernetes ResourceDrawer) shows Overview (key fields +
  // tags), Metrics (CloudWatch via MetricsPanel) and Raw JSON via JsonTree.
  import { untrack } from 'svelte';
  import { aws } from '../../lib/stores/aws.svelte';
  import { awsApi, isLoginRequired } from '../../lib/api/aws';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import JsonTree from '../database/JsonTree.svelte';
  import ViewToolbar from './ViewToolbar.svelte';
  import AwsDrawer from './AwsDrawer.svelte';
  import MetricsPanel from './MetricsPanel.svelte';
  import { fmtAgo, fmtDate } from './util';
  import type { AwsAccount, Ec2Action, Ec2Instance, Ec2InstanceDetail } from '../../lib/api/types';

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  }
  let { account, onsignin }: Props = $props();

  $effect(() => { void resourceAccess.load('aws_account', account.id); });
  const canStart = $derived(resourceAccess.can('aws_account', account.id, 'ec2_start', 'aws_ec2', 'edit'));
  const canStop = $derived(resourceAccess.can('aws_account', account.id, 'ec2_stop', 'aws_ec2', 'edit'));
  const canReboot = $derived(resourceAccess.can('aws_account', account.id, 'ec2_reboot', 'aws_ec2', 'edit'));
  const canEdit = $derived(canStart || canStop || canReboot);
  const STATES = ['pending', 'running', 'stopping', 'stopped', 'shutting-down', 'terminated'];

  // The view is remounted per account (AwsPage {#key}s on account+service), so
  // seeding the region from the initial account is intentional.
  // svelte-ignore state_referenced_locally
  let region = $state(account.region);
  let stateFilter = $state('');
  let filter = $state('');
  let auto = $state(false);
  let loading = $state(false);
  let error = $state('');
  const instances = $derived(aws.ec2[`${account.id}:${region}`] ?? null);

  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    let list = instances ?? [];
    if (stateFilter) list = list.filter((i) => i.state === stateFilter);
    if (q)
      list = list.filter((i) =>
        [i.name, i.instance_id, i.type, i.private_ip, i.public_ip, i.az, ...Object.values(i.tags ?? {})]
          .filter(Boolean)
          .some((v) => String(v).toLowerCase().includes(q)),
      );
    return list;
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      await aws.loadEc2(account.id, region);
      error = '';
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  // Load when the region changes (and on mount); `region` is the only dep.
  $effect(() => {
    void region;
    untrack(() => {
      void aws.loadRegions();
      void load();
    });
  });

  // Actions in flight per instance (disables its row buttons).
  let busy = $state<Record<string, boolean>>({});

  async function act(i: Ec2Instance, action: Ec2Action): Promise<void> {
    if (!resourceAccess.can('aws_account', account.id, `ec2_${action}`, 'aws_ec2', 'edit')) return;
    const label = i.name ? `${i.name} (${i.instance_id})` : i.instance_id;
    if (action === 'start') {
      const ok = await confirmer.ask(`Start ${label}?`, { title: 'Start instance', confirmLabel: 'Start', danger: false });
      if (!ok) return;
    } else {
      const typed = await confirmer.promptText(
        `${action === 'stop' ? 'Stop' : 'Reboot'} ${label}${account.environment === 'prod' ? ' — this is PRODUCTION' : ''}? Type the instance id to confirm.`,
        { title: `${action === 'stop' ? 'Stop' : 'Reboot'} instance`, confirmLabel: action === 'stop' ? 'Stop' : 'Reboot', placeholder: i.instance_id },
      );
      if (typed === null) return;
      if (typed !== i.instance_id) {
        toasts.warn('Instance id did not match — cancelled');
        return;
      }
    }
    busy = { ...busy, [i.instance_id]: true };
    try {
      const r = await awsApi.ec2Action(account.id, i.instance_id, action, region);
      toasts.success(`${action} sent`, `${i.instance_id}: ${r.previous_state} → ${r.current_state}`);
      await load();
    } catch (e) {
      toasts.error(`${action} failed`, e instanceof Error ? e.message : String(e));
    } finally {
      busy = { ...busy, [i.instance_id]: false };
    }
  }

  // Detail drawer. The drawer's `inst` is refreshed from the list after a
  // power action so the state pill follows the instance.
  type DrawerTab = 'overview' | 'metrics' | 'raw';
  const DRAWER_TABS: { id: DrawerTab; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    { id: 'metrics', label: 'Metrics' },
    { id: 'raw', label: 'Raw JSON' },
  ];
  let drawerTab = $state<DrawerTab>('overview');
  let detail = $state<{ inst: Ec2Instance; full: Ec2InstanceDetail | null; error: string } | null>(null);
  const drawerInst = $derived.by(() => {
    const d = detail;
    if (!d) return null;
    return instances?.find((i) => i.instance_id === d.inst.instance_id) ?? d.inst;
  });
  function pillClass(state: string): string {
    if (state === 'running') return 'ok';
    if (state === 'terminated') return 'bad';
    if (state === 'pending' || state === 'stopping' || state === 'shutting-down') return 'warn';
    return '';
  }
  async function openDetail(i: Ec2Instance): Promise<void> {
    if (detail?.inst.instance_id !== i.instance_id) drawerTab = 'overview';
    detail = { inst: i, full: null, error: '' };
    try {
      const d = await awsApi.ec2Instance(account.id, i.instance_id, region);
      if (detail?.inst.instance_id === i.instance_id) detail = { inst: i, full: d, error: '' };
    } catch (e) {
      if (detail?.inst.instance_id === i.instance_id)
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

  function menu(e: MouseEvent | KeyboardEvent, i: Ec2Instance): void {
    const stopped = i.state === 'stopped';
    const running = i.state === 'running';
    ctxMenu.show(e, [
      { label: 'Details', icon: 'info', action: () => void openDetail(i) },
      { label: 'Copy instance id', icon: 'copy', action: () => void copy(i.instance_id, 'instance id') },
      ...(i.private_ip ? [{ label: `Copy private IP ${i.private_ip}`, icon: 'copy', action: () => void copy(i.private_ip!, 'IP') }] : []),
      ...(i.public_ip ? [{ label: `Copy public IP ${i.public_ip}`, icon: 'copy', action: () => void copy(i.public_ip!, 'IP') }] : []),
      ...(canEdit
        ? [
            { separator: true },
            { label: 'Start', icon: 'play', disabled: !canStart || !stopped, action: () => void act(i, 'start') },
            { label: 'Reboot', icon: 'refresh', disabled: !canReboot || !running, action: () => void act(i, 'reboot') },
            { label: 'Stop', icon: 'x', danger: true, disabled: !canStop || !running, action: () => void act(i, 'stop') },
          ]
        : []),
    ]);
  }

  const loginNeeded = $derived(isLoginRequired(new Error(error)));
  const counts = $derived.by(() => {
    const c: Record<string, number> = {};
    for (const i of instances ?? []) c[i.state] = (c[i.state] ?? 0) + 1;
    return c;
  });
</script>

<ViewToolbar
  title="EC2"
  subtitle={instances ? `${instances.length} instances · ${counts.running ?? 0} running` : ''}
  bind:filter
  filterPlaceholder="Filter name, id, IP, tag…"
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
  <label class="sel">
    <span class="lbl">State</span>
    <select bind:value={stateFilter} aria-label="State filter">
      <option value="">all</option>
      {#each STATES as s (s)}<option value={s}>{s}{counts[s] ? ` (${counts[s]})` : ''}</option>{/each}
    </select>
  </label>
</ViewToolbar>

<div class="body">
<div class="tbl-wrap">
  {#if loading && !instances}
    <div class="pad"><Skeleton rows={8} /></div>
  {:else if error}
    <EmptyState icon="cloud" title="Couldn't list instances" body={error} actionLabel={loginNeeded ? 'Sign in' : 'Retry'} onaction={loginNeeded ? onsignin : () => void load()} />
  {:else if shown.length === 0}
    <EmptyState icon="box" title={filter || stateFilter ? 'No matching instances' : `No instances in ${region}`} />
  {:else}
    <table class="tbl">
      <thead>
        <tr>
          <th>State</th><th>Name</th><th>Instance</th><th class="hide-sm">Type</th><th class="hide-sm">AZ</th>
          <th class="hide-md">Private IP</th><th class="hide-md">Public IP</th><th class="hide-md">Launched</th><th class="act"></th>
        </tr>
      </thead>
      <tbody>
        {#each shown as i (i.instance_id)}
          <tr
            class="trow"
            class:sel={detail?.inst.instance_id === i.instance_id}
            tabindex="0"
            onclick={() => void openDetail(i)}
            onkeydown={(e) => { if (e.key === 'Enter') void openDetail(i); }}
            oncontextmenu={(e) => menu(e, i)}
          >
            <td><span class="pill {i.state}">{i.state}</span></td>
            <td class="strong" title={i.name ?? ''}>{i.name ?? '—'}</td>
            <td class="mono">{i.instance_id}</td>
            <td class="mono hide-sm">{i.type}</td>
            <td class="mono hide-sm">{i.az ?? '—'}</td>
            <td class="mono hide-md">{i.private_ip ?? '—'}</td>
            <td class="mono hide-md">{i.public_ip ?? '—'}</td>
            <td class="dim hide-md" title={fmtDate(i.launch_time)}>{fmtAgo(i.launch_time)}</td>
            <td class="act">
              <button class="icon-btn" onclick={(e) => menu(e, i)} aria-label={`Actions for ${i.instance_id}`} title="Actions" disabled={busy[i.instance_id]}>⋯</button>
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
    kind="instance"
    name={inst.name ?? inst.instance_id}
    id={inst.instance_id}
    status={inst.state}
    statusClass={pillClass(inst.state)}
    tabs={DRAWER_TABS}
    tab={drawerTab}
    ontab={(t) => (drawerTab = t as DrawerTab)}
    onclose={() => (detail = null)}
  >
    {#if drawerTab === 'overview'}
      <div class="dt">
        <div class="dt-top">
          <span class="mono">{inst.type}</span>
          <span class="mono dim">{inst.az ?? ''}</span>
          {#if canEdit}
            <span class="spacer"></span>
            <button class="ghost sm" onclick={() => void act(inst, 'start')} disabled={!canStart || inst.state !== 'stopped' || busy[inst.instance_id]}><Icon name="play" size={12} /> Start</button>
            <button class="ghost sm" onclick={() => void act(inst, 'reboot')} disabled={!canReboot || inst.state !== 'running' || busy[inst.instance_id]}><Icon name="refresh" size={12} /> Reboot</button>
            <button class="ghost sm danger" onclick={() => void act(inst, 'stop')} disabled={!canStop || inst.state !== 'running' || busy[inst.instance_id]}><Icon name="x" size={12} /> Stop</button>
          {/if}
        </div>
        <dl class="kv">
          <dt>Instance id</dt><dd class="mono">{inst.instance_id}</dd>
          <dt>Type</dt><dd class="mono">{inst.type}</dd>
          <dt>AZ</dt><dd class="mono">{inst.az ?? '—'}</dd>
          <dt>Private IP</dt><dd class="mono">{inst.private_ip ?? '—'}</dd>
          <dt>Public IP</dt><dd class="mono">{inst.public_ip ?? '—'}</dd>
          <dt>VPC / subnet</dt><dd class="mono">{inst.vpc_id ?? '—'} / {inst.subnet_id ?? '—'}</dd>
          <dt>Platform</dt><dd>{inst.platform ?? 'linux'}</dd>
          <dt>Launched</dt><dd>{fmtDate(inst.launch_time)} <span class="dim">({fmtAgo(inst.launch_time)})</span></dd>
        </dl>
        <h3>Tags</h3>
        {#if Object.keys(inst.tags ?? {}).length === 0}
          <p class="dim">No tags.</p>
        {:else}
          <div class="tags">
            {#each Object.entries(inst.tags).sort(([a], [b]) => a.localeCompare(b)) as [k, v] (k)}
              <span class="tag" title={`${k}=${v}`}><strong>{k}</strong>={v}</span>
            {/each}
          </div>
        {/if}
      </div>
    {:else if drawerTab === 'metrics'}
      {#key `${account.id}/${region}/${inst.instance_id}`}
        <MetricsPanel accountId={account.id} namespace="AWS/EC2" dimValue={inst.instance_id} {region} instanceType={inst.type} {onsignin} />
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
  .trow.sel {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .tbl {
    width: 100%;
    /* With the drawer open the pane gets narrow; keep the columns readable
       and let .tbl-wrap scroll sideways instead of squeezing every cell. */
    min-width: 760px;
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
  .pill.running {
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
  }
  .pill.pending,
  .pill.stopping,
  .pill.shutting-down {
    color: var(--status-warn);
    background: color-mix(in srgb, var(--status-warn) 16%, transparent);
  }
  .pill.terminated {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 14%, transparent);
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
  .dt-top {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .spacer {
    flex: 1;
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
  .tag {
    font-size: 11.5px;
    padding: 2px 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .raw {
    font-size: 12px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 8px;
    background: var(--bg);
  }
  .ghost {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 10px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    font-size: 12px;
  }
  .ghost.danger {
    color: var(--status-exited);
  }
  .ghost:disabled {
    opacity: 0.5;
    cursor: default;
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
