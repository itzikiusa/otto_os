///<reference types="svelte" />
;
import { NO_WORKSPACE } from '../../lib/labels';
// Info tab: shows active session metadata + the attached tracker issue (Jira or Linear).
import { ws } from '../../lib/stores/workspace.svelte';
import { toastError } from '../../lib/toastError';
import { auth } from '../../lib/stores/auth.svelte';
import { toasts } from '../../lib/toast.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { events } from '../../lib/events.svelte';
import { sessionState } from '../../lib/status';
import AttachIssue from '../agents/AttachIssue.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import type { AttachedIssue } from '../../lib/api/types';
import { allProviders } from '../../lib/providers';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import { confirmer } from '../../lib/confirm.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const session = $derived(ws.activeSession);
  const workspace = $derived(ws.current);
  const attachedIssue = $derived(
    (session?.meta?.issue as AttachedIssue | undefined) ?? null,
  );
  // The same live state the tab strip and navigator show (events-fed status,
  // needs-you flag, reconnecting) — not the row's raw `status` enum.
  const liveState = $derived(
    session
      ? sessionState(session, ws.statusMap[session.id], ws.needsYou[session.id] === true, {
          stale: events.state !== 'connected',
        })
      : null,
  );

  // Per-workspace default agent (overrides the global default). '' = inherit.
  const providers = $derived(allProviders());
  const wsDefaultAgent = $derived(
    typeof workspace?.settings?.default_provider === 'string'
      ? (workspace.settings.default_provider as string)
      : '',
  );

  async function onDefaultAgentChange(e: Event): Promise<void> {
    const value = (e.currentTarget as HTMLSelectElement).value;
    try {
      await ws.saveDefaultAgent(value);
      toasts.info(
        value === '' ? 'Workspace uses the global default agent' : `Workspace default agent: ${value}`,
      );
    } catch (err) {
      toastError('Couldn’t save default agent', err);
    }
  }

  // extra_dirs from meta — always a string array
  const extraDirs = $derived(
    Array.isArray(session?.meta?.extra_dirs)
      ? (session!.meta!.extra_dirs as string[])
      : [],
  );

  let attachOpen = $state(false);
  let folderPickerOpen = $state(false);

  function fmt(iso: string): string {
    try {
      return new Date(iso).toLocaleString(undefined, {
        dateStyle: 'medium',
        timeStyle: 'short',
      });
    } catch {
      return iso;
    }
  }

  async function detach(): Promise<void> {
    if (!session) return;
    try {
      await ws.detachIssue(session.id);
      toasts.info('Issue detached');
    } catch (e) {
      toastError('Couldn’t detach the issue', e);
    }
  }

  /** Save the folder list and restart so `--add-dir` takes effect. A WORKING
   *  agent would lose its in-flight turn, so that one case asks first — the
   *  same rule as every other restart path (`ws.requestRestart`); this one
   *  used to restart it silently. Restarting through `ws.restartSession` also
   *  bumps the restart nonce, so the terminal re-attaches to the new PTY. */
  async function applyDirs(dirs: string[], verb: 'added' | 'removed'): Promise<void> {
    if (!session) return;
    const id = session.id;
    if (ws.statusMap[id] === 'working') {
      const ok = await confirmer.ask(
        `“${session.title}” is working right now. Changing its folders restarts it, which stops its current turn (it resumes its saved conversation where it can).`,
        { title: 'Restart working session?', confirmLabel: `Restart and ${verb === 'added' ? 'add' : 'remove'} folder`, danger: true },
      );
      if (!ok) return;
    }
    try {
      await ws.updateSessionMeta(id, { extra_dirs: dirs });
      await ws.restartSession(id, { quiet: true });
      toasts.success(verb === 'added' ? 'Folder added' : 'Folder removed', 'The session restarted with the new folders.');
    } catch (e) {
      toastError(verb === 'added' ? 'Couldn’t add the folder' : 'Couldn’t remove the folder', e);
    }
  }

  async function addDir(path: string): Promise<void> {
    folderPickerOpen = false;
    if (extraDirs.includes(path)) {
      toasts.info('Already added', path);
      return;
    }
    await applyDirs([...extraDirs, path], 'added');
  }

  async function removeDir(dir: string): Promise<void> {
    await applyDirs(extraDirs.filter((d) => d !== dir), 'removed');
  }
