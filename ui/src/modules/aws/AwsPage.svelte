<script lang="ts">
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  // AWS console module. Routes: `#/aws` (accounts overview) ·
  // `#/aws/<accountId>/<service>` (service ∈ s3|sqs|ec2|athena|eks|rds) · deep link
  // `#/aws/<id>/s3/<bucket>?prefix=<encoded>` (the S3 browser reads/writes it).
  // First run: when `/aws/status` says the CLI is missing the InstallPanel takes
  // over the whole page. Layout: account/service rail + content; on mobile the
  // rail collapses and the overview / service view take the full width.
  import { untrack } from 'svelte';
  import { aws } from '../../lib/stores/aws.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { router } from '../../lib/router.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import InstallPanel from './InstallPanel.svelte';
  import AccountsOverview from './AccountsOverview.svelte';
  import AccountWizard from './AccountWizard.svelte';
  import AccountRail from './AccountRail.svelte';
  import LoginSheet from './LoginSheet.svelte';
  import S3Browser from './S3Browser.svelte';
  import SqsView from './SqsView.svelte';
  import Ec2View from './Ec2View.svelte';
  import AthenaView from './AthenaView.svelte';
  import EksView from './EksView.svelte';
  import RdsView from './RdsView.svelte';
  import EnvBadge from '../../lib/components/EnvBadge.svelte';
  import type { AwsAccount, AwsService, Feature } from '../../lib/api/types';

  const SERVICES: readonly AwsService[] = ['s3', 'sqs', 'ec2', 'athena', 'eks', 'rds'];

  const routeAccountId = $derived(router.parts[1] ?? null);
  const routeService = $derived.by<AwsService | null>(() => {
    const s = router.parts[2];
    return s && (SERVICES as readonly string[]).includes(s) ? (s as AwsService) : null;
  });
  const account = $derived(aws.account(routeAccountId));
  const serviceFeature = $derived<Feature | null>(
    routeService ? (`aws_${routeService}` as Feature) : null,
  );
  $effect(() => { if (routeAccountId) void resourceAccess.load('aws_account', routeAccountId); });
  const serviceAllowedByRbac = $derived(serviceFeature && routeAccountId ? resourceAccess.can('aws_account', routeAccountId, routeService === 's3' ? 'discover' : `${routeService}_view`, serviceFeature, 'view') : true);

  // Wizard (create / edit) — mounted fresh per open.
  let wizardOpen = $state(false);
  let wizardAccount = $state<AwsAccount | null>(null);

  $effect(()=>resourceAccess.subscribe(change=>{
    if(change.type==='reset' || (change.type==='decision' && change.kind==='aws_account' && change.before?.operations.configure?.allowed && !change.after?.operations.configure?.allowed)){wizardOpen=false;wizardAccount=null;}
  }));
  // Initial load: status (install gate) + accounts. `untrack` so the effect
  // doesn't re-run on the stores it kicks (loaders write `$state`).
  $effect(() => {
    void aws.accessRevision;
    untrack(() => {
      void aws.loadStatus();
      void aws.loadAccounts();
    });
  });

  function openCreate(): void {
    wizardAccount = null;
    wizardOpen = true;
  }
  function openEdit(a: AwsAccount): void {
    wizardAccount = a;
    wizardOpen = true;
  }

  async function deleteAccount(a: AwsAccount): Promise<void> {
    const ok = await confirmer.ask(
      `Delete AWS account “${a.name}”? Its Keychain secret is removed too. Kubernetes clusters imported from it are kept.`,
      { title: 'Delete account', confirmLabel: 'Delete' },
    );
    if (!ok) return;
    try {
      await aws.deleteAccount(a.id);
      toasts.success('Account deleted', a.name);
      if (routeAccountId === a.id) router.go('aws');
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
    }
  }

  /** Start the `aws sso login` PTY for an account and open the sign-in sheet. */
  async function signIn(a: AwsAccount): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.error('No workspace', 'Select a workspace to attach the sign-in session to');
      return;
    }
    if (!resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'edit')) {
      toasts.warn('Not allowed', 'Signing in needs Edit on the AWS feature');
      return;
    }
    try {
      await aws.beginLogin(a.id, wsId);
    } catch (e) {
      toasts.error('Sign-in failed to start', e instanceof Error ? e.message : String(e));
    }
  }

  const canAdmin = $derived(auth.isRoot);
  // No accounts at all → the list pane is hidden and one page-level EmptyState
  // (with the single "Add account" CTA) owns the page.
  const noAccounts = $derived(aws.accountsLoaded && aws.accounts.length === 0 && !routeAccountId);
  const showRail = $derived(!noAccounts && (!viewport.isMobile || !routeAccountId));
  const showContent = $derived(noAccounts || !viewport.isMobile || !!routeAccountId);
  // The overview's account filter lives in the header; AccountsOverview filters by it.
  let filter = $state('');
  const SERVICE_LABEL: Record<AwsService, string> = { s3: 'S3', sqs: 'SQS', ec2: 'EC2', athena: 'Athena', eks: 'EKS', rds: 'RDS' };
</script>

