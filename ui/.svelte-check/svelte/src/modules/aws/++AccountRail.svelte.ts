///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { rowMenu } from '../../lib/rowMenu';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
// Left rail inside the AWS module: accounts → services. A service is greyed
// (still navigable, so the AccessDenied is visible) when the account's IAM
// probe said `denied`, and hidden outright when the user lacks the feature's
// View grant. Right-click / ⋯ on an account → edit / refresh perms / delete.
import { aws, AWS_SERVICES } from '../../lib/stores/aws.svelte';
import { router } from '../../lib/router.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import EnvBadge from '../../lib/components/EnvBadge.svelte';
import { pollWhileVisible } from '../../lib/poll';
import { awsErrorText, logsRoute } from './util';
import type { AwsAccount, AwsService, Feature } from '../../lib/api/types';
;

  interface Props {
    activeId: string | null;
    activeService: AwsService | null;
    onedit: (a: AwsAccount) => void;
    ondelete: (a: AwsAccount) => void;
    /** Opens the `aws sso login` sheet — offered ahead of a sign-in expiry. */
    onsignin?: (a: AwsAccount) => void;
    /** The CloudWatch Logs view is open for `activeId`. */
    logsActive?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { activeId, activeService, onedit, ondelete, onsignin, logsActive = false }: Props = $props();

  // A-5: warn BEFORE the sign-in lapses instead of failing mid-action. The
  // expiry is absolute, so a 30 s client clock is enough (no polling).
  const WARN_MS = 15 * 60_000;
  let now = $state(Date.now());
  $effect(() => {
    const p = pollWhileVisible(() => { now = Date.now(); }, { ms: 30_000, immediate: false });
    return () => p.stop();
  });
  function sessionNote(a: AwsAccount): { text: string; expired: boolean } | null {
    const s = a.session;
    if (!s) return null;
    const left = Date.parse(s.expires_at) - now;
    if (Number.isNaN(left)) return null;
    // Refreshable sessions renew silently; only a lapsed one needs the user.
    if (left <= 0 && !s.refreshable) return { text: 'Sign-in expired', expired: true };
    if (!s.refreshable && left < WARN_MS) {
      const min = Math.max(1, Math.round(left / 60_000));
      return { text: `Sign-in ends in ${min} min`, expired: false };
    }
    return null;
  }

  let collapsed: Record<string, boolean> = $state({});

  function featureOf(svc: AwsService): Feature {
    return `aws_${svc}` as Feature;
  }

  $effect(() => { for (const a of aws.accounts) void resourceAccess.load('aws_account', a.id); });
  function services(a: AwsAccount) {
    return AWS_SERVICES.filter((s) => resourceAccess.can('aws_account', a.id, s.id === 's3' ? 'discover' : `${s.id}_view`, featureOf(s.id), 'view')).map((s) => ({
      ...s,
      denied: !aws.serviceAllowed(a.id, s.id),
    }));
  }

  function menu(e: MouseEvent | KeyboardEvent, a: AwsAccount): void {
    ctxMenu.show(e, [
      { label: 'Overview', icon: 'grid', action: () => router.go('aws') },
      {
        label: 'Re-check permissions',
        disabled: !resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'view'),
        icon: 'refresh',
        action: () => void aws.loadPermissions(a.id, true),
      },
      ...(resourceAccess.can('aws_account', a.id, 'configure', 'aws', 'admin')
        ? [
            { label: 'Edit account…', icon: 'edit', action: () => onedit(a) },
            { separator: true },
            { label: 'Delete account…', icon: 'trash', danger: true, action: () => ondelete(a) },
          ]
        : []),
    ]);
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  // Keyed by account and expiry state (not the ticking minutes), so the
  // countdown doesn't re-announce every minute.
  const sessionNotes = $derived(aws.accounts.map((a) => { const n = sessionNote(a); return n ? `${a.name}: ${n.expired ? 'sign-in expired' : 'sign-in ends soon'}` : ''; }).filter(Boolean).join('. '));
  $effect(() => { if (sessionNotes) announce(`${sessionNotes}.`, { assertive: sessionNotes.includes('expired') }); });
;
async () => {

 { svelteHTML.createElement("nav", {   "class":`rail`,"aria-label":`AWS accounts`,});
   { svelteHTML.createElement("div", { "class":`rail-head`,});
     { svelteHTML.createElement("span", {});  }
   }
     for(let a of __sveltets_2_ensureArray(aws.accounts)){a.id;
    const open = !collapsed[a.id];
    const note = sessionNote(a);
     { svelteHTML.createElement("div", {  "class":`acct`,});a.id === activeId;
       {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {        "class":`acct-row`,"role":`group`,"aria-label":a.name,"oncontextmenu":(e) => menu(e, a),});
         { svelteHTML.createElement("button", {         "class":`acct-toggle`,"aria-expanded":open,"title":a.identity?.arn ? `${a.name}\n${a.identity.arn}` : a.profile ? `${a.name} · profile ${a.profile}` : a.name,"onclick":() => (collapsed = { ...collapsed, [a.id]: open }),});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});}
           { svelteHTML.createElement("span", {   "class":`dot`,"style":`background:${a.color || 'var(--text-dim)'}`,}); }
           { svelteHTML.createElement("span", { "class":`name`,});a.name; }
         }
         { const $$_egdaBvnE3C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE3C({ target: __sveltets_2_any(), props: {  "env":a.environment,}});}
         { svelteHTML.createElement("button", {         "class":`icon-btn more`,"onclick":(e) => menu(e, a),"aria-label":`Actions for ${a.name}`,"title":`Actions`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
       }}
      if(note){
         { svelteHTML.createElement("div", {  "class":`sess`,});note.expired;
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
           { svelteHTML.createElement("span", {});note.text; }
          if(onsignin && a.auth_mode === 'profile'){
             { svelteHTML.createElement("button", {     "class":`sess-btn`,"onclick":() => onsignin?.(a),"title":`Run aws sso login for ${a.name}`,});   }
          }
         }
      }
      if(open){
         { svelteHTML.createElement("ul", { "class":`svcs`,});
             for(let s of __sveltets_2_ensureArray(services(a))){s.id;
             { svelteHTML.createElement("li", {});
               { svelteHTML.createElement("a", {         "href":`#/aws/${a.id}/${s.id}`,"title":s.denied ? `${s.label}: access denied for this account` : s.label,"aria-current":a.id === activeId && s.id === activeService ? 'page' : undefined,});a.id === activeId && s.id === activeService;s.denied;
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":s.icon,"size":13,}});}
                 { svelteHTML.createElement("span", {});s.label; }
                if(s.denied){ { svelteHTML.createElement("span", { "class":`deny`,});  }}
               }
             }
          }
          if(resourceAccess.can('aws_account', a.id, 'metrics', 'aws', 'view')){
            const on = a.id === activeId && logsActive;
             { svelteHTML.createElement("li", {});
               { svelteHTML.createElement("a", {      "href":`#/${logsRoute(a.id)}`,"title":`CloudWatch Logs`,"aria-current":on ? 'page' : undefined,});on;
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`text`,"size":13,}});}
                 { svelteHTML.createElement("span", {});  }
               }
             }
          }
         }
      }
     }
  }
    if(!aws.accountsLoaded){
       { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":4,"height":28,"label":`AWS accounts`,}});}
    } else if (aws.accountsError){
      
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`AWS accounts`,"error":awsErrorText(aws.accountsError),"empty":true,"onretry":() => void aws.loadAccounts(),}});}
    }else{
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {    "icon":`cloud`,"title":`No AWS accounts yet`,}});}
    }
  
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AccountRail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AccountRail__SvelteComponent_ = ReturnType<typeof AccountRail__SvelteComponent_>;
/*Ωignore_endΩ*/export default AccountRail__SvelteComponent_;