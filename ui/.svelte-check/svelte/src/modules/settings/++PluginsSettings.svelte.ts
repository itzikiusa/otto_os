///<reference types="svelte" />
;
import PageHeader from '../../lib/components/PageHeader.svelte';
import { announceShown } from '../../lib/announce.svelte';
import { sectionLabel } from './sections';
import SectionIntro from './SectionIntro.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
// Runtime custom-plugins management (root). Install from a local path or git
// URL, enable/disable (starts/stops the helper process), remove. Access for non-root
// users is granted per-plugin in Settings → Users.
import { onMount } from 'svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { api } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';
import { plugins, type PluginRecord } from '../../lib/stores/plugins.svelte';
import Icon, { asIcon } from '../../lib/components/Icon.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { loadErrorText } from '../../lib/loadError';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let list = $state<PluginRecord[]>([]);
  let loading = $state(true);
  let loadError = $state('');
  let source = $state('');
  // Which action is in flight: 'install', or a plugin slug (toggle/remove).
  let busy = $state<string | null>(null);
  // Install failures stay inline under the field (with the daemon's reason),
  // since the fix is usually editing the source.
  let installError = $state('');
  // Local-folder picker for the plugin source (daemon-host filesystem).
  let pickerOpen = $state(false);
  let sourceEl = $state<HTMLInputElement | null>(null);

  async function load(attempt = 0): Promise<void> {
    loading = true;
    try {
      list = await api.get<PluginRecord[]>('/plugin-admin');
      loadError = '';
    } catch (e) {
      // The daemon can be briefly unreachable right after an app update (it
      // restarts), which surfaces as a fetch "Load failed". Retry once on a fresh
      // connection before showing an error.
      if (attempt < 1) {
        await new Promise((r) => setTimeout(r, 600));
        return load(attempt + 1);
      }
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  onMount(() => void load());

  function errText(e: unknown): string {
    return loadErrorText(e);
  }

  /** Enabled but its service is down (failed to start, or exited). */
  function stopped(p: PluginRecord): boolean {
    return p.enabled && p.running === false;
  }

  /** Start an enabled plugin's stopped service again (re-runs enable). */
  async function startAgain(p: PluginRecord): Promise<void> {
    busy = p.slug;
    try {
      await api.post(`/plugin-admin/${p.slug}/enable`);
      toasts.success(`${p.name} is running again`);
      await load();
      await plugins.load();
    } catch (e) {
      toasts.error(`Couldn’t start ${p.name}`, errText(e));
      await load();
    } finally {
      busy = null;
    }
  }

  async function install(): Promise<void> {
    const src = source.trim();
    if (!src || busy) return;
    busy = 'install';
    installError = '';
    try {
      await api.post('/plugin-admin/install', { source: src });
      source = '';
      toasts.success('Plugin installed', 'Enable it to start its helper process.');
      await load();
      await plugins.load();
    } catch (e) {
      installError = `Couldn’t install from “${src}”. ${errText(e)}`;
    } finally {
      busy = null;
    }
  }

  async function toggle(p: PluginRecord): Promise<void> {
    // Enabling starts the plugin's helper process (its code runs as you) and
    // puts its UI in every user's sidebar — ask first. Disabling needs no ask.
    if (
      !p.enabled &&
      !(await confirmer.ask(
        `Enable “${p.name}”? Its helper process starts and runs with your permissions, and its page appears in every user’s sidebar. Only enable plugins you trust.`,
        { title: 'Enable plugin?', confirmLabel: 'Enable plugin', danger: true },
      ))
    )
      return;
    busy = p.slug;
    try {
      await api.post(`/plugin-admin/${p.slug}/${p.enabled ? 'disable' : 'enable'}`);
      await load();
      await plugins.load();
    } catch (e) {
      toasts.error(`Couldn’t ${p.enabled ? 'disable' : 'enable'} ${p.name}`, errText(e));
    } finally {
      busy = null;
    }
  }

  async function remove(p: PluginRecord): Promise<void> {
    if (
      !(await confirmer.ask(
        `Remove the plugin “${p.name}”? Its helper process stops and it leaves every user’s sidebar. Its files under ~/otto-plugins are kept, so you can install it again.`,
        { title: 'Remove plugin', confirmLabel: 'Remove', danger: true },
      ))
    )
      return;
    busy = p.slug;
    try {
      await api.del(`/plugin-admin/${p.slug}`);
      toasts.success(`Removed ${p.name}`, `Its files are kept — install it again any time.`);
      await load();
      await plugins.load();
    } catch (e) {
      toasts.error(`Couldn’t remove ${p.name}`, errText(e));
    } finally {
      busy = null;
    }
  }

  function installedLabel(ts: string): string {
    const d = new Date(ts);
    return Number.isNaN(d.getTime()) ? '' : `Installed ${d.toLocaleDateString()}`;
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":sectionLabel('plugins'),"subtitle":`Helper processes installed at runtime, no rebuild`,}});}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
                            
                  { svelteHTML.createElement("strong", {});   }
   SectionIntro}

   { svelteHTML.createElement("div", { "class":`install`,});
     { svelteHTML.createElement("label", {   "class":`sr-only`,"for":`plugin-source`,});  }
     { const $$_input3 = svelteHTML.createElement("input", {                        "id":`plugin-source`,"dir":`ltr`,"class":`input grow mono-in`,"placeholder":`~/otto-plugins/dora-metrics or https://github.com/org/plugin.git`,"bind:value":source,"spellcheck":`false`,"autocomplete":`off`,"aria-invalid":installError ? 'true' : undefined,"aria-describedby":installError ? 'plugin-install-err' : undefined,"oninput":() => (installError = ''),"onkeydown":(e) => e.key === 'Enter' && void install(),});sourceEl = $$_input3;/*Ωignore_startΩ*/() => source = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("button", {     "class":`btn`,"onclick":() => (pickerOpen = true),"title":`Browse for a local plugin folder on this Mac`,});
       { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":13,}});} 
     }
     { svelteHTML.createElement("button", {         "class":`btn primary`,"onclick":() => void install(),"disabled":busy !== null || !source.trim(),"title":source.trim() ? 'Install this plugin' : 'Enter a local path or a git URL first',});
      busy === 'install' ? 'Installing…' : 'Install';
     }
   }
  if(installError){ { svelteHTML.createElement("p", {     "class":`field-err`,"id":`plugin-install-err`,"role":`alert`,});installError; }}

   { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`plugins`,loading,"error":loadError,"empty":list.length === 0,"rows":3,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {       "icon":`box`,"title":`No plugins installed`,"body":`Plugins add pages to Otto without a rebuild — dashboards, internal tools, integrations. Paste a folder path or git URL above, or Browse…, to install one.`,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("div", { "class":`plist`,});
         for(let p of __sveltets_2_ensureArray(list)){p.slug;
         { svelteHTML.createElement("div", {  "class":`prow`,});!p.enabled;
           { svelteHTML.createElement("span", { "class":`picon`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":asIcon(p.icon, 'box'),"size":16,}});} }
           { svelteHTML.createElement("div", { "class":`pmain`,});
             { svelteHTML.createElement("div", { "class":`pname`,});
               { svelteHTML.createElement("span", {   "class":`name-text`,"title":p.name,});p.name; }
              if(p.version){ { svelteHTML.createElement("span", {     "class":`chip version`,"dir":`ltr`,"title":`v${p.version}`,}); p.version; }}
              if(stopped(p)){
                 { const $$_egdaBsutatS7C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS7C({ target: __sveltets_2_any(), props: {      "variant":`text`,"tone":`warning`,"label":`Enabled · not running`,}});}
              }else{
                 { const $$_egdaBsutatS7C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS7C({ target: __sveltets_2_any(), props: {      "variant":`text`,"tone":p.enabled ? 'success' : 'neutral',"label":p.enabled ? 'Enabled' : 'Disabled',}});}
              }
             }
            if(stopped(p)){
               {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(`${p.name}: its service isn’t running${p.last_error ? ` — ${p.last_error}` : ''}.`)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`pstopped`,});
                   p.last_error ? ` — ${p.last_error}` : '';           
               }}
            }
            if(p.description){ { svelteHTML.createElement("div", {   "class":`pdesc`,"title":p.description,});p.description; }}
             { svelteHTML.createElement("div", {     "class":`psrc mono`,"dir":`ltr`,"title":p.source,});p.slug;  p.source;if(installedLabel(p.installed_at)){' · ';installedLabel(p.installed_at);} }
           }
           { svelteHTML.createElement("div", { "class":`pactions`,});
            if(stopped(p)){
               { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":() => void startAgain(p),"disabled":busy !== null,});
                busy === p.slug ? 'Starting…' : 'Start again';
               }
            }
             { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => void toggle(p),"disabled":busy !== null,});
              busy === p.slug ? 'Working…' : p.enabled ? 'Disable' : 'Enable';
             }
             { svelteHTML.createElement("button", {           "class":`icon-btn danger-icon`,"onclick":() => void remove(p),"disabled":busy !== null,"aria-label":`Remove ${p.name}`,"title":`Remove ${p.name}`,});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
             }
           }
           { svelteHTML.createElement("details", { "class":`plugin-details`,});
             { svelteHTML.createElement("summary", {});  }
             { svelteHTML.createElement("dl", {});
               { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});p.name; }
               { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "dir":`ltr`,});p.version || 'Not specified'; }
               { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {   "class":`mono`,"dir":`ltr`,});p.source; }
               { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {   "class":`mono`,"dir":`ltr`,});p.slug; }
              if(p.description){ { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});p.description; }}
             }
           }
         }
      }
     }
   LoadState}
   PageBody}
 }

if(pickerOpen){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {         "title":`Choose a local plugin folder`,"start":`~/otto-plugins`,"onpick":(p) => {
      source = p;
      installError = '';
      pickerOpen = false;
      queueMicrotask(() => sourceEl?.focus());
    },"onclose":() => (pickerOpen = false),}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PluginsSettings__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PluginsSettings__SvelteComponent_ = ReturnType<typeof PluginsSettings__SvelteComponent_>;
/*Ωignore_endΩ*/export default PluginsSettings__SvelteComponent_;