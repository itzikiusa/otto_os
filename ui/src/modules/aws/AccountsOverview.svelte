<script lang="ts">
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import ResourceAccess from '../../lib/components/ResourceAccess.svelte';
  // Accounts overview: one card per AWS account — identity (account id + role
  // from the caller ARN), region, per-service permission chips (green allowed /
  // grey denied / hollow unknown), "Sign in" when the probe says credentials
  // expired, and the service links. Gear/⋯ → edit / re-check / delete (Admin).
  import { aws, AWS_SERVICES } from '../../lib/stores/aws.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { router } from '../../lib/router.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EnvBadge from '../../lib/components/EnvBadge.svelte';
  import { fmtAgo, roleFromArn, awsErrorText } from './util';
  import { createLimiter } from '../../lib/poll';
  import { onDestroy } from 'svelte';
  import type { AwsAccount, Feature } from '../../lib/api/types';

  interface Props {
    onadd: () => void;
    onedit: (a: AwsAccount) => void;
    ondelete: (a: AwsAccount) => void;
    onsignin: (a: AwsAccount) => void;
  }
  let { onadd, onedit, ondelete, onsignin }: Props = $props();

  // The account filter sits above the cards it narrows (only once there are
  // enough accounts to need it) — not in the page header.
  let filter = $state('');
  const showFilter = $derived(aws.accounts.length > 3);

  const canAdmin = $derived(auth.isRoot);
  const visible = $derived.by(() => {
    const q = filter.trim().toLowerCase();
    if (!q) return aws.accounts;
    return aws.accounts.filter(
      (a) =>
        a.name.toLowerCase().includes(q) ||
        (a.profile ?? '').toLowerCase().includes(q) ||
        (a.identity?.account ?? '').includes(q) ||
        a.region.toLowerCase().includes(q),
    );
  });

  let accessFor = $state<string | null>(null);
  $effect(() => { for (const resource of aws.accounts) void resourceAccess.load('aws_account', resource.id); });

  function menu(e: MouseEvent | KeyboardEvent, a: AwsAccount): void {
    ctxMenu.show(e, [
      ...(resourceAccess.can('aws_account', a.id, 'manage_access', 'aws', 'admin')
        ? [{ label: 'Manage access…', icon: 'key', action: () => { accessFor = a.id; } }] : []),
      { label: 'Re-check permissions', disabled: !resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'view'), icon: 'refresh', action: () => void aws.loadPermissions(a.id, true) },
      ...(resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'edit') && a.auth_mode === 'profile'
        ? [{ label: 'Sign in (aws sso login)', icon: 'key', action: () => onsignin(a) }]
        : []),
      ...(resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'admin')
        ? [
            { label: 'Edit…', icon: 'edit', action: () => onedit(a) },
            { separator: true },
            { label: 'Delete…', icon: 'trash', danger: true, action: () => ondelete(a) },
          ]
        : []),
    ]);
  }

  function chipState(a: AwsAccount, svc: (typeof AWS_SERVICES)[number]['id']): 'allowed' | 'denied' | 'unknown' {
    return aws.perms(a.id)?.services[svc] ?? 'unknown';
  }

  // First paint of a card whose row carries no cached probe (accounts created
  // through the API / MCP, or whose 10-min snapshot expired server-side and
  // was never re-requested) used to show five hollow "unknown" chips until
  // someone clicked re-check. Probe such accounts once per mount instead —
  // at most 2 at a time: each probe spawns 7 `aws` CLI processes, and firing
  // every un-probed account at once (10 accounts → 70 processes) pinned the
  // CPU. Queued probes are dropped on leave; an explicit re-check bypasses
  // the gate.
  const probed = new Set<string>();
  const probeGate = createLimiter(2);
  const probeLife = new AbortController();
  onDestroy(() => probeLife.abort());
  $effect(() => {
    for (const a of aws.accounts) {
      if (!resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'view')) continue;
      if (probed.has(a.id) || aws.perms(a.id) || aws.permLoading[a.id]) continue;
      probed.add(a.id);
      const id = a.id;
      // Re-checked while queued (the menu) → skip the now-redundant probe.
      probeGate(async () => { if (!aws.perms(id) && !aws.permLoading[id]) await aws.loadPermissions(id); }, probeLife.signal).catch(() => {});
    }
  });
