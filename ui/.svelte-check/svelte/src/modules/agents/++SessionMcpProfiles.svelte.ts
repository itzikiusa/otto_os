///<reference types="svelte" />
;// Session ⋯ → MCP tools: change which Otto MCP profiles this agent session
// runs under. The profile is captured when the agent starts, so the change
// applies on the next restart — said here, and offered as "Save & restart".

import Modal from '../../lib/components/Modal.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
import { mcpProfiles } from '../../lib/stores/mcp-profiles.svelte';
import { sessionProfileIds } from '../../lib/mcpProfiles';
import McpProfilePicker from './McpProfilePicker.svelte';
import type { McpSkippedServer } from '../../lib/api/types';
;

  interface Props {
    sessionId: string;
    meta: Record<string, unknown> | null | undefined;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { sessionId, meta, onclose }: Props = $props();

  // svelte-ignore state_referenced_locally
  let ids = $state<string[]>(sessionProfileIds(meta));
  let busy = $state(false);
  // svelte-ignore state_referenced_locally
  const before = sessionProfileIds(meta).join(',');
  const changed = $derived(ids.join(',') !== before);
  // error-handling-6-06: the user servers the launcher could not attach at the
  // last start (`meta.mcp_skipped`), so a missing server is visible here
  // instead of only in the daemon log.
  const skipped = $derived(
    (Array.isArray(meta?.mcp_skipped) ? (meta?.mcp_skipped as McpSkippedServer[]) : []).filter(
      (s) => typeof s?.server === 'string',
    ),
  );

  async function save(alsoRestart: boolean): Promise<void> {
    if (busy) return;
    if (alsoRestart && !(await ws.confirmRestart(sessionId))) return;
    busy = true;
    try {
      try {
        await ws.updateSessionMeta(sessionId, { mcp_profiles: ids.length ? ids : null });
      } catch (e) {
        toastError('Couldn’t save the MCP tools', e);
        return;
      }
      onclose();
      const label = mcpProfiles.labelFor(ids);
      if (!alsoRestart) {
        toasts.success(`MCP tools: ${label}`, 'Applies the next time the agent restarts.');
        return;
      }
      try {
        await ws.restartSession(sessionId, { quiet: true });
        toasts.success(`MCP tools: ${label}`, 'Agent restarted with the new tool set.');
      } catch (e) {
        toasts.push('error', 'MCP tools saved, but the agent didn’t restart', loadErrorText(e), 12000, {
          action: { label: 'Restart', run: () => ws.requestRestart(sessionId) },
        });
      }
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`MCP tools`,"width":480,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy || !changed,"onclick":() => void save(false),});busy ? 'Saving…' : 'Save'; }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy || !changed,"onclick":() => void save(true),...__sveltets_2_empty({"data-testid":`mcp-tools-save-restart`}),});
      busy ? 'Saving…' : 'Save & restart';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("p", { "class":`lead`,});
                     
        { svelteHTML.createElement("strong", {});     }
   }
   { const $$_rekciPeliforPpcM1C = __sveltets_2_ensureComponent(McpProfilePicker); new $$_rekciPeliforPpcM1C({ target: __sveltets_2_any(), props: {        "value":ids,"onchange":(next) => (ids = next),"disabled":busy,"idPrefix":`sv-mcp-tools`,}});}
  if(skipped.length){
     { svelteHTML.createElement("section", {     "class":`skipped`,"aria-labelledby":`sv-mcp-skipped-title`,...__sveltets_2_empty({"data-testid":`mcp-skipped`}),});
       { svelteHTML.createElement("h3", { "id":`sv-mcp-skipped-title`,});
        skipped.length === 1 ? '1 MCP server wasn’t attached' : `${skipped.length} MCP servers weren’t attached`;
           
       }
       { svelteHTML.createElement("ul", {});
           for(let s of __sveltets_2_ensureArray(skipped)){s.server;
           { svelteHTML.createElement("li", {}); { svelteHTML.createElement("strong", {});s.server; } s.reason; }
        }
       }
       { svelteHTML.createElement("p", { "class":`hint`,});            }
     }
  }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SessionMcpProfiles__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SessionMcpProfiles__SvelteComponent_ = ReturnType<typeof SessionMcpProfiles__SvelteComponent_>;
/*Ωignore_endΩ*/export default SessionMcpProfiles__SvelteComponent_;