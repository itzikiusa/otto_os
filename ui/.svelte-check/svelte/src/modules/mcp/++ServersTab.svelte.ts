///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// The governed MCP server registry: each row shows transport, a health pill,
// tool count, injection-risk badge, and an enabled toggle, with Discover /
// Health check / Delete actions. "Add server" opens the create form.
import ResourceAccess from '../../lib/components/ResourceAccess.svelte';
import { toastError } from '../../lib/toastError';
import Modal from '../../lib/components/Modal.svelte';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { mcpCpApi } from '../../lib/api/mcp';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import type { McpServerDetail } from '../../lib/api/types';
import Switch from '../../lib/components/Switch.svelte';
import McpPill from './McpPill.svelte';
import RulesDrawer from './RulesDrawer.svelte';
import ServerForm from './ServerForm.svelte';
import ToolsTab from './ToolsTab.svelte';
;

  interface Props {
    wsId: string;
    servers: McpServerDetail[];
    loading: boolean;
    /** Last list-load failure (human text) — inline with Retry, never "no servers". */
    error?: string | null;
    selectedServerId: string | null;
    onReload: () => Promise<void> | void;
    onPatch: (s: McpServerDetail) => void;
    onSelect: (id: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { wsId, servers, loading, error = null, onReload, onPatch }: Props = $props();

  let accessId = $state<string | null>(null);
  const can = (id: string, op: string) => resourceAccess.can('mcp_server',id,op,'mcp','admin');
  const loadedAccessIds = new Set<string>();
  $effect(() => {
    for (const server of servers) {
      if (loadedAccessIds.has(server.id)) continue;
      loadedAccessIds.add(server.id);
      void resourceAccess.load('mcp_server', server.id);
    }
  });
  let formOpen = $state(false);
  let rulesOpen = $state(false);
  let expandedId = $state<string | null>(null);
  /** Per-server in-flight action so the right buttons spin without blocking others. */
  let busy = $state<Record<string, string>>({});

  function setBusy(id: string, what: string | null): void {
    if (what) busy = { ...busy, [id]: what };
    else {
      const n = { ...busy };
      delete n[id];
      busy = n;
    }
  }

  async function discover(s: McpServerDetail): Promise<void> {
    setBusy(s.id, 'discover');
    try {
      const tools = await mcpCpApi.cpDiscover(s.id);
      toasts.success('Discovered tools', `${plural(tools.length, 'tool')} from ${s.name}`);
      await onReload();
    } catch (e) {
      toastError('Couldn’t discover tools', e);
    } finally {
      setBusy(s.id, null);
    }
  }

  async function health(s: McpServerDetail): Promise<void> {
    setBusy(s.id, 'health');
    try {
      const updated = await mcpCpApi.cpHealth(s.id);
      onPatch(updated);
      if (updated.health_status === 'healthy') {
        toasts.success('Healthy', `${s.name} · ${updated.health_latency_ms ?? '?'}ms`);
      } else {
        toasts.warn('Unhealthy', updated.health_error ?? s.name);
      }
    } catch (e) {
      toastError('Couldn’t check the server', e);
    } finally {
      setBusy(s.id, null);
    }
  }

  async function toggleEnabled(s: McpServerDetail): Promise<void> {
    setBusy(s.id, 'toggle');
    try {
      const updated = await mcpCpApi.cpUpdate(s.id, { enabled: !s.enabled });
      onPatch(updated);
    } catch (e) {
      toastError('Couldn’t update server', e);
    } finally {
      setBusy(s.id, null);
    }
  }

  async function remove(s: McpServerDetail): Promise<void> {
    const ok = await confirmer.ask(`Delete MCP server "${s.name}"? Its discovered tools and allowlist entries are deleted too.`, {
      title: 'Delete server',
      confirmLabel: 'Delete',
      danger: true,
    });
    if (!ok) return;
    setBusy(s.id, 'delete');
    try {
      await mcpCpApi.cpDelete(s.id);
      toasts.success('Server deleted', s.name);
      await onReload();
    } catch (e) {
      toastError('Couldn’t delete the server', e);
    } finally {
      setBusy(s.id, null);
    }
  }

  /** Tooltip for a greyed-out row action — never a disabled button with no reason. */
  const NO_CONFIGURE = 'You need configure access to this server';
  const ROOT_ONLY = 'Only an Otto admin can add MCP servers';

  function rowMenu(e: MouseEvent, s: McpServerDetail): void {
    const configure = can(s.id, 'configure');
    const items: MenuItem[] = [
      { label: busy[s.id] === 'health' ? 'Checking health…' : 'Check health', icon: 'radar', disabled: !configure, action: () => void health(s) },
      { label: expandedId === s.id ? 'Hide tools' : 'View tools', icon: 'eye', action: () => toggleExpanded(s.id) },
    ];
    if (auth.isRoot || can(s.id, 'manage_access')) items.push({ label: 'Manage access…', icon: 'key', action: () => (accessId = s.id) });
    items.push({ separator: true }, { label: 'Delete…', icon: 'trash', danger: true, disabled: !configure, action: () => void remove(s) });
    ctxMenu.show(e, items);
  }

  function toggleExpanded(id: string): void {
    expandedId = expandedId === id ? null : id;
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`servers`,});
   { svelteHTML.createElement("div", { "class":`bar`,});
     { svelteHTML.createElement("span", { "class":`count`,});plural(servers.length, 'server'); }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("button", {     "class":`btn small`,...__sveltets_2_empty({"data-testid":`mcp-rules-btn`}),"onclick":() => (rulesOpen = true),});  }
     { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void onReload(),"title":`Refresh`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} 
     }
    if(servers.length > 0 || error || loading){
     { svelteHTML.createElement("button", {         "class":`btn primary small`,...__sveltets_2_empty({"data-testid":`mcp-add-server`}),"disabled":!auth.isRoot,"title":auth.isRoot ? undefined : ROOT_ONLY,"onclick":() => (formOpen = true),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}  
     }
    }
   }

  if(error){
    
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`MCP servers`,loading,error,"empty":servers.length === 0,"onretry":() => void onReload(),}});}
  }
  if(error && servers.length === 0){
    
  } else if (loading && servers.length === 0){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {    "what":`MCP servers`,"loading":true,"empty":true,"rows":3,}});}
  } else if (servers.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {             "icon":`server`,"title":`No external servers yet`,"body":`Register an external MCP server to govern its tools with policies, approvals and audit. Otto’s own server lives on the Otto server tab.`,"actionLabel":auth.isRoot ? 'Add server' : undefined,"actionIcon":`plus`,"onaction":auth.isRoot ? () => (formOpen = true) : undefined,}});}
  }else{
     { svelteHTML.createElement("div", { "class":`grid`,});
       { svelteHTML.createElement("div", { "class":`thead`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", { "class":`num`,});  }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", { "class":`actions-h`,}); { svelteHTML.createElement("span", { "class":`sr-only`,});  } }
       }
         for(let s of __sveltets_2_ensureArray(servers)){s.id;
        const endpoint = s.transport === 'stdio' ? `${s.command} ${s.args.join(' ')}`.trim() : (s.url ?? '');
         { svelteHTML.createElement("div", { "class":`srow`,});
           { svelteHTML.createElement("button", {         "class":`name`,"onclick":() => toggleExpanded(s.id),"title":expandedId === s.id ? 'Hide tools' : 'View tools',"aria-expanded":expandedId === s.id,});
             { svelteHTML.createElement("span", { "class":`nm`,});s.name; }
            if(s.has_secret){ { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":12,}});}}
            if(s.description){ { svelteHTML.createElement("span", {   "class":`desc`,"title":s.description,});s.description; }}
             { svelteHTML.createElement("span", {   "class":`endpoint mono`,"title":endpoint,});endpoint; }
           }
           { svelteHTML.createElement("span", {   "class":`cell`,...__sveltets_2_empty({"data-label":`Transport`}),}); { svelteHTML.createElement("span", { "class":`transport`,});s.transport; } }
           { svelteHTML.createElement("span", {   "class":`cell`,...__sveltets_2_empty({"data-label":`Health`}),});
              { const $$_lliPpcM4C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM4C({ target: __sveltets_2_any(), props: {    "kind":`health`,"value":s.health_status,"small":true,}});}
            if(s.health_latency_ms != null && s.health_status === 'healthy'){ { svelteHTML.createElement("span", { "class":`lat`,});s.health_latency_ms;  }}
           }
           { svelteHTML.createElement("span", {   "class":`cell num`,...__sveltets_2_empty({"data-label":`Tools`}),});s.tools_count; }
           { svelteHTML.createElement("span", {   "class":`cell`,...__sveltets_2_empty({"data-label":`Injection`}),});  { const $$_lliPpcM4C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM4C({ target: __sveltets_2_any(), props: {    "kind":`injection`,"value":s.injection_risk,"small":true,}});} }
           { svelteHTML.createElement("span", {   "class":`cell`,...__sveltets_2_empty({"data-label":`Enabled`}),});
             { const $$_hctiwS4C = __sveltets_2_ensureComponent(Switch); new $$_hctiwS4C({ target: __sveltets_2_any(), props: {           "checked":s.enabled,"disabled":busy[s.id] === 'toggle' || !can(s.id, 'configure'),"onchange":() => void toggleEnabled(s),"label":`Enable ${s.name}`,"title":!can(s.id, 'configure') ? NO_CONFIGURE : s.enabled ? 'Enabled — click to disable' : 'Disabled — click to enable',}});}
           }
           { svelteHTML.createElement("span", { "class":`cell actions`,});
             { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":!!busy[s.id] || !can(s.id,'configure'),"title":can(s.id,'configure') ? 'Fetch the tool list from this server' : NO_CONFIGURE,"onclick":() => void discover(s),});
              busy[s.id] === 'discover' ? 'Discovering…' : 'Discover';
             }
             { svelteHTML.createElement("button", {           "class":`icon-btn`,"aria-label":`More actions for ${s.name}`,"title":`More actions`,"disabled":!!busy[s.id],"onclick":(e) => rowMenu(e, s),});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
             }
           }
          if(expandedId === s.id){
             { svelteHTML.createElement("div", {   "class":`row-body`,...__sveltets_2_empty({"data-testid":`mcp-server-tools`}),});
               { const $$_baTslooT4C = __sveltets_2_ensureComponent(ToolsTab); new $$_baTslooT4C({ target: __sveltets_2_any(), props: {        wsId,servers,"selectedServerId":s.id,"onSelect":() => {},"embedded":true,"ondiscovered":() => void onReload(),}});}
             }
          }
         }
      }
     }
  }
 }

if(accessId){ { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`MCP server access`,"width":820,"onclose":() => accessId=null,}}); { const $$_sseccAecruoseR1C = __sveltets_2_ensureComponent(ResourceAccess); new $$_sseccAecruoseR1C({ target: __sveltets_2_any(), props: {    "kind":`mcp_server`,"resourceId":accessId,}});} Modal}}

if(formOpen){
   { const $$_mroFrevreS0C = __sveltets_2_ensureComponent(ServerForm); new $$_mroFrevreS0C({ target: __sveltets_2_any(), props: {     wsId,"onclose":() => (formOpen = false),"onsaved":() => void onReload(),}});}
}

if(rulesOpen){
   { const $$_rewarDseluR0C = __sveltets_2_ensureComponent(RulesDrawer); new $$_rewarDseluR0C({ target: __sveltets_2_any(), props: {    wsId,servers,"onClose":() => (rulesOpen = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ServersTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ServersTab__SvelteComponent_ = ReturnType<typeof ServersTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default ServersTab__SvelteComponent_;