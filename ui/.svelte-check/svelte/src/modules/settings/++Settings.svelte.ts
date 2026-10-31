///<reference types="svelte" />
;
import { rowMenu as rowMenuKeys } from '../../lib/rowMenu';
// Settings layout: subnav + routed page (#/settings/<page>). The section
// list itself lives in ./sections.ts (labels, groups, keywords, access) so
// the nav, each page title and the ⌘K "Settings: …" commands can't drift.
// Appearance is the default section, so it ships with the page; every other
// section is its own chunk (see VIEWS below).
import Appearance from './Appearance.svelte';
import { router } from '../../lib/router.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import Icon from '../../lib/components/Icon.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LazyMount from '../../lib/components/LazyMount.svelte';
import { lazyComponent, type LazyComponent } from '../../lib/lazy-component.svelte';
import type { Component } from 'svelte';
import {
    SETTINGS_GROUPS,
    availableSections,
    canOpenSection,
    filterSections,
    findSection,
    groupLabel,
    type SettingsSection,
    type SettingsSectionId,
  } from './sections';
function $$render() {
 const navRow/*Ωignore_positionΩ*/ = (s: SettingsSection, detail?: string)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   {const $$action_0 = __sveltets_2_ensureAction(rowMenuKeys(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {               "class":`settings-nav-item`,"aria-current":pageId === s.id ? 'page' : undefined,"onclick":() => open(s),"onpointerenter":() => VIEWS[s.id].prefetch(),"onfocus":() => VIEWS[s.id].prefetch(),"oncontextmenu":(e) => rowMenu(e, s),});pageId === s.id;
     { svelteHTML.createElement("span", { "class":`settings-nav-label`,});s.label; }
    if(detail){ { svelteHTML.createElement("span", { "class":`settings-nav-detail`,});detail; }}
   }}
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Explicit exit: go back to where you were, or home (Agents) if there's no
  // history — so you don't have to click another module to leave Settings.
  function exit(): void {
    if (router.canBack) router.back();
    else router.go('agents');
  }

  // The section components, keyed by registry id — the only per-section thing
  // that needs Svelte. The Record type makes a new registry entry without a
  // view a compile error. Each section is a lazy chunk (one promise each, see
  // lib/lazy-component.svelte.ts): a static map dragged all 28 into the
  // Settings chunk — CodeMirror via Context Library, the whole Database store
  // via Appearance — for a page that shows one section at a time.
  const VIEWS: Record<SettingsSectionId, LazyComponent<Component>> = {
    appearance: lazyComponent(async () => ({ default: Appearance })),
    'session-names': lazyComponent(() => import('./SessionNames.svelte')),
    notifications: lazyComponent(() => import('./Notifications.svelte')),
    snipping: lazyComponent(() => import('./SnipSettings.svelte')),
    browser: lazyComponent(() => import('./BrowserSettings.svelte')),
    tokens: lazyComponent(() => import('./PersonalAccessTokens.svelte')),
    'git-accounts': lazyComponent(() => import('../git/GitAccounts.svelte')),
    jira: lazyComponent(() => import('./IssueAccounts.svelte')),
    channels: lazyComponent(() => import('./Channels.svelte')),
    'mcp-servers': lazyComponent(() => import('./McpServers.svelte')),
    'language-servers': lazyComponent(() => import('./LanguageServers.svelte')),
    sharing: lazyComponent(() => import('./EmailSenderSetup.svelte')),
    assistant: lazyComponent(() => import('./AssistantSettings.svelte')),
    providers: lazyComponent(() => import('./Providers.svelte')),
    'context-soul': lazyComponent(() => import('./ContextSoul.svelte')),
    'self-improvement': lazyComponent(() => import('./SelfImprovement.svelte')),
    insights: lazyComponent(() => import('./InsightsSettings.svelte')),
    skills: lazyComponent(() => import('./SkillsLibrary.svelte')),
    'skill-eval': lazyComponent(() => import('./SkillEvalSettings.svelte')),
    'context-library': lazyComponent(() => import('./ContextLibrary.svelte')),
    daemon: lazyComponent(() => import('./Daemon.svelte')),
    plugins: lazyComponent(() => import('./PluginsSettings.svelte')),
    'trust-safety': lazyComponent(() => import('./TrustSafety.svelte')),
    logs: lazyComponent(() => import('./Logs.svelte')),
    backup: lazyComponent(() => import('./BackupRestore.svelte')),
    users: lazyComponent(() => import('./Users.svelte')),
    'access-groups': lazyComponent(() => import('./AccessGroups.svelte')),
    sessions: lazyComponent(() => import('./AdminSessions.svelte')),
  };

  const pageId = $derived(router.parts[1] || 'appearance');
  const section = $derived(findSection(pageId));
  // A routed section is openable, known-but-forbidden (a stale link or a role
  // change) or unknown — the last two get an explicit empty state instead of
  // silently rendering Appearance with nothing selected in the nav.
  const View = $derived(section && canOpenSection(section, auth) ? VIEWS[section.id] : null);

  const sections = $derived(availableSections(auth));
  // Grouped like the main sidebar (a macOS source list); sections the role
  // can't open are simply absent.
  const groups = $derived(
    SETTINGS_GROUPS.map((g) => ({ ...g, items: sections.filter((s) => s.group === g.id) })).filter(
      (g) => g.items.length > 0,
    ),
  );

  // ---- filter ----
  // Typing narrows the nav to a flat, ranked result list (label matches
  // first, group as dim detail); Enter opens the first result, ↓ walks into
  // the results, Esc clears.
  let query = $state('');
  let filterEl = $state<HTMLInputElement | null>(null);
  let listEl = $state<HTMLDivElement | null>(null);
  const results = $derived(query.trim() ? filterSections(sections, query) : null);

  function open(s: SettingsSection): void {
    router.go(`settings/${s.id}`);
  }

  function navItems(): HTMLButtonElement[] {
    return listEl ? [...listEl.querySelectorAll<HTMLButtonElement>('.settings-nav-item')] : [];
  }

  function onFilterKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      const first = results?.[0];
      if (first) {
        e.preventDefault();
        open(first);
      }
    } else if (e.key === 'Escape') {
      // Esc clears first; a second Esc leaves the field.
      e.preventDefault();
      e.stopPropagation();
      if (query) query = '';
      else filterEl?.blur();
    } else if (e.key === 'ArrowDown') {
      const first = navItems()[0];
      if (first) {
        e.preventDefault();
        first.focus();
      }
    }
  }

  // Arrow keys move between rows; ↑ from the first row returns to the filter,
  // and typing a character on a row carries it into the filter.
  function onListKey(e: KeyboardEvent): void {
    const items = navItems();
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    if (i < 0) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      items[Math.min(i + 1, items.length - 1)]?.focus();
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (i === 0) filterEl?.focus();
      else items[i - 1]?.focus();
    } else if (e.key === 'Escape' && query) {
      e.preventDefault();
      e.stopPropagation();
      query = '';
      filterEl?.focus();
    } else if (e.key.length === 1 && e.key !== ' ' && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      query += e.key;
      filterEl?.focus();
    }
  }

  function rowMenu(e: MouseEvent, s: SettingsSection): void {
    ctxMenu.show(e, [{ label: `Open ${s.label}`, icon: 'gear', action: () => open(s) }]);
  }

  /** Plain-words names for the feature grants (never the raw snake_case id). */
  const FEATURE_LABEL: Record<string, string> = {
    agents: 'Agents',
    connections: 'Connections',
    database: 'the Database Explorer',
    git: 'Git',
    issues: 'Issues',
    product: 'Product',
    swarm: 'Swarm',
    api_client: 'the API client',
    workflows: 'Workflows',
    channels: 'Channels',
    skill_eval: 'Skills evaluator',
    skills: 'Skills',
    insights: 'Insights',
    usage: 'Usage',
    self_improvement: 'Self-improvement',
    context: 'the Context library',
    settings: 'Settings',
    users: 'Users',
    canvas: 'Canvas',
    design: 'Design Hall',
    proof_pack: 'Proof packs',
    mcp: 'the MCP control plane',
    mission_control: 'Mission Control',
    scheduled_tasks: 'Scheduled tasks',
    run_with_otto: 'Run with Otto',
    browser: 'the Browser',
    aws: 'AWS',
    aws_s3: 'AWS S3',
    aws_sqs: 'AWS SQS',
    aws_ec2: 'AWS EC2',
    aws_athena: 'AWS Athena',
    aws_eks: 'AWS EKS',
    aws_rds: 'AWS RDS',
    kubernetes: 'Kubernetes',
  };

  function gateHint(s: SettingsSection): string {
    const g = s.gate;
    if (g === 'root') return 'Only the root account can open this section.';
    if (g == null) return '';
    return `Ask a workspace admin for ${g.level} access to ${FEATURE_LABEL[g.feature] ?? g.feature.replace(/_/g, ' ')}.`;
  }
