///<reference types="svelte" />
;
import { confirmer } from '../confirm.svelte';
import { announce } from '../announce.svelte';
import Skeleton from './Skeleton.svelte';
import { toasts } from '../toast.svelte';
import { untrack } from 'svelte';
import { accessApi } from '../api/access';
import { api } from '../api/client';
import { ws } from '../stores/workspace.svelte';
import { auth } from '../stores/auth.svelte';
import { resourceAccess } from '../stores/resource-access.svelte';
import { accessOperations, operationLabel, scopeHint } from '../access-options';
import type {
    ResourceKind,
    AccessPolicy,
    AccessSubjects,
    AccessRule,
    AccessPreview,
    EffectiveAccess,
    Connection,
  } from '../api/types';

;type $$ComponentProps =  { kind: ResourceKind; resourceId: string };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  let { kind, resourceId }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let policy = $state<AccessPolicy | null>(null);
  let original = $state('');
  let subjects = $state<AccessSubjects>({ users: [], groups: [], roles: [] });
  let credentials = $state<Connection[]>([]);
  let preview = $state<AccessPreview | null>(null);
  let previewSnapshot = $state('');
  let error = $state('');
  let notice = $state('');
  let busy = $state(false);
  let loading = $state(true);
  let tab = $state<'rules' | 'effective'>('rules');
  let userId = $state('');
  let child = $state('');
  let effective = $state<EffectiveAccess | null>(null);
  let generation = 0;
  let effectiveGeneration = 0;
  const dirty = $derived(policy !== null && JSON.stringify(policy) !== original);
  const previewCurrent = $derived(preview !== null && JSON.stringify(policy) === previewSnapshot);
  async function load(k: ResourceKind, id: string) {
    const g = ++generation;
    loading = true;
    policy = null;
    error = '';
    preview = null;
    effective = null;
    subjects = { users: [], groups: [], roles: [] };
    credentials = [];
    try {
      const [p, s] = await Promise.all([accessApi.policy(k, id), accessApi.subjects(k, id)]);
      if (g !== generation) return;
      policy = p;
      original = JSON.stringify(p);
      subjects = s;
      userId = s.users[0]?.id ?? '';
      if (k === 'connection' && auth.isRoot) {
        try {
          const profiles = await api.get<Connection[]>(`/workspaces/${ws.currentId}/connections`);
          if (g === generation && auth.isRoot) credentials = profiles;
        } catch {
          credentials = [];
        }
      }
    } catch (e) {
      if (g === generation) error = String(e);
    } finally {
      if (g === generation) loading = false;
    }
  }
  $effect(() => {
    const k = kind,
      id = resourceId;
    untrack(() => {
      void load(k, id);
    });
    return () => {
      generation++;
      effectiveGeneration++;
    };
  });
  async function reload() {
    if (
      dirty &&
      !(await confirmer.ask('Discard unsaved access edits and reload the saved policy?', {
        title: 'Reload access',
        confirmLabel: 'Reload saved access',
      }))
    )
      return;
    await load(kind, resourceId);
  }
  $effect(() => {
    const k = kind, id = resourceId;
    return resourceAccess.subscribe((change) => {
      if (change.type === 'reset' && change.identity) void load(k, id);
    });
  });

  function addRule() {
    if (!policy) return;
    policy.rules.push({
      id: crypto.randomUUID(),
      subject_kind: 'group',
      subject_id: subjects.groups[0]?.id ?? '',
      effect: 'allow',
      operations: [],
      children: null,
      grantable_operations: [],
      credential_connection_id: null,
    });
  }
  function toggle(
    rule: AccessRule,
    field: 'operations' | 'grantable_operations',
    op: string,
    checked: boolean,
  ) {
    rule[field] = checked ? [...rule[field], op] : rule[field].filter((x) => x !== op);
  }
  function applyRole(rule: AccessRule, roleId: string) {
    const role = subjects.roles.find((r) => r.id === roleId);
    if (!role) return;
    rule.operations = [...role.operations];
    rule.grantable_operations = rule.effect === 'allow' ? [...role.grantable_operations] : [];
  }
  async function review() {
    if (!policy) return;
    busy = true;
    error = '';
    notice = '';
    preview = null;
    const snapshot = JSON.stringify(policy);
    try {
      const result = await accessApi.preview(JSON.parse(snapshot));
      if (snapshot === JSON.stringify(policy)) {
        preview = result;
        previewSnapshot = snapshot;
      }
    } catch (e) {
      error = String(e);
    } finally {
      busy = false;
    }
  }
  async function save() {
    if (!policy || !previewCurrent || preview?.issues.length) return;
    busy = true;
    error = '';
    try {
      const saved = await accessApi.save(JSON.parse(JSON.stringify(policy)), preview?.token);
      policy = saved;
      original = JSON.stringify(saved);
      preview = null;
      notice = 'Access saved.';
      announce(notice);
      toasts.success('Access saved');
      resourceAccess.invalidate();
    } catch (e) {
      error = String(e);
      preview = null;
    } finally {
      busy = false;
    }
  }
  async function inspect() {
    const g = ++effectiveGeneration;
    effective = null;
    error = '';
    if (!userId) return;
    try {
      const result = await accessApi.effective(kind, resourceId, userId, child.trim() || undefined);
      if (g === effectiveGeneration) effective = result;
    } catch (e) {
      if (g === effectiveGeneration) error = String(e);
    }
  }
  function changedOperations(
    before: EffectiveAccess['operations'],
    after: EffectiveAccess['operations'],
  ) {
    return Object.keys({ ...before, ...after }).filter(
      (op) => before[op]?.allowed !== after[op]?.allowed,
    );
  }
