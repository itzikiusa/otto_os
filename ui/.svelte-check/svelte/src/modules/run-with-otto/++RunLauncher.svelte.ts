///<reference types="svelte" />
;// THE one button. A single input where the user pastes a Jira key, a
// GitHub/Confluence URL, or a finding/story/test id — or just describes what
// they want (free text → a channel run). As they type we debounce-detect the
// source and show what Otto will run; the prominent "Run with Otto" button
// launches it. Around the input: source chips (what you can paste), the
// pipeline rail (how a run executes — and how this differs from Workflows),
// and the launch parameters (mode, repo, provider/model). There is no
// "auto-open PR" toggle: the daemon stores `auto_open_pr` but no engine path
// reads it, and opening the PR is the one outward action that must stay an
// explicit, confirmed click in RunDetail (patterns.md §5).

import { onDestroy } from 'svelte';
import { latestOnly } from '../../lib/latest';
import { api } from '../../lib/api/client';
import { loadErrorText } from '../../lib/loadError';
import { auth } from '../../lib/stores/auth.svelte';
import { runWithOtto } from '../../lib/stores/runWithOtto.svelte';
import { runWithOttoApi } from '../../lib/api/runWithOtto';
import { router } from '../../lib/router.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import Icon from '../../lib/components/Icon.svelte';
import RunStageRail from './RunStageRail.svelte';
import { SOURCE_KINDS, sourceLabel } from './runStatus';
import type { OttoRun, Repo, RunDetectResp, RunMode } from '../../lib/api/types';
import { agentProviders, defaultAgentProvider, providerSupportsModel } from '../../lib/providers';
import { registry } from '../../lib/commands.svelte';
import ModelPicker from '../../lib/components/ModelPicker.svelte';
;

  interface Props {
    wsId: string;
    onLaunched: (run: OttoRun) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { wsId, onLaunched }: Props = $props();

  let query = $state('');
  let mode = $state<RunMode>('single_agent');
  let provider = $state(defaultAgentProvider());
  let model = $state('');
  let repoId = $state('');
  let inputEl: HTMLTextAreaElement | undefined = $state();

  let detected = $state<RunDetectResp['detected'] | null>(null);
  let detecting = $state(false);
  /** The detect call failed (not "no source found") — say so, offer Retry. */
  let detectFailed = $state(false);
  let busy = $state(false);
  let error = $state('');

  // Registered repos → the Repo select. Reloaded when the workspace changes.
  let repos = $state<Repo[]>([]);
  let picking = $state(false);
  let registering = $state(false);
  /** Failed repo-list load: the select would otherwise silently offer only "Auto". */
  let reposError = $state('');
  let reposLoading = $state(false);
  async function loadRepos(id: string): Promise<void> {
    reposLoading = true;
    try {
      const list = await api.get<Repo[]>(`/workspaces/${id}/repos`);
      if (id !== wsId) return; // a newer workspace's load owns the list
      repos = list;
      // A pick that isn't one of THIS workspace's repos must never launch.
      if (repoId && !list.some((r) => r.id === repoId)) repoId = '';
      reposError = '';
    } catch (e) {
      if (id === wsId) reposError = loadErrorText(e);
    } finally {
      if (id === wsId) reposLoading = false;
    }
  }
  $effect(() => {
    const id = wsId;
    repos = [];
    // The previous workspace's repo pick would otherwise survive the switch
    // (blank select) and launch a run in another workspace's repository.
    repoId = '';
    void loadRepos(id);
  });

  // Providers from the live registry (built-ins + custom, e.g. grok); `shell`
  // can't take an agent prompt. Both single-agent and goal-loop modes honor the
  // chosen provider (single-agent runs claude on the fast PTY, any other
  // provider as a real session — see run_engine::execute_single_agent).
  const providers = $derived(agentProviders());
  const effectiveProvider = $derived(provider);

  // --- debounced source detection -----------------------------------------
  let debounceTimer: ReturnType<typeof setTimeout> | null = null;
  let detectAbort: AbortController | null = null;
  const detectSeq = latestOnly();
  let alive = true;
  onDestroy(() => {
    alive = false;
    if (debounceTimer) clearTimeout(debounceTimer);
    detectAbort?.abort();
    detectSeq.cancel();
  });

  function onInput(): void {
    detected = null;
    detectFailed = false;
    error = '';
    if (debounceTimer) clearTimeout(debounceTimer);
    detectAbort?.abort();
    detectSeq.cancel();
    const q = query.trim();
    if (q.length < 3) {
      detecting = false;
      return;
    }
    detecting = true;
    debounceTimer = setTimeout(() => void runDetect(q), 250);
  }

  async function runDetect(q: string): Promise<void> {
    detectAbort?.abort();
    const controller = new AbortController();
    detectAbort = controller;
    const ticket = detectSeq.begin();
    const current = () => alive && ticket.current && q === query.trim();
    try {
      const resp = await runWithOttoApi.detect(wsId, q, controller.signal);
      if (!current()) return;
      detected = resp.detected ?? null;
      detectFailed = false;
    } catch {
      if (controller.signal.aborted || !current()) return;
      detected = null;
      detectFailed = true;
    } finally {
      if (current()) detecting = false;
    }
  }

  function runDetectNow(): Promise<void> {
    const q = query.trim();
    if (q.length < 3) return Promise.resolve();
    detectFailed = false;
    detecting = true;
    return runDetect(q);
  }

  /** A source chip inserts its paste template and focuses the input. */
  function useTemplate(template: string): void {
    query = template;
    detected = null;
    inputEl?.focus();
    onInput();
  }

  /** Browse… → any folder inside a git repo registers (or finds) that repo and
   *  selects it — the daemon resolves the git toplevel via repos/detect. */
  async function onPickFolder(path: string): Promise<void> {
    picking = false;
    registering = true;
    error = '';
    try {
      const repo = await api.post<Repo>(`/workspaces/${wsId}/repos/detect`, { path });
      if (!repos.some((r) => r.id === repo.id)) repos = [...repos, repo];
      repoId = repo.id;
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      registering = false;
    }
  }

  async function launch(): Promise<void> {
    if (busy) return;
    error = '';
    const q = query.trim();
    if (!q) {
      error = 'Paste a source or describe what you want.';
      return;
    }
    busy = true;
    try {
      // ⌘↩ right after a paste lands while detection is still debouncing/in
      // flight — launching now would send a Jira key or URL as FREE TEXT (the
      // daemon only parses url/source_ref, never seed_text). Finish detecting.
      if (detecting) {
        if (debounceTimer) clearTimeout(debounceTimer);
        detectAbort?.abort();
        await runDetect(q);
      }
      if (!alive || q !== query.trim()) return;
      const run = await runWithOtto.launch(wsId, {
        source_kind: detected?.source_kind,
        source_ref: detected?.source_ref,
        url: detected?.url,
        seed_text: detected ? undefined : q,
        mode,
        provider: effectiveProvider,
        model: model.trim() || undefined,
        // Only a repo of the workspace being launched in (belt and braces —
        // the daemon rejects a foreign repo too).
        repo_id: repoId && repos.some((r) => r.id === repoId) ? repoId : undefined,
      });
      if (!alive) return;
      if (query.trim() === q) { query = ''; detected = null; }
      onLaunched(run);
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      busy = false;
    }
  }

  // ⌘K: the page's main verb — jump to the launcher input.
  $effect(() =>
    registry.register('run-with-otto', [
      {
        id: 'rwo.new',
        title: 'New run with Otto…',
        group: 'Run with Otto',
        keywords: 'launch jira github issue pr finding story test run',
        run: () => inputEl?.focus(),
      },
    ]),
  );

  function onKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') {
      e.preventDefault();
      void launch();
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`launcher`,});
  
   { svelteHTML.createElement("div", {     "class":`sources`,"role":`group`,"aria-label":`Insert a source template`,});
       for(let s of __sveltets_2_ensureArray(SOURCE_KINDS)){s.kind;
       { svelteHTML.createElement("button", {         "type":`button`,"class":`src-chip`,"title":`Insert a ${s.label} reference — e.g. ${s.example}`,"onclick":() => useTemplate(s.template),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":s.icon,"size":12,}});}
        s.label;
       }
    }
     { svelteHTML.createElement("span", { "class":`src-free`,});      }
   }

   { const $$_textarea1 = svelteHTML.createElement("textarea", {                   "dir":`auto`,"class":`big-input`,"bind:value":query,"oninput":onInput,"onkeydown":onKeydown,"rows":2,"aria-label":`Source or description`,"aria-describedby":`rwo-detect`,"placeholder":`e.g. PAY-142, https://github.com/acme/api/pull/7, or “add rate limiting to the login endpoint”`,});inputEl = $$_textarea1;/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/ }

  
   { svelteHTML.createElement("div", { "class":`submit-row`,});
     { svelteHTML.createElement("div", {     "class":`detect`,"id":`rwo-detect`,"aria-live":`polite`,});
      if(detecting){
         { svelteHTML.createElement("span", { "class":`muted`,});  }
      } else if (detected){
         { svelteHTML.createElement("span", { "class":`muted`,});  }
         { svelteHTML.createElement("span", { "class":`chip`,});sourceLabel(detected.source_kind); }
         { svelteHTML.createElement("span", {   "class":`ref mono`,"title":detected.source_ref,});detected.source_ref; }
        if(detected.url){
           { svelteHTML.createElement("a", {         "class":`link`,"href":detected.url,"target":`_blank`,"rel":`noreferrer`,"title":detected.url,});   { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});} }
        }
      } else if (detectFailed){
         { svelteHTML.createElement("span", { "class":`warn-text`,});           }
         { svelteHTML.createElement("button", {     "type":`button`,"class":`btn small ghost`,"onclick":() => void runDetectNow(),});  }
      } else if (query.trim().length >= 3){
         { svelteHTML.createElement("span", { "class":`muted`,});       }
         { svelteHTML.createElement("span", { "class":`chip`,});sourceLabel('channel'); }
      }else{
         { svelteHTML.createElement("span", { "class":`muted`,});       }
      }
     }
     { svelteHTML.createElement("span", {   "class":`kbd-hint`,"aria-hidden":`true`,});  }
     { svelteHTML.createElement("button", {       "class":`btn primary run`,"disabled":busy,"onclick":launch,"title":`Launch the run (⌘↩)`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}
      busy ? 'Launching…' : 'Run with Otto';
     }
   }
  if(error){ { svelteHTML.createElement("div", {   "class":`err`,"role":`alert`,});error; }}

   { svelteHTML.createElement("div", {     "class":`options`,"role":`group`,"aria-label":`Run options`,});
     { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Run mode`,});
       { svelteHTML.createElement("button", {          "type":`button`,"aria-pressed":mode === 'single_agent',"onclick":() => (mode = 'single_agent'),"title":`One headless agent makes the change on an isolated branch`,});mode === 'single_agent';  }
       { svelteHTML.createElement("button", {          "type":`button`,"aria-pressed":mode === 'goal_loop',"onclick":() => (mode = 'goal_loop'),"title":`A full Plan → Execute → Evaluate loop iterates until the goal is met`,});mode === 'goal_loop';  }
     }

     { svelteHTML.createElement("div", { "class":`ctl`,});
       { svelteHTML.createElement("label", { "for":`rwo-repo`,});  }
       { svelteHTML.createElement("select", {         "id":`rwo-repo`,"class":`input`,"bind:value":repoId,"aria-label":`Repository`,"title":repos.find((r) => r.id === repoId)?.path ?? 'Auto: the repo named by the source, else the first registered repo',});/*Ωignore_startΩ*/() => repoId = __sveltets_2_any(null);/*Ωignore_endΩ*/
         { svelteHTML.createElement("option", {"value":"",});   }
           for(let r of __sveltets_2_ensureArray(repos)){r.id;
           { svelteHTML.createElement("option", {   "value":r.id,"title":r.path,});r.name; }
        }
       }
      if(reposError){
         { svelteHTML.createElement("span", {     "class":`warn-text`,"role":`alert`,"title":reposError,});   }
         { svelteHTML.createElement("button", {       "type":`button`,"class":`btn small`,"disabled":reposLoading,"onclick":() => void loadRepos(wsId),});reposLoading ? 'Retrying…' : 'Retry'; }
      }
       { svelteHTML.createElement("button", {         "type":`button`,"class":`btn`,"disabled":registering,"onclick":() => (picking = true),"title":`Pick any folder inside a git repository to add it`,});
        registering ? 'Adding…' : 'Browse…';
       }
     }

     { svelteHTML.createElement("div", { "class":`ctl`,});
       { svelteHTML.createElement("label", { "for":`rwo-provider`,});  }
      
       { svelteHTML.createElement("select", {         "id":`rwo-provider`,"class":`input`,"bind:value":provider,"aria-label":`Provider`,"onchange":() => (model = ''),});/*Ωignore_startΩ*/() => provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let p of __sveltets_2_ensureArray(providers)){p;
           { svelteHTML.createElement("option", { "value":p,});p; }
        }
       }
     }

    
    if(providerSupportsModel(provider)){
       { svelteHTML.createElement("div", {   "class":`ctl model-ctl`,"title":`Pins the model for this run only. Empty uses the provider’s default.`,});
         { svelteHTML.createElement("span", {   "class":`ctl-lbl`,"aria-hidden":`true`,});  }
          { const $$_rekciPledoM3C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM3C({ target: __sveltets_2_any(), props: {       provider,"value":model,"onchange":(m) => (model = m),"id":`rwo-model`,"compact":true,}});}
       }
    }
   }

  
   { svelteHTML.createElement("div", { "class":`how`,});
     { const $$_liaRegatSnuR2C = __sveltets_2_ensureComponent(RunStageRail); new $$_liaRegatSnuR2C({ target: __sveltets_2_any(), props: {}});}
     { svelteHTML.createElement("p", { "class":`how-note`,});
                         
          
       { svelteHTML.createElement("a", {    "href":`#/workflows`,"onclick":(e) => {
          e.preventDefault();
          router.go('workflows');
        },});   }
     }
   }
 }

if(picking){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {        "title":`Choose a repository`,"gitOnly":true,"onpick":(p) => void onPickFolder(p),"onclose":() => (picking = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RunLauncher__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunLauncher__SvelteComponent_ = ReturnType<typeof RunLauncher__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunLauncher__SvelteComponent_;