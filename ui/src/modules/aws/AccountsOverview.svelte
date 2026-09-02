<script lang="ts">
  // Accounts overview: one card per AWS account — identity (account id + role
  // from the caller ARN), region, per-service permission chips (green allowed /
  // grey denied / hollow unknown), "Sign in" when the probe says credentials
  // expired, and the service links. Gear/⋯ → edit / re-check / delete (Admin).
  import { aws, AWS_SERVICES } from '../../lib/stores/aws.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { router } from '../../lib/router.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EnvPill from './EnvPill.svelte';
  import { fmtAgo, roleFromArn } from './util';
  import type { AwsAccount, Feature } from '../../lib/api/types';

  interface Props {
    onadd: () => void;
    onedit: (a: AwsAccount) => void;
    ondelete: (a: AwsAccount) => void;
    onsignin: (a: AwsAccount) => void;
  }
  let { onadd, onedit, ondelete, onsignin }: Props = $props();

  const canAdmin = $derived(auth.can('aws', 'admin'));
  const canLogin = $derived(auth.can('aws', 'edit'));
  let filter = $state('');
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

  function menu(e: MouseEvent | KeyboardEvent, a: AwsAccount): void {
    ctxMenu.show(e, [
      { label: 'Re-check permissions', icon: 'refresh', action: () => void aws.loadPermissions(a.id, true) },
      ...(canLogin && a.auth_mode === 'profile'
        ? [{ label: 'Sign in (aws sso login)', icon: 'key', action: () => onsignin(a) }]
        : []),
      ...(canAdmin
        ? [
            { label: 'Edit…', icon: 'edit', action: () => onedit(a) },
            { separator: true },
            { label: 'Delete', icon: 'trash', danger: true, action: () => ondelete(a) },
          ]
        : []),
    ]);
  }

  function chipState(a: AwsAccount, svc: (typeof AWS_SERVICES)[number]['id']): 'allowed' | 'denied' | 'unknown' {
    return aws.perms(a.id)?.services[svc] ?? 'unknown';
  }
</script>

