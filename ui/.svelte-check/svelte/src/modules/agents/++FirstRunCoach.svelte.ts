///<reference types="svelte" />
;// First-run coach: a guided checklist shown on the Agents page when the
// workspace has no agent sessions yet. It walks a fresh user from "just
// finished setup" to "first agent running" by (1) confirming an agent CLI is
// installed, (2) ensuring a workspace exists, (3) optionally installing a
// couple of recommended bundled skills, and (4) launching a first session
// seeded with a friendly starter prompt. Every step reuses an existing API —
// no new backend is invented here. Dismissable; the dismissal is remembered
// per-machine so it doesn't nag after the user has found their footing.

import { contextApi } from '../../lib/api/context';
import { announceShown } from '../../lib/announce.svelte';
import { toastError } from '../../lib/toastError';
import type { BundledSkill } from '../../lib/api/types';
import { auth } from '../../lib/stores/auth.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { router } from '../../lib/router.svelte';
import { toasts } from '../../lib/toast.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import { api } from '../../lib/api/client';
import { checkFolder } from '../../lib/folderCheck';
import { agentProviders as registryAgentProviders, providerReadiness } from '../../lib/providers';
;

  interface Props {
    /** Called when the user dismisses the coach (falls back to the bare empty state). */
    ondismiss: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { ondismiss }: Props = $props();

  // --- Step 1: agent CLIs ----------------------------------------------------
  // `providers` is the STATIC provider registry — it always lists claude/codex/agy
  // regardless of what's installed, so it can't gate the launch. Real detection
  // lives in /meta.tools: a coding agent is usable only when its tool is `found`.
  const agentProviders = $derived(registryAgentProviders());
  // Use the same registry and readiness model as New Session and Handover.
  const agentTools = $derived(
    agentProviders.map((name) => ({ name, found: providerReadiness(name).available, checked: providerReadiness(name).checked, version: providerReadiness(name).version })),
  );
  // The gate: at least one agent CLI is actually installed on PATH.
  const hasAgentCli = $derived(agentTools.some((t) => t.found));
  // The provider names whose CLI is actually present.
  const foundProviders = $derived(agentTools.filter((t) => t.found).map((t) => t.name));
  let checkingProviders = $state(false);
  let providerCheckFailed = $state(false);

  async function recheckProviders(): Promise<void> {
    if (checkingProviders) return;
    checkingProviders = true;
    providerCheckFailed = false;
    try {
      providerCheckFailed = !(await auth.refreshMeta());
    } finally {
      checkingProviders = false;
    }
  }

  // --- Step 2: workspace -----------------------------------------------------
  const hasWorkspace = $derived(ws.workspaces.length > 0 && ws.current !== null);
  let wsName = $state('');
  // Empty (not `~/`): the home folder must not be one click from becoming the
  // workspace root. Typed paths are checked like onboarding's (folderCheck).
  let wsPath = $state('');
  let pathError = $state('');
  let wsBusy = $state(false);
  let pickerOpen = $state(false);
  // Suggest a name from the trailing path segment until the user types one.
  let wsNameTouched = $state(false);
  $effect(() => {
    if (wsNameTouched) return;
    const seg = wsPath.replace(/\/+$/, '').split('/').pop() ?? '';
    if (seg !== '' && seg !== '~') wsName = seg;
  });
  const canCreateWs = $derived(wsName.trim() !== '' && wsPath.trim() !== '' && !wsBusy);

  async function createWorkspace(): Promise<void> {
    if (!canCreateWs) return;
    wsBusy = true;
    pathError = '';
    try {
      const check = await checkFolder(wsPath, (url) => api.get(url));
      if (!check.ok) {
        pathError = check.message;
        return;
      }
      wsPath = check.path;
      const w = await ws.createWorkspace(wsName, wsPath);
      toasts.success('Workspace created', w.name);
    } catch (e) {
      toastError('Couldn’t create workspace', e);
    } finally {
      wsBusy = false;
    }
  }

  // --- Step 3: recommended skills (optional) ---------------------------------
  // A short, opinionated set of bundled skills that pay off on a first run.
  // We only surface the ones Otto actually ships (matched against /library/bundled),
  // so the list never advertises a skill that isn't installable.
  const RECOMMENDED = ['grill', 'correctness-review', 'security-review', 'insights'];
  let bundled: BundledSkill[] = $state([]);
  let skillsLoaded = $state(false);
  let skillsLoading = $state(false);
  let skillsError = $state<string | null>(null);
  let skillBusy: Set<string> = $state(new Set());
  const recommendedSkills = $derived(
    RECOMMENDED.map((n) => bundled.find((b) => b.name === n)).filter(
      (b): b is BundledSkill => b !== undefined,
    ),
  );

  $effect(() => {
    if (skillsLoaded || skillsLoading || !auth.isRoot) return;
    void loadSkills();
  });

  async function loadSkills(): Promise<void> {
    if (skillsLoading) return;
    skillsLoading = true;
    skillsError = null;
    try {
      bundled = await contextApi.listBundled();
    } catch (e) {
      // Optional setup must stay recoverable without blocking the first agent.
      skillsError = loadErrorText(e);
    } finally {
      skillsLoaded = true;
      skillsLoading = false;
    }
  }

  function setSkillBusy(name: string, on: boolean): void {
    const next = new Set(skillBusy);
    if (on) next.add(name);
    else next.delete(name);
    skillBusy = next;
  }

  async function installSkill(s: BundledSkill): Promise<void> {
    setSkillBusy(s.name, true);
    try {
      await contextApi.installBundled(s.name);
      toasts.success('Skill installed', s.name);
      await loadSkills();
    } catch (e) {
      toastError('Couldn’t install the skill', e);
    } finally {
      setSkillBusy(s.name, false);
    }
  }

  // --- Step 4: launch first session ------------------------------------------
  // Friendly starter prompt: orients the agent at the workspace and asks for a
  // grounded first task that produces immediate, visible value.
  const STARTER_PROMPT =
    'You are starting fresh in this repository. Give me a short, friendly orientation: ' +
    "summarise what this project is, the main folders, and how to build/run/test it. " +
    "Then suggest 3 small first tasks I could ask you to do. Keep it concise.";

  let launchBusy = $state(false);

  // First useful provider: prefer one whose CLI is actually installed — the
  // global default if it's installed, else claude, else the first found agent.
  // Falls back to the registry only if detection is somehow empty.
  const launchProvider = $derived.by(() => {
    const def = auth.meta?.default_provider;
    if (def && foundProviders.includes(def)) return def;
    if (foundProviders.includes('claude')) return 'claude';
    if (foundProviders.length > 0) return foundProviders[0];
    return agentProviders.includes('claude') ? 'claude' : (agentProviders[0] ?? '');
  });

  async function launchFirstSession(): Promise<void> {
    if (launchBusy || !hasWorkspace || !hasAgentCli) return;
    launchBusy = true;
    try {
      // The daemon delivers the starter prompt once the CLI is actually ready
      // (cold start, trust dialog) — a fixed client timer used to race it and
      // drop the prompt (A6). `route` → addSession → navigateToSession.
      await ws.openSessionWithPrompt(
        {
          provider: launchProvider,
          title: 'First session',
          cwd: ws.current?.root_path ?? null,
          prompt: STARTER_PROMPT,
          meta: { source: 'onboarding' },
        },
        { route: true },
      );
      remember();
      ondismiss();
    } catch (e) {
      toastError('Couldn’t start session', e);
    } finally {
      launchBusy = false;
    }
  }

  // Persist dismissal so the coach doesn't reappear on every empty Agents view.
  const LS_DISMISSED = 'otto_firstrun_dismissed';
  function remember(): void {
    try {
      localStorage.setItem(LS_DISMISSED, '1');
    } catch {
      /* private mode — coach just shows again next time */
    }
  }
  function dismiss(): void {
    remember();
    ondismiss();
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`coach-wrap`,});
   { svelteHTML.createElement("div", { "class":`coach card`,});
     { svelteHTML.createElement("button", {       "class":`coach-close`,"title":`Dismiss`,"aria-label":`Dismiss the getting-started guide`,"onclick":dismiss,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":13,}});} }

     { svelteHTML.createElement("div", { "class":`coach-head`,});
       { svelteHTML.createElement("div", { "class":`coach-mark`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":24,}});} }
       { svelteHTML.createElement("div", {});
         { svelteHTML.createElement("h2", {});     }
         { svelteHTML.createElement("p", {});             }
       }
     }

    
     { svelteHTML.createElement("div", {  "class":`step`,});hasAgentCli;
       { svelteHTML.createElement("span", {  "class":`step-mark`,});hasAgentCli;
        if(hasAgentCli){ { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}else{ { svelteHTML.createElement("span", { "class":`num`,});  }}
       }
       { svelteHTML.createElement("div", { "class":`step-body`,});
         { svelteHTML.createElement("div", { "class":`step-title`,});hasAgentCli ? 'Agent CLI detected' : 'Install an agent CLI'; }
        if(hasAgentCli){
           { svelteHTML.createElement("div", { "class":`tool-chips`,});
               for(let t of __sveltets_2_ensureArray(agentTools)){t.name;
               { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"tone":t.found && t.checked ? 'ok' : 'neutral',"title":providerReadiness(t.name).message,}});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":!t.checked ? 'clock' : t.found ? 'check' : 'x',"size":12,}});}
                t.name;!t.checked ? ' · not checked yet' : t.found && t.version ? ` ${t.version}` : '';
               Badge}
            }
            if(agentTools.length === 0){
               { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {    "tone":`ok`,"label":agentProviders.join(', '),}});}
            }
           }
        }else{
           { svelteHTML.createElement("div", { "class":`step-hint`,});
                   { svelteHTML.createElement("span", { "class":`mono`,});  }   
             
             { svelteHTML.createElement("ul", { "class":`install-list`,});
               { svelteHTML.createElement("li", {});
                 { svelteHTML.createElement("strong", {});  } 
                 { svelteHTML.createElement("span", { "class":`mono`,});    }
               }
               { svelteHTML.createElement("li", {});
                 { svelteHTML.createElement("strong", {});  } 
                 { svelteHTML.createElement("span", { "class":`mono`,});    }
               }
             }
             { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":checkingProviders,"onclick":recheckProviders,});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});} checkingProviders ? 'Checking…' : 'Re-check';
             }
            if(providerCheckFailed){
               {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: 'Couldn’t check agent CLIs. Try again.', assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {  });      }}
            }
           }
        }
       }
     }

    
     { svelteHTML.createElement("div", {  "class":`step`,});hasWorkspace;
       { svelteHTML.createElement("span", {  "class":`step-mark`,});hasWorkspace;
        if(hasWorkspace){ { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}else{ { svelteHTML.createElement("span", { "class":`num`,});  }}
       }
       { svelteHTML.createElement("div", { "class":`step-body`,});
         { svelteHTML.createElement("div", { "class":`step-title`,});  }
        if(hasWorkspace){
           { svelteHTML.createElement("div", { "class":`step-hint`,});
             { svelteHTML.createElement("span", { "class":`mono`,});ws.current?.name; }
             { svelteHTML.createElement("span", { "class":`dim mono`,}); ws.current?.root_path; }
           }
        }else{
           { svelteHTML.createElement("div", { "class":`step-hint`,});           }
           { svelteHTML.createElement("div", { "class":`ws-form`,});
             { svelteHTML.createElement("input", {            "dir":`auto`,"class":`input`,"aria-label":`Workspace name`,"bind:value":wsName,"oninput":() => (wsNameTouched = true),"placeholder":`my-project`,});/*Ωignore_startΩ*/() => wsName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("div", { "class":`path-row`,});
               { svelteHTML.createElement("input", {                   "class":`input mono path-input`,"aria-label":`Workspace folder`,"dir":`ltr`,"bind:value":wsPath,"oninput":() => (pathError = ''),"aria-invalid":pathError !== '',"aria-describedby":`frc-path-err`,"spellcheck":`false`,"placeholder":`~/code/my-project`,});/*Ωignore_startΩ*/() => wsPath = __sveltets_2_any(null);/*Ωignore_endΩ*/}
               { svelteHTML.createElement("button", {     "class":`btn`,"type":`button`,"onclick":() => (pickerOpen = true),});  }
             }
             { svelteHTML.createElement("span", {     "id":`frc-path-err`,"class":`path-err`,"role":`status`,});pathError; }
             { svelteHTML.createElement("button", {     "class":`btn small primary ws-create`,"disabled":!canCreateWs,"onclick":createWorkspace,});
              wsBusy ? 'Creating…' : 'Create workspace';
             }
           }
        }
       }
     }

    
    if(auth.isRoot && (!skillsLoaded || skillsError || recommendedSkills.length > 0)){
       { svelteHTML.createElement("div", { "class":`step optional`,});
         { svelteHTML.createElement("span", { "class":`step-mark`,}); { svelteHTML.createElement("span", { "class":`num`,});  } }
         { svelteHTML.createElement("div", { "class":`step-body`,});
           { svelteHTML.createElement("div", { "class":`step-title`,});
               { svelteHTML.createElement("span", { "class":`opt-tag`,});  }
           }
           { svelteHTML.createElement("div", { "class":`step-hint`,});
                          
               { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => router.go('settings/skills'),});   }
           }
           { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"variant":`compact`,"what":`recommended skills`,"loading":skillsLoading,"error":skillsError,"empty":recommendedSkills.length === 0,"onretry":loadSkills,}});
           { svelteHTML.createElement("div", { "class":`skill-rows`,});
               for(let s of __sveltets_2_ensureArray(recommendedSkills)){s.name;
              const installed = s.state === 'up_to_date' || s.state === 'ahead';
               { svelteHTML.createElement("div", { "class":`skill-row`,});
                 { svelteHTML.createElement("span", { "class":`mono skill-name`,});s.name; }
                if(installed){
                   { const $$_egdaB7C = __sveltets_2_ensureComponent(Badge); new $$_egdaB7C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"tone":`ok`,}}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  Badge}
                }else{
                   { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":skillBusy.has(s.name),"onclick":() => installSkill(s),});
                    skillBusy.has(s.name) ? 'Installing…' : s.state === 'update_available' ? 'Update' : 'Install';
                   }
                }
               }
            }
           }
           LoadState}
         }
       }
    }

    
     { svelteHTML.createElement("div", { "class":`coach-launch`,});
       { svelteHTML.createElement("button", {       "class":`btn primary launch-btn`,"disabled":!hasAgentCli || !hasWorkspace || launchBusy,"onclick":launchFirstSession,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":13,}});}
        launchBusy ? 'Starting…' : 'Start your first agent';
       }
       { svelteHTML.createElement("p", { "class":`launch-hint`,});
           { svelteHTML.createElement("span", { "class":`mono`,});launchProvider || 'claude'; }      
           
       }
     }
   }
 }

if(pickerOpen){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {         "title":`Choose project directory`,"start":wsPath || '~',"onpick":(p) => {
      wsPath = p;
      pathError = '';
      pickerOpen = false;
    },"onclose":() => (pickerOpen = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const FirstRunCoach__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type FirstRunCoach__SvelteComponent_ = ReturnType<typeof FirstRunCoach__SvelteComponent_>;
/*Ωignore_endΩ*/export default FirstRunCoach__SvelteComponent_;