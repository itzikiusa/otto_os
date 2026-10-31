///<reference types="svelte" />
;// Auto-approve rules: the explicit, opt-in list of mutating otto.* tools that
// run WITHOUT a per-call approval — per tool or per category, everywhere / in
// one workspace / for one agent session. Off by default; every auto-approved
// call is still audited ("Auto approved", naming the rule). Irreversible
// tools need a per-tool rule plus a second confirmation.

import Icon from '../../lib/components/Icon.svelte';
import { toastError } from '../../lib/toastError';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import type { McpAutoApproveList, McpAutoApproveRule } from '../../lib/api/types';
import { mcpCpExtraApi } from './cp-api';
import AutoApproveForm from './AutoApproveForm.svelte';
;

  interface Props {
    isMcpAdmin: boolean;
    /** Bumped by the parent after it changed rules (catalog switches). */
    revision: number;
    /** Tell the parent the rules changed (its catalog badges reload). */
    onchange: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { isMcpAdmin, revision, onchange }: Props = $props();

  let data = $state<McpAutoApproveList | null>(null);
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let formOpen = $state(false);

  const rules = $derived(data?.rules ?? []);
  const activeCount = $derived(rules.filter((r) => r.enabled).length);

  async function load(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      data = await mcpCpExtraApi.autoApproveRules();
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void revision;
    void load();
  });

  function targetLabel(rule: McpAutoApproveRule): string {
    return rule.target_kind === 'category' ? `${rule.target} writes` : `otto.${rule.target}`;
  }

  function scopeLabel(rule: McpAutoApproveRule): string {
    if (rule.scope === 'workspace') return `Workspace · ${rule.workspace_name ?? rule.workspace_id}`;
    if (rule.scope === 'session') return `Session · ${rule.session_title ?? rule.session_id}`;
    return 'Everywhere';
  }

  async function setEnabled(rule: McpAutoApproveRule, enabled: boolean, input: HTMLInputElement): Promise<void> {
    if (
      enabled &&
      !(await confirmer.ask(
        `Turn “${rule.name}” back on? ${targetLabel(rule)} will run without asking (${scopeLabel(rule).toLowerCase()}). Every call is still audited.`,
        { title: 'Auto-approve', confirmLabel: 'Turn on', danger: true },
      ))
    ) {
      input.checked = false;
      return;
    }
    busy = rule.id;
    try {
      await mcpCpExtraApi.updateAutoApprove(rule.id, { enabled });
      await load();
      onchange();
    } catch (e) {
      input.checked = rule.enabled;
      toastError('Couldn’t update the rule', e);
    } finally {
      busy = null;
    }
  }

  async function remove(rule: McpAutoApproveRule): Promise<void> {
    if (
      !(await confirmer.ask(`Delete “${rule.name}”? ${targetLabel(rule)} will ask before each call again.`, {
        title: 'Delete auto-approve rule',
        confirmLabel: 'Delete',
        danger: true,
      }))
    )
      return;
    busy = rule.id;
    try {
      await mcpCpExtraApi.deleteAutoApprove(rule.id);
      toasts.success('Rule deleted', `${targetLabel(rule)} asks again`);
      await load();
      onchange();
    } catch (e) {
      toastError('Couldn’t delete the rule', e);
    } finally {
      busy = null;
    }
  }
;
async () => {

 { svelteHTML.createElement("section", {   "class":`panel`,...__sveltets_2_empty({"data-testid":`mcp-auto-approve-panel`}),});
   { svelteHTML.createElement("div", { "class":`head`,});
     { svelteHTML.createElement("div", { "class":`title`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`shield`,"size":14,}});}
       { svelteHTML.createElement("h4", { "class":`sec`,});  }
      if(activeCount){ { svelteHTML.createElement("span", { "class":`count`,});activeCount;  }}
     }
     { svelteHTML.createElement("button", {         "class":`btn small`,...__sveltets_2_empty({"data-testid":`mcp-auto-approve-new`}),"disabled":!isMcpAdmin || !data,"onclick":() => (formOpen = true),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
     }
   }
   { svelteHTML.createElement("p", { "class":`muted small intro`,});
                       
                   
            { svelteHTML.createElement("em", {});   }    
                   
                  
      
   }

   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`auto-approve rules`,loading,"error":loadError,"empty":!!data && rules.length === 0,"rows":2,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {       "icon":`shield`,"title":`No auto-approve rules`,"body":`Every mutating call asks a person first.`,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("ul", { "class":`rules`,});
         for(let rule of __sveltets_2_ensureArray(rules)){rule.id;
         { svelteHTML.createElement("li", {    "class":`rule`,...__sveltets_2_empty({"data-testid":`mcp-auto-approve-rule-${rule.target}`}),});!rule.enabled;
           { svelteHTML.createElement("label", {   "class":`switch`,"title":rule.enabled ? 'On — runs without asking' : 'Off — asks before each call',});
             { svelteHTML.createElement("input", {           "type":`checkbox`,"checked":rule.enabled,"aria-label":`${rule.name} enabled`,"disabled":!isMcpAdmin || busy === rule.id,"onchange":(event) => void setEnabled(rule, event.currentTarget.checked, event.currentTarget),});}
           }
           { svelteHTML.createElement("span", { "class":`meta`,});
             { svelteHTML.createElement("span", { "class":`name`,});rule.name; }
             { svelteHTML.createElement("span", { "class":`chips`,});
               { svelteHTML.createElement("span", { "class":`aa-chip mono`,});targetLabel(rule); }
               { svelteHTML.createElement("span", { "class":`aa-chip`,});scopeLabel(rule); }
              if(rule.allow_irreversible){ { svelteHTML.createElement("span", { "class":`aa-chip danger`,});  }}
              if(rule.irreversible && !rule.allow_irreversible){ { svelteHTML.createElement("span", { "class":`aa-chip`,});     }}
             }
           }
           { svelteHTML.createElement("button", {           "class":`icon-btn`,"aria-label":`Delete ${rule.name}`,"title":`Delete rule`,"disabled":!isMcpAdmin || busy === rule.id,"onclick":() => void remove(rule),});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":13,}});}
           }
         }
      }
     }
   LoadState}
 }

if(formOpen && data){
   { const $$_mroFevorppAotuA0C = __sveltets_2_ensureComponent(AutoApproveForm); new $$_mroFevorppAotuA0C({ target: __sveltets_2_any(), props: {       "catalog":data.categories,"onclose":() => (formOpen = false),"onsaved":() => {
      void load();
      onchange();
    },}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AutoApprovePanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AutoApprovePanel__SvelteComponent_ = ReturnType<typeof AutoApprovePanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default AutoApprovePanel__SvelteComponent_;