;
async () => {

 { svelteHTML.createElement("section", {   "class":`access-editor`,"aria-label":`Resource access`,});
   { svelteHTML.createElement("header", {});
     { svelteHTML.createElement("div", {});
       { svelteHTML.createElement("h3", {});  }
       { svelteHTML.createElement("p", {});           }
     }
     { svelteHTML.createElement("button", {      "class":`btn small`,"disabled":busy || loading,"onclick":reload,});   }
   }
  if(error){ { svelteHTML.createElement("p", {   "class":`error`,"role":`alert`,});error; }}
  
  if(notice){ { svelteHTML.createElement("p", {});notice; }}
  if(loading){ { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {      "rows":2,"height":28,"label":`access`,}});}
  } else if (policy){
     { svelteHTML.createElement("nav", { "aria-label":`Access views`,});
       { svelteHTML.createElement("button", {     "class":`btn`,"onclick":() => (tab = 'rules'),});tab === 'rules';  } { svelteHTML.createElement("button", {     "class":`btn`,"onclick":() => {
          tab = 'effective';
          void inspect();
        },});tab === 'effective';  }
     }
    if(tab === 'rules'){
       { svelteHTML.createElement("label", {  "class":`mode`,});  { svelteHTML.createElement("select", {       "aria-label":`Access mode`,"bind:value":policy.mode,"disabled":!auth.isRoot || busy,});/*Ωignore_startΩ*/() => policy.mode = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", { "value":`legacy`,});   } { svelteHTML.createElement("option", {  "value":`enforced`,});   } } }
       { svelteHTML.createElement("p", { "class":`hint`,});
        policy.mode === 'legacy'
          ? 'Legacy mode uses existing feature permissions. Configure rules and review the comparison before activation.'
          : 'Only matching grants allow access. Root is the owner exception. Feature, workspace, token and native permissions still apply.';
       }
       { svelteHTML.createElement("fieldset", { "disabled":busy,});
            for(let rule of __sveltets_2_ensureArray(policy.rules)){let i = 1;rule.id;
           { svelteHTML.createElement("article", {   "class":`rule`,"aria-label":`Rule ${i + 1}`,});
             { svelteHTML.createElement("div", { "class":`rule-head`,});
               { svelteHTML.createElement("strong", {}); i + 1; } { svelteHTML.createElement("button", {    "class":`btn small`,"onclick":() => {
                  if (policy) policy.rules = policy.rules.filter((r) => r.id !== rule.id);
                },});  }
             }
             { svelteHTML.createElement("div", { "class":`fields`,});
               { svelteHTML.createElement("label", { });  { svelteHTML.createElement("select", {       "aria-label":`Subject type`,"bind:value":rule.subject_kind,"onchange":() => (rule.subject_id = ''),});/*Ωignore_startΩ*/() => rule.subject_kind = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", { "value":`group`,});  } { svelteHTML.createElement("option", { "value":`user`,});  } } }
               { svelteHTML.createElement("label", { });  { svelteHTML.createElement("select", {    "aria-label":`Subject`,"bind:value":rule.subject_id,});/*Ωignore_startΩ*/() => rule.subject_id = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", {"value":"",}); rule.subject_kind; }if(rule.subject_kind === 'group'){  for(let g of __sveltets_2_ensureArray(subjects.groups)){ { svelteHTML.createElement("option", {  "value":g.id,});g.name; }}}else{  for(let u of __sveltets_2_ensureArray(subjects.users)){ { svelteHTML.createElement("option", {  "value":u.id,});u.display_name; u.username;  }}} } }
               { svelteHTML.createElement("label", { });  { svelteHTML.createElement("select", {      "aria-label":`Effect`,"bind:value":rule.effect,"onchange":() => {
                    if (rule.effect === 'deny') {
                      rule.grantable_operations = [];
                      rule.credential_connection_id = null;
                    }
                  },});/*Ωignore_startΩ*/() => rule.effect = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", { "value":`allow`,});  } { svelteHTML.createElement("option", { "value":`deny`,});  } } }
             }
             { svelteHTML.createElement("label", { });  { svelteHTML.createElement("select", {       "aria-label":`Scope`,"value":rule.children === null ? 'all' : 'selected',"onchange":(e) => (rule.children = e.currentTarget.value === 'all' ? null : []),}); { svelteHTML.createElement("option", { "value":`all`,});     } { svelteHTML.createElement("option", {  "value":`selected`,});   } } }
            if(rule.children !== null){ { svelteHTML.createElement("label", { });  { svelteHTML.createElement("textarea", {         "dir":`auto`,"rows":3,"value":rule.children.join('\n'),"onchange":(e) =>
                    (rule.children = [
                      ...new Set(
                        e.currentTarget.value
                          .split('\n')
                          .map((x) => x.trim())
                          .filter(Boolean),
                      ),
                    ]),"placeholder":scopeHint[kind],}); } }
               { svelteHTML.createElement("p", { "class":`hint`,});scopeHint[kind];     }}
             { svelteHTML.createElement("label", { });   { svelteHTML.createElement("select", {      "aria-label":`Copy role preset`,"value":"","onchange":(e) => {
                  applyRole(rule, e.currentTarget.value);
                  e.currentTarget.value = '';
                },}); { svelteHTML.createElement("option", {"value":"",});  }  for(let role of __sveltets_2_ensureArray(subjects.roles.filter((r) => r.kind === kind))){ { svelteHTML.createElement("option", {  "value":role.id,});role.name; }} } }
             { svelteHTML.createElement("p", { "class":`hint`,});
                          
                       
             }
             { svelteHTML.createElement("div", { "class":`operations`,});
                for(let op of __sveltets_2_ensureArray(accessOperations[kind])){ { svelteHTML.createElement("label", { }); { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":rule.operations.includes(op),"onchange":(e) => toggle(rule, 'operations', op, e.currentTarget.checked),});}operationLabel(op); }}
             }
            if(rule.effect === 'allow'){
               { svelteHTML.createElement("details", {});
                 { svelteHTML.createElement("summary", {});  }
                 { svelteHTML.createElement("p", { "class":`hint`,});
                              
                      
                 }
                 { svelteHTML.createElement("div", { "class":`operations`,});
                    for(let op of __sveltets_2_ensureArray(accessOperations[kind])){ { svelteHTML.createElement("label", { }); { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":rule.grantable_operations.includes(op),"onchange":(e) =>
                          toggle(rule, 'grantable_operations', op, e.currentTarget.checked),});}operationLabel(op); }}
                 }
               }
              if(kind === 'connection'){ { svelteHTML.createElement("label", { });   { svelteHTML.createElement("select", {       "aria-label":`Native credential profile`,"bind:value":rule.credential_connection_id,"disabled":!auth.isRoot,});/*Ωignore_startΩ*/() => rule.credential_connection_id = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", { "value":null,});   }  for(let c of __sveltets_2_ensureArray(credentials.filter((c) => c.id !== resourceId))){ { svelteHTML.createElement("option", {  "value":c.id,});c.name; }}if(rule.credential_connection_id && !credentials.some((c) => c.id === rule.credential_connection_id)){ { svelteHTML.createElement("option", {  "value":rule.credential_connection_id,});  }} } }
                 { svelteHTML.createElement("p", { "class":`hint`,});
                             
                      
                 }}
            }
           }
        }
        if(policy.rules.length === 0){ { svelteHTML.createElement("p", {});
                      
           }}
         { svelteHTML.createElement("button", {   "class":`btn`,"onclick":addRule,});  }
       }
       { svelteHTML.createElement("div", { "class":`actions`,});
         { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy || !dirty,"onclick":review,});  } { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy || !dirty || !previewCurrent || !!preview?.issues.length,"onclick":save,});policy.mode === 'enforced' && JSON.parse(original).mode === 'legacy'
            ? 'Activate enforced access'
            : 'Save access'; }
       }
      if(previewCurrent && preview){ { svelteHTML.createElement("section", {   "class":`preview`,"aria-label":`Access comparison`,});
           { svelteHTML.createElement("h4", {});   }
            for(let issue of __sveltets_2_ensureArray(preview.issues)){ { svelteHTML.createElement("p", { "class":`error`,});issue; }}
           { svelteHTML.createElement("p", { "class":`hint`,});
                       
           }
            for(let change of __sveltets_2_ensureArray(preview.changes)){const ops = changedOperations(
              change.before,
              change.after,
            );if(ops.length){ { svelteHTML.createElement("div", {});
                 { svelteHTML.createElement("strong", {});change.display_name; }
                 { svelteHTML.createElement("ul", {});
                    for(let op of __sveltets_2_ensureArray(ops)){ { svelteHTML.createElement("li", {});
                      operationLabel(op); change.before[op]?.allowed ? 'Allowed' : 'Denied';  change
                        .after[op]?.allowed
                        ? 'Allowed'
                        : 'Denied';
                     }}
                 }
               }}  for(let scope of __sveltets_2_ensureArray(change.children ?? [])){const childOps = changedOperations(
                scope.before,
                scope.after,
              );if(childOps.length){ { svelteHTML.createElement("div", {});
                   { svelteHTML.createElement("strong", {});change.display_name;  scope.child; }
                   { svelteHTML.createElement("ul", {});
                      for(let op of __sveltets_2_ensureArray(childOps)){ { svelteHTML.createElement("li", {});
                        operationLabel(op); scope.before[op]?.allowed ? 'Allowed' : 'Denied';  scope
                          .after[op]?.allowed
                          ? 'Allowed'
                          : 'Denied';
                       }}
                   }
                 }}}}if(preview.changes.every((c) => changedOperations(c.before, c.after).length === 0 && (c.children ?? []).every((s) => changedOperations(s.before, s.after).length === 0))){ { svelteHTML.createElement("p", { });
                    
             }}
         }}
    }else{
       { svelteHTML.createElement("div", { "class":`fields`,});
         { svelteHTML.createElement("label", { });  { svelteHTML.createElement("select", {       "aria-label":`User`,"bind:value":userId,"onchange":() => {
              effective = null;
            },});/*Ωignore_startΩ*/() => userId = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", {"value":"",});  }  for(let u of __sveltets_2_ensureArray(subjects.users)){ { svelteHTML.createElement("option", {  "value":u.id,});u.display_name; u.username;  }} } } { svelteHTML.createElement("label", { });  { svelteHTML.createElement("input", {        "dir":`auto`,"bind:value":child,"oninput":() => {
              effective = null;
            },"placeholder":scopeHint[kind],});/*Ωignore_startΩ*/() => child = __sveltets_2_any(null);/*Ωignore_endΩ*/} } { svelteHTML.createElement("button", {     "class":`btn`,"disabled":!userId,"onclick":inspect,});  }
       }
       { svelteHTML.createElement("p", { "class":`hint`,});
                    
           
       }
      if(effective){ { svelteHTML.createElement("div", { "class":`decisions`,});
            for(let [op, decision] of __sveltets_2_ensureArray(Object.entries(effective.operations))){ { svelteHTML.createElement("div", { "class":`decision`,});
               { svelteHTML.createElement("strong", {});operationLabel(op); } { svelteHTML.createElement("span", {  });decision.allowed;decision.allowed ? 'Allowed' : 'Denied'; }
               { svelteHTML.createElement("p", {});decision.reason; }
              if(decision.matched_rule_ids.length){ { svelteHTML.createElement("small", { });  decision.matched_rule_ids
                    .map((id) => {
                      const index = policy?.rules.findIndex((r) => r.id === id) ?? -1;
                      return index < 0 ? id : `Rule ${index + 1}`;
                    })
                    .join(', '); }}
             }}
         }}
    }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ResourceAccess__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ResourceAccess__SvelteComponent_ = ReturnType<typeof ResourceAccess__SvelteComponent_>;
/*Ωignore_endΩ*/export default ResourceAccess__SvelteComponent_;