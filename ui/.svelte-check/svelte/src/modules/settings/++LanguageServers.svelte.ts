///<reference types="svelte" />
;
import PageHeader from '../../lib/components/PageHeader.svelte';
import { NO_WORKSPACE } from '../../lib/labels';
import { plural } from '../../lib/plural';
import { sectionLabel } from './sections';
import PageBody from '../../lib/components/PageBody.svelte';
// Settings → Language Servers: shows LSP server availability and lets users
// install missing servers via a spawned shell session.
import { api } from '../../lib/api/client';
import { ws } from '../../lib/stores/workspace.svelte';
import { toasts } from '../../lib/toast.svelte';
import type { LspCapabilities, LspServerStatus } from '../../lib/api/types';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import SectionIntro from './SectionIntro.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
import { loadErrorText } from '../../lib/loadError';
import type { Session } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ── State ──────────────────────────────────────────────────────────────────

  let caps: LspCapabilities | null = $state(null);
  let loading = $state(true);
  let error = $state('');
  let installing: Set<string> = $state(new Set());
  let installingAll = $state(false);

  // ── Load ───────────────────────────────────────────────────────────────────

  $effect(() => {
    void load();
  });

  async function load(): Promise<void> {
    loading = true;
    error = '';
    try {
      caps = await api.get<LspCapabilities>('/lsp/capabilities');
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  // ── Install helpers ────────────────────────────────────────────────────────

  const wsId = $derived(ws.currentId);

  async function installLang(lang: string, command: string): Promise<void> {
    if (!wsId) { toasts.error("Couldn’t start the install", `${NO_WORKSPACE}.`); return; }
    // It installs software on this Mac: say exactly what runs first.
    const ok = await confirmer.ask(
      `Run “${command}” in a new terminal session? It installs the ${langLabel(lang)} language server on this Mac.`,
      { title: `Install ${langLabel(lang)} server`, confirmLabel: 'Install', danger: false },
    );
    if (!ok) return;
    installing = new Set([...installing, lang]);
    try {
      const session = await api.post<Session>(`/workspaces/${wsId}/lsp/install`, { langs: [lang] });
      ws.addSession(session); // navigates to the install session
      toasts.info(`Installing the ${langLabel(lang)} server`, 'Follow it in the terminal session, then Refresh here.');
    } catch (e) {
      toasts.error("Couldn’t start the install", loadErrorText(e));
    } finally {
      installing = new Set([...installing].filter((l) => l !== lang));
    }
  }

  async function installAll(): Promise<void> {
    if (!wsId) { toasts.error("Couldn’t start the install", `${NO_WORKSPACE}.`); return; }
    const cmds = missingWithInstall.map((s) => s.install_command).join('\n');
    const ok = await confirmer.ask(
      `Run these in a new terminal session to install ${plural(missingWithInstall.length, 'missing language server')} on this Mac?\n\n${cmds}`,
      { title: 'Install missing servers', confirmLabel: 'Install', danger: false },
    );
    if (!ok) return;
    installingAll = true;
    try {
      const session = await api.post<Session>(`/workspaces/${wsId}/lsp/install`, {});
      ws.addSession(session); // navigates to the install session
      toasts.info('Installing the missing servers', 'Follow it in the terminal session, then Refresh here.');
    } catch (e) {
      toasts.error("Couldn’t start the install", loadErrorText(e));
    } finally {
      installingAll = false;
    }
  }

  // ── Derived ────────────────────────────────────────────────────────────────

  const missingWithInstall = $derived(
    (caps as LspCapabilities | null)?.servers.filter((s: LspServerStatus) => !s.available && s.install_command) ?? [],
  );

  // Language display names
  const LANG_LABELS: Record<string, string> = {
    go: 'Go',
    python: 'Python',
    typescript: 'TypeScript',
    javascript: 'JavaScript',
    rust: 'Rust',
    json: 'JSON',
    html: 'HTML',
    css: 'CSS',
    markdown: 'Markdown',
    java: 'Java',
    typescriptreact: 'TypeScript (React)',
    javascriptreact: 'JavaScript (React)',
    scss: 'SCSS',
  };

  function langLabel(lang: string): string {
    return LANG_LABELS[lang] ?? lang;
  }

  async function copyCmd(cmd: string): Promise<void> {
    try {
      await copyTextOrThrow(cmd);
      toasts.success('Install command copied');
    } catch {
      toasts.error("Couldn’t copy", 'Select the command and copy it manually.');
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('language-servers'),"subtitle":`Code intelligence for Otto’s editors`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {       "class":`btn small`,...__sveltets_2_empty({"data-icon":`refresh`}),"disabled":loading,"onclick":() => void load(),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} loading ? 'Checking…' : 'Refresh';
       }
      if(missingWithInstall.length > 0){
         { svelteHTML.createElement("button", {     "class":`btn small primary`,"disabled":installingAll,"onclick":installAll,});
          installingAll ? 'Starting…' : `Install missing (${missingWithInstall.length})…`;
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});                    { svelteHTML.createElement("code", {});  }  { svelteHTML.createElement("code", {});  }        SectionIntro}

   { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"what":`language servers`,loading,error,"empty":!caps || caps.servers.length === 0,"onretry":() => void load(),"rows":5,emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`function`,"title":`No language servers configured`,"body":`This daemon doesn’t list any supported languages.`,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
    
    if(caps){
       { svelteHTML.createElement("div", { "class":`table-wrap card`,});
         { svelteHTML.createElement("table", { "class":`ls-table`,});
           { svelteHTML.createElement("thead", {});
             { svelteHTML.createElement("tr", {});
               { svelteHTML.createElement("th", {});  }
               { svelteHTML.createElement("th", {});  }
               { svelteHTML.createElement("th", {});  }
               { svelteHTML.createElement("th", {}); { svelteHTML.createElement("span", { "class":`sr-only`,});  } }
             }
           }
           { svelteHTML.createElement("tbody", {});
               for(let server of __sveltets_2_ensureArray(caps.servers)){server.lang;
               { svelteHTML.createElement("tr", {});
                 { svelteHTML.createElement("td", { "class":`lang-name`,});langLabel(server.lang); }
                 { svelteHTML.createElement("td", {});
                  if(server.available){
                     { svelteHTML.createElement("span", { "class":`chip ok`,}); { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  }
                  }else{
                     { svelteHTML.createElement("span", { "class":`chip`,});  }
                  }
                 }
                 { svelteHTML.createElement("td", { "class":`cmd-cell`,});
                  if(server.available){
                     { svelteHTML.createElement("span", {   "class":`mono path`,"title":server.command,});server.command; }
                  } else if (server.install_command){
                     { svelteHTML.createElement("button", {     "class":`mono path copy`,"title":`Copy: ${server.install_command}`,"onclick":() => void copyCmd(server.install_command ?? ''),});
                      server.install_command;
                     }
                  }else{
                     { svelteHTML.createElement("span", {   "class":`mono path dim`,"title":server.command,});server.command;     }
                  }
                 }
                 { svelteHTML.createElement("td", { "class":`action-cell`,});
                  if(!server.available && server.install_command){
                     { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":installing.has(server.lang),"onclick":() => installLang(server.lang, server.install_command ?? ''),});
                      installing.has(server.lang) ? 'Starting…' : 'Install…';
                     }
                  }
                 }
               }
            }
           }
         }
       }
    }
   LoadState}
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LanguageServers__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LanguageServers__SvelteComponent_ = ReturnType<typeof LanguageServers__SvelteComponent_>;
/*Ωignore_endΩ*/export default LanguageServers__SvelteComponent_;