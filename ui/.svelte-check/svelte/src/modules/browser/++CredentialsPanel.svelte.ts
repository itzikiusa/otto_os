///<reference types="svelte" />
;
import Badge from '../../lib/components/Badge.svelte';
import { toastError } from '../../lib/toastError';
// Site credentials for the in-app browser: list/add/edit/delete + a
// confirm-gated reveal. The password is NEVER in the list/edit payload —
// `BrowserCredential` has no password field at all (mirrors
// `otto_state::browser_credentials::BrowserCredential`); the only place a
// plaintext password ever appears client-side is the one-shot reveal
// banner below, and it is never logged or persisted (no localStorage, no
// console.log — just the in-memory `revealed` state, cleared on dismiss).
//
// Mounted from the Browser toolbar in a workspace-scoped modal.
import * as browserApi from '../../lib/api/browser';
import type { BrowserCredential } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';

;type $$ComponentProps =  { workspaceId: string };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  
  
  
  
  
  
  
  
  
  

  let { workspaceId }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  let creds: BrowserCredential[] = $state([]);
  let loading = $state(true);
  /** Cause of the last failed list load (inline + Retry; a toast is for actions). */
  let loadError = $state('');

  // ── Add/Edit form (shared modal) ────────────────────────────────────────
  let formOpen = $state(false);
  let editing: BrowserCredential | null = $state(null);
  let fDomain = $state('');
  let fUsername = $state('');
  let fPassword = $state('');
  let fAllowAgentUse = $state(false);
  let fNotes = $state('');
  let saving = $state(false);

  // ── Reveal ───────────────────────────────────────────────────────────────
  /** The one credential whose password is currently shown, plus the value
   *  itself — cleared on dismiss so it doesn't linger in memory/DOM. */
  let revealed: { id: string; password: string } | null = $state(null);
  let revealingId: string | null = $state(null);

  $effect(() => {
    if (workspaceId) void load();
  });

  async function load(): Promise<void> {
    loading = true;
    try {
      creds = await browserApi.listCredentials(workspaceId);
      loadError = '';
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  function openAdd(): void {
    editing = null;
    fDomain = '';
    fUsername = '';
    fPassword = '';
    fAllowAgentUse = false;
    fNotes = '';
    formOpen = true;
  }

  function openEdit(c: BrowserCredential): void {
    editing = c;
    fDomain = c.domain;
    fUsername = c.username;
    fPassword = '';
    fAllowAgentUse = c.allow_agent_use;
    fNotes = c.notes;
    formOpen = true;
  }

  function closeForm(): void {
    formOpen = false;
    fPassword = '';
    fieldErrors = {};
  }

  /** Field errors shown inline under each field (components.md: never a
   *  toast for a field error); a field's entry clears when it is edited. */
  let fieldErrors = $state<Partial<Record<'domain' | 'username' | 'password', string>>>({});

  async function save(): Promise<void> {
    if (!editing) {
      const errs: typeof fieldErrors = {};
      if (!fDomain.trim()) errs.domain = 'Enter the site’s domain, like example.com.';
      if (!fUsername.trim()) errs.username = 'Enter the username to fill in.';
      if (!fPassword) errs.password = 'Enter the password to store in the Keychain.';
      fieldErrors = errs;
      if (Object.keys(errs).length) return;
    }
    saving = true;
    try {
      if (editing) {
        const updated = await browserApi.updateCredential(editing.id, {
          username: fUsername.trim(),
          allow_agent_use: fAllowAgentUse,
          notes: fNotes,
          ...(fPassword ? { password: fPassword } : {}),
        });
        creds = creds.map((c) => (c.id === updated.id ? updated : c));
        toasts.success('Credential updated', `${updated.domain} · ${updated.username}`);
      } else {
        const created = await browserApi.createCredential(workspaceId, {
          domain: fDomain.trim(),
          username: fUsername.trim(),
          password: fPassword,
          allow_agent_use: fAllowAgentUse,
          notes: fNotes,
        });
        creds = [...creds, created].sort(
          (a, b) => a.domain.localeCompare(b.domain) || a.username.localeCompare(b.username),
        );
        toasts.success('Credential saved', `${created.domain} · ${created.username}`);
      }
      closeForm();
    } catch (e) {
      toastError('Couldn’t save', e);
    } finally {
      saving = false;
    }
  }

  async function remove(c: BrowserCredential): Promise<void> {
    const ok = await confirmer.ask(
      `Delete the credential for "${c.username}" on ${c.domain}? This also removes it from the Keychain.`,
      { title: 'Delete credential', confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
    try {
      await browserApi.deleteCredential(c.id);
      creds = creds.filter((x) => x.id !== c.id);
      if (revealed?.id === c.id) revealed = null;
      toasts.success('Credential deleted', `${c.domain} · ${c.username}`);
    } catch (e) {
      toastError('Couldn’t delete', e);
    }
  }

  async function reveal(c: BrowserCredential): Promise<void> {
    const ok = await confirmer.ask(
      `Show the password for "${c.username}" on ${c.domain}? Make sure no one else can see your screen.`,
      { title: 'Reveal password', confirmLabel: 'Reveal', danger: true },
    );
    if (!ok) return;
    revealingId = c.id;
    try {
      const resp = await browserApi.revealCredential(c.id);
      revealed = { id: c.id, password: resp.password };
    } catch (e) {
      toastError('Couldn’t reveal', e);
    } finally {
      revealingId = null;
    }
  }

  function dismissReveal(): void {
    revealed = null;
  }

  async function copyRevealed(): Promise<void> {
    if (!revealed) return;
    try {
      await copyTextOrThrow(revealed.password);
      toasts.success('Copied', 'Password copied to clipboard.');
    } catch {
      toasts.error('Couldn’t copy', 'Select the password and copy it manually.');
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`panel`,});
   { svelteHTML.createElement("div", { "class":`panel-head`,});
     { svelteHTML.createElement("div", {});
       { svelteHTML.createElement("h2", {});  }
       { svelteHTML.createElement("div", { "class":`sub`,});
                    
         
       }
     }
     { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":openAdd,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});}   }
   }

   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        children:() => { return __sveltets_2_any(0); },"what":`credentials`,loading,"error":loadError,"empty":creds.length === 0,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`key`,"title":`No saved site credentials`,"body":`Add a credential to fill a sign-in form when you choose.`,}});}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("ul", { "class":`list`,});
         for(let c of __sveltets_2_ensureArray(creds)){c.id;
         { svelteHTML.createElement("li", { "class":`row`,});
           { svelteHTML.createElement("div", { "class":`body`,});
             { svelteHTML.createElement("div", { "class":`domain-line`,});
               { svelteHTML.createElement("span", { "class":`domain`,});c.domain; }
              if(c.allow_agent_use){
                 { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {      "tone":`accent`,"label":`Agents may use`,"title":`Agent sessions may autofill this credential`,}});}
              }
             }
             { svelteHTML.createElement("div", { "class":`username`,});c.username; }
            if(c.notes){ { svelteHTML.createElement("div", { "class":`notes`,});c.notes; }}
            if(revealed?.id === c.id){
               { svelteHTML.createElement("div", { "class":`reveal-banner`,});
                 { svelteHTML.createElement("code", { "class":`reveal-value`,});revealed.password; }
                 { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":copyRevealed,});  }
                 { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":dismissReveal,});  }
               }
            }
           }
           { svelteHTML.createElement("div", { "class":`actions`,});
             { svelteHTML.createElement("button", {           "class":`icon-btn`,"title":`Reveal password`,"aria-label":`Reveal password`,"disabled":revealingId === c.id,"onclick":() => reveal(c),});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":13,}});}
             }
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Edit credential`,"aria-label":`Edit credential`,"onclick":() => openEdit(c),});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":13,}});}
             }
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Delete credential`,"aria-label":`Delete credential`,"onclick":() => remove(c),});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":13,}});}
             }
           }
         }
      }
     }
   LoadState}
 }

