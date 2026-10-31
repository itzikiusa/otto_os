///<reference types="svelte" />
;
import Badge from '../../lib/components/Badge.svelte';
import { NO_WORKSPACE } from '../../lib/labels';
import { plural } from '../../lib/plural';
// MCP Control Plane — three focused sections: Otto's built-in server,
// governed external servers, and approval/audit activity.
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import { onTabKey } from '../../lib/tabKeys';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { router } from '../../lib/router.svelte';
import { registry } from '../../lib/commands.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { mcpCpApi } from '../../lib/api/mcp';
import { liveQuery } from '../../lib/live';
import { loadErrorText } from '../../lib/loadError';
import type { McpServerDetail } from '../../lib/api/types';
import ServersTab from './ServersTab.svelte';
import ApprovalsTab from './ApprovalsTab.svelte';
import AuditTab from './AuditTab.svelte';
import OttoServerHome from './OttoServerHome.svelte';
import LazyMount from '../../lib/components/LazyMount.svelte';
import { lazyComponent } from '../../lib/lazy-component.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  // Profiles is its own section, behind a dynamic import so the editor and
  // tool-group tables don't ride on every MCP page open (bundle budget).
  const ProfilesTab = lazyComponent(() => import('./ProfilesTab.svelte'));

  type Section = 'otto' | 'profiles' | 'servers' | 'activity';
  const section = $derived<Section>(
    (['otto', 'profiles', 'servers', 'activity'] as const).includes(router.parts[1] as Section)
      ? (router.parts[1] as Section)
      : 'otto',
  );
  const wsId = $derived(ws.currentId);

  // The registry is shared by the external-server view and activity filters.
  let accessRevision = $state(0);
  let loadGeneration = 0;
  $effect(() =>
    resourceAccess.subscribe((change) => {
      if (
        change.type === 'decision' &&
        (change.kind !== 'mcp_server' ||
          !change.before ||
          !Object.keys(change.before.operations).some(
            (operation) =>
              change.before?.operations[operation]?.allowed &&
              !change.after?.operations[operation]?.allowed,
          ))
      )
        return;
      accessRevision++;
      loadGeneration++;
      servers = [];
      loadError = null;
      selectedServerId = null;
      void loadServers();
    }),
  );
  let servers = $state<McpServerDetail[]>([]);
  let loading = $state(false);
  /** Failed server-list load — ServersTab renders it inline with Retry. */
  let loadError = $state<string | null>(null);
  let selectedServerId = $state<string | null>(null);
  let pending = $state(0);

  async function loadServers(): Promise<void> {
    const generation = ++loadGeneration;
    const id = wsId;
    if (!id) {
      servers = [];
      loading = false;
      return;
    }
    loading = true;
    try {
      const result = await mcpCpApi.cpList(id);
      if (generation !== loadGeneration) return;
      servers = result;
      loadError = null;
      if (selectedServerId && !servers.some((server) => server.id === selectedServerId)) {
        selectedServerId = null;
      }
      if (!selectedServerId && servers.length > 0) selectedServerId = servers[0].id;
    } catch (e) {
      if (generation === loadGeneration) loadError = loadErrorText(e);
    } finally {
      if (generation === loadGeneration) loading = false;
    }
  }

  async function loadPending(): Promise<void> {
    try {
      // The count route, not 200 whole rows just to take their length.
      pending = (await mcpCpApi.cpApprovalsCount('pending')).count;
    } catch {
      pending = 0;
    }
  }

  $effect(() => {
    void wsId;
    selectedServerId = null;
    void loadServers();
  });

  // Pending-approvals badge: event-fed (`mcp_approval_changed`); the 15 s
  // poll chain only while the event socket is down.
  $effect(() => {
    const p = liveQuery({ run: () => loadPending(), on: ['mcp_approval_changed'], fallbackMs: 15_000 });
    return () => p.stop();
  });

  function patchServer(updated: McpServerDetail): void {
    servers = servers.map((server) => (server.id === updated.id ? updated : server));
  }

  function go(id: Section): void {
    router.go('mcp/' + id);
  }

  const SECTIONS: { id: Section; label: string }[] = [
    { id: 'otto', label: 'Otto server' },
    { id: 'profiles', label: 'Profiles' },
    { id: 'servers', label: 'External servers' },
    { id: 'activity', label: 'Activity' },
  ];

  // ⌘K: the page's verbs.
  $effect(() =>
    registry.register('mcp', [
      { id: 'mcp.otto', title: 'Show the Otto MCP server', group: 'MCP', keywords: 'built-in gateway tools expose attach sessions token', run: () => go('otto') },
      { id: 'mcp.profiles', title: 'Show MCP profiles', group: 'MCP', keywords: 'profile tool set subset database issues default session connection', run: () => go('profiles') },
      { id: 'mcp.servers', title: 'Show external MCP servers', group: 'MCP', keywords: 'governed servers tools discover allowlist policy', run: () => go('servers') },
      { id: 'mcp.activity', title: 'Show MCP approvals and audit', group: 'MCP', keywords: 'activity pending approve deny audit log', run: () => go('activity') },
      { id: 'mcp.refresh', title: 'Refresh MCP servers', group: 'MCP', keywords: 'reload list', disabled: !wsId, run: () => void loadServers() },
    ]),
  );
