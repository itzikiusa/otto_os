///<reference types="svelte" />
;// The Proof Pack modal: assembles GET /reviews/{id}/proof-pack (summary counts +
// every finding's evidence/reasoning/artifacts + its event timeline + the repo
// rules generated from this review), and offers Export which persists a markdown
// snapshot (POST .../proof-pack/export) and lets the user copy it.

import { getProofPack, exportProofPack } from '../../lib/api/client';
import { toastError } from '../../lib/toastError';
import { severityLabel, sentenceCase } from '../../lib/labels';
import { loadErrorText } from '../../lib/loadError';
import type { ReviewProofPack, Finding, FindingEvent } from '../../lib/api/types';
import Modal from '../../lib/components/Modal.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { toasts } from '../../lib/toast.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
;

  interface Props {
    reviewId: string;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { reviewId, onclose }: Props = $props();

  let pack: ReviewProofPack | null = $state(null);
  let loading = $state(true);
  let loadErr = $state('');
  let exporting = $state(false);
  let exportedMd = $state('');
  let loadGeneration = 0;

  $effect(() => {
    void load(reviewId);
    return () => { loadGeneration++; };
  });

  async function load(rid: string): Promise<void> {
    const generation = ++loadGeneration;
    loading = true;
    loadErr = '';
    pack = null;
    exportedMd = '';
    exporting = false;
    try {
      const result = await getProofPack(rid);
      if (generation === loadGeneration) pack = result;
    } catch (e) {
      if (generation === loadGeneration) loadErr = loadErrorText(e);
    } finally {
      if (generation === loadGeneration) loading = false;
    }
  }

  async function doExport(): Promise<void> {
    if (exporting || loading || loadErr || !pack) return;
    const generation = loadGeneration;
    const id = reviewId;
    exporting = true;
    try {
      const res = await exportProofPack(id);
      if (generation !== loadGeneration) return;
      exportedMd = res.markdown;
      toasts.success('Proof Pack exported', 'A markdown snapshot was saved.');
    } catch (e) {
      if (generation === loadGeneration) toastError('Couldn’t export Proof Pack', e);
    } finally {
      if (generation === loadGeneration) exporting = false;
    }
  }

  async function copyMarkdown(): Promise<void> {
    try {
      await copyTextOrThrow(exportedMd);
      toasts.success('Copied to clipboard');
    } catch {
      toasts.warn('Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.');
    }
  }

  /** A short `path:Lstart–Lend` location label. */
  function loc(f: Finding): string {
    if (!f.path) return '';
    if (f.line == null) return f.path;
    if (f.line_end != null && f.line_end !== f.line) return `${f.path}:L${f.line}–L${f.line_end}`;
    return `${f.path}:L${f.line}`;
  }

  function sortedStatus(by: Record<string, number>): [string, number][] {
    return Object.entries(by).sort((a, b) => b[1] - a[1]);
  }

  /** " · from → to" suffix for an event that carried a status transition. */
  function transitionLabel(ev: FindingEvent): string {
    if (ev.from_status && ev.to_status) return ` · ${sentenceCase(ev.from_status)} → ${sentenceCase(ev.to_status)}`;
    return '';
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    "title":`Proof Pack`,"width":720,onclose,children:() => { async ()/*Ωignore_positionΩ*/ => {
    if(loading){
       { const $$_notelekS0C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS0C({ target: __sveltets_2_any(), props: {    "rows":4,"height":32,}});}
    } else if (loadErr){
       { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {      "what":`the proof pack`,"error":loadErr,"empty":true,"onretry":() => void load(reviewId),}});}
    } else if (pack){
      
       { svelteHTML.createElement("div", { "class":`pp-summary`,});
         { svelteHTML.createElement("span", { "class":`pp-stat`,}); { svelteHTML.createElement("strong", {});pack.summary.total; }  }
         { svelteHTML.createElement("span", { "class":`pp-stat pp-ok`,}); { svelteHTML.createElement("strong", {});pack.summary.verified; }  }
         { svelteHTML.createElement("span", { "class":`pp-stat`,}); { svelteHTML.createElement("strong", {});pack.summary.fixed; }  }
         { svelteHTML.createElement("span", { "class":`pp-stat`,}); { svelteHTML.createElement("strong", {});pack.summary.open; }  }
         { svelteHTML.createElement("span", { "class":`pp-stat`,}); { svelteHTML.createElement("strong", {});pack.summary.with_commit; }   }
         { svelteHTML.createElement("span", { "class":`pp-stat`,}); { svelteHTML.createElement("strong", {});pack.summary.with_test; }   }
       }
       { svelteHTML.createElement("div", { "class":`pp-breakdown`,});
           for(let [st, n] of __sveltets_2_ensureArray(sortedStatus(pack.summary.by_status))){st;
           { svelteHTML.createElement("span", { "class":`chip pp-chip status-${st}`,});sentenceCase(st); n; }
        }
           for(let [sv, n] of __sveltets_2_ensureArray(sortedStatus(pack.summary.by_severity))){sv;
           { svelteHTML.createElement("span", { "class":`chip pp-chip sev2-${sv}`,});severityLabel(sv); n; }
        }
       }

      
       { svelteHTML.createElement("div", { "class":`pp-list`,});
           for(let entry of __sveltets_2_ensureArray(pack.findings)){entry.finding.id;
          const f = entry.finding;
           { svelteHTML.createElement("div", { "class":`pp-finding card`,});
             { svelteHTML.createElement("div", { "class":`pp-finding-head`,});
               { svelteHTML.createElement("span", { "class":`chip sev2-${f.severity}`,});severityLabel(f.severity); }
               { svelteHTML.createElement("span", { "class":`chip status-${f.status}`,});sentenceCase(f.status); }
               { svelteHTML.createElement("span", { "class":`pp-title`,});f.title || f.body.split('\n')[0]; }
             }
            if(loc(f)){ { svelteHTML.createElement("div", { "class":`pp-loc mono`,});loc(f); }}
            if(f.evidence){
               { svelteHTML.createElement("pre", { "class":`pp-evidence`,});f.evidence; }
            }
            if(f.agent_reasoning_summary){
               { svelteHTML.createElement("p", { "class":`pp-reason`,}); { svelteHTML.createElement("strong", {});  } f.agent_reasoning_summary; }
            }
             { svelteHTML.createElement("div", { "class":`pp-artifacts`,});
              if(f.linked_commit){ { svelteHTML.createElement("span", { "class":`chip pp-artifact`,}); f.linked_commit.slice(0, 9); }}
              if(f.linked_test){ { svelteHTML.createElement("span", { "class":`chip pp-artifact`,}); f.linked_test; }}
              if(f.jira_key){ { svelteHTML.createElement("span", { "class":`chip pp-artifact`,});f.jira_key; }}
             }
            if(entry.events.length > 0){
               { svelteHTML.createElement("ul", { "class":`pp-timeline`,});
                   for(let ev of __sveltets_2_ensureArray(entry.events)){ev.id;
                   { svelteHTML.createElement("li", { "class":`pp-event`,});
                     { svelteHTML.createElement("span", { "class":`pp-event-kind`,});sentenceCase(ev.kind); }
                     { svelteHTML.createElement("span", { "class":`pp-event-meta dim`,});ev.actor;transitionLabel(ev); }
                   }
                }
               }
            }
           }
        }
        if(pack.findings.length === 0){
           { svelteHTML.createElement("p", {   "class":`dim`,"style":`font-size: var(--fs-s)`,});     }
        }
       }

      
      if(pack.repo_rules.length > 0){
         { svelteHTML.createElement("h3", { "class":`pp-section`,});  pack.repo_rules.length;  }
         { svelteHTML.createElement("ul", { "class":`pp-rules`,});
             for(let r of __sveltets_2_ensureArray(pack.repo_rules)){r.id;
             { svelteHTML.createElement("li", { "class":`pp-rule`,}); { svelteHTML.createElement("strong", {});r.title; }  r.body; }
          }
         }
      }

      
      if(exportedMd){
         { svelteHTML.createElement("h3", { "class":`pp-section`,});  }
         { svelteHTML.createElement("pre", { "class":`pp-export`,});exportedMd; }
      }
    }
  };return __sveltets_2_any(0)},footer:() => { async ()/*Ωignore_positionΩ*/ => {
    if(exportedMd){
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => void copyMarkdown(),});  }
    }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":exporting || loading || !!loadErr || !pack,"onclick":() => void doExport(),});
      exporting ? 'Exporting…' : 'Export';
     }
     { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":onclose,});  }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {children, footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
  
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ProofPackView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProofPackView__SvelteComponent_ = ReturnType<typeof ProofPackView__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProofPackView__SvelteComponent_;