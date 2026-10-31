///<reference types="svelte" />
;
import Badge from '../../lib/components/Badge.svelte';
import { plural } from '../../lib/plural';
// Review-SQL modal (shared by cell edits, row duplication, deletes and the
// doc editor). The textarea is the source of truth for what runs — it is
// controlled: every keystroke goes back to the owner through `onsql`.
// `diff` (path · before → after per change) is shown above it as a compact
// table so a `$set`/`$unset` on a dotted path or a replaceOne reads as WHAT
// changes, not as a statement to parse; editing the statement does not
// update the table (it describes what the builder produced).
import Icon from '../../lib/components/Icon.svelte';
import Modal from '../../lib/components/Modal.svelte';
import type { DiffLine } from './EditFlow.svelte';
;

  interface Props {
    title: string;
    sql: string;
    diff?: DiffLine[];
    /** Why the last run stopped partway (statement, engine message, what was
     *  written) — the modal stays open so the SQL can be fixed and re-run. */
    error?: string | null;
    running: boolean;
    onsql: (sql: string) => void;
    onrun: () => void;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  /** Rows shown before the table folds the rest into a count. */
  const DIFF_MAX = 200;

  let { title, sql, diff, error = null, running, onsql, onrun, onclose }: Props = $props();

  const lines = $derived(diff ?? []);
  const shownLines = $derived(lines.slice(0, DIFF_MAX));
  // The row column only earns its place when the batch spans several rows.
  const multiRow = $derived(new Set(lines.map((d) => d.row)).size > 1);
  const OP_LABEL: Record<DiffLine['op'], string> = { set: '$set', unset: '$unset', rename: '$rename', cell: 'set' };

  // Esc (and the ✕) is the shared Modal's; a running statement can't be
  // dismissed mid-flight.
  function close(): void {
    if (!running) onclose();
  }

  function onReviewKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      // Same gate as the Run button: a second ⌘↵ while the first statement is
      // still in flight must not fire it again (an UPDATE/DELETE twice).
      if (!running && sql.trim()) onrun();
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {      children:() => { return __sveltets_2_any(0); },title,"width":640,"onclose":close,"dismissable":!running,footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("span", { "class":`review-kbd mono`,});       }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("button", {     "class":`btn`,"onclick":onclose,"disabled":running,});  }
     { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":onrun,"disabled":running || !sql.trim(),});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}running ? 'Running…' : 'Run';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
  
   { svelteHTML.createElement("div", {     "class":`review-modal`,"role":`presentation`,"onkeydown":onReviewKeydown,});
     { svelteHTML.createElement("div", { "class":`review-body`,});
      if(lines.length > 0){
         { svelteHTML.createElement("div", { "class":`review-diff-wrap`,});
           { svelteHTML.createElement("table", { "class":`review-diff mono`,});
             { svelteHTML.createElement("thead", {});
               { svelteHTML.createElement("tr", {});
                if(multiRow){ { svelteHTML.createElement("th", { "class":`rd-row`,});  }}
                 { svelteHTML.createElement("th", { "class":`rd-op`,}); { const $$_egdaB8C = __sveltets_2_ensureComponent(Badge); new $$_egdaB8C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":plural(lines.length, 'change'),}});} }
                 { svelteHTML.createElement("th", {});  }
                 { svelteHTML.createElement("th", {});  }
                 { svelteHTML.createElement("th", {   "class":`rd-arrow`,"aria-hidden":`true`,}); }
                 { svelteHTML.createElement("th", {});  }
               }
             }
             { svelteHTML.createElement("tbody", {});
                  for(let d of __sveltets_2_ensureArray(shownLines)){let i = 1;i;
                 { svelteHTML.createElement("tr", { "class":`op-${d.op}`,});
                  if(multiRow){ { svelteHTML.createElement("td", { "class":`rd-row`,});d.row < 0 ? 'new' : `#${d.row + 1}`; }}
                   { svelteHTML.createElement("td", { "class":`rd-op`,});OP_LABEL[d.op]; }
                   { svelteHTML.createElement("td", { "class":`rd-path`,});d.path; }
                   { svelteHTML.createElement("td", {  "class":`rd-val`,});d.before === '∅';d.before; }
                   { svelteHTML.createElement("td", {   "class":`rd-arrow`,"aria-hidden":`true`,});  }
                   { svelteHTML.createElement("td", {  "class":`rd-val`,});d.after === '∅';d.after; }
                 }
              }
              if(lines.length > DIFF_MAX){
                 { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("td", {   "colspan":multiRow ? 6 : 5,"class":`rd-more`,}); lines.length - DIFF_MAX;        } }
              }
             }
           }
         }
      }
      
       { svelteHTML.createElement("div", {    "class":`review-error`,"role":`alert`,});!!error;
        if(error){
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
           { svelteHTML.createElement("span", { "class":`review-error-text`,});error; }
        }
       }
       { svelteHTML.createElement("p", { "class":`review-hint`,});             }
       { svelteHTML.createElement("textarea", {                 "dir":`ltr`,"aria-label":`Statement to run`,"class":`review-sql mono`,"value":sql,"oninput":(e) => onsql(e.currentTarget.value),"disabled":running,"spellcheck":`false`,...__sveltets_2_empty({"data-autofocus":true}),"rows":5,}); }
     }
   }
  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ReviewModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ReviewModal__SvelteComponent_ = ReturnType<typeof ReviewModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default ReviewModal__SvelteComponent_;