;
async () => {

if(!session && !workspace){
   { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`info`,"title":NO_WORKSPACE,"body":`Its details show here.`,}});}
}else{
   { svelteHTML.createElement("div", { "class":`info`,});
    if(workspace){
       { svelteHTML.createElement("section", { "class":`section`,});
         { svelteHTML.createElement("div", { "class":`section-title`,});  }
         { svelteHTML.createElement("div", { "class":`info-row`,});
           { svelteHTML.createElement("span", { "class":`key`,});  }
           { svelteHTML.createElement("span", { "class":`val`,});
             { svelteHTML.createElement("select", {       "aria-label":`Default agent`,"class":`ws-select`,"value":wsDefaultAgent,"onchange":onDefaultAgentChange,});
               { svelteHTML.createElement("option", {"value":"",});  }
                 for(let p of __sveltets_2_ensureArray(providers)){p;
                 { svelteHTML.createElement("option", { "value":p,});p; }
              }
             }
           }
         }
         { svelteHTML.createElement("p", { "class":`hint`,});               }
       }
    }

    if(session){
     { svelteHTML.createElement("section", { "class":`section`,});
       { svelteHTML.createElement("div", { "class":`section-title`,});  }
       { svelteHTML.createElement("div", { "class":`info-row`,});
         { svelteHTML.createElement("span", { "class":`key`,});  }
         { svelteHTML.createElement("span", { "class":`val`,});session.title; }
       }
       { svelteHTML.createElement("div", { "class":`info-row`,});
         { svelteHTML.createElement("span", { "class":`key`,});  }
         { svelteHTML.createElement("span", { "class":`val val-provider`,}); { const $$_nocIredivorP4C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP4C({ target: __sveltets_2_any(), props: {    "provider":session.provider,"size":13,}});}session.provider; }
       }
       { svelteHTML.createElement("div", { "class":`info-row`,});
         { svelteHTML.createElement("span", { "class":`key`,});  }
         { svelteHTML.createElement("span", { "class":`val`,});if(liveState){ { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {    "status":liveState,"variant":`text`,}});}} }
       }
      if(session.cwd){
         { svelteHTML.createElement("div", { "class":`info-row`,});
           { svelteHTML.createElement("span", { "class":`key`,});  }
           { svelteHTML.createElement("span", {   "class":`val mono cwd`,"title":session.cwd,});session.cwd; }
         }
      }
       { svelteHTML.createElement("div", { "class":`info-row`,});
         { svelteHTML.createElement("span", { "class":`key`,});  }
         { svelteHTML.createElement("span", { "class":`val`,});fmt(session.created_at); }
       }
     }

    if(session.kind === 'agent'){
       { svelteHTML.createElement("section", { "class":`section`,});
         { svelteHTML.createElement("div", { "class":`section-title`,});  }
        if(extraDirs.length > 0){
           { svelteHTML.createElement("ul", { "class":`dir-list`,});
               for(let dir of __sveltets_2_ensureArray(extraDirs)){dir;
               { svelteHTML.createElement("li", { "class":`dir-row`,});
                 { svelteHTML.createElement("span", {   "class":`dir-path mono`,"title":dir,});dir; }
                 { svelteHTML.createElement("button", {         "class":`icon-btn dir-remove`,"title":`Remove ${dir} (restarts the session)`,"aria-label":`Remove ${dir} (restarts the session)`,"onclick":() => removeDir(dir),});
                   { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
                 }
               }
            }
           }
        }else{
           { svelteHTML.createElement("p", { "class":`dim no-dirs`,});   }
        }
         { svelteHTML.createElement("button", {   "class":`btn small add-dir-btn`,"onclick":() => (folderPickerOpen = true),});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
         }
         { svelteHTML.createElement("p", { "class":`hint`,});        }
       }
    }

     { svelteHTML.createElement("section", { "class":`section`,});
       { svelteHTML.createElement("div", { "class":`section-title`,});  }
      if(attachedIssue){
         { svelteHTML.createElement("div", { "class":`issue-card`,});
           { svelteHTML.createElement("div", { "class":`issue-head`,});
            
            if(attachedIssue.url){
               { svelteHTML.createElement("a", {           "class":`issue-key`,"href":attachedIssue.url,"target":`_blank`,"rel":`noopener noreferrer`,"title":`Open in browser`,});
                attachedIssue.key;
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}
               }
            }else{
               { svelteHTML.createElement("span", { "class":`issue-key plain`,});attachedIssue.key; }
            }
            if(attachedIssue.status){
               { svelteHTML.createElement("span", { "class":`chip`,});attachedIssue.status; }
            }
           }
           { svelteHTML.createElement("div", { "class":`issue-summary`,});attachedIssue.summary; }
           { svelteHTML.createElement("div", { "class":`issue-actions`,});
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (attachOpen = true),});  }
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":detach,});  }
           }
         }
      }else{
         { svelteHTML.createElement("div", { "class":`no-issue dim`,});
           { svelteHTML.createElement("p", {});   }
          
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => (attachOpen = true),});
             
           }
         }
      }
     }
    }
   }
}

if(attachOpen && session){
   { const $$_eussIhcattA0C = __sveltets_2_ensureComponent(AttachIssue); new $$_eussIhcattA0C({ target: __sveltets_2_any(), props: {    "sessionId":session.id,"onclose":() => (attachOpen = false),}});}
}

if(folderPickerOpen && session){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {         "title":`Add folder`,"start":session.cwd,"onpick":(path) => addDir(path),"onclose":() => (folderPickerOpen = false),}});}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const InfoPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type InfoPanel__SvelteComponent_ = ReturnType<typeof InfoPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default InfoPanel__SvelteComponent_;