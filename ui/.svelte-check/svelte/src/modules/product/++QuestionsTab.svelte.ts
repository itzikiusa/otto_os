///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
// Questions tab — filters by status/category; inline edit, answer/discard,
// delete, add question; multi-select + post to Jira/Confluence.
import { product } from '../../lib/stores/product.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { confirmOutward } from '../../lib/confirmOutward';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { rel } from '../../lib/stores/now.svelte';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import type { Tone } from '../../lib/status';
import type { ProductQuestion, NewQuestionReq, UpdateQuestionReq } from './types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ── Load questions when tab becomes active ─────────────────────────────────
  // A failed load shows inline with Retry — never as "No questions yet".
  let loadError = $state<string | null>(null);
  async function load(): Promise<void> {
    loadError = null;
    try {
      await product.loadQuestions();
    } catch (e) {
      loadError = loadErrorText(e);
    }
  }
  $effect(() => {
    // Track selectedId reactively so we reload on story change.
    product.selectedId;
    if (product.selectedId) {
      void load();
    }
  });

  // ── Filter state ───────────────────────────────────────────────────────────
  type StatusFilter = 'all' | 'open' | 'posted' | 'answered' | 'discarded';
  type CatFilter = 'all' | 'scope' | 'data' | 'ux' | 'edge-case' | 'dependency' | 'other';

  let filterStatus = $state<StatusFilter>('all');
  let filterCat = $state<CatFilter>('all');

  const STATUS_OPTIONS: { value: StatusFilter; label: string }[] = [
    { value: 'all', label: 'All statuses' },
    { value: 'open', label: 'Open' },
    { value: 'posted', label: 'Posted' },
    { value: 'answered', label: 'Answered' },
    { value: 'discarded', label: 'Discarded' },
  ];

  const CAT_OPTIONS: { value: CatFilter; label: string }[] = [
    { value: 'all', label: 'All categories' },
    { value: 'scope', label: 'Scope' },
    { value: 'data', label: 'Data' },
    { value: 'ux', label: 'UX' },
    { value: 'edge-case', label: 'Edge case' },
    { value: 'dependency', label: 'Dependency' },
    { value: 'other', label: 'Other' },
  ];

  // ── Derived filtered list ──────────────────────────────────────────────────
  const filtered = $derived(
    product.questions.filter((q) => {
      if (filterStatus !== 'all' && q.status !== filterStatus) return false;
      if (filterCat !== 'all' && q.category !== filterCat) return false;
      return true;
    }),
  );

  // ── Multi-select ───────────────────────────────────────────────────────────
  let selectedIds = $state<Set<string>>(new Set());

  function toggleSelect(id: string): void {
    const next = new Set(selectedIds);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    selectedIds = next;
  }

  function toggleSelectAll(): void {
    if (selectedIds.size === filtered.length && filtered.length > 0) {
      selectedIds = new Set();
    } else {
      selectedIds = new Set(filtered.map((q) => q.id));
    }
  }

  // Clear selection when story changes.
  $effect(() => {
    product.selectedId;
    selectedIds = new Set();
    postingIds = false;
  });

  // ── Inline edit state ──────────────────────────────────────────────────────
  // editingId = which question is in edit mode; null = none.
  let editingId = $state<string | null>(null);
  let editText = $state('');
  let editRationale = $state('');
  let editCategory = $state('');

  function startEdit(q: ProductQuestion): void {
    editingId = q.id;
    editText = q.text;
    editRationale = q.rationale ?? '';
    editCategory = q.category ?? '';
  }

  function cancelEdit(): void {
    editingId = null;
  }

  // ── Answer modal state ─────────────────────────────────────────────────────
  let answeringId = $state<string | null>(null);
  let answerText = $state('');

  function startAnswer(q: ProductQuestion): void {
    answeringId = q.id;
    answerText = q.answer ?? '';
  }

  function cancelAnswer(): void {
    answeringId = null;
  }

  // ── Add question form ──────────────────────────────────────────────────────
  let addOpen = $state(false);
  let newText = $state('');
  let newRationale = $state('');
  let newCategory = $state('scope');
  let addWorking = $state(false);

  function openAdd(): void {
    addOpen = true;
    newText = '';
    newRationale = '';
    newCategory = 'scope';
  }
  function closeAdd(): void {
    addOpen = false;
  }

  // ── Busy flags ─────────────────────────────────────────────────────────────
  let savingId = $state<string | null>(null);
  let postingIds = $state(false);
  let deletingId = $state<string | null>(null);

  // ── Actions ─────────────────────────────────────────────────────────────────

  async function saveEdit(): Promise<void> {
    if (!editingId) return;
    const req: UpdateQuestionReq = {
      text: editText.trim() || undefined,
      rationale: editRationale.trim() || undefined,
      category: editCategory.trim() || undefined,
    };
    savingId = editingId;
    try {
      await product.updateQuestion(editingId, req);
      editingId = null;
    } catch (e) {
      toastError('Couldn’t save question', e);
    } finally {
      savingId = null;
    }
  }

  async function saveAnswer(): Promise<void> {
    if (!answeringId) return;
    const req: UpdateQuestionReq = {
      answer: answerText.trim() || undefined,
      status: 'answered',
    };
    savingId = answeringId;
    try {
      await product.updateQuestion(answeringId, req);
      answeringId = null;
    } catch (e) {
      toastError('Couldn’t save answer', e);
    } finally {
      savingId = null;
    }
  }

  async function discard(q: ProductQuestion): Promise<void> {
    savingId = q.id;
    try {
      await product.updateQuestion(q.id, { status: 'discarded' });
    } catch (e) {
      toastError('Couldn’t discard question', e);
    } finally {
      savingId = null;
    }
  }

  /** Reopen a discarded question (the undo for Discard). */
  async function reopen(q: ProductQuestion): Promise<void> {
    savingId = q.id;
    try {
      await product.updateQuestion(q.id, { status: 'open' });
    } catch (e) {
      toastError('Couldn’t reopen question', e);
    } finally {
      savingId = null;
    }
  }

  /** The row's ⋯ menu: everything past the one inline verb (Answer) —
   *  guidelines: more than two row actions go into a menu. */
  function rowMenu(e: MouseEvent, q: ProductQuestion): void {
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: 'Edit', icon: 'edit', action: () => startEdit(q) },
      q.status === 'discarded'
        ? { label: 'Reopen', icon: 'refresh', action: () => void reopen(q) }
        : { label: 'Discard', icon: 'x', action: () => void discard(q) },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void deleteQ(q) },
    ]);
  }

  async function deleteQ(q: ProductQuestion): Promise<void> {
    if (!(await confirmer.ask(`Delete question?\n\n“${q.text}”`, { title: 'Delete question', confirmLabel: 'Delete', danger: true }))) return;
    deletingId = q.id;
    try {
      await product.deleteQuestion(q.id);
      selectedIds = new Set([...selectedIds].filter((id) => id !== q.id));
    } catch (e) {
      toastError('Couldn’t delete question', e);
    } finally {
      deletingId = null;
    }
  }

  async function addQuestion(): Promise<void> {
    const text = newText.trim();
    if (!text) return;
    addWorking = true;
    try {
      const req: NewQuestionReq = {
        text,
        rationale: newRationale.trim() || null,
        category: newCategory || null,
      };
      await product.addQuestion(req);
      addOpen = false;
      toasts.success('Question added');
    } catch (e) {
      toastError('Couldn’t add question', e);
    } finally {
      addWorking = false;
    }
  }

  // Where a post lands — named on the button and in the confirm (patterns.md §5:
  // an outward action says where it goes, what is sent and who sees it).
  const story = $derived(product.detail?.story ?? null);
  const isJira = $derived(story?.source_kind === 'jira');
  const postTarget = $derived(isJira ? (story?.source_key ?? 'Jira') : 'Confluence');

  async function postSelected(): Promise<void> {
    const current = product.captureSelection();
    const ids = [...selectedIds];
    if (ids.length === 0) return;
    const texts = product.questions.filter((q) => selectedIds.has(q.id)).map((q) => `• ${q.text}`);
    const n = ids.length;
    const ok = await confirmOutward({
      verb: 'Post comment',
      title: `Post ${plural(n, 'question')} to ${postTarget}?`,
      where: isJira
        ? `Jira ${story?.source_key ?? ''} “${story?.title ?? ''}” — as one comment`
        : `Confluence page “${story?.title ?? ''}” — as one comment`,
      what: texts.join('\n'),
      who: 'Everyone who can see it; Jira/Confluence notify its watchers.',
    });
    if (!ok || !current()) return;
    postingIds = true;
    try {
      await product.postQuestions({ ids });
      if (!current()) return;
      selectedIds = new Set();
      toasts.success(`Posted ${plural(ids.length, 'question')}`);
    } catch (e) {
      if (current()) toastError('Couldn’t post the questions', e);
    } finally {
      if (current()) postingIds = false;
    }
  }

  // ── Helpers ────────────────────────────────────────────────────────────────

  /** Question status → one tone + a sentence-case word (StatusBadge). */
  function statusTone(status: string): Tone {
    switch (status) {
      case 'posted': return 'info';
      case 'answered': return 'success';
      default: return 'neutral';
    }
  }
  function statusLabel(status: string): string {
    return STATUS_OPTIONS.find((o) => o.value === status)?.label ?? status;
  }
  /** Category enum → its word ("edge-case" → "Edge case"); a neutral chip, no hues. */
  function catLabel(cat: string): string {
    return CAT_OPTIONS.find((o) => o.value === cat)?.label ?? cat;
  }

  function fmtDate(s: string): string {
    try { return rel(s); } catch { return s; }
  }
