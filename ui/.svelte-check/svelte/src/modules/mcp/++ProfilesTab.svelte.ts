///<reference types="svelte" />
;// MCP → Profiles: named variants of the Otto MCP connection, each exposing
// only a subset of the tools (Database, Issues, Git & PRs, Cloud, or your
// own). One profile is the default every new session preselects; Full —
// every tool, the connection as it always was — can't be edited or deleted.
// Each profile also gets its own connection config, so an outside Claude
// Code or Codex can attach "Otto · Database" only.

import { tick, untrack } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import Tabs from '../../lib/components/Tabs.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { COPY_FAILED_BODY, copyTextOrThrow } from '../../lib/clipboard';
import { baseUrl } from '../../lib/api/client';
import { auth } from '../../lib/stores/auth.svelte';
import { mcpCpApi } from '../../lib/api/mcp';
import { mcpProfiles } from '../../lib/stores/mcp-profiles.svelte';
import type { McpProfile } from '../../lib/api/types';
import {
    connectionConfigs,
    draftOf,
    duplicateName,
    emptyDraft,
    profileSummary,
    profileTools,
    type ProfileDraft,
  } from '../../lib/mcpProfiles';
import ProfileEditor from './ProfileEditor.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const isAdmin = $derived(auth.can('mcp', 'admin'));
  let selectedId = $state<string | null>(null);
  let editor = $state<{ editing: McpProfile | null; initial: ProfileDraft } | null>(null);
  let busy = $state(false);
  const CONFIG_TABS = [
    { id: 'stdio', label: '.mcp.json' },
    { id: 'http', label: 'HTTP' },
    { id: 'codex', label: 'Codex' },
  ] as const;
  let configTab = $state<'stdio' | 'http' | 'codex'>('stdio');

  // Untracked: `load` reads the store's `loaded`, so a tracked call re-ran
  // this effect when the first response flipped it and fetched twice.
  $effect(() => {
    untrack(() => void mcpProfiles.load(true));
  });

  const profiles = $derived(mcpProfiles.profiles);
  const groups = $derived(mcpProfiles.groups);
  // List/detail opens on an item: the selection, else the default.
  const selected = $derived(
    profiles.find((p) => p.id === selectedId) ?? profiles.find((p) => p.is_default) ?? profiles[0] ?? null,
  );
  const cfg = $derived(selected ? connectionConfigs(selected, baseUrl()) : null);
  const tools = $derived(selected ? profileTools(groups, selected) : []);
  const configText = $derived(
    !cfg ? '' : configTab === 'stdio' ? cfg.stdioJson : configTab === 'http' ? cfg.claudeHttp : cfg.codexToml,
  );

  async function reload(selectId?: string): Promise<void> {
    await mcpProfiles.load(true);
    if (selectId) selectedId = selectId;
  }

  function newProfile(): void {
    editor = { editing: null, initial: emptyDraft() };
  }

  function duplicate(p: McpProfile): void {
    editor = { editing: null, initial: { ...draftOf(p), name: duplicateName(p.name, profiles) } };
  }

  /** The selected profile's ⋯ button: where focus lands when the inline
   *  "Make default" button unmounts itself (it only shows for non-defaults). */
  let moreBtn = $state<HTMLButtonElement | null>(null);

  async function makeDefault(p: McpProfile, refocus = false): Promise<void> {
    busy = true;
    try {
      await mcpCpApi.cpSetDefaultProfile(p.id);
      toasts.success(`${p.name} is the default`, 'New sessions preselect it; sessions without a profile use it on their next start.');
      await reload(p.id);
      if (refocus) {
        await tick();
        moreBtn?.focus();
      }
    } catch (e) {
      toastError('Couldn’t change the default', e);
    } finally {
      busy = false;
    }
  }

  async function remove(p: McpProfile): Promise<void> {
    const tail = p.is_default ? ' It is the default — Full becomes the default again.' : '';
    if (
      !(await confirmer.ask(
        `Delete “${p.name}”? Sessions that use it fall back to their other profiles, or the default, the next time they start.${tail}`,
        { title: 'Delete MCP profile', confirmLabel: 'Delete', danger: true },
      ))
    )
      return;
    busy = true;
    try {
      await mcpCpApi.cpDeleteProfile(p.id);
      toasts.success('Profile deleted', p.name);
      selectedId = null;
      await reload();
    } catch (e) {
      toastError('Couldn’t delete the profile', e);
    } finally {
      busy = false;
    }
  }

  /** Why a profile action is greyed out for a non-admin (a disabled control is never unexplained). */
  const NEEDS_ADMIN = 'Needs MCP admin';

  function menu(e: MouseEvent | KeyboardEvent, p: McpProfile): void {
    // The locked-profile reason wins: it holds even for an admin.
    const locked = p.locked ? 'Built in — duplicate it to make your own' : undefined;
    const admin = isAdmin ? undefined : NEEDS_ADMIN;
    const items: MenuItem[] = [
      { label: 'Edit…', icon: 'edit', disabled: !isAdmin || p.locked, title: locked ?? admin, action: () => (editor = { editing: p, initial: draftOf(p) }) },
      { label: 'Duplicate…', icon: 'copy', disabled: !isAdmin, title: admin, action: () => duplicate(p) },
      { label: 'Make default', icon: 'star', disabled: !isAdmin || p.is_default, title: admin, action: () => void makeDefault(p) },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, disabled: !isAdmin || p.locked, title: (p.locked ? 'Full can’t be deleted' : undefined) ?? admin, action: () => void remove(p) },
    ];
    ctxMenu.show(e, items);
  }

  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`${what} copied`);
    } catch {
      toasts.error('Couldn’t copy', COPY_FAILED_BODY);
    }
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`profiles`,...__sveltets_2_empty({"data-testid":`mcp-profiles`}),});
   { svelteHTML.createElement("div", { "class":`head`,});
     { svelteHTML.createElement("p", { "class":`muted small intro`,});
                        
                    
                  
     }
     { svelteHTML.createElement("button", {         "class":`btn small`,...__sveltets_2_empty({"data-testid":`mcp-profile-new`}),"disabled":!isAdmin || !mcpProfiles.loaded,"title":isAdmin ? undefined : NEEDS_ADMIN,"onclick":newProfile,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
     }
   }

   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"what":`MCP profiles`,"loading":mcpProfiles.loading && !mcpProfiles.loaded,"error":mcpProfiles.error,"empty":mcpProfiles.loaded && profiles.length === 0,"rows":3,"onretry":() => void reload(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`layers`,"title":`No MCP profiles`,"body":`The built-in profiles are missing — restart the daemon to restore them.`,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("div", { "class":`split`,});
       { svelteHTML.createElement("ul", {   "class":`list`,"aria-label":`MCP profiles`,});
           for(let p of __sveltets_2_ensureArray(profiles)){p.id;
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("button", {            "class":`prof-row`,"aria-current":selected?.id === p.id ? 'true' : undefined,...__sveltets_2_empty({"data-testid":`mcp-profile-${p.id}`}),"onclick":() => (selectedId = p.id),"oncontextmenu":(e) => { e.preventDefault(); menu(e, p); },});selected?.id === p.id;
               { svelteHTML.createElement("span", { "class":`name`,});
                p.name;
                if(p.is_default){ { const $$_egdaB7C = __sveltets_2_ensureComponent(Badge); new $$_egdaB7C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":`Default`,}});}}
                if(p.locked){ { svelteHTML.createElement("span", {     "class":`lock`,"title":`Built in, read-only`,"aria-label":`Built in, read-only`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});} }}
               }
               { svelteHTML.createElement("span", { "class":`sum`,});profileSummary(groups, p); }
             }
           }
        }
       }

      if(selected && cfg){
         { svelteHTML.createElement("section", {   "class":`detail`,"aria-label":`${selected.name} profile`,});
           { svelteHTML.createElement("div", { "class":`d-head`,});
             { svelteHTML.createElement("div", { "class":`d-title`,});
               { svelteHTML.createElement("h3", {});selected.name; }
              if(selected.is_default){ { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":`Default`,}});}}
              if(selected.builtin){ { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {  "label":`Built in`,}});}}
             }
             { svelteHTML.createElement("div", { "class":`d-actions`,});
              if(!selected.is_default){
                 { svelteHTML.createElement("button", {         "class":`btn small`,"disabled":!isAdmin || busy,"title":isAdmin ? undefined : NEEDS_ADMIN,"onclick":() => void makeDefault(selected, true),...__sveltets_2_empty({"data-testid":`mcp-profile-make-default`}),});  }
              }
               { const $$_button6 = svelteHTML.createElement("button", {          "class":`icon-btn`,"aria-label":`More actions for ${selected.name}`,"title":`More…`,"aria-haspopup":`menu`,"onclick":(e) => menu(e, selected),});moreBtn = $$_button6; { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
             }
           }
          if(selected.description){ { svelteHTML.createElement("p", { "class":`muted small`,});selected.description; }}
           { svelteHTML.createElement("p", { "class":`small`,});profileSummary(groups, selected); }
          if(!selected.all_tools){
             { svelteHTML.createElement("ul", {   "class":`tool-chips`,"aria-label":`Tools in ${selected.name}`,});
                 for(let t of __sveltets_2_ensureArray(tools)){t; { svelteHTML.createElement("li", { "class":`tchip mono`,});t; }}
             }
          }

           { svelteHTML.createElement("h4", { "class":`sec`,});   }
           { svelteHTML.createElement("p", { "class":`muted small`,});
                     { svelteHTML.createElement("strong", {});cfg.label; }     
             { svelteHTML.createElement("em", {});    }    selected.locked ? ' Full is the plain all-tools connection.' : '';
           }
           { const $$_sbaT4C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT4C({ target: __sveltets_2_any(), props: {          "label":`Config format`,"size":`s`,"tabs":CONFIG_TABS,"value":configTab,"onchange":(id) => (configTab = id),}});}
           { svelteHTML.createElement("div", {   "class":`snippet`,...__sveltets_2_empty({"data-testid":`mcp-profile-config`}),});
             { svelteHTML.createElement("button", {   "class":`btn small copy`,"onclick":() => void copy(configText, `${cfg.label} config`),});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} 
             }
             { svelteHTML.createElement("pre", {});configText; }
           }
          if(configTab === 'http'){ { svelteHTML.createElement("p", { "class":`muted small`,});  { svelteHTML.createElement("code", { "class":`mono`,});cfg.httpUrl; } }}
         }
      }
     }
   LoadState}
 }

if(editor){
   { const $$_rotidEeliforP0C = __sveltets_2_ensureComponent(ProfileEditor); new $$_rotidEeliforP0C({ target: __sveltets_2_any(), props: {           "editing":editor.editing,"initial":editor.initial,profiles,groups,"onclose":() => (editor = null),"onsaved":(p) => void reload(p.id),}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ProfilesTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProfilesTab__SvelteComponent_ = ReturnType<typeof ProfilesTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProfilesTab__SvelteComponent_;