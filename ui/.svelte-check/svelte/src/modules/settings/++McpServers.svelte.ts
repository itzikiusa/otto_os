///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { NO_WORKSPACE } from '../../lib/labels';
import { sectionLabel } from './sections';
import PageBody from '../../lib/components/PageBody.svelte';
import SectionIntro from './SectionIntro.svelte';
// MCP Servers settings page: per-workspace, user-managed MCP servers that Otto
// merges into the workspace's `.mcp.json` when an agent session spawns there
// (alongside Otto's own managed entries, e.g. the browser server). Nothing is
// auto-enabled — each server is off until you flip it on, and it's only written
// to `.mcp.json` the next time a session spawns in the workspace.
import { auth } from '../../lib/stores/auth.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import { mcpApi, mcpCpApi } from '../../lib/api/mcp';
import { router } from '../../lib/router.svelte';
import { mcpCpExtraApi } from '../mcp/cp-api';
import type { McpServer, CreateMcpServerReq } from '../../lib/api/types';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import SettingToggle from './SettingToggle.svelte';
import { loadErrorText } from '../../lib/loadError';
import { ctxMenu } from '../../lib/contextmenu.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // The first-party `otto` MCP server (Otto's read-only tools + the read-only DB
  // connection tools), attached to agent sessions per WORKSPACE through the same
  // session-attach endpoint the MCP page uses (`otto_mcp_enabled`, stored as a
  // per-workspace map; unlisted ⇒ default ON). It used to read/write the raw
  // setting as ONE global bool: a workspace switched off on the MCP page still
  // showed "On" here, and flipping it overwrote every workspace's choice.
  let ottoEnabled = $state(true);
  let ottoLoaded = $state(false);
  let ottoSaving = $state(false);
  let ottoError = $state('');

  $effect(() => {
    const id = ws.currentId;
    if (id) void loadOttoSetting(id);
  });

  async function loadOttoSetting(id: string): Promise<void> {
    ottoLoaded = false;
    ottoError = '';
    try {
      const r = await mcpCpExtraApi.sessionAttach(id);
      if (ws.currentId === id) ottoEnabled = r.attached;
    } catch (e) {
      if (ws.currentId === id) ottoError = errMsg(e);
    } finally {
      if (ws.currentId === id) ottoLoaded = true;
    }
  }

  async function toggleOtto(next: boolean): Promise<void> {
    const id = ws.currentId;
    if (!id) return;
    ottoSaving = true;
    try {
      const r = await mcpCpExtraApi.setSessionAttach(id, { enabled: next });
      ottoEnabled = r.attached;
      toasts.success(
        r.attached ? 'Connections MCP attached' : 'Connections MCP detached',
        'For this workspace — applies to agent sessions started from now on.',
      );
    } catch (e) {
      // SettingToggle re-syncs the box to ottoEnabled (unchanged) after this.
      toasts.error("Couldn’t change the Connections MCP setting", errMsg(e));
    } finally {
      ottoSaving = false;
    }
  }

  let servers: McpServer[] = $state([]);
  let loading = $state(false);
  let loadError = $state('');
  let loadGeneration = 0;
  const canConfigure = (id: string) => resourceAccess.can('mcp_server', id, 'configure', 'mcp', 'admin');
  $effect(() => { for (const server of servers) void resourceAccess.load('mcp_server', server.id); });
  $effect(() => resourceAccess.subscribe(change => {
    if (change.type === 'reset' || (change.kind === 'mcp_server' && change.before?.operations.configure?.allowed && !change.after?.operations.configure?.allowed)) {
      loadGeneration++; servers = []; closeForm();
      if (ws.currentId) void load(ws.currentId);
    }
  }));
  let busyId: string | null = $state(null);

  // Add/edit form state. `editing` holds the id being edited (null = creating a
  // new one once the form is open).
  let formOpen = $state(false);
  let editing: string | null = $state(null);
  let fName = $state('');
  let fCommand = $state('');
  let fArgs = $state(''); // one arg per line
  let fEnv = $state(''); // KEY=value per line
  // Secret env: values go to the macOS Keychain, never the DB row. On edit the
  // stored keys show as KEY= lines (value blank = keep the stored value).
  let fSecretEnv = $state('');
  let fEnabled = $state(false);
  let saving = $state(false);

  const wsId = $derived(ws.currentId);
  const formValid = $derived(fName.trim() !== '' && (!auth.isRoot || fCommand.trim() !== ''));

  function errMsg(e: unknown): string {
    return loadErrorText(e);
  }

  $effect(() => {
    if (wsId) void load(wsId);
  });

  async function load(id: string): Promise<void> {
    const generation = ++loadGeneration;
    loading = true;
    loadError = '';
    try {
      const result = await mcpApi.list(id);
      if (generation === loadGeneration) servers = result;
    } catch (e) {
      if (generation === loadGeneration) loadError = errMsg(e);
    } finally {
      loading = false;
    }
  }

  let showSecrets = $state(false);

  function resetForm(): void {
    editing = null;
    fName = '';
    fCommand = '';
    fArgs = '';
    fEnv = '';
    fSecretEnv = '';
    fEnabled = false;
    showSecrets = false;
  }

  function openCreate(): void {
    if (!auth.isRoot) return;
    resetForm();
    formOpen = true;
  }

  function openEdit(s: McpServer): void {
    editing = s.id;
    fName = s.name;
    fCommand = s.command;
    fArgs = s.args.join('\n');
    fEnv = Object.entries(s.env)
      .map(([k, v]) => `${k}=${v}`)
      .join('\n');
    // Stored secret values are never returned — surface the key names so a
    // blank value means "keep what's in the Keychain".
    fSecretEnv = s.secret_env_keys.map((k) => `${k}=`).join('\n');
    fEnabled = s.enabled;
    showSecrets = false;
    formOpen = true;
  }

  function closeForm(): void {
    formOpen = false;
    resetForm();
  }

  // Split a multi-line textarea into trimmed, non-empty lines.
  function lines(text: string): string[] {
    return text
      .split('\n')
      .map((l) => l.trim())
      .filter(Boolean);
  }

  // Parse "KEY=value" lines into an object (first '=' splits; later ones kept).
  // `allowEmpty` keeps KEY= lines (used by the secret editor's keep-sentinel).
  function parseEnv(text: string, allowEmpty = false): Record<string, string> {
    const out: Record<string, string> = {};
    for (const line of lines(text)) {
      const eq = line.indexOf('=');
      if (eq <= 0) continue; // skip lines without a key
      const v = line.slice(eq + 1).trim();
      if (v === '' && !allowEmpty) continue;
      out[line.slice(0, eq).trim()] = v;
    }
    return out;
  }

  async function save(): Promise<void> {
    if (!wsId || (!editing && !auth.isRoot)) return;
    const name = fName.trim();
    const command = fCommand.trim();
    if (!formValid) return;
    // Secret env: parse KEY=value lines; a KEY= line with an empty value keeps
    // the currently stored Keychain value (edit flow surfaces keys that way).
    const secretPairs = parseEnv(fSecretEnv, true);
    const keptKeys = lines(fSecretEnv)
      .map((l) => l.split('=')[0]?.trim() ?? '')
      .filter(Boolean);
    const existing = editing ? servers.find((x) => x.id === editing) : undefined;
    const secret_env: Record<string, string> = {};
    for (const k of keptKeys) {
      if (secretPairs[k] !== undefined && secretPairs[k] !== '') secret_env[k] = secretPairs[k];
      else if (existing?.secret_env_keys.includes(k)) secret_env[k] = ''; // sentinel: keep stored
    }
    // Drop keep-sentinels for create (nothing stored yet) and warn once.
    if (!editing) for (const k of Object.keys(secret_env)) if (secret_env[k] === '') delete secret_env[k];

    const body: CreateMcpServerReq = {
      name,
      command,
      args: lines(fArgs),
      env: parseEnv(fEnv),
      secret_env,
      enabled: fEnabled,
    };
    saving = true;
    try {
      if (editing) {
        await mcpApi.update(editing, auth.isRoot ? body : { name, enabled: fEnabled });
        toasts.success('MCP server updated', name);
      } else {
        await mcpApi.create(wsId, body);
        toasts.success('MCP server added', name);
      }
      closeForm();
      await load(wsId);
    } catch (e) {
      toasts.error(editing ? "Couldn’t save the MCP server" : "Couldn’t add the MCP server", errMsg(e));
    } finally {
      saving = false;
    }
  }

  async function toggleEnabled(s: McpServer): Promise<void> {
    if (!wsId) return;
    busyId = s.id;
    try {
      await mcpApi.update(s.id, { enabled: !s.enabled });
      await load(wsId);
    } catch (e) {
      toasts.error(`Couldn’t ${s.enabled ? 'disable' : 'enable'} ${s.name}`, errMsg(e));
    } finally {
      busyId = null;
    }
  }

  // Two registries exist on purpose for now: this ungoverned `.mcp.json` list
  // and the governed MCP Control Plane (per-tool permissions, approvals,
  // audit). "Move to Control Plane" registers the same command there and turns
  // this entry OFF (never deletes it — the user removes it when satisfied).
  // Secret env values are Keychain-only and never readable back, so a server
  // carrying them can't be copied: it is re-added on the Control Plane instead.
  let movingId = $state<string | null>(null);
  async function moveToControlPlane(s: McpServer): Promise<void> {
    if (!wsId || s.secret_env_keys.length) return;
    const ok = await confirmer.ask(
      `Register “${s.name}” in the MCP Control Plane, where its tools are permissioned, approved and audited? This entry is then turned off here (not deleted), so the server isn’t written to .mcp.json twice.`,
      { title: 'Move to Control Plane', confirmLabel: 'Move' },
    );
    if (!ok) return;
    movingId = s.id;
    try {
      const created = await mcpCpApi.cpCreate(wsId, { name: s.name, transport: 'stdio', command: s.command, args: s.args, env: s.env, enabled: true });
      if (s.enabled) {
        try {
          await mcpApi.update(s.id, { enabled: false });
        } catch (e) {
          // Never leave the server live in BOTH registries (written to
          // .mcp.json twice): undo the Control Plane copy, then report.
          try {
            await mcpCpApi.cpDelete(created.id);
          } catch (rollback) {
            // The undo failed too: the server IS live in both registries.
            // Say so (with both causes) and point at the Control Plane copy
            // instead of reporting a plain failed move.
            await load(wsId);
            toasts.push(
              'error',
              `${s.name} is now enabled in BOTH registries`,
              `Turning it off here failed (${errMsg(e)}), and removing the new Control Plane copy failed too (${errMsg(rollback)}). The Control Plane copy still exists and is enabled — disable one of them.`,
              12000,
              { action: { label: 'Open Control Plane', run: () => router.go('mcp/servers') } },
            );
            return;
          }
          throw e;
        }
      }
      await load(wsId);
      toasts.success(`${s.name} is now governed`, 'Review its tools on the MCP Control Plane.');
    } catch (e) {
      toasts.error(`Couldn’t move ${s.name}`, errMsg(e));
    } finally {
      movingId = null;
    }
  }

  async function remove(s: McpServer): Promise<void> {
    if (!wsId) return;
    if (
      !(await confirmer.ask(
        `Remove MCP server “${s.name}”? It stops being written to this workspace’s .mcp.json for new sessions${s.secret_env_keys.length ? ', and its secret values are removed from the Keychain' : ''}.`,
        { title: 'Remove MCP server', danger: true, confirmLabel: 'Remove' },
      ))
    )
      return;
    busyId = s.id;
    try {
      await mcpApi.remove(s.id);
      toasts.success('MCP server removed', s.name);
      await load(wsId);
    } catch (e) {
      toasts.error(`Couldn’t remove ${s.name}`, errMsg(e));
    } finally {
      busyId = null;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('mcp-servers'),"subtitle":`Extra agent tools for this workspace`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(wsId && servers.length > 0){
         { svelteHTML.createElement("button", {        "class":`btn small primary`,"disabled":!auth.isRoot,"title":auth.isRoot ? undefined : 'Only the owner can add MCP servers (they run a command on this Mac)',"onclick":openCreate,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}   }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});        { svelteHTML.createElement("code", {});  }                            SectionIntro}

   { svelteHTML.createElement("div", {   "class":`card ungoverned`,...__sveltets_2_empty({"data-testid":`mcp-ungoverned-note`}),});
     { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
     { svelteHTML.createElement("div", { "class":`ungoverned-text`,});
       { svelteHTML.createElement("strong", {});     { svelteHTML.createElement("code", {});  }  }
                   
       { svelteHTML.createElement("strong", {});   }            
                  
     }
     { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go('mcp/servers'),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`server`,"size":12,}});}     }
   }

   { svelteHTML.createElement("div", { "class":`section-title`,});  }
   { svelteHTML.createElement("div", {   "class":`card mcp-card otto`,...__sveltets_2_empty({"data-testid":`connections-mcp`}),});
     { const $$_elggoTgnitteS3C = __sveltets_2_ensureComponent(SettingToggle); new $$_elggoTgnitteS3C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"label":`Attach the Connections MCP to agent sessions`,"checked":ottoEnabled,"disabled":!wsId || !ottoLoaded || ottoSaving || !!ottoError,"title":!wsId ? NO_WORKSPACE : ottoError ? "Couldn’t read the current value — Retry below" : undefined,"onchange":toggleOtto,}});
         { svelteHTML.createElement("code", {});  }   { svelteHTML.createElement("strong", {});  }     
            { svelteHTML.createElement("code", {});  }  { svelteHTML.createElement("code", {});  }
       { svelteHTML.createElement("code", {});  }           
       { svelteHTML.createElement("code", {});  }          
                   
     SettingToggle}
    if(ottoError){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`this workspace’s setting`,"error":ottoError,"empty":true,"onretry":() => ws.currentId && void loadOttoSetting(ws.currentId),}});}
    }
   }

   { svelteHTML.createElement("div", { "class":`section-title`,});  }
  if(!wsId){
     { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {       "icon":`server`,"title":NO_WORKSPACE,"body":`MCP servers are per-workspace. Choose a workspace from the sidebar to configure them.`,}});}
  }else{
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`MCP servers`,loading,"error":loadError,"empty":servers.length === 0,"onretry":() => wsId && void load(wsId),"rows":2,emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
         { svelteHTML.createElement("div", { "class":`card mcp-card`,});
           { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"icon":`server`,"title":`No MCP servers yet`,"body":`Add a Model Context Protocol server (a command such as npx @linear/mcp) to give agents extra tools in this workspace.`,"actionLabel":auth.isRoot ? 'Add server' : undefined,"actionIcon":`plus`,"onaction":auth.isRoot ? openCreate : undefined,}});
            if(!auth.isRoot){ { svelteHTML.createElement("span", { "class":`dim`,});      }}
           EmptyState}
         }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
      
       { svelteHTML.createElement("div", { "class":`server-list`,});
           for(let s of __sveltets_2_ensureArray(servers)){s.id;
          const locked = busyId === s.id || !canConfigure(s.id);
          
           {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {         "role":`presentation`,"class":`card server`,"oncontextmenu":(e) => ctxMenu.show(e, [
              { label: s.enabled ? 'Disable' : 'Enable', icon: s.enabled ? 'eyeOff' : 'eye', disabled: locked, action: () => toggleEnabled(s) },
              { label: 'Edit…', icon: 'edit', disabled: locked, action: () => openEdit(s) },
              { separator: true },
              { label: 'Remove…', icon: 'trash', danger: true, disabled: locked, action: () => remove(s) },
            ]),});!s.enabled;
             { svelteHTML.createElement("span", { "class":`server-icon`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`server`,"size":14,}});} }
             { svelteHTML.createElement("div", { "class":`server-main`,});
               { svelteHTML.createElement("span", {   "class":`server-name mono`,"title":s.name,});s.name; }
               { svelteHTML.createElement("div", {   "class":`server-cmd mono`,"title":`${s.command} ${s.args.join(' ')}`.trim(),});
                s.command;s.args.length ? ' ' + s.args.join(' ') : '';
               }
              if(Object.keys(s.env).length || s.secret_env_keys.length){
                 { svelteHTML.createElement("div", { "class":`server-env`,});
                  if(Object.keys(s.env).length){ Object.keys(s.env).join(', ');}
                  if(s.secret_env_keys.length){
                    if(Object.keys(s.env).length){ } { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});} s.secret_env_keys.join(', ');
                  }
                 }
              }
             }
             { svelteHTML.createElement("div", { "class":`server-actions`,});
              
               { svelteHTML.createElement("label", {   "class":`checkbox-row srv-enabled`,"title":!canConfigure(s.id) ? "You can’t configure this server" : s.enabled ? `Stop writing ${s.name} to .mcp.json` : `Write ${s.name} to .mcp.json for new sessions`,});
                 { svelteHTML.createElement("input", {         "type":`checkbox`,"checked":s.enabled,"disabled":locked,"onchange":async (e) => {
                    const el = e.currentTarget;
                    await toggleEnabled(s);
                    // A failed save leaves the saved value — show it, not the click.
                    el.checked = servers.find((x) => x.id === s.id)?.enabled ?? false;
                  },});}
                
               }
               { svelteHTML.createElement("button", {           "class":`btn small srv-move`,"disabled":locked || movingId !== null || s.secret_env_keys.length > 0,"aria-busy":movingId === s.id,"title":s.secret_env_keys.length > 0 ? 'Its secret values stay in the Keychain and can’t be copied — add it on the MCP Control Plane instead' : `Register ${s.name} in the governed MCP Control Plane`,"onclick":() => void moveToControlPlane(s),});movingId === s.id ? 'Moving…' : 'Move to Control Plane'; }
               { svelteHTML.createElement("button", {           "class":`icon-btn srv-tool`,"disabled":locked,"title":!canConfigure(s.id) ? "You can’t configure this server" : `Edit ${s.name}`,"aria-label":`Edit ${s.name}`,"onclick":() => openEdit(s),});
                 { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":14,}});}
               }
               { svelteHTML.createElement("button", {           "class":`icon-btn srv-tool`,"disabled":locked,"title":`Remove ${s.name}`,"aria-label":`Remove ${s.name}`,"onclick":() => remove(s),});
                 { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
               }
             }
           }}
        }
       }
     LoadState}
  }
   PageBody}
 }