<div class="ov">
  <header class="head">
    <div>
      <h1>AWS</h1>
      <p class="sub">
        Accounts are Otto rows (+ Keychain); the console shells out to the <code>aws</code> CLI
        {#if aws.status?.version}<span class="ver mono">v{aws.status.version}</span>{/if}
      </p>
    </div>
    <div class="head-actions">
      {#if aws.accounts.length > 3}
        <label class="filter">
          <Icon name="search" size={13} />
          <input type="search" placeholder="Filter accounts…" bind:value={filter} aria-label="Filter accounts" />
        </label>
      {/if}
      {#if canAdmin}
        <button class="primary" onclick={onadd} data-testid="aws-add-account">
          <Icon name="plus" size={13} /> Add account
        </button>
      {/if}
    </div>
  </header>

  {#if aws.accountsLoading && !aws.accountsLoaded}
    <div class="pad"><Skeleton rows={3} height={90} /></div>
  {:else if aws.accountsError && aws.accounts.length === 0}
    <EmptyState icon="cloud" title="Couldn't load accounts" body={aws.accountsError} actionLabel="Retry" onaction={() => void aws.loadAccounts()} />
  {:else if aws.accounts.length === 0}
    <EmptyState
      icon="cloud"
      title="No AWS accounts yet"
      body={canAdmin
        ? 'Add one from an existing ~/.aws profile (SSO, assume-role…) or with access keys. Otto never writes your ~/.aws files.'
        : 'An administrator needs to add an AWS account before you can browse S3, SQS, EC2, Athena or EKS.'}
      actionLabel={canAdmin ? 'Add account' : undefined}
      onaction={canAdmin ? onadd : undefined}
    />
  {:else}
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
            <EnvPill env={a.environment} />
            <button class="more" onclick={(e) => menu(e, a)} aria-label={`Actions for ${a.name}`} title="Actions">⋯</button>
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
            {#if p}
              <dt>Checked</dt>
              <dd class="dim">{fmtAgo(p.checked_at)}</dd>
            {/if}
          </dl>
          <div class="chips" aria-label="Service permissions">
            {#each AWS_SERVICES as s (s.id)}
              {@const st = chipState(a, s.id)}
              {@const rbac = auth.can(`aws_${s.id}` as Feature, 'view')}
              <a
                class="chip {st}"
                class:norbac={!rbac}
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
              class:spin={loadingP}
              onclick={() => void aws.loadPermissions(a.id, true)}
              title="Re-check permissions"
              aria-label="Re-check permissions"
              disabled={loadingP}
            >
              <Icon name="refresh" size={12} />
            </button>
          </div>
          {#if p?.login_required}
            <div class="login-row">
              <span class="warn">Credentials expired or missing.</span>
              {#if a.auth_mode === 'profile' && canLogin}
                <button class="primary sm" onclick={() => onsignin(a)}>
                  <Icon name="key" size={12} /> Sign in
                </button>
              {:else if a.auth_mode === 'access_keys'}
                <span class="dim">Update the keys via Edit.</span>
              {/if}
            </div>
          {/if}
          <div class="card-foot">
            <button class="link" onclick={() => router.go(`aws/${a.id}/s3`)}>Open</button>
            {#if canAdmin}
              <button class="link" onclick={() => onedit(a)}><Icon name="gear" size={12} /> Edit</button>
            {/if}
          </div>
        </article>
      {/each}
    </div>
  {/if}
</div>

<style>
  .ov {
    display: flex;
    flex-direction: column;
    min-height: 0;
    overflow: auto;
    height: 100%;
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
    padding: 16px 16px 8px;
  }
  h1 {
    margin: 0;
    font-size: 18px;
  }
  .sub {
    margin: 4px 0 0;
    font-size: 12.5px;
    color: var(--text-dim);
  }
  .ver {
    margin-left: 6px;
    font-size: 11px;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--surface-2);
  }
  code {
    font-family: var(--font-mono);
  }
  .head-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .filter {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    color: var(--text-dim);
  }
  .filter input {
    border: 0;
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: 12.5px;
    outline: none;
    width: 160px;
  }
  .primary {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 12px;
    border-radius: var(--radius-m);
    border: 1px solid var(--accent);
    background: var(--accent);
    color: var(--accent-contrast);
    font-weight: 600;
    font-size: 12.5px;
    cursor: pointer;
  }
  .primary.sm {
    padding: 4px 10px;
    font-size: 12px;
  }
  .pad {
    padding: 16px;
  }
  .cards {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
    gap: 12px;
    padding: 8px 16px 24px;
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
    font-size: 14px;
    font-weight: 600;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .more {
    border: 0;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    font-size: 16px;
    line-height: 1;
    padding: 0 4px;
  }
  .meta {
    display: grid;
    grid-template-columns: auto 1fr;
    gap: 3px 10px;
    margin: 0;
    font-size: 12px;
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
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 8px;
    border-radius: 999px;
    font-size: 11.5px;
    text-decoration: none;
    border: 1px solid var(--border);
    color: var(--text);
  }
  .chip.allowed {
    border-color: color-mix(in srgb, var(--status-working) 55%, transparent);
    background: color-mix(in srgb, var(--status-working) 14%, transparent);
    color: var(--status-working);
  }
  .chip.denied {
    background: var(--surface-2);
    color: var(--text-dim);
    text-decoration: line-through;
  }
  .chip.unknown {
    background: transparent;
    color: var(--text-dim);
    border-style: dashed;
  }
  .chip.norbac {
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
  .chip-refresh.spin :global(svg) {
    animation: spin 0.9s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .login-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: 12px;
  }
  .warn {
    color: var(--status-warn);
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
    color: var(--accent);
    font-size: 12.5px;
    cursor: pointer;
    padding: 0;
  }
  @media (max-width: 640px) {
    .cards {
      grid-template-columns: 1fr;
      padding: 8px 10px 24px;
    }
    .head {
      padding: 12px 10px 4px;
    }
  }
</style>
