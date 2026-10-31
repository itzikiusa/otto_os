///<reference types="svelte" />
;// Renders one iteration's multi-signal eval score (composite + the five
// signals that feed it) and, on demand, the assembled proof pack. Read-only —
// it never mutates the eval; rating happens elsewhere.

import type { EvalScore } from '../../lib/api/types';
import { untrack } from 'svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { kindLabel } from '../../lib/labels';
import { skillsEvalApi } from '../../lib/api/skillsEval';
import Icon from '../../lib/components/Icon.svelte';
import { sentenceCase } from '../../lib/status';
;

  interface Props {
    score?: EvalScore | null;
    evalId?: string;
    iterId?: string;
    compact?: boolean;
  };;

  type Artifact = {
    kind: string;
    title: string;
    status: string;
    preview: string;
    truncated: boolean;
    metadata: unknown;
  };;;

  type Row = { label: string; score: number; detail: string; ran: boolean };;function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { score, evalId, iterId, compact = false }: Props = $props();


  let expanded = $state(false);
  let loading = $state(false);
  let proofError: string | null = $state(null);
  let proofRetry = $state(0);
  let artifacts = $state<Artifact[]>([]);

  // Proof-status pill palette: soft tone tint + text-safe tone, per the
  // design tokens (passed/failed/partial/waived; anything else = neutral).
  const grey = { bg: 'var(--surface-2)', fg: 'var(--text-dim)' };
  const PROOF: Record<string, { bg: string; fg: string }> = {
    passed: { bg: 'var(--success-soft)', fg: 'var(--success)' },
    failed: { bg: 'var(--danger-soft)', fg: 'var(--danger)' },
    partial: { bg: 'var(--warning-soft)', fg: 'var(--warning)' },
    waived: { bg: 'var(--accent-soft)', fg: 'var(--accent-text)' },
  };
  const proofTone = $derived(score ? (PROOF[score.proof_status] ?? grey) : grey);

  // Same bands as the score chips in RunDetail / CompareView (85 / 60), so a
  // bar never reads green next to an amber chip for the same number.
  function barColor(s: number): string {
    if (s >= 85) return 'var(--status-working)';
    if (s >= 60) return 'var(--status-warn)';
    return 'var(--status-exited)';
  }

  const rows = $derived<Row[]>(
    score
      ? [
          { label: 'Tests', score: score.tests.score, detail: score.tests.detail, ran: score.tests.ran },
          { label: 'Lint', score: score.lint.score, detail: score.lint.detail, ran: score.lint.ran },
          { label: 'Diff', score: score.diff.score, detail: score.diff.detail, ran: score.diff.ran },
          { label: 'Review', score: score.review.score, detail: score.review.detail, ran: score.review.ran },
          { label: 'Human', score: score.human.score, detail: score.human.note, ran: score.human.rating != null },
        ]
      : [],
  );

  function dotTone(status: string): string {
    if (status === 'passed') return 'var(--status-working)';
    if (status === 'failed') return 'var(--status-exited)';
    return 'var(--text-dim)';
  }

  function clamp(n: number): number {
    return Math.max(0, Math.min(100, n));
  }

  function clip(s: string): string {
    return s.length > 400 ? s.slice(0, 400) : s;
  }

  // Evidence changes when a rating or validator result is published. Keep an
  // open pack current, and discard a response for an older score or iteration.
  $effect(() => {
    const opened = expanded;
    const evaluation = evalId;
    const iteration = iterId;
    JSON.stringify(score);
    proofRetry;
    // A pending score may have a saved rating but only the previous published
    // proof artifacts. Do not present those as evidence of the new score.
    if (!opened || !evaluation || !iteration || score?.proof_status === 'pending') return;
    let current = true;
    untrack(() => {
      loading = true;
      proofError = null;
      artifacts = [];
      void skillsEvalApi.iterProofPack(evaluation, iteration).then(pack => {
        if (current) artifacts = pack.artifacts ?? [];
      }).catch(error => {
        if (current) proofError = loadErrorText(error);
      }).finally(() => {
        if (current) loading = false;
      });
    });
    return () => { current = false; };
  });

