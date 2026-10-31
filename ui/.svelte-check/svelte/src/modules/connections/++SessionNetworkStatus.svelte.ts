///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { untrack } from 'svelte';
import { api } from '../../lib/api/client';
import { loadErrorText } from '../../lib/loadError';
import { pollWhileVisible } from '../../lib/poll';
import type { SessionNetwork } from '../../lib/api/types';
import NetworkProfilePicker from './NetworkProfilePicker.svelte';
import Icon from '../../lib/components/Icon.svelte';

;type $$ComponentProps =  {
    sessionId: string; workspaceId: string; selectedProfileId?: string; editable?: boolean; manageEditable?: boolean;
    /** Start expanded (opened on purpose from the session's ⋯ menu). */
    defaultOpen?: boolean;
    onchange?: (id: string) => Promise<void>;
  };function $$render() {

  
  
  
  
  
  
  
  
  let { sessionId, workspaceId, selectedProfileId = '', editable = false, manageEditable = false, defaultOpen = false, onchange }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let network = $state<SessionNetwork | null>(null);
  let error = $state('');
  let open = $state(untrack(() => defaultOpen));
  let busy = $state(false);
  let refresh = $state(0);
  // Status enum → words ("disabled" means no profile: a direct connection).
  const STATUS_LABEL: Record<SessionNetwork['status'], string> = {
    connected: 'Connected', error: 'Error', stopped: 'Stopped', disabled: 'Direct',
  };
  $effect(() => {
    const id = sessionId; void selectedProfileId; void refresh;
    // lib/poll.ts: no timer wake-ups while the window is hidden (one catch-up
    // tick on return), no overlapping requests, failure backoff, and the
    // in-flight read is aborted when the pane unmounts. After the first read,
    // only a profiled (or expanded) strip keeps polling.
    let first = true;
    const p = pollWhileVisible(async (signal) => {
      const due = first || untrack(() => open || selectedProfileId || network?.profile_id);
      first = false;
      if (!due) return;
      try {
        // Background lane: a status poll never takes an interactive socket.
        const next = await api.bg.get<SessionNetwork>(`/sessions/${id}/network`, signal);
        if (signal.aborted) return;
        network = next; error = '';
      } catch (e) {
        if (signal.aborted) return;
        error = loadErrorText(e);
        return false;
      }
    }, { ms: 5000 });
    return () => p.stop();
  });
  async function choose(id: string) {
    if (!onchange) return;
    busy = true;
    try { await onchange(id); refresh++; }
    catch (e) { error = e instanceof Error ? e.message : String(e); }
    finally { busy = false; }
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  // A derived string, so a poll that returns the same error doesn't re-announce it.
  const networkError = $derived(network?.error ?? '');
  $effect(() => { if (networkError) announce(`${networkError} Restart the session to rebuild its forwards.`, { assertive: true }); });
  $effect(() => { if (error) announce(error, { assertive: true }); });
;
async () => {
 { svelteHTML.createElement("details", {   "class":`network`,"bind:open":open,...__sveltets_2_empty({"data-testid":`network-status`}),});/*Ωignore_startΩ*/() => open = __sveltets_2_any(null);/*Ωignore_endΩ*/
   { svelteHTML.createElement("summary", { "aria-expanded":open,});
     { svelteHTML.createElement("span", {   "class":`chev`,"aria-hidden":`true`,});open; { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});} }
     { svelteHTML.createElement("span", { "class":`sum-text`,}); network?.profile_name || (selectedProfileId ? 'selected profile' : 'none');  network ? (STATUS_LABEL[network.status] ?? network.status) : 'Checking…';if(network?.restart_required){' · '; { svelteHTML.createElement("span", { "class":`warn`,});  }} }
   }
   { svelteHTML.createElement("div", { "class":`contents`,});
    if(network?.status === 'connected'){ { svelteHTML.createElement("p", {});        }}
    if(network?.error){ { svelteHTML.createElement("p", { "class":`error`,});network.error;        }}
    if(network?.restart_required){ { svelteHTML.createElement("p", {});           }}
       for(let endpoint of __sveltets_2_ensureArray(network?.endpoints ?? [])){endpoint.name;
       { svelteHTML.createElement("div", { "class":`endpoint`,}); { svelteHTML.createElement("strong", {});endpoint.name; } { svelteHTML.createElement("code", {});endpoint.host; endpoint.port;  endpoint.remote_host; endpoint.remote_port; }
         { svelteHTML.createElement("code", {}); endpoint.name.toUpperCase(); endpoint.host; } { svelteHTML.createElement("code", {}); endpoint.name.toUpperCase(); endpoint.port; }
        if(endpoint.host_env){ { svelteHTML.createElement("code", {});endpoint.host_env; endpoint.host; }}
        if(endpoint.port_env){ { svelteHTML.createElement("code", {});endpoint.port_env; endpoint.port; }}
       }
    }
    if(error){ { svelteHTML.createElement("p", { "class":`error`,});error; }}
     { svelteHTML.createElement("button", {     "type":`button`,"class":`btn small`,"onclick":() => refresh++,});   }
    if(editable && onchange){
       { const $$_rekciPeliforPkrowteN2C = __sveltets_2_ensureComponent(NetworkProfilePicker); new $$_rekciPeliforPkrowteN2C({ target: __sveltets_2_any(), props: {         workspaceId,"value":selectedProfileId,"onchange":(id) => void choose(id),"editable":manageEditable,"disabled":busy,}});}
       { svelteHTML.createElement("p", {});       }
    }
   }
 }

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SessionNetworkStatus__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SessionNetworkStatus__SvelteComponent_ = ReturnType<typeof SessionNetworkStatus__SvelteComponent_>;
/*Ωignore_endΩ*/export default SessionNetworkStatus__SvelteComponent_;