///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
import Skeleton from '../../lib/components/Skeleton.svelte';
// One evaluation run: header summary + per-iteration report. Each iteration
// shows the implementation (+ diff), every validation's issues & suggested
// fixes, the score, before/after regression vs the previous round, and the
// improver's skill diff. Header actions: cancel, delete, promote the winning
// skill. Polls while running.
import { untrack } from 'svelte';
import { skillsEvalApi } from '../../lib/api/skillsEval';
import { skillEvalBus } from '../../lib/events.svelte';
import type { EvalFinding, EvalIteration, EvalValidationState,
    ImplDiffResp, PromoteGate, PromoteSkillReq, SkillEval } from '../../lib/api/types';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { toastError } from '../../lib/toastError';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import Badge from '../../lib/components/Badge.svelte';
import type { BadgeTone } from '../../lib/status';
import { severityLabel } from '../../lib/labels';
import { runStatus, sentenceCase, type StatusInfo } from '../../lib/status';
import Modal from '../../lib/components/Modal.svelte';
import Scorecard from './Scorecard.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
import { latestOnly } from '../../lib/latest';
;

  interface Props {
    evalId: string;
    /** Bubble the freshest run up so the parent list reflects live status. */
    onupdate?: (e: SkillEval) => void;
    /** Called after the run is deleted so the parent can drop it from the list. */
    ondeleted?: (evalId: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Run / iteration / validator status → the shared run vocabulary
  // (lib/status.ts). The eval's in-flight phases read as "running" (info,
  // pulsing dot) but keep their own word.
  const EVAL_PHASES = new Set(['implementing', 'validating', 'improving']);
  function evalStatus(raw: string): StatusInfo {
    return EVAL_PHASES.has(raw) ? { ...runStatus('running'), key: raw, label: sentenceCase(raw) } : runStatus(raw);
  }

  let { evalId, onupdate, ondeleted }: Props = $props();

  let run = $state<SkillEval | null>(null);
  let loading = $state(true);
  // First-load failure: shown inline with Retry (a toast over a blank pane
  // left nothing to act on).
  let loadError: string | null = $state(null);
  let pollTimer: ReturnType<typeof setTimeout> | null = null;
  /** JSON of the last run the poll applied (skip identical ticks). */
  let lastPolled = '';
  let pollCount = $state(0);
  // A read started before a rating write must not replace its newer result.
  let scoreRevision = 0;

  // Expansion / busy state (keyed by stable ids).
  let openTerminals = $state<Set<string>>(new Set());
  let openFindings = $state<Set<string>>(new Set());
  let openDiffs = $state<Set<string>>(new Set());
  let openImplDiffs = $state<Set<string>>(new Set());
  let implDiffs = $state<Record<string, ImplDiffResp>>({});
  let implDiffLoading = $state<Set<string>>(new Set());
  let retrying = $state<Set<string>>(new Set());
  let cancelling = $state(false);
  let deleting = $state(false);

  // Promote is a root-only server route (`promote_skill`).
  const canPromote = $derived(auth.isRoot);

  // Promote modal.
  let promoteOpen = $state(false);
  let promoteIterId = $state('');
  let promoteSource = $state<'tested' | 'improved'>('tested');
  let promoteName = $state('');
  let promoting = $state(false);

  // One generation for load + poll: opening run B while A's request is in
  // flight used to render A, push A into the parent list via `onupdate`, and
  // — since the in-flight poll re-armed its timer after the cleanup ran — keep
  // polling after unmount, for hours on a long eval.
  const seq = latestOnly();
  let disposed = false;
  $effect(() => {
    const id = evalId;
    void load(id);
    return () => {
      seq.cancel();
      if (pollTimer !== null) clearTimeout(pollTimer);
      pollTimer = null;
    };
  });
  $effect(() => () => {
    disposed = true;
  });

  // Live refresh: a `skill_eval_updated` WS event for this run triggers an
  // immediate poll instead of waiting for the 2s timer. The last-tick guard +
  // untrack keep this from self-triggering (the timed poll remains as fallback).
  let lastEvalTick = 0;
  $effect(() => {
    const t = skillEvalBus.tick;
    if (t === lastEvalTick) return;
    lastEvalTick = t;
    untrack(() => {
      if (skillEvalBus.runId === evalId && run && isActive(run)) void poll();
    });
  });

  async function load(id: string): Promise<void> {
    loading = true;
    loadError = null;
    if (pollTimer !== null) clearTimeout(pollTimer);
    pollCount = 0;
    const t = seq.begin();
    try {
      const r = await skillsEvalApi.get(id);
      if (!t.current || disposed) return;
      run = r;
      lastPolled = '';
      onupdate?.(r);
      if (isActive(r)) schedulePoll();
    } catch (e) {
      if (!t.current || disposed) return;
      const msg = loadErrorText(e);
      // Keep a loaded report on screen and just toast; with nothing to show,
      // put the error (and Retry) in the pane itself.
      if (run) toasts.error('Couldn’t refresh the evaluation', msg);
      else loadError = msg;
    } finally {
      if (t.current) loading = false;
    }
  }

  function isActive(r: SkillEval): boolean {
    return (
      r.status === 'running' ||
      r.iterations.some((it) => it.status !== 'done' && it.status !== 'error')
    );
  }

  function schedulePoll(delay = 2000): void {
    if (disposed) return;
    if (pollTimer !== null) clearTimeout(pollTimer);
    pollTimer = setTimeout(() => void poll(), delay);
  }

  // The poll result is compared before it lands: an unchanged run (most
  // ticks while an agent works) must not replace — and re-render — the
  // whole report. A hidden window skips the fetch and checks back later.
  async function poll(): Promise<void> {
    if (typeof document !== 'undefined' && document.visibilityState === 'hidden') {
      schedulePoll(5000);
      return;
    }
    pollCount++;
    // A poll never supersedes a load; a load (or unmount) supersedes the poll.
    const gen = seq.gen;
    const id = evalId;
    const revision = scoreRevision;
    const stale = () => seq.gen !== gen || disposed || id !== evalId || revision !== scoreRevision;
    try {
      const r = await skillsEvalApi.get(id);
      if (stale()) return;
      const sig = JSON.stringify(r);
      if (sig !== lastPolled) {
        lastPolled = sig;
        run = r;
        onupdate?.(r);
      }
      if (isActive(r)) schedulePoll(pollCount > 600 ? 5000 : 2000);
    } catch {
      if (!stale()) schedulePoll();
    }
  }

  function toggle(set: Set<string>, key: string): Set<string> {
    const next = new Set(set);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    return next;
  }

  /** Composite score → the shared Badge tone (≥85 good, ≥60 fair, else poor). */
  function scoreTone(n: number): BadgeTone {
    if (n >= 85) return 'ok';
    if (n >= 60) return 'warn';
    return 'bad';
  }

  function findingCount(it: EvalIteration): number {
    return it.agents.reduce((sum, a) => sum + a.findings.length, 0);
  }

  function diffLineClass(line: string): string {
    if (line.startsWith('+ ')) return 'add';
    if (line.startsWith('- ')) return 'del';
    return 'ctx';
  }

  function valKey(itId: string, a: EvalValidationState, i: number): string {
    return `${itId}:${a.name}:${i}`;
  }

  // --- before/after regression vs the previous iteration --------------------
  function fkey(validation: string, f: EvalFinding): string {
    return `${validation}|${f.issue.toLowerCase().split(/\s+/).join(' ').slice(0, 80)}`;
  }
  function iterKeys(it: EvalIteration): Set<string> {
    const s = new Set<string>();
    for (const a of it.agents) for (const f of a.findings) s.add(fkey(a.validation, f));
    return s;
  }
  function regression(idx: number): { fixed: number; introduced: number } | null {
    if (!run || idx === 0) return null;
    const prev = run.iterations[idx - 1];
    const cur = run.iterations[idx];
    if (!prev || prev.status !== 'done' || cur.status !== 'done') return null;
    const pk = iterKeys(prev);
    const ck = iterKeys(cur);
    let fixed = 0;
    let introduced = 0;
    for (const k of pk) if (!ck.has(k)) fixed++;
    for (const k of ck) if (!pk.has(k)) introduced++;
    return { fixed, introduced };
  }

  // --- header actions -------------------------------------------------------
  async function cancelRun(): Promise<void> {
    if (!run || cancelling) return;
    // Stopping loses the in-flight iteration's work; say so first.
    if (
      !(await confirmer.ask(`Stop the evaluation of ${run.source_skill}? The iteration in progress is abandoned; finished iterations and their scores are kept.`, {
        title: 'Stop evaluation',
        confirmLabel: 'Stop run',
        danger: true,
      }))
    )
      return;
    cancelling = true;
    try {
      const r = await skillsEvalApi.cancel(run.id);
      run = r;
      lastPolled = '';
      onupdate?.(r);
      toasts.info('Evaluation stopped');
    } catch (e) {
      toastError('Couldn’t stop the evaluation', e);
    } finally {
      cancelling = false;
    }
  }

  async function deleteRun(): Promise<void> {
    if (!run || deleting) return;
    if (
      !(await confirmer.ask(`Delete the evaluation of ${run.source_skill}? Its report, scores and iteration worktrees are removed. Skills you promoted to the library are kept.`, {
        title: 'Delete evaluation',
      }))
    )
      return;
    deleting = true;
    try {
      await skillsEvalApi.remove(run.id);
      toasts.info('Evaluation deleted');
      ondeleted?.(run.id);
    } catch (e) {
      toastError('Couldn’t delete the evaluation', e);
    } finally {
      deleting = false;
    }
  }

  // --- per-validation retry -------------------------------------------------
  async function retry(it: EvalIteration, index: number): Promise<void> {
    if (!run) return;
    const key = `${it.id}:${index}`;
    if (retrying.has(key)) return;
    retrying = new Set(retrying).add(key);
    try {
      const r = await skillsEvalApi.retryValidation(run.id, it.id, index);
      run = r;
      lastPolled = '';
      onupdate?.(r);
      if (isActive(r)) schedulePoll();
      toasts.info('Re-running validation…');
    } catch (e) {
      toastError('Couldn’t re-run the validation', e);
    } finally {
      retrying = new Set([...retrying].filter((k) => k !== key));
    }
  }

  // --- implementation diff --------------------------------------------------
  async function toggleImplDiff(it: EvalIteration): Promise<void> {
    const open = openImplDiffs.has(it.id);
    openImplDiffs = toggle(openImplDiffs, it.id);
    if (!open && !implDiffs[it.id] && run) {
      implDiffLoading = new Set(implDiffLoading).add(it.id);
      try {
        implDiffs[it.id] = await skillsEvalApi.implDiff(run.id, it.id);
      } catch (e) {
        toastError('Couldn’t load the code diff', e);
        // Collapse again so the toggle doesn't read "Hide code diff" over nothing.
        openImplDiffs = toggle(openImplDiffs, it.id);
      } finally {
        implDiffLoading = new Set([...implDiffLoading].filter((k) => k !== it.id));
      }
    }
  }

  // --- export / promote -----------------------------------------------------
  function skillContent(it: EvalIteration, source: 'tested' | 'improved'): string {
    return source === 'improved' ? (it.skill_after ?? '') : it.skill_before;
  }
  async function copySkill(it: EvalIteration, source: 'tested' | 'improved'): Promise<void> {
    try {
      await copyTextOrThrow(skillContent(it, source));
      toasts.success('Skill copied to clipboard');
    } catch {
      toasts.error("Couldn’t copy the skill", 'The clipboard is unavailable. Use Download instead.');
    }
  }
  function downloadSkill(it: EvalIteration, source: 'tested' | 'improved'): void {
    const blob = new Blob([skillContent(it, source)], { type: 'text/markdown' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${it.skill_name}${source === 'improved' ? '-improved' : ''}.SKILL.md`;
    a.click();
    URL.revokeObjectURL(url);
  }
  // Promote gate + force-override (root).
  let promoteForce = $state(false);
  let promoteGate = $state<PromoteGate | null>(null);
  let gateLoading = $state(false);
  let gateError = $state(false);
  // Inline name check (was a toast after clicking Promote).
  const promoteNameError = $derived(
    promoteName.trim() && !/^[A-Za-z0-9_-]+$/.test(promoteName.trim()) ? "Use letters, digits, '-' or '_' only." : '',
  );

  async function openPromote(it: EvalIteration, source: 'tested' | 'improved'): Promise<void> {
    promoteIterId = it.id;
    promoteSource = source;
    promoteName = run?.source_skill ?? it.skill_name;
    promoteForce = false;
    promoteGate = null;
    gateError = false;
    promoteOpen = true;
    if (!run) return;
    gateLoading = true;
    try {
      promoteGate = await skillsEvalApi.promoteGate(run.id, it.id);
    } catch {
      promoteGate = null;
      gateError = true;
    } finally {
      gateLoading = false;
    }
  }
  function promoteWinner(): void {
    if (!run || run.best_iteration == null) return;
    const it = run.iterations.find((i) => i.iter === run!.best_iteration);
    if (it) void openPromote(it, 'tested');
  }
  async function doPromote(): Promise<void> {
    if (!run || promoting) return;
    const name = promoteName.trim();
    if (!name || promoteNameError) return;
    promoting = true;
    try {
      const body: PromoteSkillReq = {
        iteration_id: promoteIterId,
        source: promoteSource,
        name,
        force: promoteForce,
      };
      await skillsEvalApi.promote(run.id, body);
      toasts.success('Skill saved to library', name);
      promoteOpen = false;
      // Reflect promoted state.
      void load(run.id);
    } catch (e) {
      toastError('Couldn’t promote the skill', e);
    } finally {
      promoting = false;
    }
  }

  const promoteIter = $derived.by(() => {
    if (!run) return null;
    return run.iterations.find((it) => it.id === promoteIterId) ?? null;
  });

  // --- eval-lab: human rating + regression capture ------------------------
  let rating = $state<Set<string>>(new Set());
  let savingReg = $state<Set<string>>(new Set());

  async function rate(it: EvalIteration, n: number, note = ''): Promise<void> {
    if (!run || rating.has(it.id)) return;
    const id = run.id;
    const current = () => !disposed && id === evalId;
    scoreRevision++;
    rating = new Set(rating).add(it.id);
    try {
      await skillsEvalApi.rate(id, it.id, { rating: n, note });
      if (!current()) return;
      scoreRevision++;
      // Ratings on different iterations can finish out of order. A fresh read
      // after each completed write keeps the report newer than response snapshots.
      await load(id);
    } catch (e) {
      if (!current()) return;
      scoreRevision++;
      toastError('Couldn’t finish updating your rating', e);
      // The rating may be saved while score publication is pending. Refresh
      // that durable state instead of leaving a stale successful score visible.
      await load(id);
    } finally {
      const next = new Set(rating);
      next.delete(it.id);
      rating = next;
      if (!disposed && id === evalId && run && isActive(run)) schedulePoll();
    }
  }

  async function saveRegression(it: EvalIteration): Promise<void> {
    if (!run || savingReg.has(it.id)) return;
    savingReg = new Set(savingReg).add(it.id);
    try {
      const g = await skillsEvalApi.regression(run.id, it.id, {});
      toasts.success('Saved as regression case', g.name);
    } catch (e) {
      toastError('Couldn’t save the regression case', e);
    } finally {
      const next = new Set(savingReg);
      next.delete(it.id);
      savingReg = next;
    }
  }
  function moreRunActions(e: MouseEvent): void {
    ctxMenu.show(e, [
      {
        label: 'Delete evaluation…',
        icon: 'trash',
        danger: true,
        disabled: deleting,
        action: () => void deleteRun(),
      },
    ]);
  }
  const runError = $derived(run?.error ?? '');
  $effect(() => { if (runError) announce(runError, { assertive: true }); });
;
async () => {

if((loading || loadError) && !run){
   { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {         "what":`this evaluation`,loading,"error":loadError,"empty":true,"onretry":() => load(evalId),}});}
} else if (run){
   { svelteHTML.createElement("div", { "class":`rd`,});
     { svelteHTML.createElement("header", { "class":`rd-head`,});
      
       { svelteHTML.createElement("div", { "class":`rd-title-row`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":16,}});}
         { svelteHTML.createElement("h2", {   "class":`rd-title`,"title":run.source_skill,});run.source_skill; }
         { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {  "status":evalStatus(run.status),}});}
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(isActive(run)){
          
           { svelteHTML.createElement("button", {     "class":`btn small danger`,"disabled":cancelling,"onclick":cancelRun,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});} cancelling ? 'Stopping…' : 'Stop run…';
           }
        }
        if(run.best_iteration != null && canPromote){
           { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":promoteWinner,"title":`Save the best-scoring iteration’s skill to the library`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}   
           }
        }
         { svelteHTML.createElement("button", {           "class":`icon-btn`,"disabled":deleting,"onclick":moreRunActions,"aria-label":`More actions`,"title":`More actions`,"aria-haspopup":`menu`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
         }
       }

       { svelteHTML.createElement("p", { "class":`rd-task`,});run.task; }
       { svelteHTML.createElement("div", { "class":`rd-meta`,});
        if(run.best_score != null){
           { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"tone":scoreTone(run.best_score),"title":`Best composite score`,}});
             run.best_score.toFixed(0);run.best_iteration ? ` · iteration ${run.best_iteration}` : '';
           Badge}
        }
        if(run.impl_cli){ { svelteHTML.createElement("span", {   "class":`chip`,"title":`Implementation agent`,});run.impl_cli; }}
         { svelteHTML.createElement("span", { "class":`chip`,});plural(run.target_iterations, 'iteration'); }
           for(let it of __sveltets_2_ensureArray(run.iterations)){it.id;
          if(it.scoring?.proof_status === 'pending'){
             { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {  "label":`Iteration ${it.iter}: score pending`,}});}
          }else{
             { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {    "tone":scoreTone(it.score),"label":`Iteration ${it.iter}: ${it.score.toFixed(0)}`,}});}
          }
        }
       }
      if(run.summary && !run.iterations.some(it => it.scoring?.proof_status === 'pending')){ { svelteHTML.createElement("p", { "class":`rd-summary`,});run.summary; }}
      if(run.error){ { svelteHTML.createElement("p", { "class":`rd-error`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} run.error; }}
     }

        for(let it of __sveltets_2_ensureArray(run.iterations)){let idx = 1;it.id;
      const reg = regression(idx);
       { svelteHTML.createElement("section", { "class":`iter card`,});
         { svelteHTML.createElement("div", { "class":`iter-head`,});
           { svelteHTML.createElement("span", { "class":`iter-num`,}); it.iter; }
          if(it.base_iter){ { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {  "label":`Improved from iteration ${it.base_iter}`,}});}}
           { svelteHTML.createElement("span", { "class":`mono`,}); { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {  "label":it.skill_name,}});} }
          if(reg){
             { svelteHTML.createElement("span", {  "class":`reg`,});reg.introduced > reg.fixed;
               reg.fixed;   reg.introduced;
             }
          }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
          if(it.scoring?.proof_status === 'pending'){
             { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {  "label":`Score update pending`,}});}
          } else if (it.status === 'done'){
             { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {      "tone":scoreTone(it.score),"label":it.score.toFixed(0),"title":`Iteration score`,}});}
          }
           { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {  "status":evalStatus(it.status),}});}
         }

        
         { svelteHTML.createElement("div", { "class":`impl`,});
           { svelteHTML.createElement("div", { "class":`impl-top`,});
             { svelteHTML.createElement("span", { "class":`lbl`,});  }
             { svelteHTML.createElement("span", { "class":`chip mono`,});it.impl_provider; }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
            if(it.worktree_path){
               { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => toggleImplDiff(it),});
                openImplDiffs.has(it.id) ? 'Hide code diff' : 'View code diff';
               }
            }
            if(it.impl_session_id){
               { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (openTerminals = toggle(openTerminals, it.impl_session_id ?? '')),});
                openTerminals.has(it.impl_session_id ?? '') ? 'Hide session' : 'Open session';
               }
            }
           }
          if(it.impl_summary){ { svelteHTML.createElement("p", { "class":`impl-summary`,});it.impl_summary; }}
          if(it.worktree_path){ { svelteHTML.createElement("p", { "class":`worktree mono`,});it.worktree_path; }}
          if(it.impl_session_id && openTerminals.has(it.impl_session_id)){
             { svelteHTML.createElement("div", { "class":`term`,});it.impl_session_id; { { const $$_lanimreTyzaL4C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL4C({ target: __sveltets_2_any(), props: {    "sessionId":it.impl_session_id,"preferDom":true,"resumeOnOpen":false,}});}} }
          }
          if(openImplDiffs.has(it.id)){
            if(implDiffLoading.has(it.id)){
               { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {      "rows":4,"height":14,"label":`the diff`,}});}
            } else if (implDiffs[it.id]){
              if(implDiffs[it.id].diff.trim()){
                 { svelteHTML.createElement("pre", { "class":`diff`,});    for(let line of __sveltets_2_ensureArray(implDiffs[it.id].diff.split('\n'))){let li = 1;li; { svelteHTML.createElement("span", { "class":`dl ${line.startsWith('+') ? 'add' : line.startsWith('-') ? 'del' : 'ctx'}`,});line || ' '; }'\n';} }
              }else{
                 { svelteHTML.createElement("p", { "class":`muted`,});    }
              }
            }
          }
         }

        
        
        if(it.agents.length > 0){
         { svelteHTML.createElement("div", { "class":`vals`,});
           { svelteHTML.createElement("div", { "class":`vals-head`,});  plural(findingCount(it), 'issue');  }
              for(let a of __sveltets_2_ensureArray(it.agents)){let ai = 1;valKey(it.id, a, ai);
            const key = valKey(it.id, a, ai);
            const rk = `${it.id}:${ai}`;
             { svelteHTML.createElement("div", { "class":`val card`,});
               { svelteHTML.createElement("div", { "class":`val-top`,});
                 { svelteHTML.createElement("span", { "class":`val-name`,});a.name; }
                 { svelteHTML.createElement("span", { "class":`chip mono`,});a.provider;a.model ? ' · ' + a.model : ''; }
                if(a.status === 'done'){
                   { svelteHTML.createElement("span", { "class":`pf ${a.passed ? 'pass' : 'fail'}`,});a.passed ? 'Passed' : 'Failed'; }
                   { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {      "tone":scoreTone(a.score),"label":a.score.toFixed(0),"title":`Validation score`,}});}
                }
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                if(a.session_id){
                   { svelteHTML.createElement("button", {     "class":`btn small ghost`,"aria-expanded":openTerminals.has(a.session_id ?? ''),"onclick":() => (openTerminals = toggle(openTerminals, a.session_id ?? '')),});
                    openTerminals.has(a.session_id ?? '') ? 'Hide session' : 'Open session';
                   }
                }
                 { svelteHTML.createElement("button", {       "class":`btn small ghost`,"disabled":retrying.has(rk),"onclick":() => retry(it, ai),"title":`Re-run this validation`,});
                  retrying.has(rk) ? 'Retrying…' : 'Retry';
                 }
                if(a.findings.length > 0){
                   { svelteHTML.createElement("button", {     "class":`btn small ghost`,"aria-expanded":openFindings.has(key),"onclick":() => (openFindings = toggle(openFindings, key)),});
                    openFindings.has(key) ? 'Hide issues' : `${plural(a.findings.length, 'issue')}`;
                   }
                }
                 { const $$_egdaBsutatS5C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS5C({ target: __sveltets_2_any(), props: {  "status":evalStatus(a.status),}});}
               }
              if(a.note && a.status !== 'done'){ { svelteHTML.createElement("p", { "class":`val-note`,});a.note; }}
              if(a.status === 'waiting'){
                 { svelteHTML.createElement("p", { "class":`val-waiting`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}       { svelteHTML.createElement("strong", {});  }   }
              }
              if(a.session_id && openTerminals.has(a.session_id)){
                 { svelteHTML.createElement("div", { "class":`term`,});a.session_id; { { const $$_lanimreTyzaL5C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL5C({ target: __sveltets_2_any(), props: {    "sessionId":a.session_id,"preferDom":true,"resumeOnOpen":false,}});}} }
              }
              if(openFindings.has(key)){
                 { svelteHTML.createElement("ul", { "class":`findings`,});
                      for(let f of __sveltets_2_ensureArray(a.findings)){let fi = 1;key + ':' + fi;
                     { svelteHTML.createElement("li", { "class":`finding`,});
                       { svelteHTML.createElement("div", { "class":`finding-head`,});
                         { const $$_egdaB7C = __sveltets_2_ensureComponent(Badge); new $$_egdaB7C({ target: __sveltets_2_any(), props: {    "tone":f.severity === 'fail' ? 'bad' : f.severity === 'warn' ? 'warn' : 'info',"label":severityLabel(f.severity),}});}
                        if(f.location){ { svelteHTML.createElement("span", { "class":`mono loc`,});f.location; }}
                       }
                       { svelteHTML.createElement("p", { "class":`issue`,}); { svelteHTML.createElement("span", { "class":`note-lbl`,});  } f.issue; }
                      if(f.suggestion){ { svelteHTML.createElement("p", { "class":`fix`,}); { svelteHTML.createElement("span", { "class":`note-lbl fix-tag`,});  } f.suggestion; }}
                     }
                  }
                 }
              }
             }
          }
         }
        }

        
        if(it.scoring){
           { const $$_dracerocS2C = __sveltets_2_ensureComponent(Scorecard); new $$_dracerocS2C({ target: __sveltets_2_any(), props: {      "score":it.scoring,"evalId":run.id,"iterId":it.id,}});}
        }

        if(it.scoring?.proof_status === 'pending' && (it.status === 'done' || it.status === 'error') && it.human_rating != null){
           { svelteHTML.createElement("div", { "class":`score-pending`,});
             { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {            "what":`the updated score`,"loading":rating.has(it.id),"empty":true,"variant":`compact`,"error":`Your rating is saved. Retry to finish updating the score before promoting this skill.`,"onretry":() => rate(it, it.human_rating!, it.human_note),}});}
           }
        }

        
         { svelteHTML.createElement("div", {   "class":`rate-row`,...__sveltets_2_empty({"data-testid":`rate-row`}),});
           { svelteHTML.createElement("span", {   "class":`lbl`,"id":`rate-lbl-${it.id}`,});  }
           { svelteHTML.createElement("span", {     "class":`stars`,"role":`group`,"aria-labelledby":`rate-lbl-${it.id}`,});
               for(let n of __sveltets_2_ensureArray(([1, 2, 3, 4, 5]))){n;
               { svelteHTML.createElement("button", {                "class":`star`,"disabled":rating.has(it.id),"onclick":() => rate(it, n),"title":`Rate ${n} of 5`,...__sveltets_2_empty({"data-testid":`rate-star`}),"aria-label":`Rate ${n} of 5`,"aria-pressed":(it.human_rating ?? 0) === n,});(it.human_rating ?? 0) >= n; { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`star`,"size":14,}});} }
            }
           }
          if(it.human_rating){ { svelteHTML.createElement("span", { "class":`rated dim`,});it.human_rating;  }}
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {           "class":`btn small ghost`,"disabled":savingReg.has(it.id),"onclick":() => saveRegression(it),...__sveltets_2_empty({"data-testid":`save-regression`}),"title":`Save this iteration as a per-repo regression golden task`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`target`,"size":12,}});} savingReg.has(it.id) ? 'Saving…' : 'Save as regression case';
           }
         }

        
         { svelteHTML.createElement("div", { "class":`skill-actions`,});
           { svelteHTML.createElement("span", { "class":`lbl`,});  { svelteHTML.createElement("span", {   "class":`mono lbl-name`,"title":it.skill_name,});it.skill_name; } }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => copySkill(it, 'tested'),});  }
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => downloadSkill(it, 'tested'),});  }
          if(canPromote){
             { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => openPromote(it, 'tested'),});  }
          }else{
             { svelteHTML.createElement("span", { "class":`promote-note dim`,});        }
          }
         }

        
        if(it.improvement_summary){
           { svelteHTML.createElement("div", { "class":`improve`,});
             { svelteHTML.createElement("div", { "class":`improve-top`,});
               { svelteHTML.createElement("span", { "class":`lbl`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}   }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
              if(it.skill_after){
                 { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => copySkill(it, 'improved'),});  }
                if(canPromote){
                   { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => openPromote(it, 'improved'),});  }
                }
              }
              if(it.skill_diff){
                 { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (openDiffs = toggle(openDiffs, it.id)),});
                  openDiffs.has(it.id) ? 'Hide skill diff' : 'View skill diff';
                 }
              }
             }
             { svelteHTML.createElement("p", { "class":`improve-summary`,});it.improvement_summary; }
            if(it.skill_diff && openDiffs.has(it.id)){
               { svelteHTML.createElement("pre", { "class":`diff`,});    for(let line of __sveltets_2_ensureArray(it.skill_diff.split('\n'))){let li = 1;li; { svelteHTML.createElement("span", { "class":`dl ${diffLineClass(line)}`,});line || ' '; }'\n';} }
            }
           }
        }
       }
    }

    if(run.iterations.length === 0){
       { svelteHTML.createElement("div", { "class":`rd-loading`,}); { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); }    }
    }
   }
}

if(promoteOpen && canPromote){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     "title":`Promote skill to library`,"width":460,"onclose":() => (promoteOpen = false),children:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("p", { "class":`modal-lede`,});
          promoteSource === 'improved' ? 'improved' : 'tested';     
                  
       }
       { svelteHTML.createElement("label", {   "class":`field-label`,"for":`promote-name`,});   }
       { svelteHTML.createElement("input", {              "dir":`auto`,"id":`promote-name`,"class":`input`,"bind:value":promoteName,"placeholder":`my-skill`,"aria-invalid":!!promoteNameError,"aria-describedby":`promote-name-err`,});/*Ωignore_startΩ*/() => promoteName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(promoteNameError){ { svelteHTML.createElement("p", {   "class":`field-err`,"id":`promote-name-err`,});promoteNameError; }}
      if(promoteIter?.skill_after){
         { svelteHTML.createElement("div", { "class":`src-toggle`,});
           { svelteHTML.createElement("label", {  "class":`chip-toggle`,});promoteSource === 'tested';
             { svelteHTML.createElement("input", {        "type":`radio`,"name":`promote-src`,"checked":promoteSource === 'tested',"onchange":() => (promoteSource = 'tested'),});}
             
           }
           { svelteHTML.createElement("label", {  "class":`chip-toggle`,});promoteSource === 'improved';
             { svelteHTML.createElement("input", {        "type":`radio`,"name":`promote-src`,"checked":promoteSource === 'improved',"onchange":() => (promoteSource = 'improved'),});}
             
           }
         }
      }

      
       { svelteHTML.createElement("div", {   "class":`gate`,...__sveltets_2_empty({"data-testid":`promote-gate`}),});
        if(gateLoading){
           { svelteHTML.createElement("p", { "class":`muted`,}); { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); }     }
        } else if (gateError){
           { svelteHTML.createElement("p", { "class":`muted`,});              }
        } else if (promoteGate){
          if(promoteGate.allowed){
             { svelteHTML.createElement("p", {   "class":`gate-ok`,...__sveltets_2_empty({"data-testid":`gate-ok`}),});
               { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}     promoteGate.score.toFixed(0);  promoteGate.threshold.toFixed(0);  promoteGate.proof_status || 'n/a';
             }
          }else{
             { svelteHTML.createElement("p", {   "class":`gate-bad`,...__sveltets_2_empty({"data-testid":`gate-blocked`}),}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}     }
             { svelteHTML.createElement("ul", { "class":`gate-reasons`,});
                  for(let r of __sveltets_2_ensureArray(promoteGate.reasons)){let ri = 1;ri; { svelteHTML.createElement("li", {});r; }}
             }
            if(canPromote){
               { svelteHTML.createElement("label", { "class":`force-row`,});
                 { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":promoteForce,...__sveltets_2_empty({"data-testid":`promote-force`}),});/*Ωignore_startΩ*/() => promoteForce = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                        
               }
            }
          }
        }
       }
    };return __sveltets_2_any(0)},footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (promoteOpen = false),});  }
       { svelteHTML.createElement("button", {         "class":`btn primary`,"disabled":promoting || !promoteName.trim() || !!promoteNameError || (promoteGate != null && !promoteGate.allowed && !promoteForce),"onclick":doPromote,...__sveltets_2_empty({"data-testid":`promote-confirm`}),});
        promoting ? 'Saving…' : promoteGate && !promoteGate.allowed && promoteForce ? 'Force promote' : 'Promote';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {children, footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
    
    
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RunDetail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunDetail__SvelteComponent_ = ReturnType<typeof RunDetail__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunDetail__SvelteComponent_;