<script lang="ts">
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  // EKS: clusters table (region switcher) + detail sheet with nodegroups and the
  // raw describe-cluster JSON. "Open in Kubernetes" [aws_eks Edit + kubernetes
  // Admin] imports a kubeconfig into an Otto-owned file (creates a k8s cluster
  // row) and navigates to `#/kubernetes/<clusterId>`.
  import { untrack } from 'svelte';
  import { aws } from '../../lib/stores/aws.svelte';
  import { awsApi, isLoginRequired } from '../../lib/api/aws';
  import { auth } from '../../lib/stores/auth.svelte';
  import { router } from '../../lib/router.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import JsonTree from '../database/JsonTree.svelte';
  import ViewToolbar from './ViewToolbar.svelte';
  import { fmtAgo, fmtDate } from './util';
  import type { AwsAccount, EksClusterDetail, EksClusterSummary } from '../../lib/api/types';

  interface Props {
    account: AwsAccount;
    onsignin: () => void;
  }
  let { account, onsignin }: Props = $props();

  $effect(() => { void resourceAccess.load('aws_account', account.id); });
  const canImport = $derived(resourceAccess.can('aws_account', account.id, 'eks_import', 'aws_eks', 'edit') && auth.isRoot);
  // The view is remounted per account (AwsPage {#key}s on account+service), so
  // seeding the region from the initial account is intentional.
  // svelte-ignore state_referenced_locally
  let region = $state(account.region);
  let filter = $state('');
  let auto = $state(false);
  let loading = $state(false);
  let error = $state('');
  const clusters = $derived(aws.eks[`${account.id}:${region}`] ?? null);
  const shown = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    const list = clusters ?? [];
    return q ? list.filter((c) => c.name.toLowerCase().includes(q) || (c.version ?? '').includes(q)) : list;
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      await aws.loadEks(account.id, region);
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

  let detail = $state<{ c: EksClusterSummary; d: EksClusterDetail | null; error: string } | null>(null);
  async function openDetail(c: EksClusterSummary): Promise<void> {
    detail = { c, d: null, error: '' };
    try {
      const d = await awsApi.eksCluster(account.id, c.name, region);
      if (detail?.c.name === c.name) detail = { c, d, error: '' };
    } catch (e) {
      if (detail?.c.name === c.name) detail = { c, d: null, error: e instanceof Error ? e.message : String(e) };
    }
  }

  let importing = $state<string | null>(null);
  async function openInK8s(c: EksClusterSummary): Promise<void> {
    if (!canImport) return;
    const ok = await confirmer.ask(
      `Import “${c.name}” into the Kubernetes console? Otto writes an Otto-owned kubeconfig (your ~/.kube/config is untouched) and links it to this AWS account for credentials.`,
      { title: 'Open in Kubernetes', confirmLabel: 'Import', danger: false },
    );
    if (!ok) return;
    importing = c.name;
    try {
      const k = await awsApi.eksImport(account.id, c.name, {}, region);
      toasts.success('Cluster imported', k.name);
      router.go(`kubernetes/${k.id}`);
    } catch (e) {
      toasts.error('Import failed', e instanceof Error ? e.message : String(e));
    } finally {
      importing = null;
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

  function menu(e: MouseEvent | KeyboardEvent, c: EksClusterSummary): void {
    ctxMenu.show(e, [
      { label: 'Details', icon: 'info', action: () => void openDetail(c) },
      ...(c.arn ? [{ label: 'Copy ARN', icon: 'copy', action: () => void copy(c.arn!, 'ARN') }] : []),
      ...(c.endpoint ? [{ label: 'Copy endpoint', icon: 'copy', action: () => void copy(c.endpoint!, 'endpoint') }] : []),
      ...(canImport ? [{ separator: true }, { label: 'Open in Kubernetes…', icon: 'helm', action: () => void openInK8s(c) }] : []),
    ]);
  }

  const loginNeeded = $derived(isLoginRequired(new Error(error)));
</script>

<ViewToolbar
  title="EKS"
  subtitle={clusters ? `${clusters.length} clusters` : ''}
  bind:filter
  filterPlaceholder="Filter clusters…"
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

<div class="tbl-wrap">
  {#if loading && !clusters}
    <div class="pad"><Skeleton rows={5} /></div>
  {:else if error}
    <EmptyState icon="cloud" title="Couldn't list clusters" body={error} actionLabel={loginNeeded ? 'Sign in' : 'Retry'} onaction={loginNeeded ? onsignin : () => void load()} />
  {:else if shown.length === 0}
    <EmptyState icon="helm" title={filter ? 'No matching clusters' : `No EKS clusters in ${region}`} />
  {:else}
    <table class="tbl">
      <thead><tr><th>Cluster</th><th>Status</th><th>Version</th><th class="hide-sm">Endpoint</th><th class="hide-sm">Created</th><th class="act"></th></tr></thead>
      <tbody>
        {#each shown as c (c.name)}
          <tr class="trow" tabindex="0" onclick={() => void openDetail(c)} onkeydown={(e) => { if (e.key === 'Enter') void openDetail(c); }} oncontextmenu={(e) => menu(e, c)}>
            <td class="strong"><Icon name="helm" size={13} /> {c.name}</td>
            <td><span class="pill" class:ok={c.status === 'ACTIVE'} class:warn={c.status !== 'ACTIVE'}>{c.status}</span></td>
            <td class="mono">{c.version ?? '—'}</td>
            <td class="mono dim hide-sm" title={c.endpoint ?? ''}>{c.endpoint ?? '—'}</td>
            <td class="dim hide-sm" title={fmtDate(c.created_at)}>{fmtAgo(c.created_at)}</td>
            <td class="act">
              {#if canImport}
                <button class="ghost sm" onclick={(e) => { e.stopPropagation(); void openInK8s(c); }} disabled={importing === c.name}>
                  <Icon name="helm" size={12} /> {importing === c.name ? 'Importing…' : 'Open in Kubernetes'}
                </button>
              {/if}
            </td>
          </tr>
        {/each}
      </tbody>
    </table>
  {/if}
</div>

{#if detail}
  {@const d = detail}
  <Modal title={d.c.name} width={820} onclose={() => (detail = null)}>
    <div class="dt">
      <div class="dt-top">
        <span class="pill" class:ok={d.c.status === 'ACTIVE'} class:warn={d.c.status !== 'ACTIVE'}>{d.c.status}</span>
        <span class="mono">v{d.c.version ?? '?'}</span>
        <span class="mono dim ell" title={d.c.arn ?? ''}>{d.c.arn ?? ''}</span>
        {#if canImport}
          <span class="spacer"></span>
          <button class="primary sm" onclick={() => void openInK8s(d.c)} disabled={importing === d.c.name}><Icon name="helm" size={12} /> Open in Kubernetes</button>
        {/if}
      </div>
      <h3>Node groups</h3>
      {#if d.error}
        <p class="err">{d.error}</p>
      {:else if !d.d}
        <Skeleton rows={3} />
      {:else if d.d.nodegroups.length === 0}
        <p class="dim">No managed node groups (Fargate-only or self-managed nodes).</p>
      {:else}
        <table class="tbl inner">
          <thead><tr><th>Name</th><th>Status</th><th class="num">Desired</th><th class="num">Min</th><th class="num">Max</th><th>Instance types</th><th>AMI</th></tr></thead>
          <tbody>
            {#each d.d.nodegroups as ng (ng.name)}
              <tr>
                <td class="strong">{ng.name}</td>
                <td><span class="pill" class:ok={ng.status === 'ACTIVE'} class:warn={ng.status !== 'ACTIVE'}>{ng.status}</span></td>
                <td class="num mono">{ng.desired ?? '—'}</td>
                <td class="num mono">{ng.min ?? '—'}</td>
                <td class="num mono">{ng.max ?? '—'}</td>
                <td class="mono">{ng.instance_types.join(', ')}</td>
                <td class="mono dim">{ng.ami_type ?? '—'}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      {/if}
      {#if d.d}
        <h3>Raw</h3>
        <div class="raw mono"><JsonTree value={d.d.cluster} /></div>
      {/if}
    </div>
  </Modal>
{/if}

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
  }
  .tbl-wrap {
    flex: 1;
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
    max-width: 320px;
  }
  .tbl .num {
    text-align: right;
  }
  .tbl .act {
    text-align: right;
    width: 1%;
  }
  .tbl.inner th {
    position: static;
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
  }
  .pill.ok {
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 16%, transparent);
  }
  .pill.warn {
    color: var(--status-warn);
    background: color-mix(in srgb, var(--status-warn) 16%, transparent);
  }
  .dt {
    display: flex;
    flex-direction: column;
    gap: 10px;
    font-size: 12.5px;
  }
  .dt-top {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .ell {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .spacer {
    flex: 1;
  }
  h3 {
    margin: 6px 0 0;
    font-size: 12px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .raw {
    font-size: 12px;
    max-height: 45vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 8px;
    background: var(--bg);
  }
  .primary,
  .ghost {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 4px 10px;
    border-radius: var(--radius-m);
    cursor: pointer;
    font-size: 12px;
    white-space: nowrap;
  }
  .primary {
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--accent-contrast);
    font-weight: 600;
  }
  .ghost {
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
  }
  .primary:disabled,
  .ghost:disabled {
    opacity: 0.55;
    cursor: default;
  }
  @media (max-width: 640px) {
    .hide-sm {
      display: none;
    }
  }
</style>
