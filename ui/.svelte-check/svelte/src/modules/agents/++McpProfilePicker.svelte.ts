///<reference types="svelte" />
;// "MCP tools" — which Otto MCP profiles an agent session runs under. Multi-
// select: the session sees the UNION of the picked profiles' tools. The
// default profile is preselected; picking exactly the default stores
// nothing, so the session follows a later default change (like sessions
// created before profiles existed). Narrowing only — permissions and
// approvals still apply to every tool the agent can see.

import LoadState from '../../lib/components/LoadState.svelte';
import { mcpProfiles } from '../../lib/stores/mcp-profiles.svelte';
import { selectionHint, storedProfileIds, toggleProfile } from '../../lib/mcpProfiles';
;

  interface Props {
    /** The stored ids ([] = the default profile). */
    value: string[];
    onchange: (ids: string[]) => void;
    disabled?: boolean;
    /** Field id prefix (two pickers can share a page). */
    idPrefix?: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { value, onchange, disabled = false, idPrefix = 'mcp-tools' }: Props = $props();

  $effect(() => {
    void mcpProfiles.load();
  });

  const selected = $derived(value.length ? value : [mcpProfiles.defaultId]);
  const hint = $derived(selectionHint(selected, mcpProfiles.profiles));

  function toggle(id: string): void {
    const next = toggleProfile(selected, id, mcpProfiles.defaultId);
    onchange(storedProfileIds(next, mcpProfiles.profiles, mcpProfiles.defaultId));
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`field`,...__sveltets_2_empty({"data-testid":`mcp-profile-picker`}),});
   { svelteHTML.createElement("span", {   "class":`field-label`,"id":`${idPrefix}-label`,});  }
   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"what":`MCP profiles`,"variant":`compact`,"loading":!mcpProfiles.loaded && !mcpProfiles.error,"error":mcpProfiles.error,"rows":1,"onretry":() => void mcpProfiles.load(true),}});
     { svelteHTML.createElement("div", {     "class":`chips`,"role":`group`,"aria-labelledby":`${idPrefix}-label`,});
         for(let p of __sveltets_2_ensureArray(mcpProfiles.profiles)){p.id;
         { svelteHTML.createElement("button", {              "type":`button`,"class":`pick`,"aria-pressed":selected.includes(p.id),"title":p.description || p.name,...__sveltets_2_empty({"data-testid":`mcp-profile-option-${p.id}`}),disabled,"onclick":() => toggle(p.id),});
          p.name;if(p.id === mcpProfiles.defaultId){ { svelteHTML.createElement("span", { "class":`dflt`,});  }}
         }
      }
     }
     { svelteHTML.createElement("span", { "class":`hint`,});
           { svelteHTML.createElement("strong", {});mcpProfiles.labelFor(selected); } selected.length > 1 ? ' (all picked profiles combined)' : '';      hint;
     }
   LoadState}
  if(mcpProfiles.error){ { svelteHTML.createElement("span", { "class":`hint`,});         }}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const McpProfilePicker__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type McpProfilePicker__SvelteComponent_ = ReturnType<typeof McpProfilePicker__SvelteComponent_>;
/*Ωignore_endΩ*/export default McpProfilePicker__SvelteComponent_;