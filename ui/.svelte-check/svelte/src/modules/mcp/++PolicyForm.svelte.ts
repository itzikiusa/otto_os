///<reference types="svelte" />
;
import { pluralNoun } from '../../lib/plural';
// Create / edit one policy-as-code rule. `match` is the matcher object
// (all fields optional, AND-combined): server_id, server_name, tool, tool_glob,
// risk_label, min_injection_risk, mutating, direction, caller_kind,
// workspace_id. Evaluation is most-restrictive-wins; priority only orders
// display and which reason is shown.
import { untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
import { announce } from '../../lib/announce.svelte';
import Modal from '../../lib/components/Modal.svelte';
import { mcpCpApi } from '../../lib/api/mcp';
import { toasts } from '../../lib/toast.svelte';
import type { McpPolicy, McpPolicyEffect } from '../../lib/api/types';
;

  interface Props {
    wsId: string;
    policy: McpPolicy | null;
    onclose: () => void;
    onsaved: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { wsId, policy, onclose, onsaved }: Props = $props();

  // The form edits a snapshot of `policy` (the modal is re-created per open), so
  // capture it once — untracked — to seed the fields without a reactive read.
  const init = untrack(() => policy);

  let name = $state(init?.name ?? '');
  let enabled = $state(init?.enabled ?? true);
  let priority = $state(init?.priority ?? 100);
  let effect = $state<McpPolicyEffect>(init?.effect ?? 'require_approval');
  let reason = $state(init?.reason ?? '');
  let global = $state(init ? init.workspace_id == null : false);
  let matchText = $state(
    init ? JSON.stringify(init.match ?? {}, null, 2) : '{\n  "tool_glob": "*"\n}',
  );
  const matchKeys = [
    'server_id',
    'server_name',
    'tool',
    'tool_glob',
    'risk_label',
    'min_injection_risk',
    'mutating',
    'direction',
    'caller_kind',
    'workspace_id',
  ] as const;
  const matchKeysText = matchKeys.join(', ');
  let matchError = $state<string | null>(null);
  /** Inline validation, next to the field (never a toast). */
  let nameError = $state<string | null>(null);
  let saving = $state(false);

  /** Field errors are plain text tied to their field (`aria-describedby`);
   *  the failed save is spoken once, at the edge (a role="alert" inserted
   *  with its text is silent in VoiceOver). */
  async function save(): Promise<void> {
    await validateThenSave();
    const error = nameError ?? matchError;
    if (error) announce(error, { assertive: true });
  }

  async function validateThenSave(): Promise<void> {
    if (!name.trim()) {
      nameError = 'Give the policy a name.';
      return;
    }
    let match: unknown = {};
    matchError = null;
    if (matchText.trim()) {
      try {
        match = JSON.parse(matchText);
      } catch (e) {
        matchError = `Match must be valid JSON: ${loadErrorText(e)}`;
        return;
      }
    }
    if (match === null || Array.isArray(match) || typeof match !== 'object') {
      matchError = `Match must be a JSON object. Allowed keys: ${matchKeysText}.`;
      return;
    }
    const unknownKeys = Object.keys(match).filter(
      (key) => !matchKeys.includes(key as (typeof matchKeys)[number]),
    );
    if (unknownKeys.length) {
      matchError = `Unknown ${pluralNoun(unknownKeys.length, 'match key')}: ${unknownKeys.join(', ')}. Allowed keys: ${matchKeysText}.`;
      return;
    }
    saving = true;
    try {
      if (policy) {
        await mcpCpApi.cpUpdatePolicy(policy.id, {
          name: name.trim(),
          enabled,
          priority,
          match,
          effect,
          reason: reason.trim() || null,
        });
      } else {
        await mcpCpApi.cpCreatePolicy({
          workspace_id: global ? null : wsId,
          name: name.trim(),
          enabled,
          priority,
          match,
          effect,
          reason: reason.trim() || null,
        });
      }
      toasts.success(policy ? 'Policy updated' : 'Policy created', name.trim());
      onsaved();
      onclose();
    } catch (e) {
      toastError('Couldn’t save the policy', e);
    } finally {
      saving = false;
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":policy ? 'Edit policy' : 'New policy',"width":560,onclose,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {     "class":`btn`,"onclick":onclose,"disabled":saving,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":() => void save(),"disabled":saving,});
      saving ? 'Saving…' : policy ? 'Save' : 'Create';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`form`,});
     { svelteHTML.createElement("label", { "class":`field`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("input", {            "dir":`auto`,"bind:value":name,"placeholder":`Approve all dangerous writes`,"aria-invalid":nameError != null,"aria-describedby":nameError ? 'mcp-policy-name-err' : undefined,"oninput":() => (nameError = null),});/*Ωignore_startΩ*/() => name = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(nameError){ { svelteHTML.createElement("span", {   "id":`mcp-policy-name-err`,"class":`field-error`,});nameError; }}
     }

     { svelteHTML.createElement("div", { "class":`row3`,});
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("select", { "bind:value":effect,});/*Ωignore_startΩ*/() => effect = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`allow`,});  }
           { svelteHTML.createElement("option", { "value":`deny`,});  }
           { svelteHTML.createElement("option", { "value":`require_approval`,});  }
           { svelteHTML.createElement("option", { "value":`require_dry_run`,});   }
         }
       }
       { svelteHTML.createElement("label", { "class":`field`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("input", {    "type":`number`,"bind:value":priority,});/*Ωignore_startΩ*/() => priority = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("label", { "class":`check tall`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":enabled,});/*Ωignore_startΩ*/() => enabled = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", {});  }
       }
     }

    if(!policy){
       { svelteHTML.createElement("label", { "class":`check`,});
         { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":global,});/*Ωignore_startΩ*/() => global = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("span", {});      }
       }
    }

     { svelteHTML.createElement("label", { "class":`field`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("textarea", {                "dir":`ltr`,"bind:value":matchText,"rows":7,"class":`mono`,"spellcheck":`false`,"aria-invalid":matchError != null,"aria-describedby":matchError ? 'mcp-policy-match-err' : undefined,"oninput":() => (matchError = null),});/*Ωignore_startΩ*/() => matchText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       { svelteHTML.createElement("span", { "class":`hint`,});
            matchKeysText;
       }
      if(matchError){ { svelteHTML.createElement("span", {   "id":`mcp-policy-match-err`,"class":`field-error`,});matchError; }}
     }

     { svelteHTML.createElement("label", { "class":`field`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("input", {      "dir":`auto`,"bind:value":reason,"placeholder":`Shown when this rule decides (optional)`,});/*Ωignore_startΩ*/() => reason = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PolicyForm__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PolicyForm__SvelteComponent_ = ReturnType<typeof PolicyForm__SvelteComponent_>;
/*Ωignore_endΩ*/export default PolicyForm__SvelteComponent_;