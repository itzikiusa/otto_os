///<reference types="svelte" />
;// Create / edit / duplicate an MCP profile: a name, the tool groups it is
// built from, and per-tool overrides (a tool its groups leave out can be
// added, one they include can be removed). "All tools" is the Full shape —
// every tool minus the removed ones.

import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { mcpCpApi } from '../../lib/api/mcp';
import type { McpProfile, McpToolGroup } from '../../lib/api/types';
import {
    draftToReq,
    profileAllows,
    profileTools,
    toggleGroup,
    toggleTool,
    validateDraft,
    type ProfileDraft,
  } from '../../lib/mcpProfiles';
;

  interface Props {
    /** The profile being edited; null creates (from `initial`). */
    editing: McpProfile | null;
    initial: ProfileDraft;
    profiles: McpProfile[];
    groups: McpToolGroup[];
    onclose: () => void;
    onsaved: (p: McpProfile) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { editing, initial, profiles, groups, onclose, onsaved }: Props = $props();

  // `initial` arrives as a $state proxy, which structuredClone refuses (DataCloneError).
  // svelte-ignore state_referenced_locally
  let draft = $state<ProfileDraft>($state.snapshot(initial) as ProfileDraft);
  let busy = $state(false);
  let touched = $state(false);
  let open = $state<Record<string, boolean>>({});
  const error = $derived(validateDraft(draft, profiles, editing?.id ?? null));
  const count = $derived(profileTools(groups, draft).length);

  const inGroup = (g: McpToolGroup) => draft.all_tools || draft.groups.includes(g.id);

  async function save(): Promise<void> {
    touched = true;
    if (error || busy) return;
    busy = true;
    try {
      const saved = editing
        ? await mcpCpApi.cpUpdateProfile(editing.id, draftToReq(draft))
        : await mcpCpApi.cpCreateProfile(draftToReq(draft));
      toasts.success(editing ? 'Profile saved' : 'Profile created', saved.name);
      onsaved(saved);
      onclose();
    } catch (e) {
      toastError(editing ? 'Couldn’t save the profile' : 'Couldn’t create the profile', e);
    } finally {
      busy = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":editing ? `Edit ${editing.name}` : 'New MCP profile',"width":560,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {   "class":`btn`,"onclick":onclose,});  }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy || (touched && !!error),"onclick":() => void save(),...__sveltets_2_empty({"data-testid":`mcp-profile-save`}),});
      busy ? 'Saving…' : editing ? 'Save' : 'Create profile';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`mcp-profile-name`,});  }
     { svelteHTML.createElement("input", {              "id":`mcp-profile-name`,"class":`input`,"dir":`auto`,"maxlength":60,"bind:value":draft.name,"aria-invalid":touched && !!error,"aria-describedby":`mcp-profile-err`,});/*Ωignore_startΩ*/() => draft.name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
   }
   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`mcp-profile-desc`,});  { svelteHTML.createElement("span", { "class":`dim`,});  } }
     { svelteHTML.createElement("input", {          "id":`mcp-profile-desc`,"class":`input`,"dir":`auto`,"bind:value":draft.description,"placeholder":`What sessions use this for`,});/*Ωignore_startΩ*/() => draft.description = __sveltets_2_any(null);/*Ωignore_endΩ*/}
   }
   { svelteHTML.createElement("label", { "class":`toggle-row`,});
     { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":draft.all_tools,...__sveltets_2_empty({"data-testid":`mcp-profile-all`}),});/*Ωignore_startΩ*/() => draft.all_tools = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("span", {}); { svelteHTML.createElement("strong", {});  }  { svelteHTML.createElement("span", { "class":`dim`,});         } }
   }

   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("span", {   "class":`field-label`,"id":`mcp-profile-groups`,});   { svelteHTML.createElement("span", { "class":`dim`,}); count;   } }
     { svelteHTML.createElement("ul", {   "class":`groups`,"aria-labelledby":`mcp-profile-groups`,});
         for(let g of __sveltets_2_ensureArray(groups)){g.id;
         { svelteHTML.createElement("li", { "class":`group`,});
           { svelteHTML.createElement("div", { "class":`g-row`,});
             { svelteHTML.createElement("label", {   "class":`g-pick`,"title":g.description,});
               { svelteHTML.createElement("input", {           "type":`checkbox`,"checked":inGroup(g),"disabled":draft.all_tools,...__sveltets_2_empty({"data-testid":`mcp-profile-group-${g.id}`}),"onchange":() => (draft = toggleGroup(draft, g.id)),});}
               { svelteHTML.createElement("span", { "class":`g-name`,});g.label; }
               { svelteHTML.createElement("span", { "class":`dim g-count`,});g.tools.length;g.prefixes.length ? '+' : ''; }
             }
             { svelteHTML.createElement("button", {             "type":`button`,"class":`icon-btn`,"aria-expanded":!!open[g.id],"aria-label":`${open[g.id] ? 'Hide' : 'Show'} ${g.label} tools`,"title":`${open[g.id] ? 'Hide' : 'Show'} tools`,"onclick":() => (open = { ...open, [g.id]: !open[g.id] }),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":open[g.id] ? 'chevronDown' : 'chevronRight',"noflip":true,"size":12,}});} }
           }
          if(open[g.id]){
             { svelteHTML.createElement("p", { "class":`dim small g-desc`,});g.description;g.prefixes.length ? ` Also every ${g.prefixes.map((p) => p + '*').join(', ')} tool.` : ''; }
             { svelteHTML.createElement("ul", { "class":`tools`,});
                 for(let t of __sveltets_2_ensureArray(g.tools)){t;
                 { svelteHTML.createElement("li", {});
                   { svelteHTML.createElement("label", { "class":`t-pick mono`,});
                     { svelteHTML.createElement("input", {      "type":`checkbox`,"checked":profileAllows(groups, draft, t),"onchange":() => (draft = toggleTool(groups, draft, t)),});}
                    t;
                   }
                 }
              }
             }
          }
         }
      }
     }
     { svelteHTML.createElement("span", { "class":`hint`,});              }
   }
   { svelteHTML.createElement("p", {     "id":`mcp-profile-err`,"class":`err`,"role":`status`,});touched ? error : ''; }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ProfileEditor__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProfileEditor__SvelteComponent_ = ReturnType<typeof ProfileEditor__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProfileEditor__SvelteComponent_;