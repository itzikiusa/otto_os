///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
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
;

  interface Props {
    onadd: () => void;
    onedit: (a: AwsAccount) => void;
    ondelete: (a: AwsAccount) => void;
    onsignin: (a: AwsAccount) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
;
async () => {

 { svelteHTML.createElement("div", { "class":`ov`,});
  if((aws.accountsLoading && !aws.accountsLoaded) || (aws.accountsError && aws.accounts.length === 0)){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {              "variant":`page`,"what":`accounts`,"loading":aws.accountsLoading,"error":aws.accountsError ? awsErrorText(aws.accountsError) : null,"empty":true,"rows":3,"onretry":() => void aws.loadAccounts(),}});}
  } else if (aws.accounts.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`cloud`,"title":`No AWS accounts yet`,"body":canAdmin
        ? 'Add one from an existing ~/.aws profile (SSO, assume-role…) or with access keys. Otto never writes your ~/.aws files.'
        : 'An administrator needs to add an AWS account before you can browse S3, SQS, EC2, Athena or EKS.',"actionLabel":canAdmin ? 'Add account' : undefined,"actionIcon":`plus`,"onaction":canAdmin ? onadd : undefined,}});}
  }else{
    if(showFilter){
       { svelteHTML.createElement("div", { "class":`ov-bar`,});
         { svelteHTML.createElement("label", { "class":`ov-filter input-group`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":13,}});}
           { svelteHTML.createElement("input", {          "dir":`ltr`,"type":`search`,"placeholder":`Filter accounts…`,"bind:value":filter,"aria-label":`Filter accounts`,});/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
        if(filter.trim()){ { svelteHTML.createElement("span", { "class":`dim`,});visible.length;  aws.accounts.length; }}
       }
    }
    if(visible.length === 0){
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {           "icon":`search`,"title":`No accounts match “${filter.trim()}”`,"actionLabel":`Clear filter`,"actionKind":`secondary`,"onaction":() => (filter = ''),}});}
    }
     { svelteHTML.createElement("div", { "class":`cards`,});
         for(let a of __sveltets_2_ensureArray(visible)){a.id;
        const p = aws.perms(a.id);
        const loadingP = aws.permLoading[a.id] === true;
         {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('article')));{ svelteHTML.createElement("article", __sveltets_2_union($$action_0), {         "class":`card`,...__sveltets_2_empty({"data-testid":`aws-account-card`}),"oncontextmenu":(e) => menu(e, a),});a.environment === 'prod';
           { svelteHTML.createElement("div", { "class":`card-top`,});
             { svelteHTML.createElement("span", {   "class":`dot`,"style":`background:${a.color || 'var(--text-dim)'}`,}); }
             { svelteHTML.createElement("h2", { "class":`name`,});a.name; }
             { const $$_egdaBvnE4C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE4C({ target: __sveltets_2_any(), props: {  "env":a.environment,}});}
             { svelteHTML.createElement("button", {       "class":`icon-btn more`,"onclick":(e) => menu(e, a),"aria-label":`Actions for ${a.name}`,"title":`Actions for ${a.name}`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
           }
           { svelteHTML.createElement("dl", { "class":`meta`,});
             { svelteHTML.createElement("dt", {});  }
             { svelteHTML.createElement("dd", {   "class":`mono`,"title":a.identity?.arn ?? '',});
              if(a.identity){a.identity.account;  roleFromArn(a.identity);}else{ { svelteHTML.createElement("span", { "class":`dim`,});  }}
             }
             { svelteHTML.createElement("dt", {});  }
             { svelteHTML.createElement("dd", { "class":`mono`,});
              a.auth_mode === 'profile' ? `profile ${a.profile ?? ''}` : `keys ${a.access_key_id ?? ''}`;
              if(a.role_arn){ { svelteHTML.createElement("span", { "class":`dim`,});  a.role_arn.split('/').pop(); }}
             }
             { svelteHTML.createElement("dt", {});  }
             { svelteHTML.createElement("dd", { "class":`mono`,});a.region; }
            if(a.endpoint_url){
               { svelteHTML.createElement("dt", {});  }
               { svelteHTML.createElement("dd", {     "class":`mono ep`,"title":a.endpoint_url,...__sveltets_2_empty({"data-testid":`aws-account-endpoint`}),});a.endpoint_url; }
            }
            if(p){
               { svelteHTML.createElement("dt", {});  }
               { svelteHTML.createElement("dd", { "class":`dim`,});fmtAgo(p.checked_at); }
            }
           }
           { svelteHTML.createElement("div", {   "class":`chips`,"aria-label":`Service permissions`,});
               for(let s of __sveltets_2_ensureArray(AWS_SERVICES)){s.id;
              const st = chipState(a, s.id);
              const rbac = resourceAccess.can('aws_account', a.id, s.id === 's3' ? 'discover' : `${s.id}_view`, `aws_${s.id}` as Feature, 'view');
               { svelteHTML.createElement("a", {             "class":`chip perm`,"href":rbac ? `#/aws/${a.id}/${s.id}` : undefined,"title":!rbac ? `${s.label}: you lack View on this feature` : st === 'denied' ? `${s.label}: AccessDenied for this account` : st === 'unknown' ? `${s.label}: not probed yet` : s.label,"aria-disabled":!rbac,});st === 'allowed';st === 'denied';st === 'unknown';!rbac;
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":s.icon,"size":12,}});}
                s.label;
               }
            }
             { svelteHTML.createElement("button", {           "class":`chip-refresh`,"onclick":() => void aws.loadPermissions(a.id, true),"title":`Re-check permissions`,"aria-label":`Re-check permissions`,"disabled":loadingP || !resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'view'),});
              if(loadingP){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 12px`,"aria-hidden":`true`,}); }}else{ { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}}
             }
           }
          if(p?.login_required){
             { svelteHTML.createElement("div", { "class":`login-row`,});
               { svelteHTML.createElement("span", { "class":`warn`,});    }
              if(a.auth_mode === 'profile' && resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'edit')){
                 { svelteHTML.createElement("button", {   "class":`btn primary small`,"onclick":() => onsignin(a),});
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}  
                 }
              } else if (a.auth_mode === 'access_keys'){
                 { svelteHTML.createElement("span", { "class":`dim`,});     }
              }
             }
          }
           { svelteHTML.createElement("div", { "class":`card-foot`,});
             { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => router.go(`aws/${a.id}/s3`),});  }
            if(resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'admin')){
               { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => onedit(a),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`gear`,"size":12,}});}  }
            }
           }
         }}
      }
     }
  }
 }

if(accessFor){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Manage access`,"width":780,"onclose":() => (accessFor = null),}});
     { const $$_sseccAecruoseR1C = __sveltets_2_ensureComponent(ResourceAccess); new $$_sseccAecruoseR1C({ target: __sveltets_2_any(), props: {    "kind":`aws_account`,"resourceId":accessFor,}});}
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AccountsOverview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AccountsOverview__SvelteComponent_ = ReturnType<typeof AccountsOverview__SvelteComponent_>;
/*Ωignore_endΩ*/export default AccountsOverview__SvelteComponent_;