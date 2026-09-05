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
  import EnvPill from './EnvPill.svelte';
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

  const showRail = $derived(!viewport.isMobile || !routeAccountId);
  const showContent = $derived(!viewport.isMobile || !!routeAccountId || aws.accounts.length === 0);
</script>

{#if !aws.statusLoaded}
  <div class="pad"><Skeleton rows={4} /></div>
{:else if aws.statusError}
  <EmptyState
    icon="cloud"
    title="AWS console unavailable"
    body={aws.statusError}
    actionLabel="Retry"
    onaction={() => void aws.loadStatus()}
  />
{:else if !aws.installed}
  <InstallPanel />
{:else}
  <div class="aws" class:mobile={viewport.isMobile}>
    {#if showRail}
      <aside class="rail-col">
        {#if viewport.isMobile}
          <AccountsOverview onadd={openCreate} onedit={openEdit} ondelete={(a) => void deleteAccount(a)} onsignin={(a) => void signIn(a)} />
        {:else}
          <AccountRail
            activeId={routeAccountId}
            activeService={routeService}
            onadd={openCreate}
            onedit={openEdit}
            ondelete={(a) => void deleteAccount(a)}
          />
        {/if}
      </aside>
    {/if}
    {#if showContent}
      <section class="content">
        {#if viewport.isMobile && routeAccountId}
          <div class="mobile-bar">
            <button class="back" onclick={() => router.go('aws')} aria-label="Back to accounts">
              <Icon name="chevronLeft" size={14} /> Accounts
            </button>
            {#if account}
              <span class="mb-name">{account.name}</span>
              <EnvPill env={account.environment} />
            {/if}
          </div>
        {/if}
        {#if !routeAccountId}
          {#if !viewport.isMobile}
            <AccountsOverview onadd={openCreate} onedit={openEdit} ondelete={(a) => void deleteAccount(a)} onsignin={(a) => void signIn(a)} />
          {/if}
        {:else if !aws.accountsLoaded}
          <div class="pad"><Skeleton rows={5} /></div>
        {:else if !account}
          <EmptyState
            icon="cloud"
            title="Account not found"
            body="This AWS account was removed or the link is stale."
            actionLabel="Back to accounts"
            onaction={() => router.go('aws')}
          />
        {:else if !routeService}
          <EmptyState icon="cloud" title={account.name} body="Pick a service from the rail." />
        {:else if !serviceAllowedByRbac}
          <EmptyState
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
  .pad {
    padding: 16px;
  }
  .aws {
    display: grid;
    grid-template-columns: 224px minmax(0, 1fr);
    height: 100%;
    min-height: 0;
    overflow: hidden;
  }
  .aws.mobile {
    grid-template-columns: minmax(0, 1fr);
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
  .mobile-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .back {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    border: 0;
    background: transparent;
    color: var(--accent);
    font: inherit;
    font-size: 13px;
    cursor: pointer;
    padding: 4px 0;
  }
  .mb-name {
    margin-left: auto;
    font-weight: 600;
    font-size: 13px;
  }
</style>