;
async () => {

if(score){
   { svelteHTML.createElement("div", {   "class":`scorecard`,...__sveltets_2_empty({"data-testid":`scorecard`}),});
     { svelteHTML.createElement("div", { "class":`head`,});
       { svelteHTML.createElement("div", { "class":`composite`,});
         { svelteHTML.createElement("span", {   "class":`num`,...__sveltets_2_empty({"data-testid":`scorecard-composite`}),});score.composite.toFixed(0); }
         { svelteHTML.createElement("span", { "class":`lbl`,});score.proof_status === 'pending' ? 'Provisional composite' : 'Composite'; }
       }
       { svelteHTML.createElement("div", { "class":`meta`,});
         { svelteHTML.createElement("span", {         "class":`proof`,...__sveltets_2_empty({"data-testid":`scorecard-proof`}),"style":`background:${proofTone.bg};color:${proofTone.fg}`,"title":`Proof pack status`,});
           sentenceCase(score.proof_status || 'missing');
         }
         { svelteHTML.createElement("span", {   "class":`done`,"title":`How much of the done contract (tests, lint, proof) this iteration met`,});  score.done_score;  }
       }
     }

     { svelteHTML.createElement("div", { "class":`signals`,});
         for(let r of __sveltets_2_ensureArray(rows)){r.label;
         { svelteHTML.createElement("div", {  "class":`row`,});!r.ran;
           { svelteHTML.createElement("span", { "class":`rlabel`,});r.label; }
          if(r.ran){
             { svelteHTML.createElement("span", { "class":`track`,});
               { svelteHTML.createElement("span", {   "class":`fill`,"style":`width:${clamp(r.score)}%;background:${barColor(r.score)}`,}); }
             }
             { svelteHTML.createElement("span", { "class":`rscore`,});r.score.toFixed(0); }
          }else{
             { svelteHTML.createElement("span", { "class":`notrun`,});r.label === 'Human' ? 'Not rated' : 'Not run'; }
             { svelteHTML.createElement("span", { "class":`rscore`,});  }
          }
          if(r.detail){ { svelteHTML.createElement("span", {   "class":`rdetail`,"title":r.detail,});r.detail; }}
         }
      }
     }

    if(!compact){
       { svelteHTML.createElement("button", {       "class":`btn small ghost proofpack-btn`,...__sveltets_2_empty({"data-testid":`scorecard-proofpack-btn`}),"aria-expanded":expanded,"onclick":() => (expanded = !expanded),});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":expanded ? 'chevronDown' : 'chevronRight',"size":12,}});}
        expanded ? 'Hide proof pack' : 'View proof pack';
       }

      if(expanded){
         { svelteHTML.createElement("div", { "class":`pack`,});
          if(score.proof_status === 'pending'){
             { svelteHTML.createElement("div", {   "class":`pmsg`,...__sveltets_2_empty({"data-testid":`proof-pending-evidence`}),});          }
          } else if (loading || proofError){
             { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {           "what":`the proof pack`,loading,"error":proofError,"empty":true,"variant":`compact`,"onretry":() => proofRetry++,}});}
          } else if (artifacts.length === 0){
             { svelteHTML.createElement("div", { "class":`pmsg`,});        }
          }else{
                for(let a of __sveltets_2_ensureArray(artifacts)){let i = 1;i;
               { svelteHTML.createElement("div", { "class":`artifact`,});
                 { svelteHTML.createElement("div", { "class":`ahead`,});
                   { svelteHTML.createElement("span", {   "class":`adot`,"style":`background:${dotTone(a.status)}`,}); }
                   { svelteHTML.createElement("span", { "class":`akind`,});kindLabel(a.kind); }
                   { svelteHTML.createElement("span", { "class":`sep`,});  }
                   { svelteHTML.createElement("span", { "class":`atitle`,});a.title; }
                   { svelteHTML.createElement("span", { "class":`sep`,});  }
                   { svelteHTML.createElement("span", { "class":`astatus`,});sentenceCase(a.status); }
                 }
                if(a.preview){
                   { svelteHTML.createElement("pre", { "class":`apreview`,});clip(a.preview);a.truncated ? '\n…' : ''; }
                }
               }
            }
          }
         }
      }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Scorecard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Scorecard__SvelteComponent_ = ReturnType<typeof Scorecard__SvelteComponent_>;
/*Ωignore_endΩ*/export default Scorecard__SvelteComponent_;