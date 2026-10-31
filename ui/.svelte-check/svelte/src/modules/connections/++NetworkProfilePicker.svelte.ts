///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { api } from '../../lib/api/client';
import { loadErrorText } from '../../lib/loadError';
import type { NetworkProfile } from '../../lib/api/types';
import NetworkProfiles from './NetworkProfiles.svelte';

;type $$ComponentProps =  {
    workspaceId: string; value?: string; onchange: (id: string) => void; editable?: boolean; disabled?: boolean;
  };function $$render() {

  
  
  
  
  
  let { workspaceId, value = '', onchange, editable = false, disabled = false }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let profiles = $state<NetworkProfile[]>([]);
  let loading = $state(false);
  let error = $state('');
  let managing = $state(false);
  let reload = $state(0);
  $effect(() => {
    const scope = workspaceId; void reload;
    let alive = true;
    profiles = []; loading = true; error = ''; managing = false;
    void api.get<NetworkProfile[]>(`/workspaces/${scope}/network-profiles`).then((rows) => { if (alive) profiles = rows; })
      .catch((e) => { if (alive) error = loadErrorText(e); }).finally(() => { if (alive) loading = false; });
    return () => { alive = false; };
  });
  function saved(profile: NetworkProfile) {
    profiles = [...profiles.filter((p) => p.id !== profile.id), profile];
    if (!profile.archived) onchange(profile.id);
    else if (value === profile.id) onchange('');
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (error) announce(`${error} Retry loading profiles.`, { assertive: true }); });
;
async () => {
 { svelteHTML.createElement("div", { "class":`network-picker`,});
   { svelteHTML.createElement("label", {});  { svelteHTML.createElement("select", {       "aria-label":`Network profile`,"value":value,"disabled":disabled || loading,"onchange":(e) => onchange(e.currentTarget.value),});
     { svelteHTML.createElement("option", {"value":"",});    }
       for(let profile of __sveltets_2_ensureArray(profiles.filter((p) => !p.archived))){profile.id; { svelteHTML.createElement("option", { "value":profile.id,});profile.name; }}
    if(value && !profiles.some((p) => p.id === value && !p.archived)){ { svelteHTML.createElement("option", {  "value":value,"disabled":true,});   }}
   } }
  if(editable){ { svelteHTML.createElement("button", {       "type":`button`,"aria-expanded":managing,"disabled":disabled || loading,"onclick":() => managing = !managing,});   }}
  if(error){ { svelteHTML.createElement("p", {});error;  { svelteHTML.createElement("button", {   "type":`button`,"onclick":() => reload++,});   } }}
  if(managing){workspaceId; { { const $$_seliforPkrowteN1C = __sveltets_2_ensureComponent(NetworkProfiles); new $$_seliforPkrowteN1C({ target: __sveltets_2_any(), props: {    workspaceId,profiles,"onsaved":saved,}});}}}
 }

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NetworkProfilePicker__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NetworkProfilePicker__SvelteComponent_ = ReturnType<typeof NetworkProfilePicker__SvelteComponent_>;
/*Ωignore_endΩ*/export default NetworkProfilePicker__SvelteComponent_;