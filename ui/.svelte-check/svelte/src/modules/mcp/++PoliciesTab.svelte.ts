///<reference types="svelte" />
;
import { latestOnly } from '../../lib/latest';
import { toastError } from '../../lib/toastError';
// Policy-as-code rules (global + this workspace). List / create / edit /
// delete, export the whole ruleset to JSON, import a ruleset (append or
// replace), and an Evaluate preview that shows the decision a (server, tool)
// would get under the current rules.
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { plural } from '../../lib/plural';
import { loadErrorText } from '../../lib/loadError';
import { mcpCpApi } from '../../lib/api/mcp';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import type {
    CreateMcpPolicyReq,
    McpEvaluatePreview,
    McpPolicy,
    McpServerDetail,
  } from '../../lib/api/types';
import McpPill from './McpPill.svelte';
import PolicyForm from './PolicyForm.svelte';
;

  interface Props {
    wsId: string;
    servers: McpServerDetail[];
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { wsId, servers }: Props = $props();

  let policies = $state<McpPolicy[]>([]);
  let loading = $state(false);
  /** Failed load — inline with Retry, never the empty state. */
  let loadError = $state<string | null>(null);
  let editing = $state<McpPolicy | null>(null);
  let formOpen = $state(false);

  // A workspace switch mid-load must not show (or let you edit) the previous
  // workspace's policies under the new one.
  const loads = latestOnly();
  async function load(): Promise<void> {
    const t = loads.begin();
    const id = wsId;
    loading = true;
    try {
      const rows = await mcpCpApi.cpPolicies(id);
      if (!t.current) return;
      policies = rows;
      loadError = null;
    } catch (e) {
      if (!t.current) return;
      loadError = loadErrorText(e);
    } finally {
      if (t.current) loading = false;
    }
  }

  $effect(() => {
    void wsId;
    void load();
  });

  function openNew(): void {
    editing = null;
    formOpen = true;
  }
  function openEdit(p: McpPolicy): void {
    editing = p;
    formOpen = true;
  }

  async function remove(p: McpPolicy): Promise<void> {
    if (!(await confirmer.ask(`Delete policy "${p.name}"?`, { title: 'Delete policy', danger: true, confirmLabel: 'Delete' })))
      return;
    try {
      await mcpCpApi.cpDeletePolicy(p.id);
      toasts.success('Policy deleted', p.name);
      await load();
    } catch (e) {
      toastError('Couldn’t delete', e);
    }
  }

  async function exportJson(): Promise<void> {
    try {
      const doc = await mcpCpApi.cpExportPolicies();
      const blob = new Blob([JSON.stringify(doc, null, 2)], { type: 'application/json' });
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = `mcp-policies-${new Date().toISOString().slice(0, 10)}.json`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (e) {
      toastError('Couldn’t export', e);
    }
  }

  // ---- import ----
  let importOpen = $state(false);
  let importText = $state('');
  let importReplace = $state(false);
  let importing = $state(false);

  async function doImport(): Promise<void> {
    let parsed: unknown;
    try {
      parsed = JSON.parse(importText);
    } catch (e) {
      toastError('Invalid JSON', e);
      return;
    }
    // Accept either the exported {version, policies} doc or a bare policies array.
    const list = Array.isArray(parsed)
      ? parsed
      : ((parsed as { policies?: unknown }).policies ?? []);
    if (!Array.isArray(list)) {
      toasts.error('Expected a policies array or an exported {version, policies} document');
      return;
    }
    if (importing) return;
    const replace = importReplace;
    importing = true;
    try {
      if (replace) {
        // Replacement is instance-wide, even if this workspace's list is empty.
        const affected = await mcpCpApi.cpPolicies();
        const workspaces = new Set(affected.flatMap((rule) => rule.workspace_id ? [rule.workspace_id] : []));
        const globals = affected.filter((rule) => !rule.workspace_id).length;
        if (!(await confirmer.ask(
          `Replace all ${affected.length} existing rules across every workspace with ${list.length} imported rules? This deletes ${globals} global rules and the rules in ${workspaces.size} workspaces.`,
          { title: 'Replace policy rules everywhere', danger: true, confirmLabel: 'Replace all rules' },
        ))) return;
      }
      const res = await mcpCpApi.cpImportPolicies({
        policies: list as CreateMcpPolicyReq[],
        replace,
      });
      toasts.success(
        'Policies imported',
        `${res.imported} imported${res.replaced ? ' (replaced existing)' : ''}`,
      );
      importOpen = false;
      importText = '';
      await load();
    } catch (e) {
      toastError('Couldn’t import', e);
    } finally {
      importing = false;
    }
  }

  // ---- evaluate preview ----
  let evalServerId = $state('');
  let evalTool = $state('');
  let evalResult = $state<McpEvaluatePreview | null>(null);
  let evaluating = $state(false);

  $effect(() => {
    if (!evalServerId && servers.length) evalServerId = servers[0].id;
  });

  async function evaluate(): Promise<void> {
    // The button is disabled until both are set (components.md: a field
    // error is inline, never a toast) — this is only a guard.
    if (!evalServerId || !evalTool.trim()) return;
    evaluating = true;
    evalResult = null;
    try {
      evalResult = await mcpCpApi.cpEvaluate({
        server_id: evalServerId,
        tool: evalTool.trim(),
        workspace_id: wsId,
      });
    } catch (e) {
      toastError('Couldn’t evaluate', e);
    } finally {
      evaluating = false;
    }
  }

  function matchSummary(m: unknown): string {
    if (!m || typeof m !== 'object') return 'any';
    const entries = Object.entries(m as Record<string, unknown>);
    if (entries.length === 0) return 'any';
    return entries.map(([k, v]) => `${k}=${typeof v === 'object' ? JSON.stringify(v) : String(v)}`).join('  ');
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`pol`,});
   { svelteHTML.createElement("div", { "class":`bar`,});
     { svelteHTML.createElement("span", { "class":`count`,});plural(policies.length, 'rule'); }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void exportJson(),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":13,}});}  }
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (importOpen = !importOpen),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":13,}});}  }
     { svelteHTML.createElement("button", {   "class":`btn primary small`,"onclick":openNew,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}   }
   }

  if(importOpen){
     { svelteHTML.createElement("div", { "class":`import-panel`,});
       { svelteHTML.createElement("textarea", {              "dir":`ltr`,"aria-label":`Policies JSON to import`,"bind:value":importText,"rows":5,"class":`mono`,"spellcheck":`false`,"placeholder":'Paste an exported {"version":1,"policies":[…]} document or a bare [ … ] array',});/*Ωignore_startΩ*/() => importText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       { svelteHTML.createElement("div", { "class":`import-actions`,});
         { svelteHTML.createElement("label", { "class":`check`,});
           { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":importReplace,});/*Ωignore_startΩ*/() => importReplace = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", {});    }
         }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (importOpen = false),});  }
         { svelteHTML.createElement("button", {     "class":`btn primary small`,"onclick":() => void doImport(),"disabled":importing,});
          importing ? 'Importing…' : 'Import';
         }
       }
     }
  }

  
   { svelteHTML.createElement("div", { "class":`evaluate`,});
     { svelteHTML.createElement("div", { "class":`eval-row`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`gauge`,"size":14,}});}
       { svelteHTML.createElement("span", { "class":`el`,});  }
       { svelteHTML.createElement("select", {   "aria-label":`Server to evaluate`,"bind:value":evalServerId,});/*Ωignore_startΩ*/() => evalServerId = __sveltets_2_any(null);/*Ωignore_endΩ*/
        if(servers.length === 0){ { svelteHTML.createElement("option", {"value":"",});  }}
           for(let s of __sveltets_2_ensureArray(servers)){s.id; { svelteHTML.createElement("option", { "value":s.id,});s.name; }}
       }
       { svelteHTML.createElement("input", {          "dir":`ltr`,"aria-label":`Tool name to evaluate`,"bind:value":evalTool,"placeholder":`tool name`,"class":`mono`,});/*Ωignore_startΩ*/() => evalTool = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":() => void evaluate(),"disabled":evaluating || servers.length === 0 || !evalServerId || !evalTool.trim(),"title":!evalTool.trim() ? 'Enter a tool name to preview its decision' : undefined,});
        evaluating ? '…' : 'Preview decision';
       }
      if(evalResult){
         { const $$_lliPpcM3C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM3C({ target: __sveltets_2_any(), props: {    "kind":`decision`,"value":evalResult.policy_decision === 'allow' ? 'allowed' : evalResult.policy_decision,}});}
          { const $$_lliPpcM3C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM3C({ target: __sveltets_2_any(), props: {    "kind":`risk`,"value":evalResult.risk_label,"small":true,}});}
          { const $$_lliPpcM3C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM3C({ target: __sveltets_2_any(), props: {    "kind":`injection`,"value":evalResult.injection_risk,"small":true,}});}
        if(evalResult.reason){ { svelteHTML.createElement("span", { "class":`reason`,});evalResult.reason; }}
      }
     }
   }

   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        children:() => { return __sveltets_2_any(0); },"what":`policies`,loading,"error":loadError,"empty":policies.length === 0,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {          "icon":`split`,"title":`No policy rules yet`,"body":`Without rules, calls fall through to allowlists and per-tool permission.`,"actionLabel":`Create a rule`,"onaction":openNew,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("div", { "class":`grid`,});
       { svelteHTML.createElement("div", { "class":`thead`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", { "class":`num`,});  }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {}); }
       }
         for(let p of __sveltets_2_ensureArray(policies)){p.id;
         { svelteHTML.createElement("div", { "class":`prow`,});
           { svelteHTML.createElement("div", { "class":`pname`,});
             { svelteHTML.createElement("span", { "class":`nm`,});p.name; }
            if(p.reason){ { svelteHTML.createElement("span", {   "class":`desc`,"title":p.reason,});p.reason; }}
           }
          
           { svelteHTML.createElement("span", { "class":`cell`,}); { svelteHTML.createElement("span", { "class":`cl`,});  } { svelteHTML.createElement("span", { "class":`scope`,});p.workspace_id == null ? 'Global' : 'Workspace'; } }
           { svelteHTML.createElement("span", { "class":`cell num`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }p.priority; }
           { svelteHTML.createElement("span", { "class":`cell`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }  { const $$_lliPpcM5C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM5C({ target: __sveltets_2_any(), props: {    "kind":`decision`,"value":p.effect === 'allow' ? 'allowed' : p.effect === 'deny' ? 'denied' : p.effect === 'require_approval' ? 'pending_approval' : 'dry_run',"small":true,}});} }
           { svelteHTML.createElement("span", { "class":`cell mono match-cell`,}); { svelteHTML.createElement("span", { "class":`cl`,});  } { svelteHTML.createElement("span", {   "class":`match`,"title":matchSummary(p.match),});matchSummary(p.match); } }
           { svelteHTML.createElement("span", { "class":`cell`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }if(p.enabled){ { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":14,}});} { svelteHTML.createElement("span", { "class":`sr-only`,});  }}else{ { svelteHTML.createElement("span", { "class":`off`,});  }} }
           { svelteHTML.createElement("span", { "class":`cell actions`,});
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => openEdit(p),});  }
             { svelteHTML.createElement("button", {   "class":`btn small danger`,"onclick":() => void remove(p),});  }
           }
         }
      }
     }
   LoadState}
 }

if(formOpen){
   { const $$_mroFyciloP0C = __sveltets_2_ensureComponent(PolicyForm); new $$_mroFyciloP0C({ target: __sveltets_2_any(), props: {        wsId,"policy":editing,"onclose":() => (formOpen = false),"onsaved":() => void load(),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PoliciesTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PoliciesTab__SvelteComponent_ = ReturnType<typeof PoliciesTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default PoliciesTab__SvelteComponent_;