;
async () => {



 { svelteHTML.createElement("div", { "class":`settings`,});
   { svelteHTML.createElement("nav", {   "class":`settings-nav`,"aria-label":`Settings sections`,});
     { svelteHTML.createElement("div", { "class":`settings-nav-title`,});
       { svelteHTML.createElement("span", {});  }
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Close settings (⌘⇧←)`,"aria-label":`Close settings`,"onclick":exit,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
       }
     }
     { svelteHTML.createElement("label", { "class":`settings-nav-filter`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
       { const $$_input3 = svelteHTML.createElement("input", {                     "dir":`ltr`,"bind:value":query,"class":`settings-filter-input`,"type":`search`,"placeholder":`Filter settings…`,"aria-label":`Filter settings`,"aria-controls":`settings-nav-list`,"autocomplete":`off`,"spellcheck":`false`,"onkeydown":onFilterKey,});filterEl = $$_input3;/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
    
    
     { const $$_div2 = svelteHTML.createElement("div", {      "class":`settings-nav-list`,"id":`settings-nav-list`,"onkeydown":onListKey,});listEl = $$_div2;
      if(results){
           for(let s of __sveltets_2_ensureArray(results)){s.id;
          ;__sveltets_2_ensureSnippet(navRow(s, groupLabel(s.group)));
        }
           { svelteHTML.createElement("p", { "class":`settings-nav-empty`,});   query.trim();  }
        
      }else{
           for(let g of __sveltets_2_ensureArray(groups)){g.id;
           { svelteHTML.createElement("div", {     "class":`settings-nav-group`,"role":`group`,"aria-label":g.label,});
             { svelteHTML.createElement("div", {   "class":`settings-nav-heading`,"aria-hidden":`true`,});g.label; }
               for(let s of __sveltets_2_ensureArray(g.items)){s.id;
              ;__sveltets_2_ensureSnippet(navRow(s));
            }
           }
        }
      }
     }
   }

   { svelteHTML.createElement("div", { "class":`settings-body`,});
    if(View){
       { const $$_tnuoMyzaL2C = __sveltets_2_ensureComponent(LazyMount); new $$_tnuoMyzaL2C({ target: __sveltets_2_any(), props: {    "lazy":View,"what":`${section?.label ?? 'these'} settings`,}});}
    } else if (section){
       { const $$_redaeHegaP2C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP2C({ target: __sveltets_2_any(), props: {  "title":section.label,}});}
       { const $$_ydoBegaP2C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
         { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"variant":`page`,"icon":`lock`,"title":`You don’t have access to ${section.label}`,"body":gateHint(section),"actionLabel":`Open Appearance`,"onaction":() => router.go('settings/appearance'),}});
          if(router.canBack){
             { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => router.back(),});  }
          }
         EmptyState}
       PageBody}
    }else{
       { const $$_redaeHegaP2C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP2C({ target: __sveltets_2_any(), props: {  "title":`Settings`,}});}
       { const $$_ydoBegaP2C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
         { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {             "variant":`page`,"icon":`gear`,"title":`No settings section called “${pageId}”`,"body":`The link may be out of date. Pick a section from the list, or filter it by name.`,"actionLabel":`Open Appearance`,"onaction":() => router.go('settings/appearance'),}});}
       PageBody}
    }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Settings__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Settings__SvelteComponent_ = ReturnType<typeof Settings__SvelteComponent_>;
/*Ωignore_endΩ*/export default Settings__SvelteComponent_;