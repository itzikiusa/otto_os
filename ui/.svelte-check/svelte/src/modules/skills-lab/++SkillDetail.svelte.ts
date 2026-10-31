///<reference types="svelte" />
;
  export type DetailTab = 'overview' | 'edit' | 'evals' | 'usage';
;;

import { announce } from '../../lib/announce.svelte';
// Skills Lab → Skills: the detail pane for one skill (grouped by name).
//
//   header   — name, description, category, the copies it has (Library ·
//              Claude · Codex · Bundled) as a value picker, drift notes with
//              "Compare with …", and the skill’s actions (Review, Evaluate,
//              Install/Update, Copy to library, Delete in ⋯)
//   Overview — SKILL.md frontmatter as a metadata card (description,
//              triggers, allowed tools, version…) + its files, then the body
//              rendered through the sanitized GFM renderer
//   Edit     — the multi-file editor (SkillEditor)
//   Evals / Usage — SkillActivity
import { onDestroy, untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
import Tabs from '../../lib/components/Tabs.svelte';
import type { SkillFileEntry } from '../../lib/api/types';
import { skillLabApi } from '../../lib/api/skillLab';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { toasts } from '../../lib/toast.svelte';
import { renderMarkdownGfm } from '../../lib/md';
import { formatBytes } from '../../lib/metric-format';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import DiffView from '../../lib/components/DiffView.svelte';
import SkillEditor from './SkillEditor.svelte';
import SkillActivity from './SkillActivity.svelte';
import { metaList, parseFrontmatter, sourceLabel, type SkillGroup, type VariantSource } from './skillGroups';
;

  interface Props {
    group: SkillGroup;
    source: VariantSource;
    tab: DetailTab;
    wsId: string;
    /** Library SKILL.md bodies are in the list payload; others are fetched. */
    libraryBody: string | undefined;
    /** Fetched provider bodies (shared cache with the list’s drift check). */
    bodyOf: (source: VariantSource) => string | undefined;
    onsource: (s: VariantSource) => void;
    ontab: (t: DetailTab) => void;
    onchanged: (select?: { name: string; source: VariantSource }) => void;
    ondeleted: () => void;
    onbody: (source: VariantSource, body: string) => void;
    onreview: () => void;
    onevaluate: () => void;
    /** The Edit tab has unsaved changes (the browser guards skill switches). */
    ondirty?: (dirty: boolean) => void;
    onopenrun?: (id: string) => void;
    /** Open one existing review in the Review tab. */
    onopenreview?: (id: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { group, source, tab, wsId, libraryBody, bodyOf, onsource, ontab, onchanged, ondeleted, onbody, onreview, onevaluate, ondirty, onopenrun, onopenreview }: Props = $props();

  // Unsaved edits in the Edit tab: leaving the tab (or the copy) unmounts the
  // editor, so ask first instead of silently dropping the draft.
  let editorDirty = $state(false);
  function setDirty(d: boolean): void {
    editorDirty = d;
    ondirty?.(d);
  }
  async function confirmDiscard(): Promise<boolean> {
    if (!editorDirty) return true;
    const ok = await confirmer.ask(`You have unsaved changes to ${group.name}. Leaving the editor discards them.`, { title: 'Discard unsaved changes?', danger: true, confirmLabel: 'Discard', cancelLabel: 'Keep editing' });
    if (ok) setDirty(false);
    return ok;
  }
  async function goTab(t: DetailTab): Promise<void> {
    if (t === tab) return;
    if (tab === 'edit' && !(await confirmDiscard())) return;
    ontab(t);
  }
  async function goSource(s: VariantSource): Promise<void> {
    if (s === source) return;
    if (!(await confirmDiscard())) return;
    onsource(s);
  }

  const variant = $derived(group.variants.find((v) => v.source === source) ?? group.variants[0]);
  const isLibrary = $derived(variant.source === 'library');
  const isBundled = $derived(variant.source === 'bundled');
  const hasLibrary = $derived(group.variants.some((v) => v.source === 'library'));

  // ---- Load the selected copy’s SKILL.md + file list ----------------------
  let body = $state<string>('');
  let files = $state<SkillFileEntry[]>([]);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let loadGeneration = 0;
  let disposed = false;
  onDestroy(() => { disposed = true; loadGeneration++; });

  async function load(name: string, src: VariantSource): Promise<void> {
    const generation = ++loadGeneration;
    loading = true;
    loadError = null;
    try {
      if (src === 'library') {
        const [fs, f] = await Promise.all([skillLabApi.listFiles(name), skillLabApi.getFile(name, 'SKILL.md')]);
        if (generation !== loadGeneration) return;
        files = fs;
        body = f.content;
      } else if (src === 'bundled') {
        const b = await skillLabApi.getBundled(name);
        if (generation !== loadGeneration) return;
        files = b.files;
        body = b.body;
      } else {
        const p = await skillLabApi.getProvider(src, name);
        if (generation !== loadGeneration) return;
        files = p.files;
        body = p.body;
        onbody(src, p.body);
      }
    } catch (e) {
      if (generation === loadGeneration) loadError = loadErrorText(e);
    } finally {
      if (generation === loadGeneration) loading = false;
    }
  }
  const selectionKey = $derived(`${variant.source}:${group.name}`);
  $effect(() => {
    void selectionKey;
    untrack(() => void load(group.name, variant.source));
  });

  const fm = $derived(parseFrontmatter(body));
  const metaMap = $derived(new Map(fm.meta));
  const rendered = $derived(renderMarkdownGfm(fm.body));
  const SKIP = new Set(['name', 'description', 'category', 'version']);
  const LIST_KEYS = new Set(['allowed-tools', 'allowed_tools', 'tools', 'triggers', 'tags', 'keywords']);
  const extraMeta = $derived(fm.meta.filter(([k]) => !SKIP.has(k)));

  // ---- Compare against the reference copy ----------------------------------
  let comparing = $state<VariantSource | null>(null);
  // A copy that can’t be read ends the "Loading both copies…" wait with a
  // reason instead of leaving it up forever.
  let compareError = $state<string | null>(null);
  $effect(() => {
    void group.name;
    comparing = null;
    compareError = null;
  });
  const refBody = $derived(group.reference === 'library' ? libraryBody : bodyOf(group.reference));
  async function compare(s: VariantSource): Promise<void> {
    const owner = selectionKey;
    const name = group.name;
    const reference = group.reference;
    const generation = loadGeneration;
    if (comparing !== s && tab === 'edit' && !(await confirmDiscard())) return;
    if (disposed || owner !== selectionKey || generation !== loadGeneration) return;
    comparing = comparing === s ? null : s;
    compareError = null;
    // The diff lives on Overview, above the rendered SKILL.md.
    if (comparing && tab !== 'overview') ontab('overview');
    if (comparing && s !== 'library' && s !== 'bundled' && bodyOf(s) == null) {
      try {
        const p = await skillLabApi.getProvider(s, name);
        if (disposed || owner !== selectionKey || generation !== loadGeneration || comparing !== s) return;
        onbody(s, p.body);
      } catch (e) {
        if (disposed || owner !== selectionKey || generation !== loadGeneration || comparing !== s) return;
        compareError = `Couldn’t read the ${sourceLabel(s)} copy: ${loadErrorText(e)}`;
      }
    }
    if (comparing && group.reference !== 'library' && bodyOf(group.reference) == null) {
      try {
        const p = await skillLabApi.getProvider(reference, name);
        if (disposed || owner !== selectionKey || generation !== loadGeneration || comparing !== s) return;
        onbody(reference, p.body);
      } catch (e) {
        if (disposed || owner !== selectionKey || generation !== loadGeneration || comparing !== s) return;
        compareError = `Couldn’t read the ${sourceLabel(reference)} copy: ${loadErrorText(e)}`;
      }
    }
  }
  let bundledBody = $state<string | null>(null);
  $effect(() => {
    if (comparing === 'bundled' && bundledBody == null) {
      // On failure don’t fake an empty body (that renders as "everything was
      // deleted"); say the bundled copy couldn’t be read.
      const owner = selectionKey;
      let active = true;
      void skillLabApi.getBundled(group.name)
        .then((b) => { if (active && !disposed && owner === selectionKey) bundledBody = b.body; })
        .catch((e) => { if (active && !disposed && owner === selectionKey) compareError = `Couldn’t read the bundled copy: ${loadErrorText(e)}`; });
      return () => { active = false; };
    }
  });
  $effect(() => {
    void group.name;
    bundledBody = null;
  });
  const compareAfter = $derived(comparing === 'bundled' ? bundledBody : comparing ? bodyOf(comparing) : null);
  const compareBefore = $derived(comparing === 'bundled' ? libraryBody : refBody);

  // ---- Actions ----------------------------------------------------------
  let busy = $state(false);
  async function install(): Promise<void> {
    const name = group.name;
    const owner = selectionKey;
    const generation = loadGeneration;
    const b = group.variants.find((v) => v.source === 'bundled');
    const updating = hasLibrary;
    if (updating && !(await confirmer.ask(`Replace the library copy of ${group.name} with bundled v${b?.bundledVersion}? Your current copy is backed up first.`, { title: 'Replace from bundled', confirmLabel: 'Replace', danger: false }))) return;
    if (disposed || owner !== selectionKey || generation !== loadGeneration) return;
    busy = true;
    try {
      await skillLabApi.install(name);
      if (disposed || owner !== selectionKey || generation !== loadGeneration) return;
      toasts.success(updating ? 'Library copy updated' : 'Installed to library', updating ? 'The previous copy was backed up.' : 'You can edit it now.');
      onchanged({ name, source: 'library' });
    } catch (e) {
      toastError(`Couldn’t install ${group.name}`, e);
    } finally {
      busy = false;
    }
  }
  async function copyToLibrary(): Promise<void> {
    const name = group.name;
    const owner = selectionKey;
    const generation = loadGeneration;
    if (hasLibrary) return;
    busy = true;
    try {
      const src = bodyOf(variant.source) ?? body;
      await skillLabApi.create({ name: group.name, category: group.category === 'uncategorized' ? '' : group.category, description: group.description, body: src });
      if (disposed || owner !== selectionKey || generation !== loadGeneration) return;
      toasts.success('Copied to library', `Only SKILL.md was copied from the ${sourceLabel(variant.source)} copy.`);
      onchanged({ name, source: 'library' });
    } catch (e) {
      toastError(`Couldn’t copy ${group.name} to the library`, e);
    } finally {
      busy = false;
    }
  }
  async function remove(): Promise<void> {
    const name = group.name;
    const owner = selectionKey;
    const generation = loadGeneration;
    if (!(await confirmer.ask(`Delete “${group.name}” from the Otto library? Its files are removed; copies in ${group.variants.filter((v) => v.source !== 'library').map((v) => sourceLabel(v.source)).join(', ') || 'other places'} are not touched.`, { title: 'Delete skill' }))) return;
    try {
      if (disposed || owner !== selectionKey || generation !== loadGeneration) return;
      await skillLabApi.remove(name);
      if (disposed || owner !== selectionKey || generation !== loadGeneration) return;
      toasts.success('Skill deleted', group.name);
      ondeleted();
    } catch (e) {
      toastError(`Couldn’t delete ${group.name}`, e);
    }
  }
  function more(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'Evaluate', icon: 'target', action: onevaluate },
      ...(!hasLibrary && !isBundled ? [{ label: 'Copy to library', icon: 'copy' as const, action: () => void copyToLibrary() }] : []),
      ...(isLibrary ? [{ separator: true }, { label: 'Delete from library…', icon: 'trash' as const, danger: true, action: () => void remove() }] : []),
    ];
    ctxMenu.show(e, items);
  }

  // ---- Tabs (roving, arrow keys) ------------------------------------------
  const TABS: { id: DetailTab; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    // "Files", not "Edit": bundled / provider copies are read-only here.
    { id: 'edit', label: 'Files' },
    { id: 'evals', label: 'Evals' },
    { id: 'usage', label: 'Usage' },
  ];

  let editorFile = $state('SKILL.md');
  function openFile(path: string): void {
    editorFile = path;
    ontab('edit');
  }

  function syncLabel(): string {
    switch (group.sync) {
      case 'in_sync':
        return 'In sync';
      case 'drifted':
        return 'Drifted';
      case 'single':
        return 'One copy';
      default:
        return 'Checking…';
    }
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (compareError) announce(compareError, { assertive: true }); });
;
async () => {



 { svelteHTML.createElement("div", {   "class":`detail`,...__sveltets_2_empty({"data-testid":`skill-detail`}),});
   { svelteHTML.createElement("header", { "class":`d-head`,});
     { svelteHTML.createElement("div", { "class":`d-title-row`,});
       { svelteHTML.createElement("h2", {     "class":`d-name mono`,"dir":`ltr`,...__sveltets_2_empty({"data-testid":`skill-name`}),});group.name; }
       { svelteHTML.createElement("span", {   "class":`sync ${group.sync}`,"title":group.drift.join('\n') || syncLabel(),}); { svelteHTML.createElement("span", { "class":`dot`,}); }syncLabel(); }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":onreview,...__sveltets_2_empty({"data-testid":`review-skill`}),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":12,}});}  }
      if(isBundled){
        if(!hasLibrary || variant.bundledState === 'update_available'){
           { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy,"onclick":install,});hasLibrary ? 'Update library copy' : 'Install to library'; }
        }
      } else if (!hasLibrary){
         { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":busy,"onclick":copyToLibrary,});   }
      }
       { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":more,"aria-label":`More actions for ${group.name}`,"title":`More actions`,"aria-haspopup":`menu`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
     }
    if(group.description){ { svelteHTML.createElement("p", {   "class":`d-desc`,"title":group.description,});group.description; }}
     { svelteHTML.createElement("div", { "class":`d-meta`,});
       { svelteHTML.createElement("span", { "class":`chip`,});group.category; }
      if(group.variants.length === 1){
        
        const v = group.variants[0];
         { svelteHTML.createElement("span", {   "class":`chip`,"title":`The only copy of this skill`,});
          if(v.source === 'library'){ { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`book`,"size":12,}});}} else if (v.source === 'bundled'){ { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`box`,"size":12,}});}}else{ { const $$_nocIredivorP4C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP4C({ target: __sveltets_2_any(), props: {    "provider":v.source,"size":12,}});}}
          sourceLabel(v.source);
         }
      }else{
       { svelteHTML.createElement("div", {     "class":`variants`,"role":`group`,"aria-label":`Copy to show`,});
           for(let v of __sveltets_2_ensureArray(group.variants)){v.source;
           { svelteHTML.createElement("button", {        "class":`variant`,"aria-pressed":v.source === variant.source,"onclick":() => goSource(v.source),...__sveltets_2_empty({"data-testid":`variant-${v.source}`}),});v.source === variant.source;
            if(v.source === 'library'){ { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`book`,"size":12,}});}} else if (v.source === 'bundled'){ { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`box`,"size":12,}});}}else{ { const $$_nocIredivorP5C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP5C({ target: __sveltets_2_any(), props: {    "provider":v.source,"size":12,}});}}
            sourceLabel(v.source);
            if(group.driftedSources.includes(v.source)){ { svelteHTML.createElement("span", {   "class":`vdot`,"title":`Differs from ${sourceLabel(group.reference)}`,}); }}
           }
        }
       }
      }
     }
    if(group.drift.length > 0){
       { svelteHTML.createElement("div", {   "class":`drift`,"role":`note`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
         { svelteHTML.createElement("ul", {});
              for(let note of __sveltets_2_ensureArray(group.drift)){let i = 1;i;
            const s = group.driftedSources[i];
             { svelteHTML.createElement("li", {});
              note;
              if(s){
                 { svelteHTML.createElement("button", {     "class":`btn small ghost`,"aria-expanded":comparing === s,"onclick":() => compare(s),});comparing === s ? 'Hide diff' : `Compare with ${sourceLabel(s === 'bundled' ? 'library' : group.reference)}`; }
              }
             }
          }
         }
       }
    }
    
     { svelteHTML.createElement("div", { "class":`skill-tabs`,});
       { const $$_sbaT3C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT3C({ target: __sveltets_2_any(), props: {            "label":`Skill detail`,"tabs":TABS,"value":tab,"onchange":(id) => void goTab(id),"idBase":`skill`,"activate":`click`,}});}
     }
   }

   { svelteHTML.createElement("div", {       "class":`d-body`,"role":`tabpanel`,"id":`skill-panel-${tab}`,"aria-labelledby":`skill-tab-${tab}`,});
    if(comparing && tab === 'overview'){
       { svelteHTML.createElement("section", { "class":`compare`,});
         { svelteHTML.createElement("div", { "class":`compare-head`,});
           { svelteHTML.createElement("span", { "class":`section-title`,});comparing === 'bundled' ? 'Library → Bundled' : `${sourceLabel(group.reference)} → ${sourceLabel(comparing)}`; }
           { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (comparing = null),"aria-label":`Close diff`,"title":`Close diff`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});} }
         }
        if(compareError){
           { svelteHTML.createElement("p", { "class":`compare-err`,});compareError; }
        } else if (compareBefore == null || compareAfter == null){
            { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {  "what":`both copies`,"loading":true,"empty":true,}});}
        }else{
           { const $$_weiVffiD3C = __sveltets_2_ensureComponent(DiffView); new $$_weiVffiD3C({ target: __sveltets_2_any(), props: {        "before":compareBefore,"after":compareAfter,"mode":`word`,"contextLines":3,}});}
        }
       }
    }

    if(loadError && (tab === 'overview' || tab === 'edit')){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {      "what":`the ${sourceLabel(variant.source)} copy of ${group.name}`,"error":loadError,"empty":true,"onretry":() => load(group.name, variant.source),}});}
    } else if (tab === 'overview'){
      if(loading && !body){
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {    "what":group.name,"loading":true,"empty":true,"rows":5,}});}
      }else{
         { svelteHTML.createElement("div", { "class":`overview`,});
           { svelteHTML.createElement("aside", {   "class":`meta card`,"aria-label":`Skill metadata`,});
             { svelteHTML.createElement("dl", {});
              
              if(metaMap.get('description') && (metaMap.get('description') !== group.description || String(metaMap.get('description')).length > 160)){
                 { svelteHTML.createElement("div", { "class":`m-row wide`,}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});metaMap.get('description'); } }
              }
              
              if(metaMap.get('category') && metaMap.get('category') !== group.category){
                 { svelteHTML.createElement("div", { "class":`m-row`,}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});metaMap.get('category'); } }
              }
              if(metaMap.get('version')){ { svelteHTML.createElement("div", { "class":`m-row`,}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {}); metaMap.get('version'); } }}
                 for(let [k, v] of __sveltets_2_ensureArray(extraMeta)){k;
                 { svelteHTML.createElement("div", { "class":`m-row wide`,});
                   { svelteHTML.createElement("dt", {});k.replace(/[-_]/g, ' '); }
                   { svelteHTML.createElement("dd", {});
                    if(Array.isArray(v) || LIST_KEYS.has(k)){
                       { svelteHTML.createElement("span", { "class":`chips`,});    for(let item of __sveltets_2_ensureArray(metaList(v))){let i = 1;i; { svelteHTML.createElement("span", { "class":`chip mono`,});item; }} }
                    }else{
                      v;
                    }
                   }
                 }
              }
              if(fm.meta.length === 0){
                 { svelteHTML.createElement("div", { "class":`m-row wide`,}); { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`dim`,});                 } }
              }
             }
             { svelteHTML.createElement("div", { "class":`m-files`,});
               { svelteHTML.createElement("div", { "class":`section-title`,});  files.length; }
               { svelteHTML.createElement("ul", {});
                   for(let f of __sveltets_2_ensureArray(files)){f.path;
                   { svelteHTML.createElement("li", {});
                     { svelteHTML.createElement("button", {     "class":`file-link`,"onclick":() => openFile(f.path),"title":`Open ${f.path}`,});
                       { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});} { svelteHTML.createElement("span", {   "class":`mono ellipsis`,"dir":`ltr`,});f.path; } { svelteHTML.createElement("span", { "class":`dim size`,});formatBytes(f.size); }
                     }
                   }
                }
               }
             }
           }
           { svelteHTML.createElement("article", {   "class":`md-body skill-md`,...__sveltets_2_empty({"data-testid":`skill-preview`}),});
            if(fm.body.trim()){
              
               rendered;
            }else{
               { svelteHTML.createElement("p", { "class":`dim`,});     }
            }
           }
         }
      }
    } else if (tab === 'edit'){
       { const $$_rotidEllikS2C = __sveltets_2_ensureComponent(SkillEditor); new $$_rotidEllikS2C({ target: __sveltets_2_any(), props: {                  "name":group.name,"source":variant.source,files,"skillMd":body,"initialFile":editorFile,"onsaved":(next, md) => {
          files = next;
          if (md != null) {
            body = md;
            onchanged();
          }
        },"oninstall":install,"oncopytolibrary":hasLibrary ? undefined : copyToLibrary,"ondirty":setDirty,}});}
    }else{
       { const $$_ytivitcAllikS2C = __sveltets_2_ensureComponent(SkillActivity); new $$_ytivitcAllikS2C({ target: __sveltets_2_any(), props: {          group,"view":tab,wsId,onevaluate,onreview,onopenrun,onopenreview,"onview":(v) => ontab(v),}});}
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SkillDetail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SkillDetail__SvelteComponent_ = ReturnType<typeof SkillDetail__SvelteComponent_>;
/*Ωignore_endΩ*/export default SkillDetail__SvelteComponent_;