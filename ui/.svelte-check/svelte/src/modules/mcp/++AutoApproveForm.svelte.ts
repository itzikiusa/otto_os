///<reference types="svelte" />
;// New auto-approve rule: WHAT (one mutating tool, or every mutating tool in a
// category) × WHERE (everywhere / one workspace / one agent session). An
// irreversible tool needs the second explicit toggle; a category never
// covers one. Every auto-approved call is still audited, naming the rule.

import { untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import Modal from '../../lib/components/Modal.svelte';
import { toasts } from '../../lib/toast.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { mcpCpExtraApi } from './cp-api';
import type {
    McpAutoApproveList,
    McpAutoApproveScope,
    McpAutoApproveTargetKind,
  } from '../../lib/api/types';
;

  interface Props {
    catalog: McpAutoApproveList['categories'];
    /** Pre-selected target (e.g. opened from a catalog row). */
    preset?: { kind: McpAutoApproveTargetKind; target: string } | null;
    /** Pre-selected scope (e.g. "Approve & always allow…" from an approval
     *  card defaults to the requesting session, else its workspace). */
    presetScope?: {
      scope: McpAutoApproveScope;
      workspace_id?: string | null;
      session_id?: string | null;
      /** Shown when the session isn't in the current workspace's list. */
      session_label?: string | null;
    } | null;
    /** Modal title / primary label overrides (the approval-card flow also approves). */
    title?: string;
    saveLabel?: string;
    onclose: () => void;
    onsaved: () => void | Promise<void>;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let {
    catalog,
    preset = null,
    presetScope = null,
    title = 'New auto-approve rule',
    saveLabel = 'Create rule',
    onclose,
    onsaved,
  }: Props = $props();

  const init = untrack(() => preset);
  const initScope = untrack(() => presetScope);
  let kind = $state<McpAutoApproveTargetKind>(init?.kind ?? 'category');
  let category = $state(init?.kind === 'category' ? init.target : (untrack(() => catalog)[0]?.category ?? ''));
  let tool = $state(init?.kind === 'tool' ? init.target.replace(/^otto\./, '') : '');
  let scope = $state<McpAutoApproveScope>(initScope?.scope ?? 'global');
  let workspaceId = $state(initScope?.workspace_id ?? untrack(() => ws.currentId) ?? '');
  let sessionId = $state(initScope?.session_id ?? '');
  let name = $state('');
  let ackIrreversible = $state(false);
  let saving = $state(false);

  const allTools = $derived(
    catalog.flatMap((group) =>
      group.tools.map((t) => ({ ...t, bare: t.name.replace(/^otto\./, ''), category: group.category })),
    ),
  );
  const selectedTool = $derived(allTools.find((t) => t.bare === tool) ?? null);
  const irreversible = $derived(kind === 'tool' && !!selectedTool?.irreversible);
  const categoryGroup = $derived(catalog.find((group) => group.category === category) ?? null);
  const excluded = $derived((categoryGroup?.tools ?? []).filter((t) => t.irreversible));
  const covered = $derived((categoryGroup?.tools ?? []).filter((t) => !t.irreversible));
  // The preset session stays choosable even when it lives in another workspace.
  const sessions = $derived.by(() => {
    const list = ws.agentSessions.map((s) => ({ id: s.id, label: `${s.title} · ${s.provider}` }));
    const pre = initScope?.session_id;
    if (pre && !list.some((s) => s.id === pre)) list.unshift({ id: pre, label: initScope?.session_label ?? pre });
    return list;
  });
  const workspaces = $derived(ws.workspaces);

  const target = $derived(kind === 'category' ? category : tool);
  const scopeReady = $derived(
    scope === 'global' || (scope === 'workspace' && !!workspaceId) || (scope === 'session' && !!sessionId),
  );
  const canSave = $derived(!!target && scopeReady && (!irreversible || ackIrreversible) && !saving);

  async function save(): Promise<void> {
    if (!canSave) return;
    saving = true;
    try {
      await mcpCpExtraApi.createAutoApprove({
        name: name.trim() || undefined,
        scope,
        workspace_id: scope === 'workspace' ? workspaceId : undefined,
        session_id: scope === 'session' ? sessionId : undefined,
        target_kind: kind,
        target,
        allow_irreversible: irreversible ? ackIrreversible : undefined,
      });
      toasts.success('Auto-approve rule created', kind === 'category' ? `${category} writes` : `otto.${tool}`);
      await onsaved();
      onclose();
    } catch (e) {
      toastError('Couldn’t create the rule', e);
    } finally {
      saving = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },title,"width":560,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {     "class":`btn`,"onclick":onclose,"disabled":saving,});  }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":() => void save(),"disabled":!canSave,...__sveltets_2_empty({"data-testid":`mcp-auto-approve-save`}),});
      saving ? 'Saving…' : saveLabel;
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", {   "class":`form`,...__sveltets_2_empty({"data-testid":`mcp-auto-approve-form`}),});
     { svelteHTML.createElement("fieldset", { "class":`field`,});
       { svelteHTML.createElement("legend", {});    }
       { svelteHTML.createElement("div", { "class":`seg`,});
         { svelteHTML.createElement("label", { "class":`check`,});
           { svelteHTML.createElement("input", {       "type":`radio`,"name":`aa-kind`,"value":`category`,});kind = __sveltets_2_any(null);}
           { svelteHTML.createElement("span", {});      }
         }
         { svelteHTML.createElement("label", { "class":`check`,});
           { svelteHTML.createElement("input", {       "type":`radio`,"name":`aa-kind`,"value":`tool`,});kind = __sveltets_2_any(null);}
           { svelteHTML.createElement("span", {});  }
         }
       }
      if(kind === 'category'){
         { svelteHTML.createElement("select", {     "bind:value":category,"aria-label":`Category`,...__sveltets_2_empty({"data-testid":`mcp-auto-approve-category`}),});/*Ωignore_startΩ*/() => category = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let group of __sveltets_2_ensureArray(catalog)){group.category;
             { svelteHTML.createElement("option", { "value":group.category,});group.category; group.tools.length;  }
          }
         }
        if(categoryGroup){
           { svelteHTML.createElement("span", { "class":`hint`,});
             covered.map((t) => t.name).join(', ') || 'no reversible tools';    
            category; 
           }
          if(excluded.length){
             { svelteHTML.createElement("span", { "class":`hint warn-text`,});
                  excluded.map((t) => t.name).join(', ');    
                      
             }
          }
        }
      }else{
         { svelteHTML.createElement("select", {     "bind:value":tool,"aria-label":`Tool`,...__sveltets_2_empty({"data-testid":`mcp-auto-approve-tool`}),});/*Ωignore_startΩ*/() => tool = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", {"value":"","disabled":true,});    }
             for(let group of __sveltets_2_ensureArray(catalog)){group.category;
             { svelteHTML.createElement("optgroup", { "label":group.category,});
                 for(let t of __sveltets_2_ensureArray(group.tools)){t.name;
                 { svelteHTML.createElement("option", { "value":t.name.replace(/^otto\./, ''),});t.name;t.irreversible ? ' — irreversible' : ''; }
              }
             }
          }
         }
      }
     }

     { svelteHTML.createElement("fieldset", { "class":`field`,});
       { svelteHTML.createElement("legend", {});  }
       { svelteHTML.createElement("div", { "class":`seg`,});
         { svelteHTML.createElement("label", { "class":`check`,});
           { svelteHTML.createElement("input", {       "type":`radio`,"name":`aa-scope`,"value":`global`,});scope = __sveltets_2_any(null);}
           { svelteHTML.createElement("span", {});  }
         }
         { svelteHTML.createElement("label", { "class":`check`,});
           { svelteHTML.createElement("input", {       "type":`radio`,"name":`aa-scope`,"value":`workspace`,});scope = __sveltets_2_any(null);}
           { svelteHTML.createElement("span", {});  }
         }
         { svelteHTML.createElement("label", { "class":`check`,});
           { svelteHTML.createElement("input", {       "type":`radio`,"name":`aa-scope`,"value":`session`,});scope = __sveltets_2_any(null);}
           { svelteHTML.createElement("span", {});   }
         }
       }
      if(scope === 'workspace'){
         { svelteHTML.createElement("select", {   "bind:value":workspaceId,"aria-label":`Workspace`,});/*Ωignore_startΩ*/() => workspaceId = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", {"value":"","disabled":true,});   }
             for(let w of __sveltets_2_ensureArray(workspaces)){w.id; { svelteHTML.createElement("option", { "value":w.id,});w.name; }}
         }
         { svelteHTML.createElement("span", { "class":`hint`,});
                           
                        
             
         }
      } else if (scope === 'session'){
        if(sessions.length){
           { svelteHTML.createElement("select", {   "bind:value":sessionId,"aria-label":`Agent session`,});/*Ωignore_startΩ*/() => sessionId = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"","disabled":true,});   }
               for(let s of __sveltets_2_ensureArray(sessions)){s.id; { svelteHTML.createElement("option", { "value":s.id,});s.label; }}
           }
           { svelteHTML.createElement("span", { "class":`hint`,});              }
        }else{
           { svelteHTML.createElement("span", { "class":`hint`,});    ws.current?.name ?? 'this workspace';  }
        }
      }
     }

     { svelteHTML.createElement("label", { "class":`field`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("input", {      "dir":`auto`,"bind:value":name,"placeholder":`Optional — shown in the audit trail`,});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }

    if(irreversible){
       { svelteHTML.createElement("div", {   "class":`danger-box`,"role":`note`,});
         { svelteHTML.createElement("strong", {}); tool;   }
         { svelteHTML.createElement("span", {});
                         
           
         }
         { svelteHTML.createElement("label", { "class":`check`,});
           { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":ackIrreversible,...__sveltets_2_empty({"data-testid":`mcp-auto-approve-ack`}),});/*Ωignore_startΩ*/() => ackIrreversible = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", {}); tool;          }
         }
       }
    }
     { svelteHTML.createElement("p", { "class":`hint`,});                }
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AutoApproveForm__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AutoApproveForm__SvelteComponent_ = ReturnType<typeof AutoApproveForm__SvelteComponent_>;
/*Ωignore_endΩ*/export default AutoApproveForm__SvelteComponent_;