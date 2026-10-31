///<reference types="svelte" />
;
import PageHeader from '../../lib/components/PageHeader.svelte';
import { toastError } from '../../lib/toastError';
import { onTabKey } from '../../lib/tabKeys';
import { sectionLabel } from './sections';
import { guardUnsaved } from '../../lib/leaveGuard';
import SectionIntro from './SectionIntro.svelte';
import { router } from '../../lib/router.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
// Context Library (root-only): author and edit the Otto-owned library of
// skills, souls, and context snippets — the single source of truth that gets
// materialized into each workspace's CLIs. Also sets the instance-wide default
// soul. Library writes are root-only on the server; this page is gated to root
// in Settings.svelte, mirroring Providers/Users/Daemon.
import { contextApi } from '../../lib/api/context';
import type {
    LibraryContext,
    LibrarySkill,
    LibrarySoul,
  } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import Icon from '../../lib/components/Icon.svelte';
import CodeEditor from '../../lib/components/CodeEditor.svelte';
import { loadErrorText } from '../../lib/loadError';
import { initialSelection, rememberSelection } from '../../lib/lastSelection';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // ---------------------------------------------------------------------------
  // State
  // ---------------------------------------------------------------------------

  type Tab = 'skills' | 'souls' | 'context';
  const TABS: Tab[] = ['skills', 'souls', 'context'];

  // A library entry as shown in the list (skills carry a description).
  interface Entry {
    name: string;
    description?: string;
  }

  let tab: Tab = $state('skills');
  /** Entry list width — the shared list-pane default/range, remembered. */
  let listW = $state(loadPaneWidth('settings.contextLibrary.listW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  let entries: Entry[] = $state([]);
  let defaultSoul: string | null = $state(null);
  let loading = $state(false);
  let loadError = $state('');

  // Editor pane state.
  let selected: string | null = $state(null); // entry name being edited; null = none
  let isNew = $state(false);
  let editName = $state('');
  let editBody = $state('');
  // The body/name as loaded, so switching away can warn about unsaved edits
  // (they used to vanish silently on a tab or entry click).
  let loadedName = $state('');
  let loadedBody = $state('');
  let bodyLoading = $state(false);
  let bodyError = $state('');
  let saving = $state(false);
  let nameError = $state('');
  // CodeEditor is uncontrolled after mount — bump to remount it on a new body.
  let editorKey = $state(0);

  const TAB_META: Record<Tab, { label: string; singular: string; ext: string }> = {
    skills: { label: 'Skills', singular: 'skill', ext: 'md' },
    souls: { label: 'Souls', singular: 'soul', ext: 'md' },
    context: { label: 'Context', singular: 'context snippet', ext: 'md' },
  };

  const meta = $derived(TAB_META[tab]);
  const dirty = $derived(selected !== null && !bodyLoading && (editName !== loadedName || editBody !== loadedBody));
  // Leaving Settings (or this page) with unsaved edits asks first.
  $effect(() => guardUnsaved(() => dirty, { what: isNew ? `the new ${meta.singular}` : `“${loadedName}”` }));

  // CodeEditor wants a path (for language detection) + a root. The library is not
  // a real workspace dir, so root is empty (LSP simply won't attach — fine for md).
  const editorPath = $derived(`${editName || 'untitled'}.md`);

  // ---------------------------------------------------------------------------
  // Load lists on tab change
  // ---------------------------------------------------------------------------

  $effect(() => {
    void loadTab(tab);
  });

  $effect(() => {
    void loadDefaultSoul();
  });

  async function fetchEntries(t: Tab): Promise<Entry[]> {
    if (t === 'skills') {
      const list = await contextApi.listSkills();
      return list.map((s: LibrarySkill) => ({ name: s.name, description: s.description }));
    }
    if (t === 'souls') {
      const list = await contextApi.listSouls();
      return list.map((s: LibrarySoul) => ({ name: s.name }));
    }
    const list = await contextApi.listContext();
    return list.map((c: LibraryContext) => ({ name: c.name }));
  }

  /** Load a tab's list and open an entry: `open` if given, else the
   *  remembered/first one — never a bare "pick one" pane when items exist. */
  async function loadTab(t: Tab, open?: string): Promise<void> {
    loading = true;
    try {
      const list = await fetchEntries(t);
      if (tab !== t) return;
      entries = list;
      loadError = '';
      const pick = open && list.some((e) => e.name === open) ? open : initialSelection(`context-library.${t}`, list, (e) => e.name);
      if (pick) void openEntry(pick);
      else closeEditor();
    } catch (e) {
      if (tab !== t) return;
      loadError = loadErrorText(e);
    } finally {
      if (tab === t) loading = false;
    }
  }

  async function loadDefaultSoul(): Promise<void> {
    try {
      const resp = await contextApi.getDefaultSoul();
      defaultSoul = resp.name;
    } catch {
      // non-fatal: leave default-soul selector empty
    }
  }

  // ---------------------------------------------------------------------------
  // Editor pane
  // ---------------------------------------------------------------------------

  /** Ask before throwing away unsaved edits. Resolves true when it's OK to go on. */
  async function okToLeave(): Promise<boolean> {
    if (!dirty) return true;
    return confirmer.ask(
      `Your changes to ${isNew ? `the new ${meta.singular}` : `“${loadedName}”`} haven’t been saved.`,
      { title: 'Discard changes?', danger: true, confirmLabel: 'Discard' },
    );
  }

  function closeEditor(): void {
    selected = null;
    isNew = false;
    editName = '';
    editBody = '';
    loadedName = '';
    loadedBody = '';
    nameError = '';
    bodyError = '';
  }

  async function switchTab(t: Tab): Promise<void> {
    if (t === tab || !(await okToLeave())) return;
    closeEditor();
    entries = [];
    tab = t;
  }

  async function startNew(): Promise<void> {
    if (!(await okToLeave())) return;
    closeEditor();
    isNew = true;
    selected = '';
    editorKey++;
    queueMicrotask(() => document.getElementById('lib-name')?.focus());
  }

  async function pickEntry(name: string): Promise<void> {
    if (name === selected && !isNew) return;
    if (!(await okToLeave())) return;
    void openEntry(name);
  }

  async function openEntry(name: string): Promise<void> {
    isNew = false;
    selected = name;
    editName = name;
    loadedName = name;
    editBody = '';
    loadedBody = '';
    nameError = '';
    bodyError = '';
    bodyLoading = true;
    rememberSelection(`context-library.${tab}`, name);
    try {
      let body: string;
      if (tab === 'skills') body = (await contextApi.getSkill(name)).body;
      else if (tab === 'souls') body = (await contextApi.getSoul(name)).body;
      else body = (await contextApi.getContext(name)).body;
      if (selected !== name) return;
      editBody = body;
      loadedBody = body;
      editorKey++;
    } catch (e) {
      if (selected === name) bodyError = loadErrorText(e);
    } finally {
      if (selected === name) bodyLoading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Save (create / update / rename = new name). The server upserts by name, so
  // renaming is "save under a new name"; we delete the old name afterwards.
  // ---------------------------------------------------------------------------

  async function save(): Promise<void> {
    const name = editName.trim();
    if (name === '') {
      nameError = `Give this ${meta.singular} a name first.`;
      return;
    }
    if (!/^[A-Za-z0-9_-]+$/.test(name)) {
      nameError = 'Use letters, numbers, “-” and “_” only.';
      return;
    }
    const previous = isNew ? null : selected;
    if (name !== previous && entries.some((e) => e.name === name)) {
      nameError = `A ${meta.singular} named “${name}” already exists.`;
      return;
    }
    nameError = '';
    saving = true;
    try {
      if (tab === 'skills') await contextApi.putSkill(name, editBody);
      else if (tab === 'souls') await contextApi.putSoul(name, editBody);
      else await contextApi.putContext(name, editBody);

      // Rename: the name changed on an existing entry → remove the old one.
      if (previous && previous !== name) {
        if (tab === 'skills') await contextApi.deleteSkill(previous);
        else if (tab === 'souls') await contextApi.deleteSoul(previous);
        else await contextApi.deleteContext(previous);
      }

      toasts.success(`${meta.singular[0].toUpperCase()}${meta.singular.slice(1)} saved`, name);
      // Mark clean before reloading so the reload doesn't ask to discard.
      loadedName = name;
      loadedBody = editBody;
      isNew = false;
      selected = name;
      await loadTab(tab, name);
    } catch (e) {
      toastError(`Couldn’t save the ${meta.singular}`, e);
    } finally {
      saving = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Delete
  // ---------------------------------------------------------------------------

  async function remove(name: string): Promise<void> {
    if (
      !(await confirmer.ask(
        `Delete the ${meta.singular} “${name}” from the library? Workspaces stop receiving it at their next session spawn.`,
        {
          title: `Delete ${meta.singular}`,
          danger: true, confirmLabel: 'Delete',
        },
      ))
    )
      return;
    try {
      if (tab === 'skills') await contextApi.deleteSkill(name);
      else if (tab === 'souls') await contextApi.deleteSoul(name);
      else await contextApi.deleteContext(name);
      toasts.info(`Deleted ${name}`);
      if (selected === name) closeEditor();
      rememberSelection(`context-library.${tab}`, null);
      await loadTab(tab);
    } catch (e) {
      toastError(`Couldn’t delete ${name}`, e);
    }
  }

  // ---------------------------------------------------------------------------
  // Global default soul
  // ---------------------------------------------------------------------------

  async function setDefaultSoul(name: string): Promise<void> {
    const prev = defaultSoul;
    try {
      const resp = await contextApi.setDefaultSoul(name);
      defaultSoul = resp.name;
      toasts.success('Default soul updated', resp.name ?? 'None');
    } catch (e) {
      defaultSoul = prev;
      toastError('Couldn’t set the default soul', e);
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":sectionLabel('context-library'),"subtitle":`Otto’s library of skills, souls and context snippets`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(selected !== null){
         { svelteHTML.createElement("button", {         "class":`btn small primary`,"disabled":saving || bodyLoading || (!dirty && !isNew),"title":dirty || isNew ? `Save this ${meta.singular}` : 'No unsaved changes',"onclick":save,});
          saving ? 'Saving…' : isNew ? `Create ${meta.singular}` : 'Save';
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});                 { svelteHTML.createElement("strong", {});   }                 { svelteHTML.createElement("button", {     "type":`button`,"class":`intro-link`,"onclick":() => router.go('settings/skills'),});   }         { svelteHTML.createElement("button", {     "type":`button`,"class":`intro-link`,"onclick":() => router.go('skills-eval'),});  }          SectionIntro}

  
   { svelteHTML.createElement("div", {     "class":`segmented lib-tabs`,"role":`tablist`,"aria-label":`Library`,});
       for(let t of __sveltets_2_ensureArray(TABS)){t;
       { svelteHTML.createElement("button", {                "id":`lib-tab-${t}`,"role":`tab`,"aria-selected":tab === t,"aria-controls":`lib-panel`,"tabindex":tab === t ? 0 : -1,"onclick":() => void switchTab(t),"onkeydown":onTabKey,});tab === t;
        TAB_META[t].label;
       }
    }
   }

   { svelteHTML.createElement("div", {     "id":`lib-panel`,"role":`tabpanel`,"aria-labelledby":`lib-tab-${tab}`,});
    
    if(tab === 'souls' && entries.length > 0){
       { svelteHTML.createElement("div", { "class":`default-soul field`,});
         { svelteHTML.createElement("label", { "for":`lib-default-soul`,});   }
         { svelteHTML.createElement("select", {         "id":`lib-default-soul`,"class":`input`,"value":defaultSoul ?? '',"onchange":(e) => setDefaultSoul(e.currentTarget.value),});
           { svelteHTML.createElement("option", {"value":"",});  }
             for(let e of __sveltets_2_ensureArray(entries)){e.name;
             { svelteHTML.createElement("option", { "value":e.name,});e.name; }
          }
         }
         { svelteHTML.createElement("span", { "class":`hint`,});           }
       }
    }

     { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL3 = new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {          children:() => { return __sveltets_2_any(0); },"what":`library ${meta.label.toLowerCase()}`,loading,"error":loadError,"empty":entries.length === 0 && !isNew,"rows":4,"onretry":() => void loadTab(tab),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
         { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {               "variant":`panel`,"icon":`book`,"title":`No ${meta.label.toLowerCase()} yet`,"body":tab === 'skills'
            ? 'Library skills are materialized into every workspace’s agent CLIs. Install bundled ones from Settings → Skills, or write your own.'
            : tab === 'souls'
              ? 'A soul is a persona injected into every interaction in a workspace.'
              : 'Context snippets are reusable markdown blocks added to agent context.',"actionLabel":`New ${meta.singular}`,"actionIcon":`plus`,"onaction":() => void startNew(),}});}
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL3.$$prop_def;/*Ωignore_endΩ*/
      
       { svelteHTML.createElement("div", { "class":`lib-body`,});
        
         { svelteHTML.createElement("div", {   "class":`list-pane`,"style":`--list-pane-w:${listW}px`,});
           { svelteHTML.createElement("div", { "class":`list-head`,});
             { svelteHTML.createElement("h2", { "class":`section-title`,});meta.label;  { svelteHTML.createElement("span", { "class":`count`,});entries.length; } }
             { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":() => void startNew(),"aria-label":`New ${meta.singular}`,"title":`New ${meta.singular}`,});
               { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});}
             }
           }
           { svelteHTML.createElement("div", {     "class":`entry-list`,"role":`group`,"aria-label":meta.label,});
            if(isNew){
               { svelteHTML.createElement("div", {   "class":`entry active new`,"aria-current":`true`,});
                 { svelteHTML.createElement("span", { "class":`entry-name mono`,});editName.trim() || `New ${meta.singular}`; }
                 { svelteHTML.createElement("span", { "class":`entry-desc dim`,});   }
               }
            }
               for(let e of __sveltets_2_ensureArray(entries)){e.name;
               { svelteHTML.createElement("button", {          "class":`entry`,"aria-current":selected === e.name && !isNew ? 'true' : undefined,"title":e.description ? `${e.name} — ${e.description}` : e.name,"onclick":() => void pickEntry(e.name),});selected === e.name && !isNew;
                 { svelteHTML.createElement("span", { "class":`entry-name mono`,});e.name; }
                if(e.description){
                   { svelteHTML.createElement("span", { "class":`entry-desc dim`,});e.description; }
                }
               }
            }
           }
         }
         { const $$_rediviDenaP5C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP5 = new $$_rediviDenaP5C({ target: __sveltets_2_any(), props: {       width:listW,"storageKey":`settings.contextLibrary.listW`,"label":`Resize the library list`,}});/*Ωignore_startΩ*/() => listW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP5.$$bindings = 'width';}

        
         { svelteHTML.createElement("div", { "class":`editor-pane`,});
          if(selected === null){
             { const $$_etatSytpmE6C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE6C({ target: __sveltets_2_any(), props: {             "icon":`book`,"title":`${entries.length} ${entries.length === 1 ? meta.singular : meta.label.toLowerCase()} in the library`,"body":`Open one from the list to edit it, or add a new ${meta.singular}.`,"actionLabel":`New ${meta.singular}`,"actionIcon":`plus`,"onaction":() => void startNew(),}});}
          }else{
             { svelteHTML.createElement("div", { "class":`field`,});
               { svelteHTML.createElement("label", { "for":`lib-name`,});  }
               { svelteHTML.createElement("input", {                    "dir":`ltr`,"id":`lib-name`,"class":`input mono`,"bind:value":editName,"spellcheck":`false`,"autocomplete":`off`,"placeholder":tab === 'skills' ? 'support-triage-router' : 'otto',"aria-invalid":nameError ? 'true' : undefined,"aria-describedby":`lib-name-hint`,"oninput":() => (nameError = ''),});/*Ωignore_startΩ*/() => editName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
              if(nameError){
                 { svelteHTML.createElement("span", {     "class":`field-err`,"id":`lib-name-hint`,"role":`alert`,});nameError; }
              }else{
                 { svelteHTML.createElement("span", {   "class":`hint`,"id":`lib-name-hint`,});
                                
                 }
              }
             }

             { svelteHTML.createElement("div", { "class":`editor-label`,});
               
              if(dirty){ { svelteHTML.createElement("span", { "class":`unsaved`,});  }}
             }
             { svelteHTML.createElement("div", { "class":`editor-box`,});
              if(bodyLoading || bodyError){
                 { const $$_etatSdaoL7C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL7C({ target: __sveltets_2_any(), props: {          "what":`“${selected}”`,"loading":bodyLoading,"error":bodyError || null,"empty":true,"rows":4,"onretry":() => selected && void openEntry(selected),}});}
              }else{
                editorKey; {
                   { const $$_rotidEedoC7C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC7C({ target: __sveltets_2_any(), props: {              "path":editorPath,"root":"","content":editBody,"language":`md`,"readOnly":false,"placeholder":tab === 'skills'
                      ? 'Markdown: front matter (name, description), then the instructions'
                      : `Write the ${meta.singular} in markdown`,"onchange":(v) => (editBody = v),}});}
                }
              }
             }

             { svelteHTML.createElement("div", { "class":`actions`,});
              if(!isNew && selected){
                 { svelteHTML.createElement("button", {   "class":`btn small danger`,"onclick":() => selected && remove(selected),});
                   { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} 
                 }
              }else{
                 { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => { closeEditor(); if (entries[0]) void openEntry(entries[0].name); },});
                  
                 }
              }
             }
          }
         }
       }
     LoadState}
   }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ContextLibrary__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ContextLibrary__SvelteComponent_ = ReturnType<typeof ContextLibrary__SvelteComponent_>;
/*Ωignore_endΩ*/export default ContextLibrary__SvelteComponent_;