</script>

<div class="ov">
  {#if (aws.accountsLoading && !aws.accountsLoaded) || (aws.accountsError && aws.accounts.length === 0)}
    <LoadState
      variant="page"
      what="accounts"
      loading={aws.accountsLoading}
      error={aws.accountsError ? awsErrorText(aws.accountsError) : null}
      empty
      rows={3}
      onretry={() => void aws.loadAccounts()}
    />
  {:else if aws.accounts.length === 0}
    <EmptyState
      variant="page"
      icon="cloud"
      title="No AWS accounts yet"
      body={canAdmin
        ? 'Add one from an existing ~/.aws profile (SSO, assume-role…) or with access keys. Otto never writes your ~/.aws files.'
        : 'An administrator needs to add an AWS account before you can browse S3, SQS, EC2, Athena or EKS.'}
      actionLabel={canAdmin ? 'Add account' : undefined}
      actionIcon="plus"
      onaction={canAdmin ? onadd : undefined}
    />
  {:else}
    {#if showFilter}
      <div class="ov-bar">
        <label class="ov-filter input-group">
          <Icon name="search" size={13} />
          <input dir="ltr" type="search" placeholder="Filter accounts…" bind:value={filter} aria-label="Filter accounts" />
        </label>
        {#if filter.trim()}<span class="dim">{visible.length} of {aws.accounts.length}</span>{/if}
      </div>
    {/if}
    {#if visible.length === 0}
      <EmptyState
        icon="search"
        title="No accounts match “{filter.trim()}”"
        actionLabel="Clear filter"
        actionKind="secondary"
        onaction={() => (filter = '')}
      />
    {/if}
    <div class="cards">
      {#each visible as a (a.id)}
        {@const p = aws.perms(a.id)}
        {@const loadingP = aws.permLoading[a.id] === true}
        <article
          class="card"
          class:prod={a.environment === 'prod'}
          data-testid="aws-account-card"
          oncontextmenu={(e) => menu(e, a)}
        >
          <div class="card-top">
            <span class="dot" style="background:{a.color || 'var(--text-dim)'}"></span>
            <h2 class="name">{a.name}</h2>
            <EnvBadge env={a.environment} />
            <button class="icon-btn more" onclick={(e) => menu(e, a)} aria-label={`Actions for ${a.name}`} title={`Actions for ${a.name}`}><Icon name="more" size={14} /></button>
          </div>
          <dl class="meta">
            <dt>Identity</dt>
            <dd class="mono" title={a.identity?.arn ?? ''}>
              {#if a.identity}{a.identity.account} · {roleFromArn(a.identity)}{:else}<span class="dim">unknown</span>{/if}
            </dd>
            <dt>Auth</dt>
            <dd class="mono">
              {a.auth_mode === 'profile' ? `profile ${a.profile ?? ''}` : `keys ${a.access_key_id ?? ''}`}
              {#if a.role_arn}<span class="dim"> → {a.role_arn.split('/').pop()}</span>{/if}
            </dd>
            <dt>Region</dt>
            <dd class="mono">{a.region}</dd>
            {#if a.endpoint_url}
              <dt>Endpoint</dt>
              <dd class="mono ep" title={a.endpoint_url} data-testid="aws-account-endpoint">{a.endpoint_url}</dd>
            {/if}
            {#if p}
              <dt>Checked</dt>
              <dd class="dim">{fmtAgo(p.checked_at)}</dd>
            {/if}
          </dl>
          <div class="chips" aria-label="Service permissions">
            {#each AWS_SERVICES as s (s.id)}
              {@const st = chipState(a, s.id)}
              {@const rbac = resourceAccess.can('aws_account', a.id, s.id === 's3' ? 'discover' : `${s.id}_view`, `aws_${s.id}` as Feature, 'view')}
              <a
                class="chip perm"
                class:ok={st === 'allowed'}
                class:perm-denied={st === 'denied'}
                class:perm-unknown={st === 'unknown'}
                class:perm-off={!rbac}
                href={rbac ? `#/aws/${a.id}/${s.id}` : undefined}
                title={!rbac ? `${s.label}: you lack View on this feature` : st === 'denied' ? `${s.label}: AccessDenied for this account` : st === 'unknown' ? `${s.label}: not probed yet` : s.label}
                aria-disabled={!rbac}
              >
                <Icon name={s.icon} size={12} />
                {s.label}
              </a>
            {/each}
            <button
              class="chip-refresh"
              onclick={() => void aws.loadPermissions(a.id, true)}
              title="Re-check permissions"
              aria-label="Re-check permissions"
              disabled={loadingP || !resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'view')}
            >
              {#if loadingP}<span class="spinner" style="--spinner-size: 12px" aria-hidden="true"></span>{:else}<Icon name="refresh" size={12} />{/if}
            </button>
          </div>
          {#if p?.login_required}
            <div class="login-row">
              <span class="warn">Credentials expired or missing.</span>
              {#if a.auth_mode === 'profile' && resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'edit')}
                <button class="btn primary small" onclick={() => onsignin(a)}>
                  <Icon name="key" size={12} /> Sign in
                </button>
              {:else if a.auth_mode === 'access_keys'}
                <span class="dim">Update the keys via Edit.</span>
              {/if}
            </div>
          {/if}
          <div class="card-foot">
            <button class="link" onclick={() => router.go(`aws/${a.id}/s3`)}>Open</button>
            {#if resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'admin')}
              <button class="link" onclick={() => onedit(a)}><Icon name="gear" size={12} /> Edit</button>
            {/if}
          </div>
        </article>
      {/each}
    </div>
  {/if}
</div>

{#if accessFor}
  <Modal title="Manage access" width={780} onclose={() => (accessFor = null)}>
    <ResourceAccess kind="aws_account" resourceId={accessFor} />
  </Modal>
{/if}

<style>
  .ov-bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 16px 20px 0;
  }
  .ov-filter {
    height: 28px;
    padding: 0 8px;
    border-radius: var(--radius-m);
    background: var(--bg);
    width: min(280px, 100%);
  }
  .ov-filter input {
    flex: 1;
    min-width: 0;
  }
  .ov {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: auto;
    height: 100%;
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 12px;
    padding: 18px 20px 40px;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    min-width: 0;
  }
  .card.prod {
    border-color: color-mix(in srgb, var(--status-exited) 45%, transparent);
  }
  .card-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .dot {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .name {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .more {
    flex-shrink: 0;
  }
  .meta {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 2px 10px;
    margin: 0;
    font-size: var(--fs-s);
  }
  .meta .ep {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  dt {
    color: var(--text-dim);
  }
  dd {
    margin: 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dim {
    color: var(--text-dim);
  }
  .chips {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  /* Service links ride the global `.chip` (with its `ok` tone when the
     account's probe allowed the service); only the per-state marks are local. */
  .perm {
    text-decoration: none;
  }
  .perm-denied {
    text-decoration: line-through;
  }
  .perm-unknown {
    background: transparent;
    border-style: dashed;
  }
  .perm-off {
    opacity: 0.45;
    pointer-events: none;
  }
  .chip-refresh {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border: 1px solid var(--border);
    border-radius: 50%;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  
  .login-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--fs-s);
  }
  .warn {
    color: var(--warning);
  }
  .card-foot {
    display: flex;
    gap: 12px;
    margin-top: auto;
  }
  .link {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 0;
    background: transparent;
    color: var(--accent-text);
    font-size: var(--fs-m);
    cursor: pointer;
    padding: 0;
  }
  @media (max-width: 640px) {
    .cards {
      grid-template-columns: 1fr;
      padding: 8px 10px 24px;
    }
  }
</style>
