///<reference types="svelte" />
;// Skills Lab → Skills → Files: the multi-file viewer/editor for one copy of a
// skill, on the shared CodeEditor (line numbers, markdown highlighting — the
// same editor as Settings → Context library). Library copies are editable in
// place (type, then Save / ⌘S; Revert drops the draft); bundled and provider
// copies are read-only, with the way to make them editable spelled out.

import { onDestroy, untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
import type { SkillFileEntry } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { skillLabApi } from '../../lib/api/skillLab';
import { toasts } from '../../lib/toast.svelte';
import { formatBytes } from '../../lib/metric-format';
import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import CodeEditor from '../../lib/components/CodeEditor.svelte';
import { sourceLabel } from './skillGroups';
import { guardUnsaved } from '../../lib/leaveGuard';
;

  interface Props {
    name: string;
    /** "library" | "bundled" | a provider name. */
    source: string;
    files: SkillFileEntry[];
    /** SKILL.md text already loaded by the detail pane. */
    skillMd: string;
    /** File to open first (e.g. clicked in Overview). */
    initialFile?: string;
    onsaved: (files: SkillFileEntry[], skillMd: string | null) => void;
    /** Read-only copies: what makes them editable. */
    oninstall?: () => void;
    oncopytolibrary?: () => void;
    /** Unsaved-edit state, so the parents can ask before a tab / skill switch
     *  unmounts the editor and drops the draft. */
    ondirty?: (dirty: boolean) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { name, source, files, skillMd, initialFile = 'SKILL.md', onsaved, oninstall, oncopytolibrary, ondirty }: Props = $props();

  const editable = $derived(source === 'library');

  // svelte-ignore state_referenced_locally
  let currentFile = $state(initialFile);
  let content = $state('');
  let original = $state('');
  let binary = $state(false);
  let saving = $state(false);
  let loading = $state(false);
  let loadGeneration = 0;
  let disposed = false;
  onDestroy(() => { disposed = true; loadGeneration++; });
  let loadError = $state<string | null>(null);
  // Bumped whenever the text is replaced from outside (another file, Revert,
  // a reload) so the uncontrolled CodeEditor remounts on the new document.
  let docKey = $state(0);
  const dirty = $derived(editable && content !== original);
  $effect(() => {
    ondirty?.(dirty);
  });
  $effect(() => () => ondirty?.(false));
  // Leaving the page (sidebar, link, back) with an unsaved edit asks first.
  $effect(() => guardUnsaved(() => dirty, { what: currentFile }));

  async function open(path: string): Promise<void> {
    const owner = `${source}:${name}`;
    if (dirty && !(await confirmer.ask(`You have unsaved changes to ${currentFile}. Opening another file discards them.`, { title: 'Discard unsaved changes?', danger: true, confirmLabel: 'Discard', cancelLabel: 'Keep editing' }))) return;
    if (disposed || owner !== `${source}:${name}`) return;
    const generation = ++loadGeneration;
    currentFile = path;
    loadError = null;
    binary = false;
    content = original = '';
    loading = path !== 'SKILL.md';
    if (path === 'SKILL.md') {
      content = original = skillMd;
      docKey++;
      return;
    }
    try {
      if (source === 'library') {
        const r = await skillLabApi.getFile(name, path);
        if (generation !== loadGeneration) return;
        content = original = r.content;
        binary = r.binary;
        docKey++;
      } else if (source === 'bundled') {
        content = original = '';
        loadError = 'Install this skill to the library to open its other files.';
      } else {
        const r = await skillLabApi.getProviderFile(source, name, path);
        if (generation !== loadGeneration) return;
        content = original = r.content;
        binary = r.binary;
        docKey++;
      }
    } catch (e) {
      if (generation === loadGeneration) loadError = loadErrorText(e);
    } finally {
      if (generation === loadGeneration) loading = false;
    }
  }

  // (Re)open when the skill or copy changes.
  let openedFor = '';
  let openedOwner = '';
  $effect(() => {
    const owner = `${source}:${name}`;
    const key = `${owner}:${initialFile}`;
    if (key === openedFor) return;
    openedFor = key;
    // The parent already confirmed leaving the previous skill. Its draft
    // belongs to that owner and must not trigger another file-open prompt
    // under the replacement skill's heading.
    if (owner !== openedOwner) {
      openedOwner = owner;
      content = original = '';
      loadGeneration++;
    }
    currentFile = files.some((f) => f.path === initialFile) ? initialFile : 'SKILL.md';
    untrack(() => void open(currentFile));
  });
  // Keep SKILL.md in step when the parent reloads it (never over a draft).
  $effect(() => {
    const md = skillMd;
    if (currentFile === 'SKILL.md' && md !== original && !untrack(() => dirty)) {
      content = original = md;
      docKey++;
    }
  });
  function revert(): void {
    content = original;
    docKey++;
  }

  async function save(): Promise<void> {
    if (!editable || saving || loading || loadError || binary || !dirty) return;
    const owner = `${source}:${name}`;
    const target = name;
    const path = currentFile;
    const submitted = content;
    const generation = loadGeneration;
    saving = true;
    try {
      const next = await skillLabApi.putFile(target, { path, content: submitted });
      // A save acknowledges only the submitted snapshot, never later typing.
      if (disposed || owner !== `${source}:${name}` || generation !== loadGeneration) return;
      original = submitted;
      onsaved(next, path === 'SKILL.md' ? submitted : null);
      toasts.success('Saved', path);
    } catch (e) {
      toastError(`Couldn’t save ${path}`, e);
    } finally {
      saving = false;
    }
  }

  async function addFile(): Promise<void> {
    const target = name;
    const owner = `${source}:${name}`;
    const generation = loadGeneration;
    const path = await confirmer.promptText('Path inside the skill, for example references/notes.md', {
      title: 'New file',
      confirmLabel: 'Create',
      placeholder: 'references/notes.md',
    });
    if (!path || disposed || owner !== `${source}:${name}` || generation !== loadGeneration) return;
    // An existing path opens that file — "New file" must never empty it (the
    // server also refuses with 409 via `create_only`, for a file the tree
    // doesn't list yet).
    const wanted = path.trim().replace(/^\.?\/+/, '');
    if (files.some((f) => f.path === wanted)) {
      toasts.info('That file already exists', 'Opened it instead.');
      await open(wanted);
      return;
    }
    try {
      const next = await skillLabApi.putFile(target, { path: wanted, content: '', create_only: true });
      if (disposed || owner !== `${source}:${name}` || generation !== loadGeneration) return;
      onsaved(next, null);
      await open(wanted);
    } catch (e) {
      toastError('Couldn’t add the file', e);
    }
  }

  async function deleteFile(path: string): Promise<void> {
    const target = name;
    const owner = `${source}:${name}`;
    const generation = loadGeneration;
    if (!(await confirmer.ask(`Delete ${path} from ${name}? The file is removed from the library copy.`, { title: 'Delete file' }))) return;
    try {
      if (disposed || owner !== `${source}:${name}` || generation !== loadGeneration) return;
      await skillLabApi.deleteFile(target, path);
      const next = await skillLabApi.listFiles(target);
      if (disposed || owner !== `${source}:${name}` || generation !== loadGeneration) return;
      onsaved(next, null);
      if (currentFile === path) await open('SKILL.md');
    } catch (e) {
      toastError(`Couldn’t delete ${path}`, e);
    }
  }

  // ⌘S inside the editor saves (captured before CodeMirror / the app keymap).
  let codeEl = $state<HTMLElement | null>(null);
  $effect(() => {
    const el = codeEl;
    if (!el) return;
    el.addEventListener('keydown', onKey, true);
    return () => el.removeEventListener('keydown', onKey, true);
  });
  function onKey(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 's' && editable) {
      e.preventDefault();
      void save();
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`editor-wrap`,});
  if(!editable){
     { svelteHTML.createElement("div", { "class":`ro-note`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":14,}});}
       { svelteHTML.createElement("span", {});
        if(source === 'bundled'){             }else{   sourceLabel(source);               }
       }
      if(source === 'bundled' && oninstall){
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":oninstall,});   }
      } else if (oncopytolibrary){
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":oncopytolibrary,});   }
      }
     }
  }
   { svelteHTML.createElement("div", { "class":`editor`,});
     { svelteHTML.createElement("nav", {   "class":`files`,"aria-label":`Files in ${name}`,});
       { svelteHTML.createElement("div", { "class":`files-head`,});
         { svelteHTML.createElement("span", { "class":`section-title`,});  }
        if(editable){
           { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":addFile,"aria-label":`New file`,"title":`New file`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});} }
        }
       }
       { svelteHTML.createElement("ul", {});
           for(let f of __sveltets_2_ensureArray(files)){f.path;
           { svelteHTML.createElement("li", { "class":`file-row`,});
             { svelteHTML.createElement("button", {        "class":`file`,"aria-current":currentFile === f.path ? 'true' : undefined,"onclick":() => open(f.path),"title":`${f.path} · ${formatBytes(f.size)}`,});currentFile === f.path;
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}
               { svelteHTML.createElement("span", {   "class":`file-name mono`,"dir":`ltr`,});f.path; }
             }
            if(editable && f.path !== 'SKILL.md'){
               { svelteHTML.createElement("button", {       "class":`icon-btn file-del`,"onclick":() => deleteFile(f.path),"aria-label":`Delete ${f.path}`,"title":`Delete ${f.path}`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} }
            }
           }
        }
       }
     }

     { svelteHTML.createElement("section", { "class":`pane`,});
       { svelteHTML.createElement("div", { "class":`pane-head`,});
         { svelteHTML.createElement("span", {     "class":`mono path`,"dir":`ltr`,"title":currentFile,});currentFile; }
        if(dirty){ { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {    "tone":`warn`,"label":`Unsaved`,}});}}
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(editable && !loading && !binary && !loadError){
          if(dirty){
             { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":revert,"title":`Drop your changes to ${currentFile}`,});  }
          }else{
             { svelteHTML.createElement("span", { "class":`dim save-hint`,});      }
          }
           { svelteHTML.createElement("button", {         "class":`btn small primary`,"disabled":saving || !dirty,"title":dirty ? 'Save (⌘S)' : 'No changes to save',"onclick":save,...__sveltets_2_empty({"data-testid":`save-skill`}),});saving ? 'Saving…' : 'Save'; }
        }
       }
      if(loadError && source === 'bundled'){
        
         { svelteHTML.createElement("p", { "class":`dim msg`,});loadError; }
      } else if (loading || loadError){
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {         "what":currentFile,loading,"error":loadError,"empty":true,"rows":6,"onretry":() => open(currentFile),}});}
      } else if (binary){
         { svelteHTML.createElement("p", { "class":`dim msg`,});      }
      }else{
         { const $$_div3 = svelteHTML.createElement("div", {      "class":`code`,"dir":`ltr`,...__sveltets_2_empty({"data-testid":editable ? 'skill-editor' : 'skill-view'}),});codeEl = $$_div3;
          docKey; {
             { const $$_rotidEedoC4C = __sveltets_2_ensureComponent(CodeEditor); new $$_rotidEedoC4C({ target: __sveltets_2_any(), props: {           "path":`${name}/${currentFile}`,"root":"",content,"language":currentFile.toLowerCase().endsWith('.md') ? 'md' : undefined,"readOnly":!editable,"onchange":(v) => (content = v),}});}
          }
         }
      }
     }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SkillEditor__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SkillEditor__SvelteComponent_ = ReturnType<typeof SkillEditor__SvelteComponent_>;
/*Ωignore_endΩ*/export default SkillEditor__SvelteComponent_;