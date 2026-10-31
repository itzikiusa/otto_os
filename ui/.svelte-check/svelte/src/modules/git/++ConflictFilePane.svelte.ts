///<reference types="svelte" />
;// One conflicted file: loads its segmented view, renders context verbatim
// (dim) and each conflict as a ConflictHunk (A/B side + per-line picking).
// Below the hunks sits a GitKraken-style OUTPUT pane: a live preview of the
// recomposed file (context + chosen lines), with "conflict k of m" navigation
// that scrolls the hunk list. When every conflict has a choice the file can
// be "marked resolved" — we recompose the full file text and POST it.

import { scrollBehavior } from '../../lib/motion';
import type { ConflictFile, ConflictSegment } from '../../lib/api/types';
import { toastError } from '../../lib/toastError';
import { git } from '../../lib/stores/git.svelte';
import { api } from '../../lib/api/client';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import Icon from '../../lib/components/Icon.svelte';
import ConflictHunk from './ConflictHunk.svelte';
import { fileIsCrlf, markAction } from './conflictEdit';
;

  interface Props {
    repoId: string;
    path: string;
    /** Side labels for the A/B pickers (checked-out branch vs merge source). */
    oursLabel?: string;
    theirsLabel?: string;
    /** Called once the file has been marked resolved on the daemon. */
    onresolved: (path: string) => void;
  };;

  type OutSeg =
    | { kind: 'context'; text: string }
    | { kind: 'resolved'; ord: number; text: string; deleted: boolean }
    | { kind: 'unresolved'; ord: number };;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { repoId, path, oursLabel = 'OURS', theirsLabel = 'THEIRS', onresolved }: Props = $props();

  // Replaced wholesale on load, never mutated: raw skips deep-proxying every
  // line of a lockfile-sized conflict.
  let file = $state.raw<ConflictFile | null>(null);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let saving = $state(false);
  /** Bumped by Retry to re-run the load effect for the same file. */
  let reloadKey = $state(0);

  // Per-conflict chosen lines. Indexed by the conflict's ordinal position
  // among `conflict` segments (0-based). null = undecided.
  //
  // $state.raw, NOT $state: the values come from ConflictHunk as plain arrays
  // and setChoice's no-change guard compares them by IDENTITY. A deep proxy
  // would re-wrap the stored array, the guard would never match, and the
  // hunk's onresolve effect → setChoice → re-render cycle would spin until
  // Svelte aborts it (effect_update_depth_exceeded). The array is only ever
  // replaced wholesale, so raw loses nothing.
  let choices = $state.raw<(string[] | null)[]>([]);

  $effect(() => {
    // Re-load whenever the selected file changes.
    const id = repoId;
    const p = path;
    void reloadKey;
    loading = true;
    loadError = null;
    file = null;
    choices = [];
    void git
      .getConflictFile(id, p)
      .then((f) => {
        file = f;
        const conflictCount = f.segments.filter((s) => s.kind === 'conflict').length;
        choices = new Array(conflictCount).fill(null);
        current = 0;
      })
      .catch((e) => {
        loadError = loadErrorText(e);
      })
      .finally(() => {
        loading = false;
      });
  });

  // Map a segment array index → its conflict ordinal (or -1 for context).
  function conflictOrdinal(segments: ConflictSegment[], segIdx: number): number {
    let ord = -1;
    for (let i = 0; i <= segIdx; i++) {
      if (segments[i].kind === 'conflict') ord++;
    }
    return ord;
  }

  const conflictCount = $derived(
    file ? file.segments.filter((s) => s.kind === 'conflict').length : 0
  );
  const decidedCount = $derived(choices.filter((c) => c !== null).length);
  const allDecided = $derived(conflictCount > 0 && decidedCount === conflictCount);
  /** Markable: every conflict decided (`compose`) — or a readable text file
   *  with NO markers left, fixed by hand (`keep`: stage its bytes as-is). A
   *  binary or absent working file is never markable: its segments are empty
   *  and the recomposition was `""` (S15-301) — it resolves by taking a side. */
  const mark = $derived(markAction(file, conflictCount, decidedCount));
  const canMark = $derived(mark !== null);
  /** The file's line endings, for hunks whose own sides can't tell (S15-308). */
  const crlfFile = $derived(file ? fileIsCrlf(file.segments) : false);

  function setChoice(ordinal: number, lines: string[] | null): void {
    if (choices[ordinal] === lines) return;
    const next = [...choices];
    next[ordinal] = lines;
    choices = next;
  }

  // ── Conflict navigation (Output pane header) ────────────────────────────────
  // 0-based index of the "current" conflict; prev/next scroll its hunk into
  // view in the hunk list so long files stay navigable.
  let current = $state(0);
  let hunkEls: (HTMLElement | null)[] = $state([]);

  function goTo(ord: number): void {
    if (conflictCount === 0) return;
    const next = ((ord % conflictCount) + conflictCount) % conflictCount;
    current = next;
    hunkEls[next]?.scrollIntoView({ behavior: scrollBehavior(), block: 'center' });
  }

  // ── Output preview ──────────────────────────────────────────────────────────
  let outputOpen = $state(true);


  /** The live recomposition shown in the Output pane: context verbatim, each
   *  conflict either its chosen lines or an explicit "unresolved" marker. */
  // Context segments are immutable per load: join each once, not on every
  // pick (a huge file re-joined all of its context per click).
  const joined = new WeakMap<object, string>();
  function contextText(seg: { lines: string[] }): string {
    let t = joined.get(seg);
    if (t === undefined) {
      t = seg.lines.join('\n');
      joined.set(seg, t);
    }
    return t;
  }
  const outputSegs = $derived.by((): OutSeg[] => {
    if (!file || !outputOpen) return [];
    const out: OutSeg[] = [];
    let ord = -1;
    for (const seg of file.segments) {
      if (seg.kind === 'context') {
        if (seg.lines.length > 0) out.push({ kind: 'context', text: contextText(seg) });
      } else {
        ord++;
        const choice = choices[ord];
        if (choice === null || choice === undefined) {
          out.push({ kind: 'unresolved', ord });
        } else {
          out.push({
            kind: 'resolved',
            ord,
            text: choice.join('\n'),
            deleted: choice.length === 0,
          });
        }
      }
    }
    return out;
  });

  /**
   * Recompose the full file text from the segments + the user's choices:
   * context segments contribute their lines verbatim; conflict segments
   * contribute the chosen lines. Lines are joined with "\n" and a trailing
   * newline is added (git's normalised file form).
   */
  function composeContent(): string {
    if (!file) return '';
    const out: string[] = [];
    let ord = -1;
    for (const seg of file.segments) {
      if (seg.kind === 'context') {
        out.push(...seg.lines);
      } else {
        ord++;
        out.push(...(choices[ord] ?? []));
      }
    }
    return out.length === 0 ? '' : out.join('\n') + (file.trailing_newline ? '\n' : '');
  }

  async function takeSide(side: 'ours' | 'theirs' | 'keep' | 'delete'): Promise<void> {
    if (!file || saving) return;
    const deleting = side === 'delete' || (side === 'ours' && !file.ours_present) || (side === 'theirs' && !file.theirs_present);
    if (deleting && !await confirmer.ask(`Resolve ${path} by deleting it?`, { title: 'Resolve as deleted', confirmLabel: 'Delete', danger: true })) return;
    saving = true;
    try {
      await api.post(`/repos/${repoId}/conflict/resolve`, { path, side });
      onresolved(path);
    } catch (e) { toastError('Couldn’t resolve the file', e); }
    finally { saving = false; }
  }

  async function markResolved(): Promise<void> {
    if (!file || !canMark || saving) return;
    if (mark === 'keep') return takeSide('keep');
    saving = true;
    try {
      const content = composeContent();
      await git.resolveConflict(repoId, path, content);
      toasts.success('File resolved', path);
      onresolved(path);
    } catch (e) {
      toastError('Couldn’t resolve the file', e);
    } finally {
      saving = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`pane`,});
   { svelteHTML.createElement("div", { "class":`pane-head`,});
     { svelteHTML.createElement("span", {   "class":`mono pane-path`,"title":path,});path; }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    if(conflictCount > 0){
       { svelteHTML.createElement("span", {  "class":`progress`,});allDecided;
        decidedCount; conflictCount; 
       }
    }
     { svelteHTML.createElement("button", {         "class":`btn small primary`,"disabled":!canMark || saving,"onclick":markResolved,"title":canMark
        ? 'Mark this file resolved'
        : file && (file.is_binary || !file.worktree_present)
          ? 'Choose a whole side or delete the file above'
          : 'Resolve every conflict first',});
      saving ? 'Saving…' : 'Mark file resolved';
     }
   }

   { svelteHTML.createElement("div", { "class":`pane-body`,});
    if(!file){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {         "what":`this conflict`,loading,"error":loadError,"empty":true,"rows":8,"onretry":() => reloadKey++,}});}
    }else{
       { svelteHTML.createElement("div", { "class":`whole-actions`,});
         { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":saving,"onclick":() => takeSide('ours'),});file.ours_present ? `Take ${oursLabel}` : `Take ${oursLabel} (delete)`; }
         { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":saving,"onclick":() => takeSide('theirs'),});file.theirs_present ? `Take ${theirsLabel}` : `Take ${theirsLabel} (delete)`; }
        if(conflictCount === 0){
           { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":saving || !file.worktree_present,"onclick":() => takeSide('keep'),});   }
           { svelteHTML.createElement("button", {     "class":`btn small danger`,"disabled":saving,"onclick":() => takeSide('delete'),});  }
        }
       }
      if(file.is_binary){
         { svelteHTML.createElement("div", { "class":`binary dim`,});
                       
         }
      }else{
            for(let seg of __sveltets_2_ensureArray(file.segments)){let si = 1;si;
          if(seg.kind === 'context'){
            if(seg.lines.length > 0){
               { svelteHTML.createElement("pre", { "class":`context mono`,});seg.lines.join('\n'); }
            }
          }else{
            const ord = conflictOrdinal(file.segments, si);
             { const $$_div2 = svelteHTML.createElement("div", {  "class":`hunk-anchor`,});hunkEls[ord] = $$_div2;
               { const $$_knuHtcilfnoC3C = __sveltets_2_ensureComponent(ConflictHunk); new $$_knuHtcilfnoC3C({ target: __sveltets_2_any(), props: {                "ours":seg.ours,"theirs":seg.theirs,"base":seg.base,"index":ord + 1,path,oursLabel,theirsLabel,"fileCrlf":crlfFile,"onresolve":(lines) => setChoice(ord, lines),}});}
             }
          }
        }
        if(conflictCount === 0){
           { svelteHTML.createElement("div", {   "class":`dim`,"style":`padding: 16px; font-size: var(--fs-s)`,});
            if(file.worktree_present){          }else{               }
           }
        }
      }
    }
   }

  
  if(file && !file.is_binary && conflictCount > 0){
     { svelteHTML.createElement("div", {  "class":`output`,});outputOpen;
       { svelteHTML.createElement("div", { "class":`output-bar`,});
         { svelteHTML.createElement("button", {       "class":`output-toggle`,"onclick":() => (outputOpen = !outputOpen),"aria-expanded":outputOpen,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":outputOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
           { svelteHTML.createElement("span", { "class":`output-title`,});  }
         }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("span", { "class":`output-nav`,});
           { svelteHTML.createElement("span", { "class":`output-count`,}); Math.min(current + 1, conflictCount);  conflictCount; }
           { svelteHTML.createElement("button", {         "class":`nav-btn`,"onclick":() => goTo(current - 1),"title":`Previous conflict`,"aria-label":`Previous conflict`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronUp`,"size":12,}});}
           }
           { svelteHTML.createElement("button", {         "class":`nav-btn`,"onclick":() => goTo(current + 1),"title":`Next conflict`,"aria-label":`Next conflict`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
           }
         }
       }
      if(outputOpen){
         { svelteHTML.createElement("div", { "class":`output-body`,});
              for(let seg of __sveltets_2_ensureArray(outputSegs)){let i = 1;i;
            if(seg.kind === 'context'){
               { svelteHTML.createElement("pre", { "class":`out-context mono`,});seg.text; }
            } else if (seg.kind === 'resolved'){
              if(seg.deleted){
                 { svelteHTML.createElement("div", { "class":`out-deleted dim mono`,}); seg.ord + 1;   }
              }else{
                 { svelteHTML.createElement("pre", { "class":`out-resolved mono`,});seg.text; }
              }
            }else{
               { svelteHTML.createElement("button", {     "class":`out-unresolved`,"onclick":() => goTo(seg.ord),"title":`Jump to conflict ${seg.ord + 1}`,});
                 { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`merge`,"size":12,}});}
                 seg.ord + 1;  
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
const ConflictFilePane__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ConflictFilePane__SvelteComponent_ = ReturnType<typeof ConflictFilePane__SvelteComponent_>;
/*Ωignore_endΩ*/export default ConflictFilePane__SvelteComponent_;