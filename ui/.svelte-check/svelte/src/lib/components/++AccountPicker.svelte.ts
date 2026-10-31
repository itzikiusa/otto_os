///<reference types="svelte" />
;
import LoadState from './LoadState.svelte';
import { announce } from '../announce.svelte';
import { api } from '../api/client';
import type { Session, ProviderAccount } from '../api/types';
import { ws } from '../stores/workspace.svelte';
import LazyTerminal from './LazyTerminal.svelte';
import { loadErrorText } from '../loadError';

;type $$ComponentProps =  {
    provider: string; value?: string; workspaceId: string; onchange: (id: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  let { provider, value = '', workspaceId, onchange }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let accounts = $state<ProviderAccount[]>([]);
  let adding = $state(false);
  let label = $state('');
  let busy = $state(false);
  let error = $state('');
  let status = $state('');
  let loginSession = $state<Session | null>(null);
  let listLoading = $state(false);
  let listError = $state('');
  let accountLoadSeq = 0;
  function cancelAccountLoad(): void { ++accountLoadSeq; }
  async function loadAccounts(): Promise<void> {
    const currentProvider = provider;
    const seq = ++accountLoadSeq;
    listLoading = true;
    listError = '';
    try {
      const all = await api.get<ProviderAccount[]>('/auth/provider-accounts');
      if (seq !== accountLoadSeq || provider !== currentProvider) return;
      accounts = all.filter((a) => a.provider === currentProvider);
    } catch (e) {
      if (seq === accountLoadSeq && provider === currentProvider) listError = loadErrorText(e);
    } finally {
      if (seq === accountLoadSeq) listLoading = false;
    }
  }
  $effect(() => {
    void provider;
    accounts = [];
    void loadAccounts();
    return cancelAccountLoad;
  });
  async function add() {
    busy = true; error = '';
    try {
      const account = await api.post<ProviderAccount>('/auth/provider-accounts', { provider, label });
      accounts = [...accounts, account]; onchange(account.id); label = ''; adding = false;
      status = 'Profile created. Sign in to connect its subscription.';
      announce(status);
    } catch (e) { error = loadErrorText(e); }
    finally { busy = false; }
  }
  async function login() {
    busy = true; error = ''; status = '';
    try {
      loginSession = await api.post<Session>(`/auth/provider-accounts/${value}/login`, { workspace_id: workspaceId });
      ws.addSession(loginSession);
    } catch (e) { error = loadErrorText(e); }
    finally { busy = false; }
  }
  async function check() {
    busy = true; error = '';
    try {
      const result = await api.get<{signed_in: boolean}>(`/auth/provider-accounts/${value}/status`);
      status = result.signed_in ? 'Signed in' : 'Sign-in required';
      announce(status);
    } catch (e) { error = loadErrorText(e); }
    finally { busy = false; }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`account-picker`,});
  
   { svelteHTML.createElement("div", { "class":`actions`,});
     { svelteHTML.createElement("label", {});
       { svelteHTML.createElement("span", {});provider;  }
       { svelteHTML.createElement("select", {       "class":`input`,"value":value,"disabled":busy,"onchange":(e) => { onchange(e.currentTarget.value); status = ''; loginSession = null; },});
         { svelteHTML.createElement("option", {"value":"",});   }
        if(value && !accounts.some((account) => account.id === value)){ { svelteHTML.createElement("option", { "value":value,});  }}
           for(let account of __sveltets_2_ensureArray(accounts)){account.id; { svelteHTML.createElement("option", { "value":account.id,});account.label; }}
       }
     }
     { svelteHTML.createElement("button", {         "class":`btn small`,"type":`button`,"disabled":busy,"aria-expanded":adding,"onclick":() => (adding = !adding),});  }
    if(value){
       { svelteHTML.createElement("button", {       "class":`btn small`,"type":`button`,"disabled":busy,"onclick":login,});  }
       { svelteHTML.createElement("button", {       "class":`btn small`,"type":`button`,"disabled":busy,"onclick":check,});  }
    }
   }
  if(listLoading){  { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {    "what":`accounts`,"variant":`compact`,"loading":true,"empty":true,}});}}
  if(listError){ { svelteHTML.createElement("p", {   "class":`error`,"role":`alert`,});listError;  { svelteHTML.createElement("button", {       "class":`btn small`,"type":`button`,"onclick":loadAccounts,"disabled":listLoading,});  } }}
  if(adding){
     { svelteHTML.createElement("div", { "class":`actions`,});
       { svelteHTML.createElement("input", {            "dir":`auto`,"class":`input`,"aria-label":`Account label`,"placeholder":`Personal, Work…`,"maxlength":80,"bind:value":label,});/*Ωignore_startΩ*/() => label = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("button", {       "class":`btn small`,"type":`button`,"disabled":busy || !label.trim(),"onclick":add,});  }
     }
     { svelteHTML.createElement("p", {});             }
  }
  
  if(status){ { svelteHTML.createElement("p", {});status; }}
  if(error){ { svelteHTML.createElement("p", {   "class":`error`,"role":`alert`,});error; }}
  if(loginSession){
     { svelteHTML.createElement("p", {});        }
     { svelteHTML.createElement("div", { "class":`login-terminal`,}); { const $$_lanimreTyzaL2C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL2C({ target: __sveltets_2_any(), props: {  "sessionId":loginSession.id,}});} }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AccountPicker__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AccountPicker__SvelteComponent_ = ReturnType<typeof AccountPicker__SvelteComponent_>;
/*Ωignore_endΩ*/export default AccountPicker__SvelteComponent_;