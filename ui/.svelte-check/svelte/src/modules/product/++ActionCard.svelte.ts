///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
// ActionCard — renders ONE agent-proposed DiscoveryAction as a trustworthy
// card with an explicit Apply button. Nothing is applied until the PO clicks;
// every apply is reversible (sticky "✓ … · Undo" row) and toasted.
//
//   apply_draft   → diff-preview against the current draft, "Replace draft"
//                   (danger, confirm when non-empty); Undo re-applies the
//                   captured prior body via product.updateDraft.
//   add_questions → per-item checkbox (default all on), "Add N questions";
//                   Undo deletes the created question ids.
//   add_notes     → per-item checkbox (default all on), "Add note(s)";
//                   Undo deletes the created note ids.
//   create_canvas → lazy Mermaid thumbnail (if any), "Open in Canvas";
//                   navigates on canvas_id, then toasts.
import Icon from '../../lib/components/Icon.svelte';
import { product } from '../../lib/stores/product.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { router } from '../../lib/router.svelte';
import { api } from '../../lib/api/client';
import DiffView from '../../lib/components/DiffView.svelte';
import { renderMermaid } from '../../lib/render/mermaid';
import { ui } from '../../lib/stores/ui.svelte';
import { canvas } from '../../lib/stores/canvas.svelte';
import type { DiscoveryAction, UpdateDraftReq } from './types';

