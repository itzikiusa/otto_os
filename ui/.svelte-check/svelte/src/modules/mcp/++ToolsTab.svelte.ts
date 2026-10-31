///<reference types="svelte" />
;
import Badge from '../../lib/components/Badge.svelte';
// Per-server discovered tools with their governance controls (enable,
// require-approval, risk-label override) plus a governed tool tester: run a
// tool with JSON arguments through the full invoke pipeline (optionally
// dry-run) and see the decision + preview / content / pending-approval id.
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import { toastError } from '../../lib/toastError';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Switch from '../../lib/components/Switch.svelte';
import { loadErrorText } from '../../lib/loadError';
import { mcpCpApi } from '../../lib/api/mcp';
import { toasts } from '../../lib/toast.svelte';
import type {
    McpInvokeResp,
    McpRiskLabel,
    McpServerDetail,
    McpToolView,
  } from '../../lib/api/types';
import McpPill from './McpPill.svelte';
;

  interface Props {
    wsId: string;
    servers: McpServerDetail[];
    selectedServerId: string | null;
    onSelect: (id: string) => void;
    embedded?: boolean;
    /** After a re-discovery, so the parent's per-server tool count isn't stale. */
    ondiscovered?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { wsId, servers, selectedServerId, onSelect, embedded = false, ondiscovered }: Props = $props();
  const NO_CONFIGURE = 'You need configure access to this server';

  const can = (operation:string, child?:string) => !!selectedServerId && resourceAccess.can('mcp_server',selectedServerId,operation,'mcp',operation==='invoke'?'edit':'admin',child);
  let tools = $state<McpToolView[]>([]);
  let loading = $state(false);
  /** Failed load — inline with Retry, never the empty state. */
  let loadError = $state<string | null>(null);
  let busyTool = $state<Record<string, boolean>>({});
  $effect(() => {
    const serverId = selectedServerId;
    if (!serverId) return;
    if (!embedded) void resourceAccess.load('mcp_server', serverId);
    return resourceAccess.retainChildren('mcp_server', serverId, tools.map((tool) => tool.name));
  });

  const server = $derived(servers.find((s) => s.id === selectedServerId) ?? null);

  let toolsGeneration = 0;
  async function loadTools(): Promise<void> {
    const generation = ++toolsGeneration;
    const id = selectedServerId;
    if (!id) {
      tools = [];
      return;
    }
    loading = true;
    tools = [];
    try {
      const rows = await mcpCpApi.cpTools(id);
      if (generation !== toolsGeneration || id !== selectedServerId) return;
      tools = rows;
      loadError = null;
    } catch (e) {
      if (generation === toolsGeneration && id === selectedServerId) loadError = loadErrorText(e);
    } finally {
      if (generation === toolsGeneration && id === selectedServerId) loading = false;
    }
  }

  $effect(() => {
    void selectedServerId;
    void loadTools();
  });

  function replaceTool(t: McpToolView): void {
    tools = tools.map((x) => (x.id === t.id ? t : x));
  }

  async function patchTool(
    t: McpToolView,
    patch: { enabled?: boolean; require_approval?: boolean; risk_label?: McpRiskLabel },
  ): Promise<void> {
    busyTool = { ...busyTool, [t.id]: true };
    try {
      const updated = await mcpCpApi.cpPatchTool(t.id, patch);
      replaceTool(updated);
    } catch (e) {
      toastError('Couldn’t update tool', e);
    } finally {
      const n = { ...busyTool };
      delete n[t.id];
      busyTool = n;
    }
  }

  async function discover(): Promise<void> {
    const id = selectedServerId;
    if (!id) return;
    loading = true;
    try {
      const found = await mcpCpApi.cpDiscover(id);
      // Switched servers meanwhile: never list server A's tools under B.
      if (selectedServerId !== id) return;
      tools = found;
      toasts.success('Discovered tools', `${found.length} found`);
      ondiscovered?.();
    } catch (e) {
      if (selectedServerId === id) toastError('Couldn’t discover tools', e);
    } finally {
      if (selectedServerId === id) loading = false;
    }
  }

  // ---- tester ----
  let testToolName = $state('');
  let argsText = $state('{}');
  let dryRun = $state(true);
  let running = $state(false);
  let result = $state<McpInvokeResp | null>(null);
  let argError = $state<string | null>(null);
  let testerOpen = $state(false);

  // Default the tester to the first tool whenever the tool set changes.
  $effect(() => {
    if (tools.length && !tools.some((t) => t.name === testToolName)) {
      testToolName = tools[0].name;
    }
  });

  function pretty(v: unknown): string {
    try {
      return JSON.stringify(v, null, 2);
    } catch {
      return String(v);
    }
  }

  async function run(): Promise<void> {
    const id = selectedServerId;
    if (!id || !testToolName || !can('invoke',testToolName)) return;
    let args: unknown = {};
    argError = null;
    if (argsText.trim()) {
      try {
        args = JSON.parse(argsText);
      } catch (e) {
        argError = `Arguments must be valid JSON: ${loadErrorText(e)}`;
        return;
      }
    }
    running = true;
    result = null;
    try {
      result = await mcpCpApi.cpInvoke(id, testToolName, {
        arguments: args,
        dry_run: dryRun,
        workspace_id: wsId,
      });
    } catch (e) {
      toastError('Couldn’t invoke the tool', e);
    } finally {
      running = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", {  "class":`tools`,});embedded;
   { svelteHTML.createElement("div", { "class":`bar`,});
    if(!embedded){
       { svelteHTML.createElement("label", { "class":`picker`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("select", {     "value":selectedServerId ?? '',"onchange":(e) => onSelect((e.currentTarget as HTMLSelectElement).value),});
          if(servers.length === 0){
             { svelteHTML.createElement("option", {"value":"",});  }
          }
             for(let s of __sveltets_2_ensureArray(servers)){s.id;
             { svelteHTML.createElement("option", { "value":s.id,});s.name; }
          }
         }
       }
    }
    if(server){
       { const $$_lliPpcM2C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM2C({ target: __sveltets_2_any(), props: {    "kind":`injection`,"value":server.injection_risk,}});}
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void discover(),"disabled":loading || !can('configure'),"title":can('configure') ? undefined : NO_CONFIGURE,});
        loading ? 'Discovering…' : 'Discover';
       }
    }
   }

  if(!server){
     { svelteHTML.createElement("p", { "class":`muted pad`,});              }
  } else if (tools.length === 0){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`tools`,loading,"error":loadError,"empty":true,"onretry":() => void loadTools(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
         { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {           "icon":`zap`,"title":`No tools discovered yet for ${server?.name ?? 'this server'}`,"body":can('configure') ? 'Discover asks the server which tools it offers.' : `${NO_CONFIGURE} to discover its tools.`,"actionLabel":can('configure') ? 'Discover tools' : undefined,"onaction":() => void discover(),}});}
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
      
     LoadState}
  }else{
     { svelteHTML.createElement("div", { "class":`grid`,});
       { svelteHTML.createElement("div", { "class":`thead`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {});  }
       }
         for(let t of __sveltets_2_ensureArray(tools)){t.id;
         { svelteHTML.createElement("div", { "class":`trow`,});
           { svelteHTML.createElement("div", { "class":`tname`,});
             { svelteHTML.createElement("span", { "class":`nm`,});t.title || t.name; }
             { svelteHTML.createElement("code", { "class":`code`,});t.name; }
            if(t.description){ { svelteHTML.createElement("span", {   "class":`desc`,"title":t.description,});t.description; }}
           }
           { svelteHTML.createElement("span", { "class":`cell`,});
              { const $$_lliPpcM4C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM4C({ target: __sveltets_2_any(), props: {    "kind":`risk`,"value":t.risk_label,"small":true,}});}
            if(t.risk_overridden){ { svelteHTML.createElement("span", {   "class":`pinned`,"title":`Human-pinned override (survives re-discovery)`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});} }}
           }
           { svelteHTML.createElement("span", { "class":`cell`,});  { const $$_lliPpcM4C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM4C({ target: __sveltets_2_any(), props: {    "kind":`injection`,"value":t.injection_risk,"small":true,}});} }
           { svelteHTML.createElement("span", { "class":`cell`,});
             { const $$_hctiwS4C = __sveltets_2_ensureComponent(Switch); new $$_hctiwS4C({ target: __sveltets_2_any(), props: {             "tone":`success`,"checked":t.enabled,"disabled":busyTool[t.id] || !can('configure', t.name),"onchange":() => void patchTool(t, { enabled: !t.enabled }),"label":`Enable ${t.name}`,"title":!can('configure', t.name) ? NO_CONFIGURE : t.enabled ? 'Enabled — click to disable' : 'Disabled — click to enable',}});}
           }
           { svelteHTML.createElement("span", { "class":`cell`,});
             { const $$_hctiwS4C = __sveltets_2_ensureComponent(Switch); new $$_hctiwS4C({ target: __sveltets_2_any(), props: {             "tone":`success`,"checked":t.require_approval,"disabled":busyTool[t.id] || !can('configure', t.name),"onchange":() => void patchTool(t, { require_approval: !t.require_approval }),"label":`Require approval for ${t.name}`,"title":!can('configure', t.name) ? NO_CONFIGURE : t.require_approval ? 'Requires approval — click to allow without approval' : 'No approval required — click to require it',}});}
           }
           { svelteHTML.createElement("span", { "class":`cell`,});
             { svelteHTML.createElement("select", {         "value":t.risk_label,"aria-label":`Risk label for ${t.name}`,"disabled":busyTool[t.id] || !can('configure',t.name),"onchange":(e) =>
                void patchTool(t, {
                  risk_label: (e.currentTarget as HTMLSelectElement).value as McpRiskLabel,
                }),});
               { svelteHTML.createElement("option", { "value":`read`,});  }
               { svelteHTML.createElement("option", { "value":`write`,});  }
               { svelteHTML.createElement("option", { "value":`dangerous`,});  }
               { svelteHTML.createElement("option", { "value":`unknown`,});  }
             }
           }
         }
      }
     }

    if(embedded){
       { svelteHTML.createElement("button", {         "class":`tester-disclose`,"type":`button`,"aria-expanded":testerOpen,"onclick":() => (testerOpen = !testerOpen),});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":testerOpen ? 'chevronDown' : 'chevronRight',"size":13,}});}   
       }
    }

    
    if(!embedded || testerOpen){
       { svelteHTML.createElement("div", { "class":`tester`,});
       { svelteHTML.createElement("div", { "class":`tester-head`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":13,}});}
         { svelteHTML.createElement("span", { "class":`th`,});  }
         { svelteHTML.createElement("span", { "class":`muted small`,});            }
       }
       { svelteHTML.createElement("div", { "class":`tester-body`,});
         { svelteHTML.createElement("div", { "class":`tcontrols`,});
           { svelteHTML.createElement("label", { "class":`picker`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("select", { "bind:value":testToolName,});/*Ωignore_startΩ*/() => testToolName = __sveltets_2_any(null);/*Ωignore_endΩ*/
                 for(let t of __sveltets_2_ensureArray(tools)){t.id;
                 { svelteHTML.createElement("option", { "value":t.name,});t.name; }
              }
             }
           }
           { svelteHTML.createElement("label", { "class":`check`,});
             { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":dryRun,});/*Ωignore_startΩ*/() => dryRun = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("span", {});    }
           }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {     "class":`btn primary small`,"onclick":() => void run(),"disabled":running || !testToolName || !can('invoke',testToolName),});
            running ? 'Running…' : 'Run';
           }
         }
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("textarea", {         "dir":`ltr`,"bind:value":argsText,"rows":4,"class":`mono`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => argsText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         }
        if(argError){ { svelteHTML.createElement("p", { "class":`warn`,});argError; }}

        if(result){
           { svelteHTML.createElement("div", { "class":`result`,});
             { svelteHTML.createElement("div", { "class":`rhead`,});
               { const $$_lliPpcM5C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM5C({ target: __sveltets_2_any(), props: {    "kind":`decision`,"value":result.decision,}});}
              if(result.dry_run){ { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":`info`,"label":`Dry run`,}});}}
              if(result.is_error){ { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":`bad`,"label":`Tool error`,}});}}
              if(result.reason){ { svelteHTML.createElement("span", { "class":`reason`,});result.reason; }}
             }
            if(result.decision === 'pending_approval'){
               { svelteHTML.createElement("p", { "class":`pending`,});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});}        { svelteHTML.createElement("strong", {});   }
                if(result.approval_id){ { svelteHTML.createElement("br", {});}   { svelteHTML.createElement("code", { "class":`code`,});result.approval_id; }}
               }
            } else if (result.preview !== undefined && result.preview !== null){
               { svelteHTML.createElement("pre", { "class":`json`,});pretty(result.preview); }
            } else if (result.content !== undefined && result.content !== null){
               { svelteHTML.createElement("pre", { "class":`json`,});pretty(result.content); }
            }else{
               { svelteHTML.createElement("p", { "class":`muted small`,});   }
            }
           }
        }
       }
       }
    }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ToolsTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ToolsTab__SvelteComponent_ = ReturnType<typeof ToolsTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default ToolsTab__SvelteComponent_;