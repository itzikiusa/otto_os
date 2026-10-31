///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import { announceShown } from '../../lib/announce.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { sectionLabel } from './sections';
import SectionIntro from './SectionIntro.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
// Personal Access Tokens (PAT) management — mint long-lived API tokens,
// view existing ones (prefix + last-seen), and revoke them individually.
// Routes: POST/GET/DELETE /api/v1/auth/tokens  (api.md #87-89).
import { api, baseUrl } from '../../lib/api/client';
import type { ApiTokenInfo, CreateApiTokenResp } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { rel } from '../../lib/stores/now.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { loadErrorText } from '../../lib/loadError';
import { copyTextOrThrow } from '../../lib/clipboard';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let tokens: ApiTokenInfo[] = $state([]);
  let loading = $state(true);
  let loadError = $state('');
  let revoking: Set<string> = $state(new Set());
  let filter = $state<'all' | 'personal' | 'session' | 'orphaned'>('all');
  const sessionToken = (t: ApiTokenInfo) => !!(t.session_id || t.legacy_session_id);
  const sessionCount = $derived(tokens.filter(sessionToken).length);
  const orphaned = $derived(tokens.filter((t) => sessionToken(t) && t.session_exists === false));
  const visible = $derived(tokens.filter((t) => filter === 'all'
    || (filter === 'personal' && !sessionToken(t))
    || (filter === 'session' && sessionToken(t))
    || (filter === 'orphaned' && sessionToken(t) && t.session_exists === false)));

  async function revokeOrphaned(): Promise<void> {
    const candidates = [...orphaned];
    if (!candidates.length) return;
    if (!await confirmer.ask(
      `Revoke ${plural(candidates.length, 'token')} referring to sessions that no longer exist? Legacy tokens are identified by their labels; review the Deleted session filter first if you named a personal token this way.`,
      { title: 'Revoke deleted-session tokens', confirmLabel: 'Revoke tokens', danger: true },
    )) return;
    let failed = 0;
    revoking = new Set([...revoking, ...candidates.map((t) => t.id)]);
    // Bounded requests; do not flood the local API for historical token lists.
    for (const t of candidates) {
      try { await api.del(`/auth/tokens/${t.id}`); tokens = tokens.filter((x) => x.id !== t.id); }
      catch { failed += 1; }
    }
    revoking = new Set([...revoking].filter((id) => !candidates.some((t) => t.id === id)));
    if (failed) toasts.error('Some tokens couldn’t be revoked', `${failed} failed; they remain listed.`);
    else toasts.success('Tokens revoked', `${candidates.length} deleted-session tokens removed.`);
  }

  // ---- create form ----
  let newLabel = $state('');
  let minting = $state(false);
  /** Raw secret returned once on creation — cleared when the user dismisses. */
  let freshSecret: string | null = $state(null);
  let freshInfo: ApiTokenInfo | null = $state(null);

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    loadError = '';
    try {
      tokens = await api.get<ApiTokenInfo[]>('/auth/tokens');
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  async function mint(): Promise<void> {
    if (minting || freshSecret) return;
    minting = true;
    try {
      const resp = await api.post<CreateApiTokenResp>('/auth/tokens', {
        label: newLabel.trim() || null,
      });
      freshSecret = resp.token;
      freshInfo = resp.info;
      tokens = [resp.info, ...tokens];
      newLabel = '';
    } catch (e) {
      toasts.error("Couldn’t create the token", loadErrorText(e));
    } finally {
      minting = false;
    }
  }

  async function revoke(t: ApiTokenInfo): Promise<void> {
    const ok = await confirmer.ask(
      `Revoke token "${t.label ?? t.token_prefix}…"? Any script or client using it will stop working immediately.`,
      { title: 'Revoke token', confirmLabel: 'Revoke', danger: true },
    );
    if (!ok) return;
    revoking = new Set([...revoking, t.id]);
    try {
      await api.del(`/auth/tokens/${t.id}`);
      tokens = tokens.filter((x) => x.id !== t.id);
      toasts.success('Token revoked', t.label ?? t.token_prefix);
    } catch (e) {
      toasts.error("Couldn’t revoke the token", loadErrorText(e));
    } finally {
      revoking = new Set([...revoking].filter((x) => x !== t.id));
    }
  }

  async function copySecret(): Promise<void> {
    if (!freshSecret) return;
    try {
      await copyTextOrThrow(freshSecret);
      toasts.success('Token copied', 'Paste it into your script or CI secret now.');
    } catch {
      toasts.error("Couldn’t copy the token", 'Select it and copy it manually.');
    }
  }

  function dismissSecret(): void {
    freshSecret = null;
    freshInfo = null;
  }

  function isExpired(t: ApiTokenInfo): boolean {
    return new Date(t.expires_at).getTime() < Date.now();
  }

  /** What a row is called: its label, else its prefix. Managed session tokens
   *  carry a machine label (`otto-mcp:<session ULID>`) — shown shortened, the
   *  full value stays in the tooltip. */
  function rowName(t: ApiTokenInfo): string {
    const l = t.label ?? '';
    const m = /^(.*:)([0-9A-HJKMNP-TV-Z]{26})$/.exec(l);
    if (m) return `${m[1]}${m[2].slice(0, 6)}…`;
    return l || `${t.token_prefix}…`;
  }
  function absDate(ts: string | null | undefined): string | undefined {
    return ts ? new Date(ts).toLocaleString() : undefined;
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":sectionLabel('tokens'),"subtitle":`For scripts, CI and the Otto CLI`,}});}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});        { svelteHTML.createElement("strong", {});   }               SectionIntro}

  
  if(freshSecret && freshInfo){
     {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),('Token created. Copy your new token now — it won’t be shown again.')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`secret-banner`,});
       { svelteHTML.createElement("div", { "class":`secret-header`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":14,}});}
         { svelteHTML.createElement("span", { "class":`secret-title`,});           }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":copySecret,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});}  }
         { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":dismissSecret,});  }
       }
       { svelteHTML.createElement("code", { "class":`secret-value`,});freshSecret; }
       { svelteHTML.createElement("div", { "class":`secret-meta`,});
        freshInfo.label ?? 'No label';  freshInfo.token_prefix;   rel(freshInfo.expires_at);
       }
     }}
  }

  
   { svelteHTML.createElement("div", { "class":`section-title`,});  }
   { svelteHTML.createElement("div", { "class":`card s-card`,});
     { svelteHTML.createElement("label", {   "class":`mint-label`,"for":`pat-label`,});  { svelteHTML.createElement("span", { "class":`dim`,});  } }
     { svelteHTML.createElement("div", { "class":`mint-row`,});
       { svelteHTML.createElement("input", {                "dir":`auto`,"id":`pat-label`,"class":`input mint-input`,"type":`text`,"placeholder":`CI pipeline`,"bind:value":newLabel,"onkeydown":(e) => { if (e.key === 'Enter') void mint(); },"maxlength":80,});/*Ωignore_startΩ*/() => newLabel = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":minting || !!freshSecret,"onclick":mint,});
         { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});} minting ? 'Creating…' : 'Create token';
       }
     }
     { svelteHTML.createElement("span", { "class":`hint`,});            { svelteHTML.createElement("code", {});  }     { svelteHTML.createElement("code", {});  }  }
   }

  
   { svelteHTML.createElement("div", { "class":`tokens-head`,});
     { svelteHTML.createElement("div", { "class":`section-title`,});  }
    if(tokens.length){
       { svelteHTML.createElement("select", {     "class":`input filter`,"aria-label":`Filter tokens`,"bind:value":filter,});/*Ωignore_startΩ*/() => filter = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", { "value":`all`,}); tokens.length;  }
         { svelteHTML.createElement("option", { "value":`personal`,}); tokens.length - sessionCount;  }
         { svelteHTML.createElement("option", { "value":`session`,}); sessionCount;  }
         { svelteHTML.createElement("option", { "value":`orphaned`,});  orphaned.length;  }
       }
    }
   }

   { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`tokens`,loading,"error":loadError,"empty":tokens.length === 0,"onretry":() => void load(),"rows":3,emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("div", { "class":`card tok-card`,});
         { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`key`,"title":`No tokens yet`,"body":`Create one above for a script, a CI job or the Otto CLI.`,}});}
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
    
    if(orphaned.length){
       { svelteHTML.createElement("div", { "class":`orphan-note`,});
         { svelteHTML.createElement("span", {});plural(orphaned.length, 'token');        }
         { svelteHTML.createElement("button", {     "class":`btn small danger`,"disabled":revoking.size > 0,"onclick":revokeOrphaned,});   }
       }
    }
    if(visible.length === 0){
       { svelteHTML.createElement("div", { "class":`card tok-card`,});
         { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"icon":`filter`,"title":`No tokens in this view`,}});
           { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (filter = 'all'),});   }
         EmptyState}
       }
    }else{
       { svelteHTML.createElement("div", {     "class":`card tok-card token-table`,"role":`table`,"aria-label":`Your tokens`,});
         { svelteHTML.createElement("div", {   "class":`token-head`,"role":`row`,});
           { svelteHTML.createElement("span", {   "class":`col-label`,"role":`columnheader`,});  }
           { svelteHTML.createElement("span", {   "class":`col-seen`,"role":`columnheader`,});  }
           { svelteHTML.createElement("span", {   "class":`col-exp`,"role":`columnheader`,});  }
           { svelteHTML.createElement("span", {   "class":`col-action`,"role":`columnheader`,}); { svelteHTML.createElement("span", { "class":`sr-only`,});  } }
         }
           for(let t of __sveltets_2_ensureArray(visible)){t.id;
           { svelteHTML.createElement("div", {    "class":`token-row`,"role":`row`,});isExpired(t);
             { svelteHTML.createElement("span", {   "class":`col-label`,"role":`cell`,});
               { svelteHTML.createElement("span", {   "class":`tok-label`,"title":t.label ?? undefined,});rowName(t); }
               { svelteHTML.createElement("span", { "class":`tok-meta`,});
                if(sessionToken(t)){t.session_id ? 'Managed session' : 'Legacy session label';t.session_exists === false ? ' · session deleted' : ''; } { svelteHTML.createElement("span", { "class":`mono`,});t.token_prefix;  }
                
                 { svelteHTML.createElement("span", {    "class":`chip exp-phone`,"title":absDate(t.expires_at),});isExpired(t);isExpired(t) ? 'Expired' : `Expires ${rel(t.expires_at)}`; }
               }
             }
             { svelteHTML.createElement("span", {     "class":`col-seen`,"role":`cell`,"title":absDate(t.last_seen_at),});t.last_seen_at ? rel(t.last_seen_at) : 'Never'; }
             { svelteHTML.createElement("span", {      "class":`col-exp`,"role":`cell`,"title":absDate(t.expires_at),});isExpired(t);
              isExpired(t) ? 'Expired' : rel(t.expires_at);
             }
             { svelteHTML.createElement("span", {   "class":`col-action`,"role":`cell`,});
               { svelteHTML.createElement("button", {       "class":`btn small danger`,"disabled":revoking.has(t.id),"onclick":() => revoke(t),});
                revoking.has(t.id) ? 'Revoking…' : 'Revoke…';
               }
             }
           }
        }
       }
       { svelteHTML.createElement("p", { "class":`usage-note`,});                        }
    }
   LoadState}

  
   { svelteHTML.createElement("div", { "class":`section-title`,});   }
   { svelteHTML.createElement("pre", { "class":`code-block`,});     baseUrl();  }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PersonalAccessTokens__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PersonalAccessTokens__SvelteComponent_ = ReturnType<typeof PersonalAccessTokens__SvelteComponent_>;
/*Ωignore_endΩ*/export default PersonalAccessTokens__SvelteComponent_;