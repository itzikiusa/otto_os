///<reference types="svelte" />
;// Snapshots (R1) + per-snapshot report export (R9). Lists a pack's immutable,
// content-hashed snapshots (seq / status / done score / sha256 tamper key /
// note / time), a "Snapshot" button to freeze the current state, and a
// .md / .html download for each frozen report.

import Icon from './Icon.svelte';
import { toastError } from '../../lib/toastError';
import ProofStatusChip from './ProofStatusChip.svelte';
import { createSnapshot, getSnapshot } from '../api/proof';
import { downloadText } from './exporters';
import { toasts } from '../toast.svelte';
import type { ProofSnapshotMeta } from '../api/types';
;

  interface Props {
    packId: string;
    snapshots: ProofSnapshotMeta[];
    onchange: () => void | Promise<void>;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let { packId, snapshots, onchange }: Props = $props();

  let busy = $state(false);
  let note = $state('');

  async function snap(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      await createSnapshot(packId, { note: note.trim() || undefined });
      note = '';
      await onchange();
      toasts.success('Snapshot created', 'An immutable, hashed copy was frozen.');
    } catch (e) {
      toastError('Couldn’t take the snapshot', e);
    } finally {
      busy = false;
    }
  }

  async function download(s: ProofSnapshotMeta, format: 'md' | 'html'): Promise<void> {
    try {
      const full = await getSnapshot(s.id);
      const text = format === 'md' ? full.report_md : full.report_html;
      downloadText(
        text,
        `proof-snapshot-${s.seq}.${format}`,
        format === 'md' ? 'text/markdown' : 'text/html',
      );
    } catch (e) {
      toastError('Couldn’t download the snapshot', e);
    }
  }

  function shortSha(sha: string): string {
    return sha ? sha.slice(0, 12) : '—';
  }

  function fmtTime(iso: string): string {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
  }
;
async () => {

 { svelteHTML.createElement("section", { "class":`snaps`,});
   { svelteHTML.createElement("div", { "class":`snaps-head`,});
     { svelteHTML.createElement("h3", { "class":`group-title`,});  { svelteHTML.createElement("span", { "class":`dim`,}); snapshots.length; } }
     { svelteHTML.createElement("div", { "class":`snap-new`,});
       { svelteHTML.createElement("input", {          "dir":`auto`,"class":`input note`,"bind:value":note,"placeholder":`Note (optional)`,"aria-label":`Snapshot note`,});/*Ωignore_startΩ*/() => note = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":snap,"disabled":busy,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`archive`,"size":12,}});} busy ? 'Snapshotting…' : 'Snapshot';
       }
     }
   }

  if(snapshots.length === 0){
     { svelteHTML.createElement("p", { "class":`dim empty`,});            }
  }else{
     { svelteHTML.createElement("div", { "class":`snap-list`,});
         for(let s of __sveltets_2_ensureArray(snapshots)){s.id;
         { svelteHTML.createElement("div", { "class":`snap-row`,});
           { svelteHTML.createElement("span", { "class":`seq`,}); s.seq; }
           { const $$_pihCsutatSfoorP3C = __sveltets_2_ensureComponent(ProofStatusChip); new $$_pihCsutatSfoorP3C({ target: __sveltets_2_any(), props: {    "status":s.status,"risk":s.risk_score,}});}
           { svelteHTML.createElement("span", {   "class":`score`,"title":`Done score`,});s.done_score;  }
           { svelteHTML.createElement("span", {   "class":`sha`,"title":s.sha256,}); shortSha(s.sha256); }
          if(s.note){ { svelteHTML.createElement("span", {   "class":`note-text ellipsis`,"title":s.note,});s.note; }}
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("span", { "class":`time`,});fmtTime(s.created_at); }
           { svelteHTML.createElement("button", {   "class":`link-btn`,"onclick":() => download(s, 'md'),});  }
           { svelteHTML.createElement("button", {   "class":`link-btn`,"onclick":() => download(s, 'html'),});  }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ProofSnapshotList__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ProofSnapshotList__SvelteComponent_ = ReturnType<typeof ProofSnapshotList__SvelteComponent_>;
/*Ωignore_endΩ*/export default ProofSnapshotList__SvelteComponent_;