if(formOpen){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":editing ? 'Edit MCP server' : 'Add MCP server',"width":540,"onclose":closeForm,footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {     "class":`btn`,"disabled":saving,"onclick":closeForm,});  }
       { svelteHTML.createElement("button", {         "class":`btn primary`,"disabled":saving || !formValid,"title":formValid ? undefined : auth.isRoot ? 'Enter a name and a command' : 'Enter a name',"onclick":save,});
        saving ? 'Saving…' : editing ? 'Save changes' : 'Add server';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`mcp-name`,});  }
       { svelteHTML.createElement("input", {              "dir":`ltr`,"id":`mcp-name`,"class":`input mono`,"bind:value":fName,"spellcheck":`false`,"autocomplete":`off`,"placeholder":`linear`,});/*Ωignore_startΩ*/() => fName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("span", { "class":`hint`,});    { svelteHTML.createElement("code", {});  }   { svelteHTML.createElement("code", {});  }    }
     }
    if(!auth.isRoot){ { svelteHTML.createElement("p", { "class":`hint owner-note`,});        }}
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`mcp-command`,});  }
       { svelteHTML.createElement("input", {                "dir":`ltr`,"id":`mcp-command`,"class":`input mono`,"bind:value":fCommand,"disabled":!auth.isRoot,"spellcheck":`false`,"autocomplete":`off`,"placeholder":`npx`,});/*Ωignore_startΩ*/() => fCommand = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`mcp-args`,});  { svelteHTML.createElement("span", { "class":`dim`,});   } }
       { svelteHTML.createElement("textarea", {                "dir":`ltr`,"id":`mcp-args`,"class":`input mono`,"rows":3,"bind:value":fArgs,"disabled":!auth.isRoot,"spellcheck":`false`,"placeholder":'-y\n@linear/mcp',});/*Ωignore_startΩ*/() => fArgs = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`mcp-env`,});  { svelteHTML.createElement("span", { "class":`dim`,});    } }
       { svelteHTML.createElement("textarea", {                "dir":`ltr`,"id":`mcp-env`,"class":`input mono`,"rows":3,"bind:value":fEnv,"disabled":!auth.isRoot,"spellcheck":`false`,"placeholder":'LOG_LEVEL=info',});/*Ωignore_startΩ*/() => fEnv = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       { svelteHTML.createElement("span", { "class":`hint`,});             }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("div", { "class":`secret-label-row`,});
         { svelteHTML.createElement("label", { "for":`mcp-secret-env`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});}    { svelteHTML.createElement("span", { "class":`dim`,});    } }
         { svelteHTML.createElement("button", {     "class":`btn small ghost`,"type":`button`,"onclick":() => (showSecrets = !showSecrets),});
          showSecrets ? 'Hide' : 'Show';
         }
       }
       { svelteHTML.createElement("textarea", {                 "dir":`ltr`,"id":`mcp-secret-env`,"class":`input mono`,"rows":3,"bind:value":fSecretEnv,"disabled":!auth.isRoot,"spellcheck":`false`,"placeholder":'API_TOKEN=…',});!showSecrets;/*Ωignore_startΩ*/() => fSecretEnv = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       { svelteHTML.createElement("span", { "class":`hint`,});
                     { svelteHTML.createElement("code", {});  }
                         
         { svelteHTML.createElement("code", {});  }     
       }
     }
     { svelteHTML.createElement("label", { "class":`checkbox-row enable-row`,});
       { svelteHTML.createElement("input", {      "id":`mcp-enabled`,"type":`checkbox`,"bind:checked":fEnabled,});/*Ωignore_startΩ*/() => fEnabled = __sveltets_2_any(null);/*Ωignore_endΩ*/}
            { svelteHTML.createElement("code", {});  }   
     }
    
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const McpServers__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type McpServers__SvelteComponent_ = ReturnType<typeof McpServers__SvelteComponent_>;
/*Ωignore_endΩ*/export default McpServers__SvelteComponent_;