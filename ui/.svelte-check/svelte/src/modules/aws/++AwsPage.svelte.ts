///<reference types="svelte" />
;
import { NO_WORKSPACE } from '../../lib/labels';
import { toastError } from '../../lib/toastError';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
// AWS console module. Routes: `#/aws` (accounts overview) ·
// `#/aws/<accountId>/<service>` (service ∈ s3|sqs|ec2|athena|eks|rds) ·
// `#/aws/<accountId>/logs[/<group>[/<region>]]` (CloudWatch Logs) · deep link
// `#/aws/<id>/s3/<bucket>?prefix=<encoded>` (the S3 browser reads/writes it).
// First run: when `/aws/status` says the CLI is missing the InstallPanel takes
// over the whole page. Layout: account/service rail + content; on mobile the
// rail collapses and the overview / service view take the full width.
import { untrack } from 'svelte';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import { aws } from '../../lib/stores/aws.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { router } from '../../lib/router.svelte';
import { registry } from '../../lib/commands.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
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
import LogsView from './LogsView.svelte';
import { awsErrorText, logsRoute } from './util';
import EnvBadge from '../../lib/components/EnvBadge.svelte';
import Badge from '../../lib/components/Badge.svelte';
import type { AwsAccount, AwsService, Feature } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const SERVICES: readonly AwsService[] = ['s3', 'sqs', 'ec2', 'athena', 'eks', 'rds'];

  const routeAccountId = $derived(router.parts[1] ?? null);
  const routeService = $derived.by<AwsService | null>(() => {
    const s = router.parts[2];
    return s && (SERVICES as readonly string[]).includes(s) ? (s as AwsService) : null;
  });
  // CloudWatch Logs is account-level (the `metrics` CloudWatch operation under
  // the `aws` feature), not one of the per-service keys.
  const routeLogs = $derived(router.parts[2] === 'logs');
  const account = $derived(aws.account(routeAccountId));
  const logsAllowed = $derived(routeAccountId ? resourceAccess.can('aws_account', routeAccountId, 'metrics', 'aws', 'view') : false);
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
      toastError('Couldn’t delete', e);
    }
  }

  /** Start the `aws sso login` PTY for an account and open the sign-in sheet. */
  async function signIn(a: AwsAccount): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.error(NO_WORKSPACE, 'The sign-in session is attached to a workspace.');
      return;
    }
    if (!resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'edit')) {
      toasts.warn('Not allowed', 'Signing in needs Edit on the AWS feature');
      return;
    }
    try {
      await aws.beginLogin(a.id, wsId);
    } catch (e) {
      toastError('Couldn’t start sign-in', e);
    }
  }

  // `#/aws/<id>` alone opens the account's first service the user may view
  // (S3 first) instead of a "pick a service" void.
  const firstService = $derived.by<AwsService | null>(() => {
    if (!routeAccountId) return null;
    for (const svc of SERVICES) {
      const feature = `aws_${svc}` as Feature;
      if (resourceAccess.can('aws_account', routeAccountId, svc === 's3' ? 'discover' : `${svc}_view`, feature, 'view')) return svc;
    }
    return null;
  });
  $effect(() => {
    if (routeAccountId && account && !routeService && !routeLogs && firstService) {
      const target = `aws/${routeAccountId}/${firstService}`;
      untrack(() => router.replace(target));
    }
  });

  const canAdmin = $derived(auth.isRoot);

  // ⌘K: the module's verbs, plus one "Open AWS account …" per account.
  $effect(() =>
    registry.register('aws', [
      { id: 'aws.accounts', title: 'Show AWS accounts', group: 'AWS', keywords: 'amazon cloud overview', run: () => router.go('aws') },
      { id: 'aws.add', title: 'Add an AWS account…', group: 'AWS', keywords: 'amazon sso profile keys', disabled: !canAdmin || !aws.installed, run: openCreate },
      ...(account && logsAllowed && !routeLogs
        ? [{ id: 'aws.logs', title: `Open CloudWatch Logs for ${account.name}`, group: 'AWS', keywords: 'cloudwatch log groups insights', run: () => router.go(logsRoute(account.id)) }]
        : []),
      ...aws.accounts.map((a) => ({
        id: `aws.open.${a.id}`,
        title: `Open AWS account ${a.name}`,
        group: 'AWS',
        detail: a.environment,
        keywords: 'amazon s3 sqs ec2 athena eks rds',
        run: () => router.go(`aws/${encodeURIComponent(a.id)}`),
      })),
    ]),
  );
  // No accounts at all → the list pane is hidden and one page-level EmptyState
  // (with the single "Add account" CTA) owns the page.
  // The accounts rail's width — drag / ←→ on the divider, remembered across visits.
  let listW = $state(loadPaneWidth('aws.listW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  const noAccounts = $derived(aws.accountsLoaded && aws.accounts.length === 0 && !routeAccountId);
  const showRail = $derived(!noAccounts && (!viewport.isMobile || !routeAccountId));
  const showContent = $derived(noAccounts || !viewport.isMobile || !!routeAccountId);
  const SERVICE_LABEL: Record<AwsService, string> = { s3: 'S3', sqs: 'SQS', ec2: 'EC2', athena: 'Athena', eks: 'EKS', rds: 'RDS' };
;
async () => {

 { svelteHTML.createElement("div", { "class":`aws-page`,});
  { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {      "title":routeAccountId && account ? account.name : 'AWS',"crumbs":routeAccountId && account && !viewport.isMobile ? [{ label: 'AWS', onclick: () => router.go('aws') }] : [],"subtitle":!routeAccountId && aws.installed ? `S3, SQS, EC2, Athena, EKS and RDS through the aws CLI${aws.status?.version ? ` ${aws.status.version}` : ''}` : undefined,leading:() => { async ()/*Ωignore_positionΩ*/ => {
    if(viewport.isMobile && routeAccountId){
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => router.go('aws'),"aria-label":`Back to accounts`,"title":`Back to accounts`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":14,}});}
       }
    }
  };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
    if(routeAccountId && account){
       { const $$_egdaBvnE0C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE0C({ target: __sveltets_2_any(), props: {  "env":account.environment,}});}
      if(routeService){ { const $$_egdaB0C = __sveltets_2_ensureComponent(Badge); new $$_egdaB0C({ target: __sveltets_2_any(), props: {  "label":SERVICE_LABEL[routeService],}});}} else if (routeLogs){ { const $$_egdaB0C = __sveltets_2_ensureComponent(Badge); new $$_egdaB0C({ target: __sveltets_2_any(), props: {  "label":`CloudWatch Logs`,}});}}
    }
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
    if(aws.installed && routeAccountId && account && !routeLogs && logsAllowed){
       { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":() => router.go(logsRoute(account.id)),...__sveltets_2_empty({"data-testid":`aws-open-logs`}),"aria-label":`CloudWatch Logs`,"title":`CloudWatch Logs`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`text`,"size":12,}});}if(!viewport.isPhone){ }
       }
    }
    if(aws.installed && canAdmin && aws.accounts.length > 0){
       { svelteHTML.createElement("button", {         "class":`btn small primary`,"onclick":openCreate,...__sveltets_2_empty({"data-testid":`aws-add-account`}),"aria-label":`Add account`,"title":`Add account`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}if(!viewport.isPhone){ }
       }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, badge, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
  
  
 PageHeader}

if(!aws.statusLoaded){
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
    
     { svelteHTML.createElement("div", {});
       { svelteHTML.createElement("p", { "class":`boot-note`,});     }
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":4,"label":`the AWS console`,}});}
     }
   PageBody}
} else if (aws.statusError){
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "variant":`page`,"what":`the AWS console`,"error":aws.statusError,"empty":true,"onretry":() => void aws.loadStatus(),}});}
   PageBody}
} else if (!aws.installed){
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}}); { const $$_lenaPllatsnI2C = __sveltets_2_ensureComponent(InstallPanel); new $$_lenaPllatsnI2C({ target: __sveltets_2_any(), props: {}});} PageBody}
}else{
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
   { svelteHTML.createElement("div", {   "class":`aws`,});viewport.isMobile;!showRail;
    if(showRail){
       { svelteHTML.createElement("aside", {   "class":`rail-col`,"style":`--list-pane-w:${listW}px`,});
        if(viewport.isMobile){
           { const $$_weivrevOstnuoccA4C = __sveltets_2_ensureComponent(AccountsOverview); new $$_weivrevOstnuoccA4C({ target: __sveltets_2_any(), props: {        "onadd":openCreate,"onedit":openEdit,"ondelete":(a) => void deleteAccount(a),"onsignin":(a) => void signIn(a),}});}
        }else{
           { const $$_liaRtnuoccA4C = __sveltets_2_ensureComponent(AccountRail); new $$_liaRtnuoccA4C({ target: __sveltets_2_any(), props: {             "activeId":routeAccountId,"activeService":routeService,"onedit":openEdit,"ondelete":(a) => void deleteAccount(a),"onsignin":(a) => void signIn(a),"logsActive":routeLogs,}});}
        }
       }
      if(!viewport.isMobile && showContent){
         { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {       width:listW,"storageKey":`aws.listW`,"label":`Resize the accounts list`,}});/*Ωignore_startΩ*/() => listW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';}
      }
    }
    if(showContent){
       { svelteHTML.createElement("section", { "class":`content`,});
        if(!routeAccountId){
          if(!viewport.isMobile || noAccounts){
             { const $$_weivrevOstnuoccA4C = __sveltets_2_ensureComponent(AccountsOverview); new $$_weivrevOstnuoccA4C({ target: __sveltets_2_any(), props: {        "onadd":openCreate,"onedit":openEdit,"ondelete":(a) => void deleteAccount(a),"onsignin":(a) => void signIn(a),}});}
          }
        } else if (aws.accountsError && !aws.accounts.length){
          
           { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {            "variant":`page`,"what":`AWS accounts`,"loading":aws.accountsLoading,"error":awsErrorText(aws.accountsError),"empty":true,"onretry":() => void aws.loadAccounts(),}});}
        } else if (!aws.accountsLoaded){
           { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS5C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS5C({ target: __sveltets_2_any(), props: {    "rows":5,"label":`accounts`,}});} }
        } else if (!account){
           { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {             "variant":`page`,"icon":`cloud`,"title":`Account not found`,"body":`This AWS account was removed or the link is stale.`,"actionLabel":`Back to accounts`,"onaction":() => router.go('aws'),}});}
        } else if (routeLogs){
          if(!logsAllowed){
             { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`lock`,"title":`No access`,"body":`Reading CloudWatch Logs needs the account’s CloudWatch (metrics) permission. Ask an administrator for a grant.`,}});}
          }else{
            `${account.id}/logs/${aws.accessRevision}`; {
               { const $$_weiVsgoL4C = __sveltets_2_ensureComponent(LogsView); new $$_weiVsgoL4C({ target: __sveltets_2_any(), props: {   account,"onsignin":() => void signIn(account),}});}
            }
          }
        } else if (!routeService){
           { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`lock`,"title":`No AWS services available`,"body":`You don’t have View on any AWS service for this account. Ask an administrator for a grant.`,}});}
        } else if (!serviceAllowedByRbac){
           { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`lock`,"title":`No access`,"body":`You don’t have View on ${routeService.toUpperCase()} for the AWS console. Ask an administrator for a grant.`,}});}
        }else{
          `${account.id}/${routeService}/${aws.accessRevision}`; {
            if(routeService === 's3'){
               { const $$_resworB3S4C = __sveltets_2_ensureComponent(S3Browser); new $$_resworB3S4C({ target: __sveltets_2_any(), props: {   account,"onsignin":() => void signIn(account),}});}
            } else if (routeService === 'sqs'){
               { const $$_weiVsqS4C = __sveltets_2_ensureComponent(SqsView); new $$_weiVsqS4C({ target: __sveltets_2_any(), props: {   account,"onsignin":() => void signIn(account),}});}
            } else if (routeService === 'ec2'){
               { const $$_weiV2cE4C = __sveltets_2_ensureComponent(Ec2View); new $$_weiV2cE4C({ target: __sveltets_2_any(), props: {   account,"onsignin":() => void signIn(account),}});}
            } else if (routeService === 'athena'){
               { const $$_weiVanehtA4C = __sveltets_2_ensureComponent(AthenaView); new $$_weiVanehtA4C({ target: __sveltets_2_any(), props: {   account,"onsignin":() => void signIn(account),}});}
            } else if (routeService === 'eks'){
               { const $$_weiVskE4C = __sveltets_2_ensureComponent(EksView); new $$_weiVskE4C({ target: __sveltets_2_any(), props: {   account,"onsignin":() => void signIn(account),}});}
            }else{
               { const $$_weiVsdR4C = __sveltets_2_ensureComponent(RdsView); new $$_weiVsdR4C({ target: __sveltets_2_any(), props: {   account,"onsignin":() => void signIn(account),}});}
            }
          }
        }
       }
    }
   }
   PageBody}
}
 }

if(wizardOpen){
   { const $$_draziWtnuoccA0C = __sveltets_2_ensureComponent(AccountWizard); new $$_draziWtnuoccA0C({ target: __sveltets_2_any(), props: {       "account":wizardAccount,"onclose":() => (wizardOpen = false),"onsignin":(a) => {
      wizardOpen = false;
      void signIn(a);
    },}});}
}

if(aws.login){
   { const $$_teehSnigoL0C = __sveltets_2_ensureComponent(LoginSheet); new $$_teehSnigoL0C({ target: __sveltets_2_any(), props: {       "accountId":aws.login.accountId,"sessionId":aws.login.sessionId,"onclose":() => aws.endLogin(),}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AwsPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AwsPage__SvelteComponent_ = ReturnType<typeof AwsPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default AwsPage__SvelteComponent_;