///<reference types="svelte" />
;// Create-PR sheet: source/target branch, title, markdown description.
// The message can be drafted by an agent from the branch diff, and the branch
// is pushed automatically (with --set-upstream) right before the PR is opened.

import Modal from '../../lib/components/Modal.svelte';
import { toastError } from '../../lib/toastError';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import { api } from '../../lib/api/client';
import { ws } from '../../lib/stores/workspace.svelte';
import type { BranchInfo, Collaborator, DraftPrResp, Id, PrSummary } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import Icon from '../../lib/components/Icon.svelte';
import AgentByline from '../../lib/components/AgentByline.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { git } from '../../lib/stores/git.svelte';
import { loadErrorText } from '../../lib/loadError';
import LoadState from '../../lib/components/LoadState.svelte';
import { router } from '../../lib/router.svelte';
import { onDestroy } from 'svelte';
;

  interface Props {
    repoId: string;
    /** When set, default the Source branch to this (e.g. opened from a branch /
     *  commit context menu). Still editable; target auto-picks a different one. */
    initialSource?: string;
    onclose: () => void;
    oncreated: (pr: PrSummary) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId, initialSource, onclose, oncreated }: Props = $props();

  /** False once the sheet unmounts: an in-flight draft or lookup must not
   *  touch global UI (a ghost confirm supersedes whatever dialog is open). */
  let alive = true;
  onDestroy(() => {
    alive = false;
    clearTimeout(revTimer);
  });

  let branches: BranchInfo[] = $state([]);
  /** Why the branch list failed to load — shown inline (the selects would
   *  otherwise sit empty with Create silently disabled). */
  let branchesError = $state<string | null>(null);
  let branchesRev = $state(0);
  const PROVIDER_LABEL: Record<string, string> = { github: 'GitHub', bitbucket: 'Bitbucket', gitlab: 'GitLab' };
  const prRepo = $derived(git.allRepos.find((r) => r.id === repoId) ?? git.repos.find((r) => r.id === repoId) ?? null);
  const providerName = $derived(prRepo?.provider ? (PROVIDER_LABEL[prRepo.provider] ?? prRepo.provider) : 'the provider');
  let source = $state('');
  let target = $state('');
  let title = $state('');
  let description = $state('');
  let busy = $state(false);
  let drafting = $state(false);
  /** Session that drafted (or is drafting). Drafting runs as a REAL Otto
   *  session, so it shows up in Agents the moment it spawns and can be opened
   *  in another window/tab and watched while this dialog stays put. */
  let draftSessionId = $state<Id | null>(null);
  /** When the agent's draft landed — drives the byline next to the fields. */
  let draftedAt = $state<number | null>(null);
  /** Seconds since the draft started — the whole point is that a long turn
   *  should never look indistinguishable from a wedged one. */
  let draftElapsed = $state(0);
  /** When the current draft started (RFC3339), to recognize its session among
   *  ws.sessions the moment its `session_created` event lands — the blocking
   *  POST only returns the id at the END of the turn. */
  let draftStartedAt = $state<string | null>(null);
  /** Embedded live draft terminal, collapsed by default. */
  let showDraftTerm = $state(false);

  /** The draft session to embed: the id from the finished POST, else (while
   *  drafting) the freshest pr-draft session created since the draft began —
   *  its `session_created` event adds it to ws.sessions long before the POST
   *  resolves. pr-draft is a background source, so it appears nowhere else. */
  const liveDraftId: Id | null = $derived.by(() => {
    if (draftSessionId) return draftSessionId;
    if (!drafting || !draftStartedAt) return null;
    const candidates = ws.sessions.filter(
      (s) =>
        (s.meta as { source?: string } | null)?.source === 'pr-draft' &&
        !s.archived &&
        s.created_at >= draftStartedAt!,
    );
    return candidates.length > 0 ? candidates[candidates.length - 1].id : null;
  });
  // What the Create button is doing, for the label.
  let phase: '' | 'pushing' | 'creating' = $state('');

  // "Open as draft" — persisted per repo (teams that always draft shouldn't
  // have to re-tick it on every PR).
  const draftKey = $derived(`otto_pr_draft:${repoId}`);
  let openAsDraft = $state(false);
  $effect(() => {
    openAsDraft = localStorage.getItem(draftKey) === '1';
  });
  function toggleDraft(): void {
    openAsDraft = !openAsDraft;
    if (openAsDraft) localStorage.setItem(draftKey, '1');
    else localStorage.removeItem(draftKey);
  }

  // Reviewers (optional): chips + provider-backed typeahead. When the
  // collaborator lookup fails (no account, unsupported plan, network), the
  // input degrades to free text — names are passed through verbatim.
  let reviewers: string[] = $state([]);
  let revQuery = $state('');
  let revSuggestions: Collaborator[] = $state([]);
  let revLookupFailed = $state(false);
  /** Keyboard-highlighted suggestion (combobox active option). */
  let revActive = $state(0);
  let revTimer: ReturnType<typeof setTimeout> | undefined;
  /** Lookup sequence: only the NEWEST query's reply may set the suggestions —
   *  a slow earlier reply used to overwrite them for the newer text. */
  let revSeq = 0;
  function queryCollaborators(q: string): void {
    clearTimeout(revTimer);
    const seq = ++revSeq;
    if (revLookupFailed) return; // degraded to free text — stop hitting the API
    revTimer = setTimeout(() => {
      void api
        .get<Collaborator[]>(`/repos/${repoId}/collaborators?q=${encodeURIComponent(q)}`)
        .then((list) => {
          if (!alive || seq !== revSeq) return;
          revSuggestions = list.filter((c) => !reviewers.includes(c.name)).slice(0, 8);
          revActive = 0;
        })
        .catch(() => {
          if (!alive || seq !== revSeq) return;
          revLookupFailed = true;
          revSuggestions = [];
        });
    }, 200);
  }
  function addReviewer(name: string): void {
    const n = name.trim();
    if (n !== '' && !reviewers.includes(n)) reviewers = [...reviewers, n];
    revQuery = '';
    revSuggestions = [];
    revActive = 0;
    revSeq++; // drop any lookup still in flight for the old text
  }
  function removeReviewer(name: string): void {
    reviewers = reviewers.filter((r) => r !== name);
  }
  const revOpen = $derived(revSuggestions.length > 0 && revQuery.trim() !== '');
  function onReviewerKey(e: KeyboardEvent): void {
    if (revOpen && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
      e.preventDefault();
      const n = revSuggestions.length;
      revActive = (revActive + (e.key === 'ArrowDown' ? 1 : n - 1)) % n;
    } else if (revOpen && e.key === 'Escape') {
      // Close the list, not the sheet.
      e.preventDefault();
      e.stopPropagation();
      revSuggestions = [];
    } else if (e.key === 'Enter' && revQuery.trim() !== '') {
      e.preventDefault();
      addReviewer((revOpen ? revSuggestions[revActive]?.name : undefined) ?? revQuery);
    } else if (e.key === 'Backspace' && revQuery === '' && reviewers.length > 0) {
      reviewers = reviewers.slice(0, -1);
    }
  }

  $effect(() => {
    void branchesRev;
    branchesError = null;
    void api
      .get<BranchInfo[]>(`/repos/${repoId}/branches`)
      .then((b) => {
        branches = b;
        if (source === '')
          source =
            (initialSource && b.some((x) => x.name === initialSource) ? initialSource : '') ||
            b.find((x) => x.is_current)?.name ||
            b[0]?.name ||
            '';
        if (target === '') {
          target =
            b.find((x) => x.name === 'develop' || x.name === 'main' || x.name === 'master')?.name ??
            b.find((x) => x.name !== source)?.name ??
            b[0]?.name ??
            '';
        }
      })
      .catch((e) => {
        branches = [];
        branchesError = loadErrorText(e);
      });
  });

  // Ask an agent to draft the title + description from the branch's diff vs the
  // target. Fills the form; the user reviews/edits before creating.
  async function draftWithAgent(): Promise<void> {
    if (!target) {
      toasts.warn('Pick a target branch first');
      return;
    }
    // The draft describes THIS head; the reply never re-points Source (the
    // daemon used to diff the checked-out branch and the UI adopted it).
    const head = source;
    drafting = true;
    draftSessionId = null;
    draftElapsed = 0;
    // Slight backdate so a session created in the same second still matches.
    draftStartedAt = new Date(Date.now() - 2000).toISOString();
    const tick = setInterval(() => (draftElapsed += 1), 1000);
    try {
      const d = await api.post<DraftPrResp>(`/repos/${repoId}/pr/draft`, { base: target, head: head || undefined });
      if (!alive) return;
      // A daemon that drafted another branch (or a Source changed mid-draft):
      // say so instead of silently swapping the PR's head.
      const draftedBranch = d.source_branch || head;
      if (draftedBranch !== source) {
        toasts.warn(
          'Draft is for a different branch',
          `The agent described ${draftedBranch}, but Source is ${source}. Check the text before opening the PR.`,
        );
      }
      // Never overwrite what the person already typed without asking.
      if (title.trim() || description.trim()) {
        const ok = await confirmer.ask(
          'The agent’s draft will replace the title and description you have typed.',
          { title: 'Replace your title and description?', confirmLabel: 'Replace', cancelLabel: 'Keep mine', danger: false },
        );
        if (!alive) return;
        if (!ok) {
          draftSessionId = d.session_id ?? null;
          draftedAt = null;
          return;
        }
      }
      title = d.title;
      description = d.description;
      draftedAt = Date.now();
      draftSessionId = d.session_id ?? null;
      toasts.info('Draft ready', 'Review and edit before creating.');
    } catch (e) {
      if (alive) toastError('Couldn’t draft the description', e);
    } finally {
      clearInterval(tick);
      drafting = false;
    }
  }

  async function create(): Promise<void> {
    busy = true;
    try {
      // Push the SELECTED source branch first (sets upstream for a fresh
      // branch) so the provider can find it — a bare push would push whatever
      // branch happens to be checked out, which may not be the PR's source.
      phase = 'pushing';
      try {
        // Through the store: the fresh ahead/behind and a graph re-sync land
        // with the push (a raw POST left both stale until the next poll). Not
        // `runPush` — its rejected-push "Pull" acts on the CHECKED-OUT branch,
        // which may not be this PR's source.
        await git.push(repoId, { branch: source });
      } catch (e) {
        // "Everything up-to-date" surfaces as a normal push; a real failure
        // (e.g. auth) should stop us before trying to open the PR.
        toastError('Couldn’t push the branch', e);
        return;
      }
      phase = 'creating';
      const pr = await api.post<PrSummary>(`/repos/${repoId}/prs`, {
        title: title.trim(),
        description,
        source_branch: source,
        target_branch: target,
        ...(openAsDraft ? { draft: true } : {}),
        ...(reviewers.length > 0 ? { reviewers } : {}),
      });
      // Every entry point gets a way into Otto's PR detail (review / post).
      toasts.push('success', 'Pull request created', `#${pr.number} ${pr.title}`, 8000, {
        action: { label: 'Open PR', run: () => router.go(`git/${repoId}/pr/${pr.number}`) },
      });
      if (pr.reviewer_warnings?.length) {
        toasts.warn('PR opened with warnings', pr.reviewer_warnings.join('; '));
      }
      oncreated(pr);
    } catch (e) {
      toastError('Couldn’t open the pull request', e);
    } finally {
      busy = false;
      phase = '';
    }
  }

  /** Stop the drafting agent's session (kill the PTY, keep the row). */
  async function stopDraftSession(id: Id): Promise<void> {
    try {
      await api.post(`/sessions/${id}/kill`, {});
    } catch {
      // Already exited / gone: nothing left to stop.
    }
  }

  /** Esc / backdrop / ✕: a stray key must not throw away a typed (or agent-
   *  drafted) title, description or reviewer list — ask first. */
  async function requestClose(): Promise<void> {
    // Mid-push/create: closing would hide the outcome of an outward action.
    if (busy) return;
    if (drafting) {
      const ok = await confirmer.ask('The agent is still drafting. Closing discards its draft and anything you entered.', {
        title: 'Stop drafting and discard?',
        confirmLabel: 'Discard',
        cancelLabel: 'Keep editing',
        danger: true,
      });
      if (!ok) return;
      // The draft runs as a real Otto session: closing alone would let it
      // finish the turn and spend tokens nobody will read (S20-305). Stop its
      // PTY (the row stays, so Agents still shows what it did). Best-effort —
      // it may have spawned too recently to be known, or just finished.
      const sid = liveDraftId;
      if (sid) void stopDraftSession(sid);
      onclose();
      return;
    }
    const edited = title.trim() !== '' || description.trim() !== '' || reviewers.length > 0 || revQuery.trim() !== '';
    if (edited) {
      const ok = await confirmer.ask('The title, description and reviewers you entered will be lost.', {
        title: 'Discard this pull request?',
        confirmLabel: 'Discard',
        cancelLabel: 'Keep editing',
        danger: true,
      });
      if (!ok) return;
    }
    onclose();
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {       children:() => { return __sveltets_2_any(0); },"title":`New pull request`,"width":520,"onclose":requestClose,"dismissable":!busy && !drafting,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("label", {   "class":`draft-toggle`,"title":`GitHub: draft PR · GitLab: 'Draft:' title prefix · Bitbucket: draft flag`,});
       { svelteHTML.createElement("input", {      "type":`checkbox`,"checked":openAsDraft,"onchange":toggleDraft,});}
        
     }
     { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":requestClose,});  }
     { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":busy || drafting || title.trim() === '' || source === '' || target === '' || source === target,"onclick":create,});
      phase === 'pushing' ? 'Pushing…' : phase === 'creating' ? 'Opening…' : 'Open pull request';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`createpr-form`,});
  if(branchesError){
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`the branches`,"error":branchesError,"empty":true,"onretry":() => branchesRev++,}});}
  }
   { svelteHTML.createElement("div", {   "class":`row branch-row`,"style":`gap: 12px; margin-bottom: 12px`,});
     { svelteHTML.createElement("div", {   "class":`field grow`,"style":`margin: 0`,});
       { svelteHTML.createElement("label", { "for":`pr-src`,});  }
       { svelteHTML.createElement("select", {     "id":`pr-src`,"class":`input`,"bind:value":source,});/*Ωignore_startΩ*/() => source = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let b of __sveltets_2_ensureArray(branches)){b.name; { svelteHTML.createElement("option", { "value":b.name,});b.name; }}
       }
     }
     { svelteHTML.createElement("span", {   "class":`dim branch-arrow`,"style":`margin-top: 16px`,});  }
     { svelteHTML.createElement("div", {   "class":`field grow`,"style":`margin: 0`,});
       { svelteHTML.createElement("label", { "for":`pr-tgt`,});  }
       { svelteHTML.createElement("select", {     "id":`pr-tgt`,"class":`input`,"bind:value":target,});/*Ωignore_startΩ*/() => target = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let b of __sveltets_2_ensureArray(branches)){b.name; { svelteHTML.createElement("option", { "value":b.name,});b.name; }}
       }
     }
   }

   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`pr-reviewers`,});  { svelteHTML.createElement("span", { "class":`dim`,});  } }
     { svelteHTML.createElement("div", {   "class":`chips-input input`,...__sveltets_2_empty({"data-testid":`pr-reviewers`}),});
         for(let r of __sveltets_2_ensureArray(reviewers)){r;
         { svelteHTML.createElement("span", { "class":`rev-chip chip`,});
          r;
           { svelteHTML.createElement("button", {       "class":`rev-chip-x`,"title":`Remove ${r}`,"aria-label":`Remove reviewer ${r}`,"onclick":() => removeReviewer(r),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
         }
      }
       { svelteHTML.createElement("input", {                            "dir":`auto`,"id":`pr-reviewers`,"class":`chips-text`,"placeholder":reviewers.length === 0 ? (revLookupFailed ? 'username, Enter to add' : 'Type to search…') : '',"bind:value":revQuery,"role":`combobox`,"aria-autocomplete":`list`,"aria-expanded":revOpen,"aria-controls":`pr-reviewers-list`,"aria-activedescendant":revOpen ? `pr-rev-opt-${revActive}` : undefined,"oninput":() => queryCollaborators(revQuery),"onkeydown":onReviewerKey,"autocomplete":`off`,"spellcheck":`false`,});/*Ωignore_startΩ*/() => revQuery = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
    if(revOpen){
       { svelteHTML.createElement("div", {       "class":`rev-suggest card`,"id":`pr-reviewers-list`,"role":`listbox`,"aria-label":`Reviewer suggestions`,});
            for(let c of __sveltets_2_ensureArray(revSuggestions)){let i = 1;c.name;
           { svelteHTML.createElement("button", {              "class":`rev-suggest-item`,"id":`pr-rev-opt-${i}`,"role":`option`,"aria-selected":i === revActive,"tabindex":-1,"onclick":() => addReviewer(c.name),});i === revActive;
             { svelteHTML.createElement("span", { "class":`mono`,});c.name; }
            if(c.display_name && c.display_name !== c.name){ { svelteHTML.createElement("span", { "class":`dim`,});c.display_name; }}
           }
        }
       }
    }
   }

   { svelteHTML.createElement("div", { "class":`draft-row`,});
     { svelteHTML.createElement("button", {     "class":`btn small ghost`,"disabled":drafting || busy || target === '',"onclick":draftWithAgent,});
      if(drafting){
         { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); } }else{
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}    }
     }
    if(drafting){
       { svelteHTML.createElement("span", { "class":`dim draft-hint`,});
        draftElapsed;      liveDraftId ? '' : '…';
       }
    } else if (draftSessionId){
      if(draftedAt){ { const $$_enilyBtnegA3C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA3C({ target: __sveltets_2_any(), props: {    "label":`Draft`,"at":draftedAt,}});}}
       { svelteHTML.createElement("span", { "class":`dim draft-hint`,});     }
    }else{
       { svelteHTML.createElement("span", { "class":`dim draft-hint`,});          target || 'target';  }
    }
    if(liveDraftId){
       { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (showDraftTerm = !showDraftTerm),});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":showDraftTerm ? 'chevronUp' : 'terminal',"size":12,}});}
        showDraftTerm ? 'Hide agent' : 'Watch agent';
       }
    }
   }

  if(liveDraftId && showDraftTerm){
    
     { svelteHTML.createElement("div", { "class":`draft-term`,});
       { const $$_lanimreTyzaL3C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL3C({ target: __sveltets_2_any(), props: {    "sessionId":liveDraftId,"preferDom":true,"showToolbar":false,}});}
     }
  }

   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`pr-title`,});  }
     { svelteHTML.createElement("input", {        "dir":`auto`,"id":`pr-title`,"class":`input`,"bind:value":title,});/*Ωignore_startΩ*/() => title = __sveltets_2_any(null);/*Ωignore_endΩ*/}
   }

   { svelteHTML.createElement("div", { "class":`field`,});
     { svelteHTML.createElement("label", { "for":`pr-desc`,});  { svelteHTML.createElement("span", { "class":`dim`,});  } }
     { svelteHTML.createElement("textarea", {         "dir":`auto`,"id":`pr-desc`,"class":`input`,"rows":6,"bind:value":description,});/*Ωignore_startΩ*/() => description = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
   }
  
   { svelteHTML.createElement("p", { "class":`cp-where dim`,});
    if(source !== '' && source === target){
       { svelteHTML.createElement("span", { "class":`cp-warn`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}         }
    }else{  { svelteHTML.createElement("span", { "class":`mono`,});source || 'the source branch'; }        
      providerName;prRepo ? ` in ${prRepo.name}` : '';            }
   }
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CreatePr__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CreatePr__SvelteComponent_ = ReturnType<typeof CreatePr__SvelteComponent_>;
/*Ωignore_endΩ*/export default CreatePr__SvelteComponent_;