;
async () => {

 { svelteHTML.createElement("div", { "class":`mcp-page`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: { "title":`MCP Control Plane`,badge:() => { async ()/*Ωignore_positionΩ*/ => {
      if(ws.current){ { svelteHTML.createElement("span", {   "class":`wsname`,"title":`Workspace: ${ws.current.name}`,});ws.current.name; }}
    };return __sveltets_2_any(0)},tabs:() => { async ()/*Ωignore_positionΩ*/ => {
      
       { svelteHTML.createElement("div", {         "class":`segmented tabs`,"role":`tablist`,"aria-label":`MCP sections`,"tabindex":-1,"onkeydown":onTabKey,});
           for(let s of __sveltets_2_ensureArray(SECTIONS)){s.id;
           { svelteHTML.createElement("button", {            "role":`tab`,"aria-selected":section === s.id,"tabindex":section === s.id ? 0 : -1,...__sveltets_2_empty({"data-testid":`mcp-nav-${s.id}`}),"onclick":() => go(s.id),});section === s.id;
            s.label;
            if(s.id === 'servers' && wsId && !loading && !loadError){ { svelteHTML.createElement("span", { "class":`count`,});servers.length; }}
            if(s.id === 'activity' && pending > 0){ { const $$_egdaB2C = __sveltets_2_ensureComponent(Badge); new $$_egdaB2C({ target: __sveltets_2_any(), props: {        "tone":`warn`,"testid":`mcp-pending-badge`,"title":`${plural(pending, 'approval')} waiting for you`,"label":String(pending),}});}}
           }
        }
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {badge, tabs} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
    
   PageHeader}

  
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"padded":section === 'otto' || section === 'profiles',}});
    accessRevision; {
      if(section === 'otto'){
         { const $$_emoHrevreSottO2C = __sveltets_2_ensureComponent(OttoServerHome); new $$_emoHrevreSottO2C({ target: __sveltets_2_any(), props: { wsId,}});}
      } else if (section === 'profiles'){
         { const $$_tnuoMyzaL2C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL2C({ target: __sveltets_2_any(), props: {    "lazy":ProfilesTab,"what":`MCP profiles`,}});}
      } else if (section === 'servers'){
        if(!wsId){
           { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`plug`,"title":NO_WORKSPACE,"body":`Governed MCP servers and their tools belong to a workspace.`,}});}
        }else{
           { const $$_baTsrevreS2C = __sveltets_2_ensureComponent(ServersTab); new $$_baTsrevreS2C({ target: __sveltets_2_any(), props: {              wsId,servers,loading,"error":loadError,"selectedServerId":null,"onReload":loadServers,"onPatch":patchServer,"onSelect":() => {},}});}
        }
      }else{
         { svelteHTML.createElement("div", { "class":`activity`,});
           { const $$_baTslavorppA3C = __sveltets_2_ensureComponent(ApprovalsTab); new $$_baTslavorppA3C({ target: __sveltets_2_any(), props: {  "ondecided":() => void loadPending(),}});}
           { const $$_baTtiduA3C = __sveltets_2_ensureComponent(AuditTab); new $$_baTtiduA3C({ target: __sveltets_2_any(), props: { servers,}});}
         }
      }
    }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const McpPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type McpPage__SvelteComponent_ = ReturnType<typeof McpPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default McpPage__SvelteComponent_;