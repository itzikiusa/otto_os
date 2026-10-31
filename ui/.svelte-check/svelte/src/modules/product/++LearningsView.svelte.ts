///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
// LearningsView — workspace-scoped knowledge base.
// Patterns to follow (kind='pattern') vs Cases to avoid (kind='avoid').
// Inactive (AI-suggested) learnings surface with an Accept button.
import Icon from '../../lib/components/Icon.svelte';
import { product } from '../../lib/stores/product.svelte';
import { renderMarkdown } from '../../lib/md';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import { focusFirstField, refocusEditTrigger } from '../../lib/inlineEdit';
import type { ProductLearning, NewLearningReq, UpdateLearningReq } from './types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ── Filter (the view's own segmented control — no side rail) ─────────────────
  type LearningFilter = 'all' | 'pattern' | 'avoid';
  let filter = $state<LearningFilter>('all');
  const FILTERS: { value: LearningFilter; label: string }[] = [
    { value: 'all', label: 'All' },
    { value: 'pattern', label: 'Patterns to follow' },
    { value: 'avoid', label: 'Cases to avoid' },
  ];

  // Ref shape parsed from refs_json.
  interface LearningRef {
    type: string;
    ref: string;
    label?: string;
  }

  // ── Local state ───────────────────────────────────────────────────────────────
  let loaded = $state(false);

  // Add-form
  let addOpen = $state(false);
  let addKind = $state<'pattern' | 'avoid'>('pattern');
  let addTitle = $state('');
  let addBody = $state('');
  let addTags = $state('');
  let addRefs = $state('');
  let adding = $state(false);

  // Edit state: id → draft fields
  let editingId = $state<string | null>(null);
  let editKind = $state<'pattern' | 'avoid'>('pattern');
  let editTitle = $state('');
  let editBody = $state('');
  let editTags = $state('');
  let editRefs = $state('');
  let editSaving = $state(false);

  // Per-card busy flags
  let acceptingId = $state<string | null>(null);
  let deletingId = $state<string | null>(null);
  let togglingId = $state<string | null>(null);


  // Load all learnings on mount / when workspace changes.
  $effect(() => {
    if (!loaded) {
      void loadAll();
    }
  });

  /** A failed load — inline with Retry, never as "No patterns yet". */
  let loadError = $state<string | null>(null);
  async function loadAll(): Promise<void> {
    loadError = null;
    try {
      await product.loadLearnings(); // load ALL (active + inactive)
      loaded = true;
    } catch (e) {
      loadError = loadErrorText(e);
    }
  }

  // ── Derived ───────────────────────────────────────────────────────────────────

  const patterns = $derived(product.learnings.filter((l) => l.kind === 'pattern'));
  const avoids = $derived(product.learnings.filter((l) => l.kind === 'avoid'));

  const showPatterns = $derived(filter === 'all' || filter === 'pattern');
  const showAvoids = $derived(filter === 'all' || filter === 'avoid');

  // ── Helpers ───────────────────────────────────────────────────────────────────

  function parseTags(tags: string): string[] {
    return tags
      ? tags
          .split(',')
          .map((t) => t.trim())
          .filter(Boolean)
      : [];
  }

  function parseRefs(refsJson: string): LearningRef[] {
    if (!refsJson) return [];
    try {
      const parsed = JSON.parse(refsJson);
      if (Array.isArray(parsed)) return parsed as LearningRef[];
    } catch {
      // ignore malformed
    }
    return [];
  }

  function refHref(r: LearningRef): string | null {
    if (r.type === 'url') return r.ref;
    if (r.type === 'jira') return r.ref.startsWith('http') ? r.ref : null;
    if (r.type === 'confluence') return r.ref.startsWith('http') ? r.ref : null;
    return null;
  }

  function refClass(type: string): string {
    switch (type) {
      case 'jira': return 'ref-jira';
      case 'confluence': return 'ref-confluence';
      case 'url': return 'ref-url';
      default: return 'ref-other';
    }
  }

  // ── CRUD actions ──────────────────────────────────────────────────────────────

  async function addLearning(): Promise<void> {
    if (!addTitle.trim() || adding) return;
    adding = true;
    try {
      const refsArr = addRefs.trim()
        ? (JSON.parse(addRefs) as unknown)
        : undefined;
      const req: NewLearningReq = {
        kind: addKind,
        title: addTitle.trim(),
        body: addBody.trim(),
        tags: addTags.trim() || null,
        refs: refsArr,
      };
      await product.addLearning(req);
      toasts.success('Learning added');
      addOpen = false;
      addTitle = '';
      addBody = '';
      addTags = '';
      addRefs = '';
      addKind = 'pattern';
    } catch (e) {
      toastError('Couldn’t add learning', e);
    } finally {
      adding = false;
    }
  }

  function startEdit(l: ProductLearning): void {
    editingId = l.id;
    editKind = l.kind as 'pattern' | 'avoid';
    editTitle = l.title;
    editBody = l.body;
    editTags = l.tags ?? '';
    // Pretty-print refs for editing
    try {
      const parsed = JSON.parse(l.refs_json || '[]');
      editRefs = JSON.stringify(parsed, null, 2);
    } catch {
      editRefs = l.refs_json ?? '';
    }
  }

  // Inline-edit focus contract (lib/inlineEdit.ts): the form takes focus on
  // open; Cancel and a successful save hand it back to the row's Edit button.
  function cancelEdit(): void {
    const id = editingId;
    editingId = null;
    if (id) void refocusEditTrigger(`learning-${id}`);
  }

  async function saveEdit(id: string): Promise<void> {
    if (editSaving) return;
    editSaving = true;
    try {
      let refsVal: unknown = undefined;
      if (editRefs.trim()) {
        refsVal = JSON.parse(editRefs) as unknown;
      }
      const req: UpdateLearningReq = {
        kind: editKind,
        title: editTitle.trim() || null,
        body: editBody.trim() || null,
        tags: editTags.trim() || null,
        refs: refsVal,
      };
      await product.updateLearning(id, req);
      toasts.success('Learning updated');
      editingId = null;
      void refocusEditTrigger(`learning-${id}`);
    } catch (e) {
      toastError('Couldn’t save learning', e);
    } finally {
      editSaving = false;
    }
  }

  async function toggleActive(l: ProductLearning): Promise<void> {
    if (togglingId) return;
    togglingId = l.id;
    try {
      await product.updateLearning(l.id, { active: !l.active });
    } catch (e) {
      toastError('Couldn’t toggle active', e);
    } finally {
      togglingId = null;
    }
  }

  async function accept(id: string): Promise<void> {
    if (acceptingId) return;
    acceptingId = id;
    try {
      await product.acceptLearning(id);
      toasts.success('Learning accepted — now active');
    } catch (e) {
      toastError('Couldn’t accept learning', e);
    } finally {
      acceptingId = null;
    }
  }

  /** Delete one learning — confirmed in the shared dialog, naming it (the old
   *  inline bar sat at the top of the page, far from the card it deleted). */
  async function askDelete(id: string): Promise<void> {
    if (deletingId) return;
    const l = product.learnings.find((x) => x.id === id);
    const ok = await confirmer.ask(
      `Delete the learning “${l?.title ?? 'untitled'}”? Future analyses stop using it. This can’t be undone.`,
      { title: 'Delete learning', confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
    deletingId = id;
    try {
      await product.deleteLearning(deletingId);
      toasts.info('Learning deleted');
    } catch (e) {
      toastError('Couldn’t delete learning', e);
    } finally {
      deletingId = null;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`learnings-view`,});
  
   { svelteHTML.createElement("div", { "class":`lv-header`,});
     { svelteHTML.createElement("div", { "class":`lv-title-row`,});
       { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Show learnings`,});
           for(let f of __sveltets_2_ensureArray(FILTERS)){f.value;
           { svelteHTML.createElement("button", {     "aria-pressed":filter === f.value,"onclick":() => (filter = f.value),});filter === f.value;f.label; }
        }
       }
       { svelteHTML.createElement("span", { "class":`lv-count`,});plural(product.learnings.length, 'learning'); }
       { svelteHTML.createElement("span", { "class":`spacer`,}); }
       { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => (addOpen = !addOpen),"title":`Add a new learning`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}
         
       }
       { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":loadAll,"disabled":product.loadingLearnings,"title":`Refresh`,"aria-label":`Refresh learnings`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});}
       }
     }

   }

  
  if(addOpen){
     { svelteHTML.createElement("div", { "class":`form-card`,});
       { svelteHTML.createElement("div", { "class":`form-head`,});  }
       { svelteHTML.createElement("div", { "class":`form-row`,});
         { svelteHTML.createElement("label", {   "class":`field-label`,"for":`add-kind`,});  }
         { svelteHTML.createElement("select", {     "id":`add-kind`,"class":`mini-select`,"bind:value":addKind,});/*Ωignore_startΩ*/() => addKind = __sveltets_2_any(null);/*Ωignore_endΩ*/
           { svelteHTML.createElement("option", { "value":`pattern`,});   }
           { svelteHTML.createElement("option", { "value":`avoid`,});   }
         }
       }
       { svelteHTML.createElement("div", { "class":`form-row col`,});
         { svelteHTML.createElement("label", {   "class":`field-label`,"for":`add-title`,});  }
         { svelteHTML.createElement("input", {            "dir":`auto`,"id":`add-title`,"class":`field-input`,"type":`text`,"bind:value":addTitle,"placeholder":`Short title`,});/*Ωignore_startΩ*/() => addTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`form-row col`,});
         { svelteHTML.createElement("label", {   "class":`field-label`,"for":`add-body`,});  }
         { svelteHTML.createElement("textarea", {         "dir":`auto`,"id":`add-body`,"class":`field-textarea`,"bind:value":addBody,"placeholder":`Detailed description (markdown)`,});/*Ωignore_startΩ*/() => addBody = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }
       { svelteHTML.createElement("div", { "class":`form-row col`,});
         { svelteHTML.createElement("label", {   "class":`field-label`,"for":`add-tags`,});  { svelteHTML.createElement("span", { "class":`hint`,});  } }
         { svelteHTML.createElement("input", {            "dir":`auto`,"id":`add-tags`,"class":`field-input`,"type":`text`,"bind:value":addTags,"placeholder":`api, auth, performance`,});/*Ωignore_startΩ*/() => addTags = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
       { svelteHTML.createElement("div", { "class":`form-row col`,});
         { svelteHTML.createElement("label", {   "class":`field-label`,"for":`add-refs`,});  { svelteHTML.createElement("span", { "class":`hint`,});    } }
         { svelteHTML.createElement("textarea", {           "dir":`ltr`,"id":`add-refs`,"class":`field-textarea mono`,"bind:value":addRefs,"rows":3,"placeholder":`JSON array, e.g. type/ref/label objects`,});/*Ωignore_startΩ*/() => addRefs = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }
       { svelteHTML.createElement("div", { "class":`form-actions`,});
         { svelteHTML.createElement("button", {     "class":`btn primary`,"onclick":addLearning,"disabled":adding || !addTitle.trim(),});
          adding ? 'Adding…' : 'Add';
         }
         { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => (addOpen = false),});  }
       }
     }
  }

  
  if((loadError || (product.loadingLearnings && !loaded)) && product.learnings.length === 0){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`learnings`,"loading":product.loadingLearnings,"error":loadError,"empty":true,"onretry":() => void loadAll(),}});}
  }else{
  
   { svelteHTML.createElement("div", {  "class":`two-col`,});!showPatterns || !showAvoids;

    
    if(showPatterns){
     { svelteHTML.createElement("div", { "class":`col-section`,});
       { svelteHTML.createElement("div", { "class":`col-header pattern-header`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":13,}});}
         { svelteHTML.createElement("span", { "class":`col-title`,});   }
         { svelteHTML.createElement("span", { "class":`col-count`,});patterns.length; }
       }

      if(patterns.length === 0){
         { svelteHTML.createElement("div", { "class":`empty-col`,});   }
      }else{
           for(let l of __sveltets_2_ensureArray(patterns)){l.id;
          const refs = parseRefs(l.refs_json);
          const tags = parseTags(l.tags);
           { svelteHTML.createElement("div", {  "class":`learning-card`,});!l.active;
            if(!l.active){
               { svelteHTML.createElement("div", { "class":`suggested-banner`,});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}
                   
               }
            }

            if(editingId === l.id){
              
               {const $$action_0 = __sveltets_2_ensureAction(focusFirstField(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {  "class":`edit-form`,});
                 { svelteHTML.createElement("div", { "class":`form-row`,});
                   { svelteHTML.createElement("label", {   "class":`field-label`,"for":`ek-${l.id}`,});  }
                   { svelteHTML.createElement("select", {     "id":`ek-${l.id}`,"class":`mini-select`,"bind:value":editKind,});/*Ωignore_startΩ*/() => editKind = __sveltets_2_any(null);/*Ωignore_endΩ*/
                     { svelteHTML.createElement("option", { "value":`pattern`,});   }
                     { svelteHTML.createElement("option", { "value":`avoid`,});   }
                   }
                 }
                 { svelteHTML.createElement("div", { "class":`form-row col`,});
                   { svelteHTML.createElement("label", {   "class":`field-label`,"for":`et-${l.id}`,});  }
                   { svelteHTML.createElement("input", {          "dir":`auto`,"id":`et-${l.id}`,"class":`field-input`,"type":`text`,"bind:value":editTitle,});/*Ωignore_startΩ*/() => editTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                 }
                 { svelteHTML.createElement("div", { "class":`form-row col`,});
                   { svelteHTML.createElement("label", {   "class":`field-label`,"for":`eb-${l.id}`,});  }
                   { svelteHTML.createElement("textarea", {       "dir":`auto`,"id":`eb-${l.id}`,"class":`field-textarea`,"bind:value":editBody,});/*Ωignore_startΩ*/() => editBody = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                 }
                 { svelteHTML.createElement("div", { "class":`form-row col`,});
                   { svelteHTML.createElement("label", {   "class":`field-label`,"for":`etg-${l.id}`,});  }
                   { svelteHTML.createElement("input", {          "dir":`auto`,"id":`etg-${l.id}`,"class":`field-input`,"type":`text`,"bind:value":editTags,});/*Ωignore_startΩ*/() => editTags = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                 }
                 { svelteHTML.createElement("div", { "class":`form-row col`,});
                   { svelteHTML.createElement("label", {   "class":`field-label`,"for":`er-${l.id}`,});  }
                   { svelteHTML.createElement("textarea", {         "dir":`ltr`,"id":`er-${l.id}`,"class":`field-textarea mono`,"rows":3,"bind:value":editRefs,});/*Ωignore_startΩ*/() => editRefs = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                 }
                 { svelteHTML.createElement("div", { "class":`form-actions`,});
                   { svelteHTML.createElement("button", {     "class":`btn primary small`,"onclick":() => saveEdit(l.id),"disabled":editSaving,});editSaving ? 'Saving…' : 'Save'; }
                   { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":cancelEdit,});  }
                 }
               }}
            }else{
              
               { svelteHTML.createElement("div", { "class":`card-header`,});
                 { svelteHTML.createElement("span", { "class":`card-title`,});l.title; }
                 { svelteHTML.createElement("div", { "class":`card-actions`,});
                  if(!l.active){
                     { svelteHTML.createElement("button", {         "class":`btn accept-btn small`,"onclick":() => accept(l.id),"disabled":acceptingId === l.id,"title":`Accept this suggested learning`,});
                      acceptingId === l.id ? 'Accepting…' : 'Accept';
                     }
                  }
                   { svelteHTML.createElement("label", {   "class":`toggle-wrap`,"title":l.active ? 'Active — click to deactivate' : 'Inactive — click to activate',});
                     { svelteHTML.createElement("input", {           "type":`checkbox`,"class":`toggle-input`,"checked":l.active,"disabled":togglingId === l.id,"onchange":() => toggleActive(l),});}
                     { svelteHTML.createElement("span", { "class":`toggle-label`,});l.active ? 'Active' : 'Inactive'; }
                   }
                   { svelteHTML.createElement("button", {         "class":`icon-act`,"onclick":() => startEdit(l),"title":`Edit`,"aria-label":`Edit ${l.title}`,...__sveltets_2_empty({"data-edit-for":`learning-${l.id}`}),});
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}
                   }
                   { svelteHTML.createElement("button", {         "class":`icon-act danger`,"onclick":() => askDelete(l.id),"disabled":deletingId === l.id,"title":`Delete`,"aria-label":`Delete`,});
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
                   }
                 }
               }

              if(l.body){
                 { svelteHTML.createElement("div", { "class":`card-body md-body`,}); renderMarkdown(l.body); }
              }

              if(tags.length > 0){
                 { svelteHTML.createElement("div", { "class":`tags-row`,});
                    for(let tag of __sveltets_2_ensureArray(tags)){
                     { svelteHTML.createElement("span", { "class":`tag-chip`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`tag`,"size":12,}});}tag; }
                  }
                 }
              }

              if(refs.length > 0){
                 { svelteHTML.createElement("div", { "class":`refs-row`,});
                    for(let r of __sveltets_2_ensureArray(refs)){
                    const href = refHref(r);
                    if(href){
                       { svelteHTML.createElement("a", {       "class":`ref-badge ${refClass(r.type)}`,"href":href,"target":`_blank`,"rel":`noopener noreferrer`,});
                         { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`link`,"size":12,}});}
                        r.label || r.ref;
                       }
                    }else{
                       { svelteHTML.createElement("span", { "class":`ref-badge ${refClass(r.type)}`,});
                        r.label || r.ref;
                       }
                    }
                  }
                 }
              }
            }
           }
        }
      }
     }
    }

    
    if(showAvoids){
     { svelteHTML.createElement("div", { "class":`col-section`,});
       { svelteHTML.createElement("div", { "class":`col-header avoid-header`,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":13,}});}
         { svelteHTML.createElement("span", { "class":`col-title`,});   }
         { svelteHTML.createElement("span", { "class":`col-count`,});avoids.length; }
       }

      if(avoids.length === 0){
         { svelteHTML.createElement("div", { "class":`empty-col`,});     }
      }else{
           for(let l of __sveltets_2_ensureArray(avoids)){l.id;
          const refs = parseRefs(l.refs_json);
          const tags = parseTags(l.tags);
           { svelteHTML.createElement("div", {  "class":`learning-card avoid-card`,});!l.active;
            if(!l.active){
               { svelteHTML.createElement("div", { "class":`suggested-banner`,});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":12,}});}
                   
               }
            }

            if(editingId === l.id){
              
               {const $$action_0 = __sveltets_2_ensureAction(focusFirstField(svelteHTML.mapElementTag('div')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {  "class":`edit-form`,});
                 { svelteHTML.createElement("div", { "class":`form-row`,});
                   { svelteHTML.createElement("label", {   "class":`field-label`,"for":`ek2-${l.id}`,});  }
                   { svelteHTML.createElement("select", {     "id":`ek2-${l.id}`,"class":`mini-select`,"bind:value":editKind,});/*Ωignore_startΩ*/() => editKind = __sveltets_2_any(null);/*Ωignore_endΩ*/
                     { svelteHTML.createElement("option", { "value":`pattern`,});   }
                     { svelteHTML.createElement("option", { "value":`avoid`,});   }
                   }
                 }
                 { svelteHTML.createElement("div", { "class":`form-row col`,});
                   { svelteHTML.createElement("label", {   "class":`field-label`,"for":`et2-${l.id}`,});  }
                   { svelteHTML.createElement("input", {          "dir":`auto`,"id":`et2-${l.id}`,"class":`field-input`,"type":`text`,"bind:value":editTitle,});/*Ωignore_startΩ*/() => editTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                 }
                 { svelteHTML.createElement("div", { "class":`form-row col`,});
                   { svelteHTML.createElement("label", {   "class":`field-label`,"for":`eb2-${l.id}`,});  }
                   { svelteHTML.createElement("textarea", {       "dir":`auto`,"id":`eb2-${l.id}`,"class":`field-textarea`,"bind:value":editBody,});/*Ωignore_startΩ*/() => editBody = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                 }
                 { svelteHTML.createElement("div", { "class":`form-row col`,});
                   { svelteHTML.createElement("label", {   "class":`field-label`,"for":`etg2-${l.id}`,});  }
                   { svelteHTML.createElement("input", {          "dir":`auto`,"id":`etg2-${l.id}`,"class":`field-input`,"type":`text`,"bind:value":editTags,});/*Ωignore_startΩ*/() => editTags = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                 }
                 { svelteHTML.createElement("div", { "class":`form-row col`,});
                   { svelteHTML.createElement("label", {   "class":`field-label`,"for":`er2-${l.id}`,});  }
                   { svelteHTML.createElement("textarea", {         "dir":`ltr`,"id":`er2-${l.id}`,"class":`field-textarea mono`,"rows":3,"bind:value":editRefs,});/*Ωignore_startΩ*/() => editRefs = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                 }
                 { svelteHTML.createElement("div", { "class":`form-actions`,});
                   { svelteHTML.createElement("button", {     "class":`btn primary small`,"onclick":() => saveEdit(l.id),"disabled":editSaving,});editSaving ? 'Saving…' : 'Save'; }
                   { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":cancelEdit,});  }
                 }
               }}
            }else{
              
               { svelteHTML.createElement("div", { "class":`card-header`,});
                 { svelteHTML.createElement("span", { "class":`card-title`,});l.title; }
                 { svelteHTML.createElement("div", { "class":`card-actions`,});
                  if(!l.active){
                     { svelteHTML.createElement("button", {         "class":`btn accept-btn small`,"onclick":() => accept(l.id),"disabled":acceptingId === l.id,"title":`Accept this suggested learning`,});
                      acceptingId === l.id ? 'Accepting…' : 'Accept';
                     }
                  }
                   { svelteHTML.createElement("label", {   "class":`toggle-wrap`,"title":l.active ? 'Active — click to deactivate' : 'Inactive — click to activate',});
                     { svelteHTML.createElement("input", {           "type":`checkbox`,"class":`toggle-input`,"checked":l.active,"disabled":togglingId === l.id,"onchange":() => toggleActive(l),});}
                     { svelteHTML.createElement("span", { "class":`toggle-label`,});l.active ? 'Active' : 'Inactive'; }
                   }
                   { svelteHTML.createElement("button", {         "class":`icon-act`,"onclick":() => startEdit(l),"title":`Edit`,"aria-label":`Edit ${l.title}`,...__sveltets_2_empty({"data-edit-for":`learning-${l.id}`}),});
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}
                   }
                   { svelteHTML.createElement("button", {         "class":`icon-act danger`,"onclick":() => askDelete(l.id),"disabled":deletingId === l.id,"title":`Delete`,"aria-label":`Delete`,});
                     { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
                   }
                 }
               }

              if(l.body){
                 { svelteHTML.createElement("div", { "class":`card-body md-body`,}); renderMarkdown(l.body); }
              }

              if(tags.length > 0){
                 { svelteHTML.createElement("div", { "class":`tags-row`,});
                    for(let tag of __sveltets_2_ensureArray(tags)){
                     { svelteHTML.createElement("span", { "class":`tag-chip`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`tag`,"size":12,}});}tag; }
                  }
                 }
              }

              if(refs.length > 0){
                 { svelteHTML.createElement("div", { "class":`refs-row`,});
                    for(let r of __sveltets_2_ensureArray(refs)){
                    const href = refHref(r);
                    if(href){
                       { svelteHTML.createElement("a", {       "class":`ref-badge ${refClass(r.type)}`,"href":href,"target":`_blank`,"rel":`noopener noreferrer`,});
                         { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`link`,"size":12,}});}
                        r.label || r.ref;
                       }
                    }else{
                       { svelteHTML.createElement("span", { "class":`ref-badge ${refClass(r.type)}`,});
                        r.label || r.ref;
                       }
                    }
                  }
                 }
              }
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
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LearningsView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LearningsView__SvelteComponent_ = ReturnType<typeof LearningsView__SvelteComponent_>;
/*Ωignore_endΩ*/export default LearningsView__SvelteComponent_;