;type $$ComponentProps =  { cid: string; action: DiscoveryAction };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { cid, action }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  // ── Shared apply state ──────────────────────────────────────────────────────
  let applying = $state(false);
  // After a successful apply the card collapses to a sticky confirmation row.
  let applied = $state(false);
  let appliedLabel = $state('');
  let undoing = $state(false);
  // Captured so Undo can reverse the change.
  let priorDraftBody = $state<string | null>(null); // apply_draft snapshot
  let createdQuestionIds = $state<string[]>([]); // add_questions
  let createdNoteIds = $state<string[]>([]); // add_notes

  // ── apply_draft: diff preview + danger confirm ─────────────────────────────
  let showDiff = $state(false);
  const currentDraftBody = $derived(product.detail?.source?.body_md ?? '');
  const currentDraftTitle = $derived(product.detail?.story?.title ?? '');
  // Force line mode on phone/tablet (split/word are too dense for narrow screens).
  const diffMode = $derived<'line' | 'word'>(
    viewport.isPhone || viewport.isTablet ? 'line' : 'word',
  );

  async function applyDraft(): Promise<void> {
    if (applying || action.type !== 'apply_draft') return;
    // Claim the card BEFORE the confirmation: a double-click used to queue two
    // applies (S18-15).
    applying = true;
    const current = currentDraftBody;
    if (current.trim()) {
      const ok = await confirmer.ask(
        'Replace the current draft? This overwrites your text.',
        { confirmLabel: 'Replace', danger: true },
      );
      if (!ok) {
        applying = false;
        return;
      }
    }
    // Snapshot the user's text once so Undo restores THEIR original, never
    // the agent's text from an earlier apply.
    if (priorDraftBody === null) priorDraftBody = current;
    try {
      await product.applyDiscoveryAction(cid, action);
      await product.loadDetail();
      applied = true;
      appliedLabel = 'Draft replaced';
      toasts.success('Draft replaced');
    } catch (e) {
      toastError('Couldn’t replace draft', e);
    } finally {
      applying = false;
    }
  }

  async function undoDraft(): Promise<void> {
    if (undoing || priorDraftBody === null) return;
    undoing = true;
    try {
      const req: UpdateDraftReq = {
        title: product.detail?.story?.title ?? currentDraftTitle,
        body_md: priorDraftBody,
      };
      await product.updateDraft(req);
      applied = false;
      priorDraftBody = null; // restored — a later apply snapshots afresh
      toasts.success('Draft restored');
    } catch (e) {
      toastError('Couldn’t undo', e);
    } finally {
      undoing = false;
    }
  }

  // ── add_questions / add_notes: per-item checkboxes ─────────────────────────
  // Track *unchecked* indices instead of a per-item boolean array: that way the
  // default ("all checked") needs no read of `action` in a $state initializer
  // (which Svelte flags) — an index is checked unless it's in this set.
  const itemCount = $derived(
    action.type === 'add_questions'
      ? action.questions.length
      : action.type === 'add_notes'
        ? action.notes.length
        : 0,
  );
  let unchecked = $state<Set<number>>(new Set());
  const checkedCount = $derived(itemCount - unchecked.size);

  function isChecked(i: number): boolean {
    return !unchecked.has(i);
  }
  function toggle(i: number, on: boolean): void {
    const next = new Set(unchecked);
    if (on) next.delete(i);
    else next.add(i);
    unchecked = next;
  }

  async function applyQuestions(): Promise<void> {
    if (applying || action.type !== 'add_questions') return;
    const picked = action.questions.filter((_, i) => isChecked(i));
    if (picked.length === 0) return;
    applying = true;
    try {
      const result = await product.applyDiscoveryAction(cid, {
        type: 'add_questions',
        questions: picked,
      });
      createdQuestionIds = result.created_question_ids;
      await product.loadQuestions();
      applied = true;
      appliedLabel = `Added ${plural(picked.length, 'question')}`;
      toasts.success(appliedLabel);
    } catch (e) {
      toastError('Couldn’t add questions', e);
    } finally {
      applying = false;
    }
  }

  async function applyNotes(): Promise<void> {
    if (applying || action.type !== 'add_notes') return;
    const picked = action.notes.filter((_, i) => isChecked(i));
    if (picked.length === 0) return;
    applying = true;
    try {
      const result = await product.applyDiscoveryAction(cid, {
        type: 'add_notes',
        notes: picked,
      });
      createdNoteIds = result.created_note_ids;
      await product.loadNotes();
      applied = true;
      appliedLabel = `Added ${plural(picked.length, 'note')}`;
      toasts.success(appliedLabel);
    } catch (e) {
      toastError('Couldn’t add notes', e);
    } finally {
      applying = false;
    }
  }

  async function undoCreated(): Promise<void> {
    if (undoing) return;
    undoing = true;
    try {
      for (const qid of createdQuestionIds) {
        await api.del(`/product/questions/${qid}`);
      }
      for (const nid of createdNoteIds) {
        await api.del(`/product/notes/${nid}`);
      }
      if (createdQuestionIds.length) await product.loadQuestions();
      if (createdNoteIds.length) await product.loadNotes();
      createdQuestionIds = [];
      createdNoteIds = [];
      applied = false;
      toasts.success('Undone');
    } catch (e) {
      toastError('Couldn’t undo', e);
    } finally {
      undoing = false;
    }
  }

  // ── create_canvas: lazy Mermaid thumbnail + open ───────────────────────────
  let canvasSvg = $state<string | null>(null);
  let canvasThumbErr = $state<string | null>(null);
  let thumbId = `actcard-mmd-${Math.random().toString(36).slice(2)}`;

  // The SVG is agent-written: it renders in a sandbox="" iframe (no scripts,
  // no same-origin), like MockupViewer — never inlined into the app origin,
  // so a Mermaid/DOMPurify bypass can't reach the daemon (S18-08).
  function svgDoc(svg: string): string {
    return `<!doctype html><html><head><meta charset="utf-8">` +
      `<style>html,body{margin:0;padding:0;background:transparent;overflow:hidden;}` +
      `svg{max-width:100%;max-height:146px;height:auto;display:block;margin:0 auto;}</style></head>` +
      `<body>${svg}</body></html>`;
  }

  $effect(() => {
    if (action.type !== 'create_canvas') return;
    const src = action.mermaid?.trim();
    if (!src) return;
    // Re-render on a theme flip — the light palette is unreadable on dark.
    const dark = ui.resolvedScheme === 'dark';
    let alive = true;
    void renderMermaid(thumbId, src, { dark }).then((r) => {
      if (!alive) return;
      if (r.svg) canvasSvg = r.svg;
      else canvasThumbErr = r.error ?? 'Diagram error';
    });
    return () => {
      alive = false;
    };
  });

  async function applyCanvas(): Promise<void> {
    if (applying || action.type !== 'create_canvas') return;
    applying = true;
    try {
      const result = await product.applyDiscoveryAction(cid, action);
      applied = true;
      appliedLabel = 'Canvas created';
      if (result.canvas_id) {
        // Deep-link: ask the Canvas module to auto-open this scene on mount, then
        // navigate there.
        canvas.pendingOpenId = result.canvas_id;
        router.go('canvas');
        toasts.success('Opened in Canvas');
      } else {
        toasts.success('Canvas created');
      }
    } catch (e) {
      toastError('Couldn’t create canvas', e);
    } finally {
      applying = false;
    }
  }

  function undoApplicable(): boolean {
    return (
      priorDraftBody !== null ||
      createdQuestionIds.length > 0 ||
      createdNoteIds.length > 0
    );
  }

  function onUndo(): void {
    if (action.type === 'apply_draft') void undoDraft();
    else void undoCreated();
  }
