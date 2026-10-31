///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { tick } from 'svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { api } from '../../lib/api/client';
import type { Connection, NetworkEndpoint, NetworkProfile, NetworkProfileInput } from '../../lib/api/types';

;type $$ComponentProps =  { workspaceId: string; profiles: NetworkProfile[]; onsaved: (profile: NetworkProfile) => void };function $$render() {

  
  
  
  
  
  
  let { workspaceId, profiles, onsaved }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let connections = $state<Connection[]>([]);
  let error = $state('');
  let connectionsLoading = $state(true);
  let connectionsError = $state<string | null>(null);
  let attempt = $state(0);
  let busy = $state(false);
  let editing = $state<NetworkProfile | null>(null);
  let name = $state('');
  let sshId = $state('');
  let endpoints = $state<NetworkEndpoint[]>([emptyEndpoint()]);
  let archived = $state(false);
  let endpointNames = $state<HTMLInputElement[]>([]);
  async function addEndpoint() {
    if (busy || endpoints.length >= 8) return;
    endpoints = [...endpoints, emptyEndpoint()];
    await tick();
    endpointNames[endpoints.length - 1]?.focus();
  }
  async function removeEndpoint(index: number) {
    if (busy || endpoints.length === 1) return;
    endpoints = endpoints.filter((_, i) => i !== index);
    await tick();
    endpointNames[Math.min(index, endpoints.length - 1)]?.focus();
  }
  let alive = true;
  $effect(() => () => { alive = false; });
  function emptyEndpoint(): NetworkEndpoint { return { name: '', remote_host: '', remote_port: 5432, host_env: '', port_env: '' }; }
  $effect(() => {
    const scope = workspaceId;
    void attempt;
    let current = true;
    connections = [];
    connectionsLoading = true;
    connectionsError = null;
    void api.get<Connection[]>(`/workspaces/${scope}/connections`).then((rows) => {
      if (current) connections = rows.filter((c) => c.kind === 'ssh');
    }).catch((e) => { if (current) connectionsError = loadErrorText(e); })
      .finally(() => { if (current) connectionsLoading = false; });
    return () => { current = false; };
  });
  function edit(profile: NetworkProfile | null) {
    editing = profile; name = profile?.name ?? ''; sshId = profile?.ssh_connection_id ?? '';
    endpoints = profile?.endpoints.map((e) => ({...e})) ?? [emptyEndpoint()];
    archived = profile?.archived ?? false; error = '';
  }
  async function reloadSaved() {
    if (!editing) return;
    busy = true; error = '';
    try { const profile = await api.get<NetworkProfile>(`/network-profiles/${editing.id}`); if (alive) edit(profile); }
    catch (e) { if (alive) error = `Couldn’t reload the saved profile. ${loadErrorText(e)}`; }
    finally { busy = false; }
  }
  const invalidEndpoints = $derived(endpoints.some((e) => !e.name.trim() || !e.remote_host.trim() || !Number.isInteger(e.remote_port) || e.remote_port < 1 || e.remote_port > 65535));
  async function save() {
    if (busy || invalidEndpoints || !name.trim() || !sshId) return;
    busy = true; error = '';
    const payload: NetworkProfileInput = { name: name.trim(), ssh_connection_id: sshId, archived,
      endpoints: endpoints.map((e) => ({...e, name: e.name.trim(), remote_host: e.remote_host.trim(), remote_port: Number(e.remote_port), host_env: e.host_env?.trim() || null, port_env: e.port_env?.trim() || null})) };
    try {
      const saved = editing
        ? await api.put<NetworkProfile>(`/network-profiles/${editing.id}`, {...payload, version: editing.version})
        : await api.post<NetworkProfile>(`/workspaces/${workspaceId}/network-profiles`, payload);
      if (alive) { edit(saved); onsaved(saved); }
    } catch (e) { error = e instanceof Error ? e.message : String(e); }
    finally { busy = false; }
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (error) announce(error, { assertive: true }); });
;
async () => {
 { svelteHTML.createElement("div", { "class":`manager`,});
   { svelteHTML.createElement("p", {});                   }
   { svelteHTML.createElement("div", { "class":`profiles`,});
       for(let profile of __sveltets_2_ensureArray(profiles)){profile.id; { svelteHTML.createElement("button", {           "class":`btn small`,"type":`button`,"disabled":busy,"onclick":() => edit(profile),"aria-pressed":editing?.id === profile.id,"aria-label":`Edit ${profile.name}`,});profile.name;profile.archived ? ' (archived)' : ''; }}
     { svelteHTML.createElement("button", {       "class":`btn small`,"type":`button`,"disabled":busy,"onclick":() => edit(null),});   }
   }
   { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {          "dir":`auto`,"aria-label":`Network profile name`,"bind:value":name,"disabled":busy,"maxlength":120,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
   { svelteHTML.createElement("label", {});  { svelteHTML.createElement("select", {     "aria-label":`SSH connection`,"bind:value":sshId,"disabled":busy || connectionsLoading || !!connectionsError,});/*Ωignore_startΩ*/() => sshId = __sveltets_2_any(null);/*Ωignore_endΩ*/
     { svelteHTML.createElement("option", {"value":"",});   }
       for(let connection of __sveltets_2_ensureArray(connections)){connection.id; { svelteHTML.createElement("option", { "value":connection.id,});connection.name; }}
   } }
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {            "what":`SSH connections`,"loading":connectionsLoading,"error":connectionsError,"empty":connections.length === 0,"variant":`compact`,"onretry":() => attempt++,}});}
  if(!connectionsLoading && !connectionsError && !connections.length){ { svelteHTML.createElement("p", {});           }}
      for(let endpoint of __sveltets_2_ensureArray(endpoints)){let i = 1;i;
     { svelteHTML.createElement("fieldset", { "disabled":busy,}); { svelteHTML.createElement("legend", {}); i + 1; }
       { svelteHTML.createElement("div", { "class":`endpoint-fields`,});
       { svelteHTML.createElement("label", {});  { const $$_input4 = svelteHTML.createElement("input", {         "dir":`ltr`,"aria-label":`Endpoint ${i + 1} name`,"bind:value":endpoint.name,"placeholder":`DB`,});endpointNames[i] = $$_input4;/*Ωignore_startΩ*/() => endpoint.name = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
       { svelteHTML.createElement("label", {});  { svelteHTML.createElement("input", {        "dir":`ltr`,"aria-label":`Endpoint ${i + 1} remote host`,"bind:value":endpoint.remote_host,"placeholder":`db.internal`,});/*Ωignore_startΩ*/() => endpoint.remote_host = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
       { svelteHTML.createElement("label", {});  { svelteHTML.createElement("input", {          "aria-label":`Endpoint ${i + 1} remote port`,"type":`number`,"min":`1`,"max":`65535`,"bind:value":endpoint.remote_port,});/*Ωignore_startΩ*/() => endpoint.remote_port = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
       { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {        "dir":`ltr`,"aria-label":`Endpoint ${i + 1} host environment`,"bind:value":endpoint.host_env,"placeholder":`PGHOST or APP_HOST`,});/*Ωignore_startΩ*/() => endpoint.host_env = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
       { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {        "dir":`ltr`,"aria-label":`Endpoint ${i + 1} port environment`,"bind:value":endpoint.port_env,"placeholder":`PGPORT or APP_PORT`,});/*Ωignore_startΩ*/() => endpoint.port_env = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
       }
       { svelteHTML.createElement("button", {         "class":`btn small remove-endpoint`,"type":`button`,"disabled":endpoints.length === 1,"title":endpoints.length === 1 ? 'A profile needs at least one endpoint' : undefined,"onclick":() => void removeEndpoint(i),});  i + 1; }
     }
  }
   { svelteHTML.createElement("button", {         "class":`btn small`,"type":`button`,"disabled":busy || endpoints.length >= 8,"title":endpoints.length >= 8 ? 'Up to 8 endpoints per profile' : undefined,"onclick":() => void addEndpoint(),});  }
   { svelteHTML.createElement("p", {});                }
  if(editing){ { svelteHTML.createElement("label", { "class":`archive`,}); { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":archived,"disabled":busy,});/*Ωignore_startΩ*/() => archived = __sveltets_2_any(null);/*Ωignore_endΩ*/}     }}
  if(error){ { svelteHTML.createElement("p", { "class":`error`,});error; }}
   { svelteHTML.createElement("div", { "class":`profiles`,}); { svelteHTML.createElement("button", {       "class":`btn small`,"type":`button`,"disabled":busy || connectionsLoading || !!connectionsError || !name.trim() || !sshId || invalidEndpoints,"onclick":save,});busy ? 'Saving…' : 'Save network profile'; }
  if(editing){ { svelteHTML.createElement("button", {       "class":`btn small`,"type":`button`,"disabled":busy,"onclick":reloadSaved,});   }}
   { svelteHTML.createElement("button", {       "class":`btn small`,"type":`button`,"disabled":busy,"onclick":() => edit(null),});  } }
 }

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NetworkProfiles__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NetworkProfiles__SvelteComponent_ = ReturnType<typeof NetworkProfiles__SvelteComponent_>;
/*Ωignore_endΩ*/export default NetworkProfiles__SvelteComponent_;