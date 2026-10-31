///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { scrollBehavior } from '../../lib/motion';
import { loops } from '../../lib/stores/loops.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
import { viewport } from '../../lib/stores/viewport.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import SessionView from '../agents/SessionView.svelte';
import IterationRow from './IterationRow.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { humanVerification } from './verification';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { loopStatus } from './loopStatus';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { formatSeconds } from '../../lib/metric-format';
import Modal from '../../lib/components/Modal.svelte';
import type { GoalLoop } from '../../lib/api/types';
import { budgetCap, extendDefaults, hasBudgetLeft } from './budget';
import { tick } from 'svelte';
;

  /** ⌘K verbs registered by the detail page (or bound via optional actions prop). */
  export interface LoopActions {
    pause: () => void;
    resume: () => void;
    stop: () => void;
    extend: () => void;
  };
;type $$ComponentProps =  { id: string; onback: () => void; actions?: LoopActions };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  


  let { id, onback, actions }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  let answers = $state<Record<string, string>>({});
  let evidence = $state<Record<string, string>>({});
  /** Lifecycle call in flight (Pause / Resume / Stop) — disables the header verbs. */
  let acting = $state(false);

  let openSessionId = $state<string | null>(null);
  let sessEl = $state<HTMLElement | null>(null);
  /** True only while focus is inside the embedded session — it must not take keys otherwise. */
  let sessFocused = $state(false);
  /** Phone has no room for an embedded transcript; open the session in Agents. */
  async function openSession(sid: string): Promise<void> {
    if (viewport.isPhone) {
      ws.navigateToSession(sid);
      return;
    }
    sessFocused = false;
    openSessionId = sid;
    await tick();
    sessEl?.scrollIntoView({ block: 'nearest', behavior: scrollBehavior() });
  }
  function sessionBlur(e: FocusEvent): void {
    const next = e.relatedTarget;
    if (!(next instanceof Node) || !sessEl?.contains(next)) sessFocused = false;
  }

  // Load + poll the open detail; stop polling when this view goes away.
  $effect(() => {
    void loops.loadDetail(id);
    loops.startPoll(id);
    return () => loops.stopPoll();
  });

  const detail = $derived(loops.detail && loops.detail.loop.id === id ? loops.detail : null);
  const loop = $derived<GoalLoop | null>(detail?.loop ?? null);
  // Newest iteration first.
  const iterations = $derived([...(detail?.iterations ?? [])].reverse());
  /** Criterion id → its text, so evaluation chips can say what they judged. */
  const critText = $derived(
    Object.fromEntries((loop?.definition.acceptance_criteria ?? []).map((c) => [c.id, c.text])) as Record<string, string>,
  );
  /** Latest verdict per criterion (from the newest evaluated iteration). */
  const latestMet = $derived.by(() => {
    const ev = iterations.find((it) => it.evaluation)?.evaluation;
    return Object.fromEntries((ev?.criteria ?? []).map((c) => [c.id, c.met])) as Record<string, boolean>;
  });

  const PHASES = ['planning', 'executing', 'evaluating', 'digesting'] as const;
  const PHASE_LABEL: Record<(typeof PHASES)[number], string> = {
    planning: 'Plan',
    executing: 'Execute',
    evaluating: 'Evaluate',
    digesting: 'Digest',
  };
  function phaseIndex(loop: GoalLoop): number {
    const p = loop.phase === 'waiting' ? 'executing' : loop.phase;
    return PHASES.indexOf(p as (typeof PHASES)[number]);
  }
  /** Succeeded marks every phase done. Anything else keeps a current phase; only a running loop pulses. */
  function stepVisual(l: GoalLoop, i: number): { past: boolean; active: boolean; current: boolean } {
    if (l.status === 'succeeded') return { past: true, active: false, current: false };
    let idx = phaseIndex(l);
    if (idx < 0) idx = PHASES.length - 1;
    const onStep = idx === i;
    return { past: idx > i, active: l.status === 'running' && onStep, current: onStep };
  }
  function progressFill(status: string): string {
    switch (status) {
      case 'running':
        return 'running';
      case 'blocked':
      case 'exhausted':
        return 'warn';
      case 'failed':
        return 'bad';
      case 'succeeded':
        return 'ok';
      default:
        return 'dim';
    }
  }
  function criterionMark(met: boolean | undefined): { icon: IconName; label: string; kind: string } {
    if (met === true) return { icon: 'check', label: 'Met', kind: 'met' };
    if (met === false) return { icon: 'x', label: 'Not met', kind: 'unmet' };
    return { icon: 'minus', label: 'Not checked', kind: 'unchecked' };
  }

  function elapsedSecs(l: GoalLoop): number {
    let s = l.elapsed_secs;
    if (l.status === 'running' && l.run_started_at) {
      s += Math.max(0, (Date.now() - new Date(l.run_started_at).getTime()) / 1000);
    }
    return s;
  }

  /** Run a lifecycle/ledger call; a failure toasts with a human title and cause. */
  async function act(fn: () => Promise<void>, failTitle: string): Promise<void> {
    acting = true;
    try {
      await fn();
    } catch (e) {
      toastError(failTitle, e);
    } finally {
      acting = false;
    }
  }
  // Irreversible: confirm first, and only leave the page once the delete landed
  // (a failed delete keeps the user on the loop with the error toast).
  async function del(): Promise<void> {
    const loopId = id;
    const name = loop?.name ?? 'this goal loop';
    if (!(await confirmer.ask(`Delete “${name}” and its iteration history? Retained work on disk is kept.`, { title: 'Delete goal loop' }))) return;
    try {
      await loops.remove(loopId);
    } catch (e) {
      toastError('Couldn’t delete the goal loop', e);
      return;
    }
    toasts.success('Goal loop deleted', name);
    if (id === loopId) onback();
  }
  // Stop is terminal (a stopped loop can't be resumed), unlike Pause.
  async function stop(): Promise<void> {
    const loopId = id;
    if (!(await confirmer.ask('Stop this goal loop? A stopped loop can’t be resumed — use Pause to continue later.', { title: 'Stop goal loop', danger: true, confirmLabel: 'Stop loop' }))) return;
    await act(() => loops.stop(loopId), 'Couldn’t stop the goal loop');
  }
  /** ⋯ next to the primary verb. A draft’s Delete lives here so Start is the only header button. */
  function moreMenu(e: MouseEvent): void {
    const status = loop?.status;
    const items: MenuItem[] = [];
    if (status && status !== 'draft') {
      items.push({ label: 'Stop loop…', icon: 'stop' as const, danger: true, action: () => void stop() });
    }
    if (status !== 'running') {
      items.push({ label: 'Delete loop…', icon: 'trash' as const, danger: true, action: () => void del() });
    }
    ctxMenu.show(e, items);
  }
  // ---- Extend budget (L1): an exhausted loop re-exhausts on a plain Resume,
  // so raise its caps (PATCH limits) and then resume, in one step.
  let extendOpen = $state(false);
  let extIters = $state(0);
  let extMinutes = $state(0);
  let extError = $state('');
  let savedExtension = $state('');
  const cap = $derived(loop ? budgetCap(loop, elapsedSecs(loop)) : null);
  const budgetLeft = $derived(!!loop && hasBudgetLeft(loop, elapsedSecs(loop)));
  const unanswered = $derived(loop?.ledger?.questions.some((q) => !q.answer) ?? false);
  function openExtend(): void {
    if (!loop || acting || unanswered) return;
    if (loop.status !== 'paused' && loop.status !== 'blocked' && loop.status !== 'exhausted') return;
    const d = extendDefaults(loop, elapsedSecs(loop));
    extIters = d.max_iterations;
    extMinutes = d.runtime_minutes;
    extError = '';
    savedExtension = '';
    extendOpen = true;
  }
  async function extendAndResume(): Promise<void> {
    if (!loop || acting) return;
    const loopId = id;
    const iters = Math.floor(Number(extIters));
    const mins = Math.floor(Number(extMinutes));
    if (!(iters > 0) || !(mins > 0)) {
      extError = 'Iterations and runtime must be positive whole numbers.';
      return;
    }
    if (iters <= loop.iterations_started || mins * 60 <= elapsedSecs(loop)) {
      extError = `Raise both above what is already used (${loop.iterations_started} iterations, ${formatSeconds(Math.round(elapsedSecs(loop)))}).`;
      return;
    }
    extError = '';
    acting = true;
    try {
      const signature = `${loopId}:${iters}:${mins}`;
      if (savedExtension !== signature) {
        await loops.updateLimits(loopId, { ...loop.limits, max_iterations: iters, max_runtime_secs: mins * 60 });
        savedExtension = signature;
      }
      await loops.resume(loopId);
      if (id === loopId) extendOpen = false;
    } catch (e) {
      const detail = loadErrorText(e);
      const message = savedExtension === `${loopId}:${iters}:${mins}`
        ? `Budget saved, but the loop did not resume. Retry to resume with the saved budget. ${detail}`
        : detail;
      if (id === loopId && extendOpen) extError = message;
      else toasts.error('Couldn’t resume the goal loop', message);
    } finally {
      acting = false;
    }
  }
  const openQuestions = $derived((loop?.ledger?.questions ?? []).filter((q) => !q.answer));
  const answeredQuestions = $derived((loop?.ledger?.questions ?? []).filter((q) => !!q.answer));
  const canVerify = $derived(!!loop && ['paused', 'blocked', 'exhausted'].includes(loop.status));
  const canResume = $derived(!!loop && (loop.status === 'paused' || loop.status === 'blocked' || loop.status === 'exhausted'));

  $effect(() => {
    const loopId = id;
    const current = loop;
    const busy = acting;
    const blocked = unanswered;
    const resumable = canResume;
    const pauseFn = () => {
      if (busy || current?.status !== 'running') return;
      void act(() => loops.pause(loopId), 'Couldn’t pause the goal loop');
    };
    const resumeFn = () => {
      if (busy || blocked || !resumable || !current) return;
      if (!hasBudgetLeft(current, elapsedSecs(current))) {
        openExtend();
        return;
      }
      void act(() => loops.resume(loopId), 'Couldn’t resume the goal loop');
    };
    const stopFn = () => {
      if (!current || current.status === 'draft') return;
      void stop();
    };
    const extendFn = () => {
      openExtend();
    };
    if (!actions) return;
    actions.pause = pauseFn;
    actions.resume = resumeFn;
    actions.stop = stopFn;
    actions.extend = extendFn;
    return () => {
      actions.pause = () => {};
      actions.resume = () => {};
      actions.stop = () => {};
      actions.extend = () => {};
    };
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`detail-page`,});
 { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: { "title":loop?.name ?? 'Goal loop',leading:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Back to Goal Loops`,"aria-label":`Back to Goal Loops`,"onclick":onback,});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":14,}});}
     }
  };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
    if(loop){ { const $$_egdaBsutatS0C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS0C({ target: __sveltets_2_any(), props: {  "status":loopStatus(loop.status),}});}}
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
    if(loop){
      if(loop.status === 'running'){
         { svelteHTML.createElement("button", {       "class":`btn small`,...__sveltets_2_empty({"data-icon":`pause`}),"disabled":acting,"onclick":() => act(() => loops.pause(id), 'Couldn’t pause the goal loop'),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`pause`,"size":12,}});}  }
         { svelteHTML.createElement("button", {               "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-10`}),...__sveltets_2_empty({"data-icon":`more`}),...__sveltets_2_empty({"data-label":`More actions`}),"onclick":moreMenu,"aria-label":`More actions`,"title":`More actions`,"aria-haspopup":`menu`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
         }
      } else if (loop.status === 'paused' || loop.status === 'blocked' || loop.status === 'exhausted'){
        if(budgetLeft){
           { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-overflow":`-1`}),...__sveltets_2_empty({"data-icon":`plus`}),"disabled":unanswered || acting,"onclick":openExtend,"title":unanswered ? 'Resume stays off until this answer is recorded.' : 'Raise the iteration and runtime caps',});  }
           { svelteHTML.createElement("button", {        "class":`btn small`,"disabled":unanswered || acting,"title":unanswered ? 'Resume stays off until this answer is recorded.' : 'Continue iterating toward the goal',"onclick":() => act(() => loops.resume(id), 'Couldn’t resume the goal loop'),});!unanswered; { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}  }
        }else{
          
           { svelteHTML.createElement("button", {        "class":`btn small`,"disabled":unanswered || acting,"title":unanswered ? 'Resume stays off until this answer is recorded.' : 'Raise the caps, then continue iterating',"onclick":openExtend,});!unanswered; { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}    }
        }
         { svelteHTML.createElement("button", {               "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-10`}),...__sveltets_2_empty({"data-icon":`more`}),...__sveltets_2_empty({"data-label":`More actions`}),"onclick":moreMenu,"aria-label":`More actions`,"title":`More actions`,"aria-haspopup":`menu`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
         }
      } else if (loop.status === 'draft'){
         { svelteHTML.createElement("button", {       "class":`btn small primary`,"disabled":acting,"title":`Start iterating toward the goal`,"onclick":() => act(() => loops.start(id), 'Couldn’t start the goal loop'),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}  }
         { svelteHTML.createElement("button", {               "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-10`}),...__sveltets_2_empty({"data-icon":`more`}),...__sveltets_2_empty({"data-label":`More actions`}),"onclick":moreMenu,"aria-label":`More actions`,"title":`More actions`,"aria-haspopup":`menu`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
         }
      }else{
         { svelteHTML.createElement("button", {             "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-2`}),...__sveltets_2_empty({"data-icon":`trash`}),...__sveltets_2_empty({"data-label":`Delete loop`}),"onclick":del,"aria-label":`Delete goal loop`,"title":`Delete goal loop`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});} }
      }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, badge, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
  
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
 { svelteHTML.createElement("div", { "class":`detail`,});
  
   { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`this goal loop`,"variant":`page`,"loading":loops.loadingDetail,"error":loops.detailError,"empty":!loop,"onretry":() => void loops.loadDetail(id),}});
  if(loop){
     { svelteHTML.createElement("section", {   "class":`overview card-box`,"aria-label":`Progress`,});
       { svelteHTML.createElement("div", { "class":`progress-row`,});
         { svelteHTML.createElement("div", {           "class":`bar`,"role":`progressbar`,"aria-label":`Progress toward the goal`,"aria-valuemin":0,"aria-valuemax":100,"aria-valuenow":loop.progress_pct,});
           { svelteHTML.createElement("span", {  "class":`bar-fill ${progressFill(loop.status)}`,});__sveltets_2_ensureType(String, Number, `${loop.progress_pct}%`); }
         }
         { svelteHTML.createElement("span", { "class":`pct`,});loop.progress_pct;  }
       }

       { svelteHTML.createElement("ol", {   "class":`stepper`,"aria-label":`Iteration phases`,});
            for(let p of __sveltets_2_ensureArray(PHASES)){let i = 1;p;
          const vis = stepVisual(loop, i);
           { svelteHTML.createElement("li", {      "class":`step`,"aria-current":vis.current ? 'step' : undefined,});vis.active;vis.current && !vis.active;vis.past;
             { svelteHTML.createElement("span", { "class":`step-dot`,});if(vis.past){ { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}} }PHASE_LABEL[p];
           }
        }
        if(loop.status === 'running' && loop.phase === 'waiting'){ { svelteHTML.createElement("li", { "class":`step waiting`,});    }}
       }

       { svelteHTML.createElement("dl", { "class":`stats`,});
         { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});loop.current_iteration;  { svelteHTML.createElement("span", {  "class":`of`,});cap === 'iterations'; loop.limits.max_iterations; }if(cap === 'iterations'){  { svelteHTML.createElement("span", { "class":`cap-hit`,});  }} } }
         { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});formatSeconds(Math.round(elapsedSecs(loop)));  { svelteHTML.createElement("span", {  "class":`of`,});cap === 'runtime'; formatSeconds(loop.limits.max_runtime_secs); }if(cap === 'runtime'){  { svelteHTML.createElement("span", { "class":`cap-hit`,});  }} } }
         { svelteHTML.createElement("div", {}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});loop.config.mode === 'research' ? 'Research' : 'Build'; } }
        if(loop.branch){ { svelteHTML.createElement("div", { "class":`wide`,}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {   "class":`mono`,"title":loop.branch,});loop.branch; } }}
       }

      if(loop.summary){ { svelteHTML.createElement("p", { "class":`summary`,});loop.summary; }}
      if(loop.error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: loop.error, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`errline`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});}  { svelteHTML.createElement("span", {});loop.error; } }}}
      if(loop.ledger?.next_action){ { svelteHTML.createElement("p", { "class":`summary`,}); { svelteHTML.createElement("strong", {});  } loop.ledger.next_action; }}
      if(loop.worktree_path){
         { svelteHTML.createElement("p", { "class":`retained`,});   { svelteHTML.createElement("code", { "title":loop.worktree_path,});loop.worktree_path; } }
      }
     }

       for(let q of __sveltets_2_ensureArray(openQuestions)){q.id;
       { svelteHTML.createElement("section", {   "class":`decision`,"aria-label":`Decision needed`,});
         { svelteHTML.createElement("h3", { "class":`section-title`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}   }
         { svelteHTML.createElement("p", { "class":`q`,});q.question; }
         { svelteHTML.createElement("p", { "class":`resume-hint`,});        }
         { svelteHTML.createElement("textarea", {           "dir":`auto`,"class":`input answer`,"rows":2,"aria-label":`Answer question`,"placeholder":`Your answer — the agents read it on Resume`,"bind:value":answers[q.id],});/*Ωignore_startΩ*/() => answers[q.id] = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         { svelteHTML.createElement("div", { "class":`row-end`,});
           { svelteHTML.createElement("button", {       "class":`btn small primary`,"disabled":!answers[q.id]?.trim() || loop.status !== 'blocked' || acting,"title":loop.status !== 'blocked' ? 'Answers are recorded while the loop is blocked' : undefined,"onclick":() => act(() => loops.answerQuestion(id, q.id, answers[q.id]), 'Couldn’t record the answer'),});  }
         }
       }
    }

    if(loop.ledger?.review_summary){
       { svelteHTML.createElement("details", { "class":`review`,});
         { svelteHTML.createElement("summary", {});
           { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":loop.ledger.review_passed ? 'check' : 'warning',"size":12,}});}
             loop.ledger.review_passed ? 'passed' : 'needs attention';
         }
         { svelteHTML.createElement("pre", {});loop.ledger.review_summary; }
       }
    }

     { svelteHTML.createElement("section", { "class":`goal`,});
       { svelteHTML.createElement("h3", { "class":`section-title`,});  }
       { svelteHTML.createElement("p", { "class":`goal-sum`,});loop.definition.summary || loop.definition.title; }
       { svelteHTML.createElement("ul", {   "class":`crit-list`,"aria-label":`Acceptance criteria`,});
           for(let c of __sveltets_2_ensureArray(loop.definition.acceptance_criteria)){c.id;
          const mark = criterionMark(latestMet[c.id]);
           { svelteHTML.createElement("li", { "class":`crit`,});
             { svelteHTML.createElement("span", {   "class":`crit-mark ${mark.kind}`,"title":mark.label,});
               { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":mark.icon,"size":12,}});}
             }
             { svelteHTML.createElement("div", { "class":`crit-body`,});
               { svelteHTML.createElement("span", { "class":`crit-state ${mark.kind}`,});mark.label; }
               { svelteHTML.createElement("span", {   "class":`crit-text`,"id":`crit-text-${c.id}`,});c.text; }
               { svelteHTML.createElement("span", { "class":`crit-how`,});  c.verify_kind === 'human' ? 'a person' : c.verify_kind === 'command' ? 'a shell command' : 'an agent';  c.verify; }
              if(c.verify_kind === 'human'){
                const approval = humanVerification(c, loop.ledger);
                if(approval){ { svelteHTML.createElement("p", { "class":`verified`,}); { const $$_nocI9C = __sveltets_2_ensureComponent(Icon); new $$_nocI9C({ target: __sveltets_2_any(), props: {    "name":`userCheck`,"size":12,}});}   approval.verified_by; approval.evidence; }
                }else{
                   { svelteHTML.createElement("textarea", {             "dir":`auto`,"class":`input answer`,"rows":2,"aria-label":`Evidence for ${c.text}`,"aria-describedby":`crit-text-${c.id}`,"placeholder":`What did you verify?`,"bind:value":evidence[c.id],});/*Ωignore_startΩ*/() => evidence[c.id] = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                   { svelteHTML.createElement("div", { "class":`row-end`,});
                     { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":!evidence[c.id]?.trim() || !canVerify || acting,"title":canVerify ? undefined : 'Record verification while the loop is paused, blocked or exhausted',"onclick":() => act(() => loops.verifyCriterion(id, c.id, evidence[c.id]), 'Couldn’t record the verification'),});  }
                   }
                }
              }
             }
           }
        }
       }
     }

    if(answeredQuestions.length){
       { svelteHTML.createElement("section", { "class":`goal`,});
         { svelteHTML.createElement("h3", { "class":`section-title`,});  }
           for(let q of __sveltets_2_ensureArray(answeredQuestions)){q.id;
           { svelteHTML.createElement("p", { "class":`q-done`,}); { svelteHTML.createElement("span", { "class":`dim`,});q.question; } { svelteHTML.createElement("br", {});}q.answer;  { svelteHTML.createElement("span", { "class":`dim`,}); q.answered_by; } }
        }
       }
    }

    if(openSessionId && !viewport.isPhone){
       { const $$_section4 = svelteHTML.createElement("section", {        "class":`sess`,"aria-label":`Agent session`,"onfocusin":() => (sessFocused = true),"onfocusout":sessionBlur,});sessEl = $$_section4;
         { svelteHTML.createElement("div", { "class":`sess-bar`,});
           { svelteHTML.createElement("button", {     "type":`button`,"class":`btn small`,"onclick":() => ws.navigateToSession(openSessionId ?? ''),});   }
         }
         { svelteHTML.createElement("div", { "class":`sess-pane`,});
           { const $$_weiVnoisseS6C = __sveltets_2_ensureComponent(SessionView); new $$_weiVnoisseS6C({ target: __sveltets_2_any(), props: {           "sessionId":openSessionId,"focused":sessFocused,"showClose":true,"onfocus":() => (sessFocused = true),"onclosepane":() => { openSessionId = null; sessFocused = false; },}});}
         }
       }
    }

     { svelteHTML.createElement("section", { "class":`timeline`,});
       { svelteHTML.createElement("h3", { "class":`section-title`,});  }
      if(iterations.length === 0){
         { svelteHTML.createElement("p", { "class":`dim empty-line`,});loop.status === 'running' && loop.phase === 'waiting' ? 'Waiting on an agent.' : loop.status === 'running' ? 'The first iteration is being planned…' : 'No iterations yet.'; }
      }else{
            for(let it of __sveltets_2_ensureArray(iterations)){let i = 1;it.id;
           { const $$_woRnoitaretI5C = __sveltets_2_ensureComponent(IterationRow); new $$_woRnoitaretI5C({ target: __sveltets_2_any(), props: {               "iter":it,"loopId":id,"loopStatus":loop.status,"executorCount":loop.config.executors.length,"criteria":critText,"open":i === 0,"onopensession":(sid) => void openSession(sid),}});}
        }
      }
     }
  }
   LoadState}
 }
 PageBody}
 }

if(extendOpen && loop){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"title":`Extend budget`,"onclose":() => { if (!acting) extendOpen = false; },footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {     "class":`btn`,"disabled":acting,"onclick":() => (extendOpen = false),});  }
       { svelteHTML.createElement("button", {         "class":`btn primary`,"type":`submit`,"form":`gl-extend-form`,"disabled":acting || unanswered,"title":unanswered ? 'Resume stays off until this answer is recorded.' : undefined,});acting ? 'Resuming…' : 'Extend & resume'; }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("form", {     "class":`extend`,"id":`gl-extend-form`,"onsubmit":(e) => { e.preventDefault(); void extendAndResume(); },});
       { svelteHTML.createElement("p", { "class":`extend-lede`,});
        if(cap === 'iterations'){    } else if (cap === 'runtime'){    }else{        }
           loop.iterations_started;  formatSeconds(Math.round(elapsedSecs(loop)));
       }
       { svelteHTML.createElement("label", {   "class":`extend-lbl`,"for":`gl-ext-iters`,});  }
       { svelteHTML.createElement("input", {            "id":`gl-ext-iters`,"class":`input`,"type":`number`,"min":loop.iterations_started + 1,"step":`1`,"bind:value":extIters,});/*Ωignore_startΩ*/() => extIters = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("label", {   "class":`extend-lbl`,"for":`gl-ext-mins`,});   }
       { svelteHTML.createElement("input", {            "id":`gl-ext-mins`,"class":`input`,"type":`number`,"min":`1`,"step":`1`,"bind:value":extMinutes,});/*Ωignore_startΩ*/() => extMinutes = __sveltets_2_any(null);/*Ωignore_endΩ*/}
      if(extError){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),({ text: extError, assertive: true })));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`errline`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});}  { svelteHTML.createElement("span", {});extError; } }}}
     }
    
   Modal}
}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LoopDetail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LoopDetail__SvelteComponent_ = ReturnType<typeof LoopDetail__SvelteComponent_>;
/*Ωignore_endΩ*/export default LoopDetail__SvelteComponent_;