;
async () => {

if(applied){
  
   { svelteHTML.createElement("div", { "class":`action-card applied-row`,});
     { svelteHTML.createElement("span", { "class":`applied-text`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} appliedLabel; }
    if(undoApplicable()){
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":onUndo,"disabled":undoing,});
        undoing ? 'Undoing…' : 'Undo';
       }
    }
   }
}else{
   { svelteHTML.createElement("div", { "class":`action-card`,});
    if(action.type === 'apply_draft'){
       { svelteHTML.createElement("div", { "class":`card-head`,});
         { svelteHTML.createElement("span", { "class":`card-kind`,});  }
         { svelteHTML.createElement("span", { "class":`card-title`,});action.title; }
       }
       { svelteHTML.createElement("div", { "class":`card-body`,});
         { svelteHTML.createElement("button", {       "class":`toggle-link`,"onclick":() => (showDiff = !showDiff),"aria-expanded":showDiff,});
          showDiff ? 'Hide changes' : 'Preview changes';
         }
         { svelteHTML.createElement("div", {  "class":`diff-wrap`,});!showDiff;
           { const $$_weiVffiD3C = __sveltets_2_ensureComponent(DiffView); new $$_weiVffiD3C({ target: __sveltets_2_any(), props: {         "before":currentDraftBody,"after":action.body_md,"mode":diffMode,"contextLines":showDiff ? 3 : undefined,}});}
         }
       }
       { svelteHTML.createElement("div", { "class":`card-actions`,});
         { svelteHTML.createElement("button", {     "class":`btn small danger`,"onclick":applyDraft,"disabled":applying,});
          applying ? 'Replacing…' : 'Replace draft';
         }
       }
    } else if (action.type === 'add_questions'){
       { svelteHTML.createElement("div", { "class":`card-head`,});
         { svelteHTML.createElement("span", { "class":`card-kind`,});  }
         { svelteHTML.createElement("span", { "class":`card-title`,});action.questions.length;  }
       }
       { svelteHTML.createElement("div", { "class":`card-body`,});
         { svelteHTML.createElement("ul", { "class":`item-list`,});
              for(let q of __sveltets_2_ensureArray(action.questions)){let i = 1;i;
             { svelteHTML.createElement("li", { "class":`item-row`,});
               { svelteHTML.createElement("label", { "class":`item-label`,});
                 { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":isChecked(i),"onchange":(e) => toggle(i, e.currentTarget.checked),});}
                 { svelteHTML.createElement("span", { "class":`item-main`,});
                   { svelteHTML.createElement("span", { "class":`item-text`,});q.text; }
                  if(q.rationale){ { svelteHTML.createElement("span", { "class":`item-sub`,});q.rationale; }}
                  if(q.category){ { svelteHTML.createElement("span", { "class":`item-cat`,});q.category; }}
                 }
               }
             }
          }
         }
       }
       { svelteHTML.createElement("div", { "class":`card-actions`,});
         { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":applyQuestions,"disabled":applying || checkedCount === 0,});
          applying
            ? 'Adding…'
            : `Add ${plural(checkedCount, 'question')}`;
         }
       }
    } else if (action.type === 'add_notes'){
       { svelteHTML.createElement("div", { "class":`card-head`,});
         { svelteHTML.createElement("span", { "class":`card-kind`,});  }
         { svelteHTML.createElement("span", { "class":`card-title`,});action.notes.length;  }
       }
       { svelteHTML.createElement("div", { "class":`card-body`,});
         { svelteHTML.createElement("ul", { "class":`item-list`,});
              for(let n of __sveltets_2_ensureArray(action.notes)){let i = 1;i;
             { svelteHTML.createElement("li", { "class":`item-row`,});
               { svelteHTML.createElement("label", { "class":`item-label`,});
                 { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":isChecked(i),"onchange":(e) => toggle(i, e.currentTarget.checked),});}
                 { svelteHTML.createElement("span", { "class":`item-main`,});
                   { svelteHTML.createElement("span", { "class":`item-text`,});n.body; }
                 }
               }
             }
          }
         }
       }
       { svelteHTML.createElement("div", { "class":`card-actions`,});
         { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":applyNotes,"disabled":applying || checkedCount === 0,});
          applying ? 'Adding…' : checkedCount === 1 ? 'Add note' : 'Add notes';
         }
       }
    } else if (action.type === 'create_canvas'){
       { svelteHTML.createElement("div", { "class":`card-head`,});
         { svelteHTML.createElement("span", { "class":`card-kind`,});  }
         { svelteHTML.createElement("span", { "class":`card-title`,});action.title; }
       }
      if(action.mermaid){
         { svelteHTML.createElement("div", { "class":`card-body`,});
          if(canvasSvg){
             { svelteHTML.createElement("div", { "class":`mmd-thumb`,});
               { svelteHTML.createElement("iframe", {          "class":`mmd-frame`,"title":`Diagram preview`,"sandbox":"","referrerpolicy":`no-referrer`,"srcdoc":svgDoc(canvasSvg),}); }
             }
          } else if (canvasThumbErr){
             { svelteHTML.createElement("div", { "class":`mmd-err`,});   }
          }else{
             { svelteHTML.createElement("div", { "class":`mmd-loading`,});  }
          }
         }
      }
       { svelteHTML.createElement("div", { "class":`card-actions`,});
         { svelteHTML.createElement("button", {     "class":`btn small primary`,"onclick":applyCanvas,"disabled":applying,});
          applying ? 'Opening…' : 'Open in Canvas';
         }
       }
    }
   }
}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ActionCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ActionCard__SvelteComponent_ = ReturnType<typeof ActionCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default ActionCard__SvelteComponent_;