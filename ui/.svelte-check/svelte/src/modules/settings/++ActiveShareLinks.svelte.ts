///<reference types="svelte" />
;// Settings → Sharing → Active links: every live share link the caller has
// minted, across ALL sessions (`GET /auth/shares`), with Revoke per row and
// a caller-wide Revoke all (`POST /auth/shares/revoke-all`). The per-session
// ShareModal only shows one session's links — without this page a forgotten
// link on another session could only be found by opening each session.

import { tick } from 'svelte';
import { api } from '../../lib/api/client';
import type { MyShare } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
import { rel } from '../../lib/stores/now.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let shares = $state<MyShare[]>([]);
  let loading = $state(true);
  let error = $state('');
  let revoking = $state<Set<string>>(new Set());
  let revokingAll = $state(false);
  let tableEl = $state<HTMLElement>();
  let headingEl = $state<HTMLElement>();

  /** After row `i` is gone, keep focus in the table: the same index's Revoke
   *  (the next row slid up), else the previous one, else the section heading —
   *  the confirm returned focus to a button that no longer exists. */
  async function settleFocus(i: number): Promise<void> {
    await tick();
    const btns = tableEl?.querySelectorAll<HTMLButtonElement>('td.act button');
    (btns?.[i] ?? btns?.[i - 1] ?? headingEl)?.focus();
  }

  async function load(): Promise<void> {
    loading = true;
    error = '';
    try {
      shares = await api.get<MyShare[]>('/auth/shares');
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => { void load(); });

  function name(s: MyShare): string {
    return s.session_title?.trim() || (s.session_title === null ? 'Deleted session' : 'Untitled session');
  }

  async function revoke(s: MyShare): Promise<void> {
    const ok = await confirmer.ask(
      `Revoke the ${s.role} link to “${name(s)}”? Guests attached through it are disconnected and it can’t be used again.`,
      { title: 'Revoke share link', danger: true, confirmLabel: 'Revoke link' },
    );
    if (!ok) return;
    revoking = new Set([...revoking, s.id]);
    try {
      await api.del(`/auth/shares/${encodeURIComponent(s.id)}`);
      const at = shares.findIndex((x) => x.id === s.id);
      shares = shares.filter((x) => x.id !== s.id);
      toasts.success('Share link revoked');
      void settleFocus(at);
    } catch (e) {
      toastError('Couldn’t revoke the link', e);
    } finally {
      const next = new Set(revoking);
      next.delete(s.id);
      revoking = next;
    }
  }

  async function revokeAll(): Promise<void> {
    const n = shares.length;
    if (!n || revokingAll) return;
    const ok = await confirmer.ask(
      `Revoke all ${n} of your share links, on every session? Every guest is disconnected and none of the links can be used again.`,
      { title: 'Revoke all share links', danger: true, confirmLabel: `Revoke ${n} ${n === 1 ? 'link' : 'links'}` },
    );
    if (!ok) return;
    revokingAll = true;
    try {
      await api.post('/auth/shares/revoke-all', {});
      shares = [];
      toasts.success('All share links revoked');
      void settleFocus(0);
    } catch (e) {
      toastError('Couldn’t revoke the links', e);
      void load();
    } finally {
      revokingAll = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`links-head`,});
   { const $$_div1 = svelteHTML.createElement("div", {    "class":`section-title`,"tabindex":-1,});headingEl = $$_div1;  }
  if(shares.length > 0){
     { svelteHTML.createElement("button", {       "class":`btn small danger`,"disabled":revokingAll,"aria-busy":revokingAll,"onclick":() => void revokeAll(),});
      revokingAll ? 'Revoking…' : 'Revoke all';
     }
  }
 }
 { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL0 = new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`your share links`,loading,error,"empty":shares.length === 0,"onretry":() => void load(),"rows":2,emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("div", { "class":`card`,}); { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`link`,"title":`No active share links`,"body":`Links you create from a session’s Share dialog show up here until they expire or you revoke them.`,}});} }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL0.$$prop_def;/*Ωignore_endΩ*/
  
   { svelteHTML.createElement("div", { "class":`card table-wrap`,});
     { const $$_table2 = svelteHTML.createElement("table", {    "class":`links`,...__sveltets_2_empty({"data-testid":`active-share-links`}),});tableEl = $$_table2;
       { svelteHTML.createElement("thead", {});
         { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", { "scope":`col`,}); { svelteHTML.createElement("span", { "class":`sr-only`,});  } } }
       }
       { svelteHTML.createElement("tbody", {});
           for(let s of __sveltets_2_ensureArray(shares)){s.id;
           { svelteHTML.createElement("tr", {});
             { svelteHTML.createElement("td", { "class":`sess`,});
              if(s.session_title !== null){
                 { svelteHTML.createElement("button", {       "type":`button`,"class":`link-btn`,"title":`Open this session`,"onclick":() => ws.navigateToSession(s.session_id),});name(s); }
              }else{
                 { svelteHTML.createElement("span", { "class":`dim`,});name(s); }
              }
               { svelteHTML.createElement("span", {   "class":`mono dim prefix`,"title":`Token prefix`,});s.token_prefix;  }
             }
             { svelteHTML.createElement("td", { ...__sveltets_2_empty({"data-label":`Access`}),});s.role === 'editor' ? 'Can type' : 'View only'; }
             { svelteHTML.createElement("td", {   "class":`dim`,...__sveltets_2_empty({"data-label":`Label`}),});s.label ?? '—'; }
             { svelteHTML.createElement("td", {     ...__sveltets_2_empty({"data-label":`Expires`}),"title":s.dormant
                ? 'Window lapsed — the link holder can still request a new code until it is revoked'
                : new Date(s.expires_at).toLocaleString(),});s.dormant ? 'Dormant (revivable)' : rel(s.expires_at); }
             { svelteHTML.createElement("td", { "class":`act`,});
               { svelteHTML.createElement("button", {         "class":`btn small`,"disabled":revoking.has(s.id) || revokingAll,"aria-busy":revoking.has(s.id),"onclick":() => void revoke(s),});
                revoking.has(s.id) ? 'Revoking…' : 'Revoke';
               }
             }
           }
        }
       }
     }
   }
 LoadState}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ActiveShareLinks__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ActiveShareLinks__SvelteComponent_ = ReturnType<typeof ActiveShareLinks__SvelteComponent_>;
/*Ωignore_endΩ*/export default ActiveShareLinks__SvelteComponent_;