<div class="aws-page">
<PageHeader
  title={routeAccountId && account ? account.name : 'AWS'}
  crumbs={routeAccountId && account && !viewport.isMobile ? [{ label: 'AWS', onclick: () => router.go('aws') }] : []}
  subtitle={!routeAccountId && aws.installed ? `Accounts are Otto rows (+ Keychain); the console shells out to the aws CLI${aws.status?.version ? ` v${aws.status.version}` : ''}` : undefined}
>
  {#snippet leading()}
    {#if viewport.isMobile && routeAccountId}
      <button class="icon-btn" onclick={() => router.go('aws')} aria-label="Back to accounts" title="Back to accounts">
        <Icon name="chevronLeft" size={15} />
      </button>
    {/if}
  {/snippet}
  {#snippet badge()}
    {#if routeAccountId && account}
      <EnvBadge env={account.environment} />
      {#if routeService}<span class="svc-badge">{SERVICE_LABEL[routeService]}</span>{/if}
    {/if}
  {/snippet}
  {#snippet actions()}
    {#if aws.installed && !routeAccountId && aws.accounts.length > 3}
      <label class="filter">
        <Icon name="search" size={13} />
        <input type="search" placeholder="Filter accounts…" bind:value={filter} aria-label="Filter accounts" />
      </label>
    {/if}
    {#if aws.installed && canAdmin && aws.accounts.length > 0}
      <button class="btn primary" onclick={openCreate} data-testid="aws-add-account">
        <Icon name="plus" size={13} /> Add account
      </button>
    {/if}
  {/snippet}
</PageHeader>

{#if !aws.statusLoaded}
  <div class="pad"><Skeleton rows={4} /></div>
{:else if aws.statusError}
  <EmptyState
    variant="page"
    icon="cloud"
    title="AWS console unavailable"
    body={aws.statusError}
    actionLabel="Retry"
    onaction={() => void aws.loadStatus()}
  />
{:else if !aws.installed}
  <div class="aws-scroll"><InstallPanel /></div>
{:else}
  <div class="aws" class:mobile={viewport.isMobile} class:solo={!showRail}>
    {#if showRail}
      <aside class="rail-col">
        {#if viewport.isMobile}
          <AccountsOverview {filter} onadd={openCreate} onedit={openEdit} ondelete={(a) => void deleteAccount(a)} onsignin={(a) => void signIn(a)} />
        {:else}
          <AccountRail
            activeId={routeAccountId}
            activeService={routeService}
            onedit={openEdit}
            ondelete={(a) => void deleteAccount(a)}
          />
        {/if}
      </aside>
    {/if}
    {#if showContent}
      <section class="content">
        {#if !routeAccountId}
          {#if !viewport.isMobile || noAccounts}
            <AccountsOverview {filter} onadd={openCreate} onedit={openEdit} ondelete={(a) => void deleteAccount(a)} onsignin={(a) => void signIn(a)} />
          {/if}
        {:else if !aws.accountsLoaded}
          <div class="pad"><Skeleton rows={5} /></div>
        {:else if !account}
          <EmptyState
            variant="page"
            icon="cloud"
            title="Account not found"
            body="This AWS account was removed or the link is stale."
            actionLabel="Back to accounts"
            onaction={() => router.go('aws')}
          />
        {:else if !routeService}
          <EmptyState variant="page" icon="cloud" title={account.name} body="Pick a service from the rail." />
        {:else if !serviceAllowedByRbac}
          <EmptyState
            variant="page"
            icon="lock"
            title="No access"
            body={`You don't have View on ${routeService.toUpperCase()} for the AWS console. Ask an administrator for a grant.`}
          />
        {:else}
          {#key `${account.id}/${routeService}/${aws.accessRevision}`}
            {#if routeService === 's3'}
              <S3Browser {account} onsignin={() => void signIn(account)} />
            {:else if routeService === 'sqs'}
              <SqsView {account} onsignin={() => void signIn(account)} />
            {:else if routeService === 'ec2'}
              <Ec2View {account} onsignin={() => void signIn(account)} />
            {:else if routeService === 'athena'}
              <AthenaView {account} onsignin={() => void signIn(account)} />
            {:else if routeService === 'eks'}
              <EksView {account} onsignin={() => void signIn(account)} />
            {:else}
              <RdsView {account} onsignin={() => void signIn(account)} />
            {/if}
          {/key}
        {/if}
      </section>
    {/if}
  </div>
{/if}
</div>

{#if wizardOpen}
  <AccountWizard
    account={wizardAccount}
    onclose={() => (wizardOpen = false)}
    onsignin={(a) => {
      wizardOpen = false;
      void signIn(a);
    }}
  />
{/if}

{#if aws.login}
  <LoginSheet
    accountId={aws.login.accountId}
    sessionId={aws.login.sessionId}
    onclose={() => aws.endLogin()}
  />
{/if}

<style>
  .aws-page {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .pad {
    padding: 18px 20px;
  }
  .aws-scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .aws {
    display: grid;
    grid-template-columns: 224px minmax(0, 1fr);
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .aws.mobile,
  .aws.solo {
    grid-template-columns: minmax(0, 1fr);
  }
  .svc-badge {
    font-size: 11px;
    font-weight: 600;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text-dim);
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
  .rail-col {
    border-right: 1px solid var(--border);
    background: var(--bg-sidebar);
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  .aws.mobile .rail-col {
    border-right: 0;
    background: transparent;
    overflow: auto;
  }
  .content {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
</style>
