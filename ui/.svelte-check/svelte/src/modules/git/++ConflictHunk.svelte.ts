///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
// One conflict segment, GitKraken-style: side A (ours) and side B (theirs)
// shown side-by-side, each with a HEADER CHECKBOX (take the whole side) and
// PER-LINE CHECKBOXES — so a resolution can mix parts of both sides. The
// output order is A's picked lines then B's picked lines; "Edit" opens a
// hand-edit buffer for anything fancier (reorder, rewrite). The resolution is
// reported up as the array of resolved lines via `onresolve` (null while the
// user hasn't touched the conflict yet).
import Icon from '../../lib/components/Icon.svelte';
import CodeEditor from '../../lib/components/CodeEditor.svelte';
import { editLines, editSeed, isCrlfBlock } from './conflictEdit';
;

  interface Props {
    ours: string[];
    theirs: string[];
    base: string[];
    /** 1-based index of this conflict within the file (for the label). */
    index: number;
    /** Repo path of the conflicted file — drives CodeEditor language detection. */
    path?: string;
    /** Work-tree root for the CodeEditor (LSP/context); optional. */
    root?: string;
    /** Side labels — the checked-out branch vs the merge source. */
    oursLabel?: string;
    theirsLabel?: string;
    /** The whole file's CRLF verdict (`fileIsCrlf`). A hunk with empty or
     *  single LF-less sides can't tell on its own (S15-308). */
    fileCrlf?: boolean;
    /** Fired whenever the resolution changes. `lines` is null while undecided. */
    onresolve: (lines: string[] | null) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let {
    ours,
    theirs,
    base,
    index,
    path = '',
    root = '',
    oursLabel = 'OURS',
    theirsLabel = 'THEIRS',
    fileCrlf,
    onresolve,
  }: Props = $props();

  // Per-line picks. Undecided until the user touches ANY control — from then on
  // the resolution is exactly the checked lines (possibly none = delete block).
  // Raw arrays, replaced wholesale on every pick (never mutated in place).
  let oursSel = $state.raw<boolean[]>([]);
  let theirsSel = $state.raw<boolean[]>([]);
  /** Pick rows mounted per side before "Show more": a lockfile / generated-
   *  file conflict is 5–30k lines a side, and every line is a button. The
   *  whole-side checkboxes and Edit still act on ALL lines. */
  const LINE_PAGE = 300;
  let shown = $state(LINE_PAGE);
  let touched = $state(false);
  let editing = $state(false);
  /** The applied hand edit ("Done editing"). While set it IS the resolution —
   *  leaving edit mode used to fall back to the picks and drop the edit. A new
   *  line/side pick replaces it (the user chose to pick again). */
  let manual = $state.raw<string[] | null>(null);
  /** An EMPTY side has no lines to tick, so its "take all" state lives here
   *  (otherwise the native checkbox ticked while the bound value stayed false). */
  let emptyTaken = $state({ ours: false, theirs: false });
  // (Re)seed the pick arrays whenever the conflict content itself changes —
  // a new file load replaces the segment arrays wholesale.
  $effect(() => {
    oursSel = ours.map(() => false);
    theirsSel = theirs.map(() => false);
    touched = false;
    editing = false;
    manual = null;
    emptyTaken = { ours: false, theirs: false };
    shown = LINE_PAGE;
  });
  const hiddenLines = $derived(Math.max(ours.length, theirs.length) - shown);
  // Edit-mode buffer (joined text the user can hand-edit).
  let editText = $state('');
  // Whether the diff3 merge base is currently expanded.
  let showBase = $state(false);

  // ≤1024 (phone + tablet): the A/B side-by-side columns are too narrow to
  // read, so the table stacks vertically (A block over B block).
  let isMobile = $state(false);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 1024px)');
    const sync = () => (isMobile = mq.matches);
    sync();
    mq.addEventListener('change', sync);
    return () => mq.removeEventListener('change', sync);
  });
  // True when the parser populated a merge base (diff3 conflict style).
  const hasBase = $derived(base.length > 0);

  const oursPicked = $derived(oursSel.filter(Boolean).length);
  const theirsPicked = $derived(theirsSel.filter(Boolean).length);
  const oursAll = $derived(ours.length > 0 ? oursPicked === ours.length : emptyTaken.ours);
  const theirsAll = $derived(theirs.length > 0 ? theirsPicked === theirs.length : emptyTaken.theirs);
  // CodeMirror drops `\r`; a CRLF block gets it re-appended on the way out.
  // The file's verdict wins when given — the block alone is often too small.
  const crlf = $derived(fileCrlf ?? isCrlfBlock(ours, theirs, base));

  // The resolved lines: A's picks in order, then B's picks in order.
  const resolved = $derived.by((): string[] | null => {
    // An empty editor still counts as a (deliberate) empty resolution.
    if (editing) return editLines(editText, crlf);
    if (manual !== null) return manual;
    if (!touched) return null;
    return [
      ...ours.filter((_, i) => oursSel[i]),
      ...theirs.filter((_, i) => theirsSel[i]),
    ];
  });

  // Propagate every resolution change to the parent.
  $effect(() => {
    onresolve(resolved);
  });

  function toggleLine(side: 'ours' | 'theirs', i: number): void {
    touched = true;
    manual = null;
    if (side === 'ours') {
      const next = [...oursSel];
      next[i] = !next[i];
      oursSel = next;
    } else {
      const next = [...theirsSel];
      next[i] = !next[i];
      theirsSel = next;
    }
  }

  /** Header checkbox: take (or drop) a whole side. Works on an EMPTY side too —
   *  taking an empty side is a deliberate "delete this block". */
  function toggleSide(side: 'ours' | 'theirs'): void {
    touched = true;
    manual = null;
    if ((side === 'ours' ? ours : theirs).length === 0) {
      emptyTaken = { ...emptyTaken, [side]: !emptyTaken[side] };
      return;
    }
    if (side === 'ours') oursSel = ours.map(() => !oursAll);
    else theirsSel = theirs.map(() => !theirsAll);
  }

  function startEdit(): void {
    if (!editing) {
      // Seed the editor with the current picks (or the "both" merge when the
      // conflict is still untouched) so users start from something sensible.
      const seed =
        manual ??
        (touched
          ? [...ours.filter((_, i) => oursSel[i]), ...theirs.filter((_, i) => theirsSel[i])]
          : [...ours, ...theirs]);
      editText = editSeed(seed);
    }
    editing = true;
  }

  /** "Done editing" APPLIES the edit: it becomes the resolution. */
  function stopEdit(): void {
    manual = editLines(editText, crlf);
    touched = true;
    editing = false;
  }

  /** Leave edit mode WITHOUT applying: back to the previous resolution. */
  function cancelEdit(): void {
    editing = false;
  }