;
async () => {

if(!product.selectedId){
   { svelteHTML.createElement("div", { "class":`muted`,});   }
}else{
   { svelteHTML.createElement("div", { "class":`qtab`,});

    
     { svelteHTML.createElement("div", { "class":`toolbar`,});
      
       { svelteHTML.createElement("select", {       "class":`filter-sel`,"bind:value":filterStatus,"aria-label":`Filter by status`,});/*Ωignore_startΩ*/() => filterStatus = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let opt of __sveltets_2_ensureArray(STATUS_OPTIONS)){opt.value;
           { svelteHTML.createElement("option", { "value":opt.value,});opt.label; }
        }
       }

       { svelteHTML.createElement("select", {       "class":`filter-sel`,"bind:value":filterCat,"aria-label":`Filter by category`,});/*Ωignore_startΩ*/() => filterCat = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let opt of __sveltets_2_ensureArray(CAT_OPTIONS)){opt.value;
           { svelteHTML.createElement("option", { "value":opt.value,});opt.label; }
        }
       }

       { svelteHTML.createElement("span", { "class":`grow`,}); }

      
      if(selectedIds.size > 0){
         { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":postSelected,"disabled":postingIds,});
          postingIds ? 'Posting…' : `Post ${selectedIds.size} to ${postTarget}…`;
         }
      }

      
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":openAdd,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
     }

    
    if((product.loadingQuestions || loadError) && product.questions.length === 0){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`questions`,"loading":product.loadingQuestions,"error":loadError,"empty":true,"onretry":() => void load(),}});}
    } else if (filtered.length === 0){
       { svelteHTML.createElement("div", { "class":`muted`,});
        product.questions.length === 0
          ? 'No questions yet. Use Add question to create one.'
          : 'No questions match the current filters.';
       }
    }else{
      
       { svelteHTML.createElement("div", { "class":`select-all-row`,});
         { svelteHTML.createElement("label", {   "class":`cb-label`,"title":`Select / deselect all visible`,});
           { svelteHTML.createElement("input", {         "type":`checkbox`,"checked":selectedIds.size === filtered.length && filtered.length > 0,"indeterminate":selectedIds.size > 0 && selectedIds.size < filtered.length,"onchange":toggleSelectAll,});}
           { svelteHTML.createElement("span", { "class":`sel-count`,});
            selectedIds.size > 0 ? `${selectedIds.size} selected` : `${plural(filtered.length, 'question')}`;
           }
         }
       }

      
       { svelteHTML.createElement("div", { "class":`q-list`,});
           for(let q of __sveltets_2_ensureArray(filtered)){q.id;
           { svelteHTML.createElement("div", {  "class":`q-card`,});q.status === 'discarded';

            
             { svelteHTML.createElement("div", { "class":`q-header`,});
               { svelteHTML.createElement("input", {           "type":`checkbox`,"class":`q-cb`,"checked":selectedIds.has(q.id),"onchange":() => toggleSelect(q.id),"aria-label":`Select question`,});}

              if(editingId === q.id){
                
                 { svelteHTML.createElement("div", { "class":`edit-form`,});
                   { svelteHTML.createElement("textarea", {            "dir":`auto`,"class":`edit-text`,"bind:value":editText,"rows":3,"aria-label":`Question`,"placeholder":`e.g. Does the limit reset weekly or on a rolling 7 days?`,});/*Ωignore_startΩ*/() => editText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                   { svelteHTML.createElement("input", {          "dir":`auto`,"class":`edit-input`,"bind:value":editRationale,"aria-label":`Rationale (optional)`,"placeholder":`e.g. Changes how we store the reset date`,});/*Ωignore_startΩ*/() => editRationale = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                   { svelteHTML.createElement("select", {     "aria-label":`Category`,"class":`edit-sel`,"bind:value":editCategory,});/*Ωignore_startΩ*/() => editCategory = __sveltets_2_any(null);/*Ωignore_endΩ*/
                     { svelteHTML.createElement("option", { "value":`scope`,});  }
                     { svelteHTML.createElement("option", { "value":`data`,});  }
                     { svelteHTML.createElement("option", { "value":`ux`,});  }
                     { svelteHTML.createElement("option", { "value":`edge-case`,});  }
                     { svelteHTML.createElement("option", { "value":`dependency`,});  }
                     { svelteHTML.createElement("option", { "value":`other`,});  }
                   }
                   { svelteHTML.createElement("div", { "class":`edit-actions`,});
                     { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":saveEdit,"disabled":savingId === q.id,});
                      savingId === q.id ? 'Saving…' : 'Save';
                     }
                     { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":cancelEdit,"disabled":savingId === q.id,});
                      
                     }
                   }
                 }
              }else{
                
                 { svelteHTML.createElement("div", { "class":`q-body`,});
                   { svelteHTML.createElement("div", { "class":`q-top`,});
                     { svelteHTML.createElement("span", { "class":`q-text`,});q.text; }
                     { svelteHTML.createElement("div", { "class":`q-chips`,});
                       { svelteHTML.createElement("span", { "class":`chip`,});catLabel(q.category); }
                       { const $$_egdaBsutatS7C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS7C({ target: __sveltets_2_any(), props: {    "tone":statusTone(q.status),"label":statusLabel(q.status),}});}
                     }
                   }
                  if(q.rationale){
                     { svelteHTML.createElement("div", { "class":`q-rationale`,});q.rationale; }
                  }
                  if(q.answer){
                     { svelteHTML.createElement("div", { "class":`q-answer`,});
                       { svelteHTML.createElement("span", { "class":`answer-label`,});  }
                      q.answer;
                     }
                  }
                  if(q.posted_ref){
                     { svelteHTML.createElement("div", { "class":`q-ref`,});  { svelteHTML.createElement("span", { "class":`mono`,});q.posted_ref; } }
                  }
                   { svelteHTML.createElement("div", { "class":`q-meta`,});fmtDate(q.created_at); }
                 }
              }

              
              if(editingId !== q.id && answeringId !== q.id){
                 { svelteHTML.createElement("div", { "class":`q-actions`,});
                  
                   { svelteHTML.createElement("button", {         "class":`btn small ghost q-act`,"onclick":() => startAnswer(q),"disabled":savingId === q.id || deletingId === q.id,"title":`Answer / add context`,});q.answer ? 'Edit answer' : 'Answer'; }
                   { svelteHTML.createElement("button", {             "class":`icon-btn q-more`,"onclick":(e) => rowMenu(e, q),"disabled":savingId === q.id || deletingId === q.id,"aria-label":`More actions for this question`,"title":`More actions`,"aria-haspopup":`menu`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
                 }
              }
             }

            
            if(answeringId === q.id){
               { svelteHTML.createElement("div", { "class":`answer-panel`,});
                 { svelteHTML.createElement("textarea", {            "dir":`auto`,"class":`edit-text`,"bind:value":answerText,"rows":3,"aria-label":`Answer`,"placeholder":`e.g. Yes — limits apply per brand, confirmed with compliance`,});/*Ωignore_startΩ*/() => answerText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                 { svelteHTML.createElement("div", { "class":`edit-actions`,});
                   { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":saveAnswer,"disabled":savingId === q.id,});
                    savingId === q.id ? 'Saving…' : 'Save answer';
                   }
                   { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":cancelAnswer,"disabled":savingId === q.id,});
                    
                   }
                 }
               }
            }
           }
        }
       }
    }

    
    if(addOpen){
       { const $$_ladoM1C = __sveltets_2_ensureComponent(Modal); const $$_ladoM1 = new $$_ladoM1C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Add Question`,"width":480,"onclose":closeAdd,footer:() => { async ()/*Ωignore_positionΩ*/ => {
           { svelteHTML.createElement("button", {     "class":`btn`,"onclick":closeAdd,"disabled":addWorking,});  }
           { svelteHTML.createElement("button", {       "class":`btn primary`,"onclick":addQuestion,"disabled":addWorking || !newText.trim(),});
            addWorking ? 'Adding…' : 'Add question';
           }
        };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM1.$$prop_def;/*Ωignore_endΩ*/
         { svelteHTML.createElement("div", { "class":`qt-add-body`,});
           { svelteHTML.createElement("label", { "class":`form-label`,});  { svelteHTML.createElement("span", { "class":`req`,});  }
             { svelteHTML.createElement("textarea", {            "dir":`auto`,"class":`form-textarea`,"bind:value":newText,"rows":3,"placeholder":`e.g. Does the limit reset weekly or on a rolling 7 days?`,"disabled":addWorking,});/*Ωignore_startΩ*/() => newText = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
           }
           { svelteHTML.createElement("label", { "class":`form-label`,});
             { svelteHTML.createElement("input", {          "dir":`auto`,"class":`form-input`,"bind:value":newRationale,"placeholder":`e.g. Changes how we store the reset date`,"disabled":addWorking,});/*Ωignore_startΩ*/() => newRationale = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
           { svelteHTML.createElement("label", { "class":`form-label`,});
             { svelteHTML.createElement("select", {     "class":`form-select`,"bind:value":newCategory,"disabled":addWorking,});/*Ωignore_startΩ*/() => newCategory = __sveltets_2_any(null);/*Ωignore_endΩ*/
               { svelteHTML.createElement("option", { "value":`scope`,});  }
               { svelteHTML.createElement("option", { "value":`data`,});  }
               { svelteHTML.createElement("option", { "value":`ux`,});  }
               { svelteHTML.createElement("option", { "value":`edge-case`,});  }
               { svelteHTML.createElement("option", { "value":`dependency`,});  }
               { svelteHTML.createElement("option", { "value":`other`,});  }
             }
           }
         }
        
       Modal}
    }
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const QuestionsTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type QuestionsTab__SvelteComponent_ = ReturnType<typeof QuestionsTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default QuestionsTab__SvelteComponent_;