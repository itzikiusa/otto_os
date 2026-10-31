///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { announce } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
import { onMount, onDestroy } from 'svelte';
import { api } from '../../lib/api/client';
import type { GitRecoveryEntry, GitInteractivePlan, GitBisectState, RepoStatusResp, MergeResult } from '../../lib/api/types';
import { git } from '../../lib/stores/git.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { gitBridge, type RecoveryMode } from './gitBridge.svelte';

;type $$ComponentProps =  {
    repoId: string; initialMode?: RecoveryMode; initialOnto?: string;
    onclose: () => void; onresolve: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId, initialMode = 'history', initialOnto, onclose, onresolve }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  // This component is mounted per repository. Requests capture that identity;
  // closing while one runs cannot apply its result to another repository.
  // svelte-ignore state_referenced_locally
  const id = repoId;
  let alive = true;
  onDestroy(() => { alive = false; });
  // svelte-ignore state_referenced_locally
  let mode = $state<RecoveryMode>(initialMode);
  let busy = $state(false);
  // Spoken through the shell's always-mounted region: a status element
  // inserted together with its text is silent in VoiceOver.
  $effect(() => {
    if (busy) announce('Working…');
  });
  let error = $state('');
  let entries = $state<GitRecoveryEntry[]>([]);
  let more = $state(true);
  // svelte-ignore state_referenced_locally
  let onto = $state(initialOnto ?? '');
  let plan = $state<GitInteractivePlan | null>(null);
  let bisect = $state<GitBisectState | null>(null);
  let good = $state(gitBridge.bisectTargets[id]?.good ?? '');
  let bad = $state(gitBridge.bisectTargets[id]?.bad ?? 'HEAD');
  const op = $derived(git.statusById[id]?.op_in_progress);

  async function run(action: () => Promise<void>): Promise<void> {
    if (busy) return;
    busy = true; error = '';
    try { await action(); }
    catch (e) { if (alive) error = e instanceof Error ? e.message : String(e); }
    finally { if (alive) busy = false; }
  }
  async function loadHistory(reset = false): Promise<void> {
    const offset = reset ? 0 : entries.length;
    const rows = await api.get<GitRecoveryEntry[]>(`/repos/${id}/reflog?limit=50&skip=${offset}`);
    if (alive) { entries = reset ? rows : [...entries, ...rows]; more = rows.length === 50; }
  }
  async function refresh(): Promise<void> {
    await git.refreshStatus(id);
    const state = await api.get<GitBisectState>(`/repos/${id}/bisect`);
    if (alive) bisect = state;
    if (alive && mode === 'history') await loadHistory(true);
  }
  onMount(() => { void run(async () => { await refresh(); if (mode !== 'history') await loadHistory(true); }); });

  async function recover(entry: GitRecoveryEntry): Promise<void> {
    const name = await confirmer.promptText(`Create a branch at ${entry.sha.slice(0, 10)}. Your current branch and files stay in place.`, {
      title: 'Recover a commit', placeholder: `recovery/${entry.sha.slice(0, 8)}`, confirmLabel: 'Create branch',
    });
    if (!name || !alive) return;
    await run(async () => {
      const status = await api.post<RepoStatusResp>(`/repos/${id}/branch`, { name, start_point: entry.sha, checkout: false });
      git.setStatus(id, status); toasts.success('Recovery branch created', name);
    });
  }
  function inspect(sha: string): void { gitBridge.focusCommit(id, sha); onclose(); }
  async function preview(): Promise<void> {
    await run(async () => {
      const result = await api.get<GitInteractivePlan>(`/repos/${id}/rebase/plan?onto=${encodeURIComponent(onto)}`);
      if (alive) plan = result;
    });
  }
  function move(index: number, delta: number): void {
    if (!plan || busy) return;
    const commits = [...plan.commits];
    const other = index + delta;
    if (other < 0 || other >= commits.length) return;
    [commits[index], commits[other]] = [commits[other], commits[index]];
    plan = { ...plan, commits };
  }
  async function startRebase(): Promise<void> {
    if (!plan) return;
    const snapshot = $state.snapshot(plan);
    const yes = await confirmer.ask(`Replay ${plural(snapshot.commits.length, 'commit')} in the displayed order onto ${snapshot.onto_sha.slice(0, 10)}? This rewrites local history.`, {
      title: 'Start interactive rebase', confirmLabel: 'Start rebase', danger: true,
    });
    if (!yes || !alive) return;
    await run(async () => {
      const status = await api.post<RepoStatusResp>(`/repos/${id}/rebase/plan`, snapshot);
      git.setStatus(id, status); if (alive) plan = null;
      await refresh();
      if (alive && status.changes.some(c => c.kind === 'conflicted')) onresolve();
    });
  }
  async function rebaseAction(action: 'continue' | 'skip' | 'abort'): Promise<void> {
    if (action !== 'continue' && !await confirmer.ask(action === 'skip' ? 'Skip the current commit and discard all uncommitted changes in this worktree?' : 'Abort the rebase and restore the original branch?', {
      title: action === 'skip' ? 'Skip commit' : 'Abort rebase', confirmLabel: action === 'skip' ? 'Skip' : 'Abort', danger: true,
    })) return;
    await run(async () => {
      let status: RepoStatusResp;
      if (action === 'continue') {
        const result = await api.post<MergeResult>(`/repos/${id}/merge/commit`, {}); status = result.repo_status;
      } else { status = await api.post<RepoStatusResp>(`/repos/${id}/${action === 'skip' ? 'rebase/skip' : 'merge/abort'}`, {}); }
      git.setStatus(id, status); await refresh();
      if (alive && status.changes.some(c => c.kind === 'conflicted')) onresolve();
    });
  }
  async function bisectAction(action: 'start' | 'good' | 'bad' | 'skip' | 'reset'): Promise<void> {
    if (action === 'start' && !await confirmer.ask('Start bisect? Git will check out candidate commits for you to test, and End bisect returns to your current branch.', { title: 'Start bisect', confirmLabel: 'Start', danger: false })) return;
    await run(async () => {
      gitBridge.bisectTargets[id] = { good, bad };
      const state = await api.post<GitBisectState>(`/repos/${id}/bisect`, { op: action, good, bad, expected_head: bisect?.current_sha });
      if (alive) bisect = state;
      await git.refreshStatus(id);
    });
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Git recovery tools`,"width":820,onclose,}});
   { svelteHTML.createElement("div", { "class":`tools`,});
     { svelteHTML.createElement("nav", { "aria-label":`Recovery tools`,});
      
       { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Recovery tool`,});
          for(let tab of __sveltets_2_ensureArray(([{ id: 'history', label: 'Recovery history' }, { id: 'rebase', label: 'Interactive rebase' }, { id: 'bisect', label: 'Bisect' }]))){
           { svelteHTML.createElement("button", {     "aria-pressed":mode === tab.id,"onclick":() => { mode = tab.id as RecoveryMode; },});mode === tab.id;tab.label; }
        }
       }
       { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":() => run(refresh),});  }
     }
    if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: error, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`error`,});error; }}}
    if(busy){ { svelteHTML.createElement("p", {});  }}
    if(mode === 'history'){
       { svelteHTML.createElement("p", {});              }
       { svelteHTML.createElement("div", { "class":`scroll`,});
           for(let entry of __sveltets_2_ensureArray(entries)){entry.selector;
           { svelteHTML.createElement("div", { "class":`entry`,});
             { svelteHTML.createElement("div", {}); { svelteHTML.createElement("code", {});entry.selector;  entry.sha.slice(0, 10); } { svelteHTML.createElement("p", {});entry.subject; } }
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => inspect(entry.sha),});  }
             { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy,"onclick":() => recover(entry),});  }
           }
        }
        if(entries.length === 0 && !busy){ { svelteHTML.createElement("p", {});     }}
       }
       { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy || !more,"onclick":() => run(() => loadHistory()),});  }
    } else if (mode === 'rebase'){
      if(op === 'rebase'){
         { svelteHTML.createElement("p", {});                    }
         { svelteHTML.createElement("div", { "class":`actions`,});
           { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy,"onclick":() => rebaseAction('continue'),});  }
           { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":onresolve,});   }
           { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":() => rebaseAction('skip'),});  }
           { svelteHTML.createElement("button", {     "class":`btn danger`,"disabled":busy,"onclick":() => rebaseAction('abort'),});  }
         }
      }else{
         { svelteHTML.createElement("p", {});                    }
         { svelteHTML.createElement("div", { "class":`actions`,}); { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {          "dir":`auto`,"bind:value":onto,"oninput":() => { plan = null; },"placeholder":`main or a commit SHA`,"disabled":busy,});/*Ωignore_startΩ*/() => onto = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
           { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy || !onto.trim(),"onclick":preview,});  } }
        if(plan){
           { svelteHTML.createElement("p", {});plural(plan.commits.length, 'commit');   plan.head_sha.slice(0, 10);   plan.onto_sha.slice(0, 10); }
           { svelteHTML.createElement("div", { "class":`scroll`,});
                for(let commit of __sveltets_2_ensureArray(plan.commits)){let index = 1;commit.sha;
               { svelteHTML.createElement("div", { "class":`entry`,});
                 { svelteHTML.createElement("span", {});index + 1; }
                 { svelteHTML.createElement("select", {     "aria-label":`Action for ${commit.subject}`,"bind:value":commit.action,"disabled":busy,});/*Ωignore_startΩ*/() => commit.action = __sveltets_2_any(null);/*Ωignore_endΩ*/
                   { svelteHTML.createElement("option", { "value":`pick`,});  } { svelteHTML.createElement("option", { "value":`edit`,});  } { svelteHTML.createElement("option", {   "value":`squash`,"disabled":index === 0,});  }
                 }
                 { svelteHTML.createElement("div", {}); { svelteHTML.createElement("code", {});commit.sha.slice(0, 10); } { svelteHTML.createElement("p", {});commit.subject; } }
                 { svelteHTML.createElement("button", {         "class":`btn small`,"aria-label":`Move ${commit.subject} up`,"title":`Move ${commit.subject} up`,"disabled":busy || index === 0,"onclick":() => move(index, -1),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":12,}});} }
                 { svelteHTML.createElement("button", {         "class":`btn small`,"aria-label":`Move ${commit.subject} down`,"title":`Move ${commit.subject} down`,"disabled":busy || index === plan!.commits.length - 1,"onclick":() => move(index, 1),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});} }
               }
            }
           }
          if(plan.commits[0]?.action === 'squash'){ { svelteHTML.createElement("p", { "class":`error`,});        }}
           { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy || plan.commits.length === 0 || plan.commits[0]?.action === 'squash',"onclick":startRebase,});  }
        }
      }
    }else{
      if(bisect?.active){
         { svelteHTML.createElement("p", {});bisect.finished ? 'First bad commit found' : 'Test the current candidate, then mark the result.'; }
         { svelteHTML.createElement("div", { "class":`entry`,}); { svelteHTML.createElement("div", {}); { svelteHTML.createElement("code", {});bisect.first_bad ?? bisect.current_sha; } { svelteHTML.createElement("p", {});bisect.current_subject; } }
           { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => inspect(bisect!.first_bad ?? bisect!.current_sha),});   } }
        if(bisect.remaining !== null && !bisect.finished){ { svelteHTML.createElement("p", {});plural(bisect.remaining, 'commit'); bisect.remaining === 1 ? 'remains' : 'remain';    }}
         { svelteHTML.createElement("div", { "class":`actions`,});
          if(!bisect.finished){
             { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy,"onclick":() => bisectAction('good'),});  }
             { svelteHTML.createElement("button", {     "class":`btn danger`,"disabled":busy,"onclick":() => bisectAction('bad'),});  }
             { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":() => bisectAction('skip'),});   }
          }
           { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":() => bisectAction('reset'),});      }
         }
        if(bisect.output){ { svelteHTML.createElement("pre", {});bisect.output; }}
         { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", {});  } { svelteHTML.createElement("pre", {});bisect.log; } }
      }else{
         { svelteHTML.createElement("p", {});                  }
         { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {        "dir":`auto`,"bind:value":good,"placeholder":`Commit SHA or revision`,"disabled":busy,});/*Ωignore_startΩ*/() => good = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
         { svelteHTML.createElement("label", {});   { svelteHTML.createElement("input", {        "dir":`auto`,"bind:value":bad,"placeholder":`HEAD`,"disabled":busy,});/*Ωignore_startΩ*/() => bad = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
         { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy || !good.trim() || !bad.trim(),"onclick":() => bisectAction('start'),});  }
      }
    }
   }
 Modal}

};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RecoveryTools__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RecoveryTools__SvelteComponent_ = ReturnType<typeof RecoveryTools__SvelteComponent_>;
/*Ωignore_endΩ*/export default RecoveryTools__SvelteComponent_;