;
async () => {

 { svelteHTML.createElement("div", {  "class":`hunk`,});resolved !== null;
   { svelteHTML.createElement("div", { "class":`hunk-bar`,});
     { svelteHTML.createElement("span", { "class":`hunk-label`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`merge`,"size":12,}});}
       index;
     }
    if(resolved !== null){
       { svelteHTML.createElement("span", { "class":`resolved-badge`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  }
    }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    if(editing){
       { svelteHTML.createElement("button", {     "class":`edit-btn`,"onclick":cancelEdit,"title":`Discard this edit and go back to picking`,});
         
       }
       { svelteHTML.createElement("button", {     "class":`edit-done`,"onclick":stopEdit,"title":`Use the edited text as this conflict’s resolution`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  
       }
    }else{
      if(manual !== null){
         { svelteHTML.createElement("span", {   "class":`side-partial`,"title":`This conflict is resolved with your hand-edited text; picking a line replaces it`,});  }
      }
       { svelteHTML.createElement("button", {     "class":`edit-btn`,"onclick":startEdit,"title":`Edit the resolution by hand`,});
        
       }
    }
   }

  if(editing){
     { svelteHTML.createElement("div", { "class":`edit-wrap`,});
       { svelteHTML.createElement("div", {     "class":`edit-editor`,"style":`height: ${Math.min(Math.max(editText.split('\n').length, 4), 24) * 18 + 14}px`,});
         { const $$_rotidEedoC3C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC3C({ target: __sveltets_2_any(), props: {         path,root,"content":editText,"readOnly":false,"onchange":(v) => (editText = v),}});}
       }
     }
  }else{
    if(hasBase){
       { svelteHTML.createElement("div", { "class":`base-block`,});
         { svelteHTML.createElement("button", {         "class":`base-toggle`,"aria-expanded":showBase,"onclick":() => (showBase = !showBase),"title":`What both sides diverged from (merge base)`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":showBase ? 'chevronDown' : 'chevronRight',"size":12,}});}
           { svelteHTML.createElement("span", { "class":`base-label`,});  }
           { svelteHTML.createElement("span", { "class":`base-hint dim`,});  plural(base.length, 'line'); }
         }
        if(showBase){
           { svelteHTML.createElement("pre", { "class":`base-code mono`,});base.join('\n'); }
        }
       }
    }

    
     { svelteHTML.createElement("div", {  "class":`side-heads`,});isMobile;
       { svelteHTML.createElement("label", {   "class":`side-head ours`,"title":`Take all of ${oursLabel}`,});
         { svelteHTML.createElement("input", {         "type":`checkbox`,"checked":oursAll,"onchange":() => toggleSide('ours'),"aria-label":`Take all of ${oursLabel} for conflict ${index}`,});}
         { svelteHTML.createElement("span", { "class":`side-tag tag-a`,});  }
         { svelteHTML.createElement("span", { "class":`side-name`,});oursLabel; }
        if(oursPicked > 0 && !oursAll){ { svelteHTML.createElement("span", { "class":`side-partial`,});oursPicked; ours.length; }}
       }
      if(!isMobile){
         { svelteHTML.createElement("label", {   "class":`side-head theirs`,"title":`Take all of ${theirsLabel}`,});
           { svelteHTML.createElement("input", {         "type":`checkbox`,"checked":theirsAll,"onchange":() => toggleSide('theirs'),"aria-label":`Take all of ${theirsLabel} for conflict ${index}`,});}
           { svelteHTML.createElement("span", { "class":`side-tag tag-b`,});  }
           { svelteHTML.createElement("span", { "class":`side-name`,});theirsLabel; }
          if(theirsPicked > 0 && !theirsAll){ { svelteHTML.createElement("span", { "class":`side-partial`,});theirsPicked; theirs.length; }}
         }
      }
     }

    if(isMobile){
      
       { svelteHTML.createElement("div", { "class":`stack-sides`,});
         { svelteHTML.createElement("div", { "class":`stack-side ours`,});
          if(ours.length === 0){
             { svelteHTML.createElement("div", { "class":`empty-side dim mono`,});       }
          }else{
                for(let line of __sveltets_2_ensureArray((ours.slice(0, shown)))){let i = 1;i;
               { svelteHTML.createElement("button", {      "class":`pick-line ours`,"aria-pressed":oursSel[i] === true,"onclick":() => toggleLine('ours', i),});oursSel[i];
                 { svelteHTML.createElement("span", { "class":`pick-box`,});if(oursSel[i]){ { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}} }
                 { svelteHTML.createElement("span", { "class":`mono pick-code`,});line; }
               }
            }
          }
         }
         { svelteHTML.createElement("label", {   "class":`side-head theirs stacked-b`,"title":`Take all of ${theirsLabel}`,});
           { svelteHTML.createElement("input", {         "type":`checkbox`,"checked":theirsAll,"onchange":() => toggleSide('theirs'),"aria-label":`Take all of ${theirsLabel} for conflict ${index}`,});}
           { svelteHTML.createElement("span", { "class":`side-tag tag-b`,});  }
           { svelteHTML.createElement("span", { "class":`side-name`,});theirsLabel; }
          if(theirsPicked > 0 && !theirsAll){ { svelteHTML.createElement("span", { "class":`side-partial`,});theirsPicked; theirs.length; }}
         }
         { svelteHTML.createElement("div", { "class":`stack-side theirs`,});
          if(theirs.length === 0){
             { svelteHTML.createElement("div", { "class":`empty-side dim mono`,});       }
          }else{
                for(let line of __sveltets_2_ensureArray((theirs.slice(0, shown)))){let i = 1;i;
               { svelteHTML.createElement("button", {      "class":`pick-line theirs`,"aria-pressed":theirsSel[i] === true,"onclick":() => toggleLine('theirs', i),});theirsSel[i];
                 { svelteHTML.createElement("span", { "class":`pick-box`,});if(theirsSel[i]){ { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}} }
                 { svelteHTML.createElement("span", { "class":`mono pick-code`,});line; }
               }
            }
          }
         }
       }
    }else{
       { svelteHTML.createElement("div", { "class":`split-grid`,});
         { svelteHTML.createElement("div", { "class":`split-col ours`,});
          if(ours.length === 0){
             { svelteHTML.createElement("div", { "class":`empty-side dim mono`,});       }
          }else{
                for(let line of __sveltets_2_ensureArray((ours.slice(0, shown)))){let i = 1;i;
               { svelteHTML.createElement("button", {      "class":`pick-line ours`,"aria-pressed":oursSel[i] === true,"onclick":() => toggleLine('ours', i),});oursSel[i];
                 { svelteHTML.createElement("span", { "class":`pick-box`,});if(oursSel[i]){ { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}} }
                 { svelteHTML.createElement("span", { "class":`mono pick-code`,});line; }
               }
            }
          }
         }
         { svelteHTML.createElement("div", { "class":`split-col theirs`,});
          if(theirs.length === 0){
             { svelteHTML.createElement("div", { "class":`empty-side dim mono`,});       }
          }else{
                for(let line of __sveltets_2_ensureArray((theirs.slice(0, shown)))){let i = 1;i;
               { svelteHTML.createElement("button", {      "class":`pick-line theirs`,"aria-pressed":theirsSel[i] === true,"onclick":() => toggleLine('theirs', i),});theirsSel[i];
                 { svelteHTML.createElement("span", { "class":`pick-box`,});if(theirsSel[i]){ { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}} }
                 { svelteHTML.createElement("span", { "class":`mono pick-code`,});line; }
               }
            }
          }
         }
       }
    }
    if(hiddenLines > 0){
       { svelteHTML.createElement("button", {   "class":`more-lines`,"onclick":() => (shown += LINE_PAGE * 10),});
         plural(Math.min(hiddenLines, LINE_PAGE * 10), 'more line'); hiddenLines;         
       }
    }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ConflictHunk__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ConflictHunk__SvelteComponent_ = ReturnType<typeof ConflictHunk__SvelteComponent_>;
/*Ωignore_endΩ*/export default ConflictHunk__SvelteComponent_;