if(formOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {   "title":editing ? 'Edit credential' : 'Add credential',"onclose":closeForm,children:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("div", { "class":`form`,});
        if(!editing){
           { svelteHTML.createElement("label", {});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("input", {                "dir":`auto`,"type":`text`,"bind:value":fDomain,"placeholder":`example.com`,"spellcheck":`false`,"aria-invalid":fieldErrors.domain ? 'true' : undefined,"aria-describedby":fieldErrors.domain ? 'cred-domain-err' : undefined,"oninput":() => (fieldErrors = { ...fieldErrors, domain: undefined }),});/*Ωignore_startΩ*/() => fDomain = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            if(fieldErrors.domain){ { svelteHTML.createElement("span", {   "class":`field-error`,"id":`cred-domain-err`,});fieldErrors.domain; }}
           }
        }else{
           { svelteHTML.createElement("label", {});
             { svelteHTML.createElement("span", {});  }
              { svelteHTML.createElement("input", {      "dir":`auto`,"type":`text`,"value":editing.domain,"disabled":true,});}
           }
        }
         { svelteHTML.createElement("label", {});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("input", {              "dir":`auto`,"type":`text`,"bind:value":fUsername,"spellcheck":`false`,"aria-invalid":fieldErrors.username ? 'true' : undefined,"aria-describedby":fieldErrors.username ? 'cred-user-err' : undefined,"oninput":() => (fieldErrors = { ...fieldErrors, username: undefined }),});/*Ωignore_startΩ*/() => fUsername = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          if(fieldErrors.username){ { svelteHTML.createElement("span", {   "class":`field-error`,"id":`cred-user-err`,});fieldErrors.username; }}
         }
         { svelteHTML.createElement("label", {});
           { svelteHTML.createElement("span", {});editing ? 'New password (leave blank to keep current)' : 'Password'; }
           { svelteHTML.createElement("input", {            "type":`password`,"bind:value":fPassword,"autocomplete":`new-password`,"aria-invalid":fieldErrors.password ? 'true' : undefined,"aria-describedby":fieldErrors.password ? 'cred-pass-err' : undefined,"oninput":() => (fieldErrors = { ...fieldErrors, password: undefined }),});/*Ωignore_startΩ*/() => fPassword = __sveltets_2_any(null);/*Ωignore_endΩ*/}
          if(fieldErrors.password){ { svelteHTML.createElement("span", {   "class":`field-error`,"id":`cred-pass-err`,});fieldErrors.password; }}
         }
         { svelteHTML.createElement("label", {});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("textarea", {     "dir":`auto`,"bind:value":fNotes,"rows":2,});/*Ωignore_startΩ*/() => fNotes = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         }
         { svelteHTML.createElement("label", { "class":`checkbox`,});
           { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":fAllowAgentUse,});/*Ωignore_startΩ*/() => fAllowAgentUse = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", {});       }
         }
        if(fAllowAgentUse){
           { svelteHTML.createElement("p", { "class":`warning`,});
                         
                        
                  
           }
        }
       }
    };return __sveltets_2_any(0)},footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {     "class":`btn`,"onclick":closeForm,"disabled":saving,});  }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":save,"disabled":saving,});
        saving ? 'Saving…' : 'Save';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {children, footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
    
    
   Modal}
}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CredentialsPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CredentialsPanel__SvelteComponent_ = ReturnType<typeof CredentialsPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default CredentialsPanel__SvelteComponent_;