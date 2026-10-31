///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
// Skills Lab → Skills. Every skill your agents can load, grouped by NAME —
// one row per skill with a badge per copy (Library · Claude · Codex ·
// Antigravity · Bundled) and a sync dot (in sync / drifted / one copy). The
// list filters by category and source and searches name + description; the
// detail pane (SkillDetail) opens on the last-viewed skill, else the first.
//
// Drift: library bodies arrive with the list; provider SKILL.md bodies are
// fetched in the background (4 at a time) only for skills that exist in more
// than one editable place, so the dots settle a moment after the list shows.
import { untrack } from 'svelte';
import type { BundledSkillView, LibrarySkill, ProviderSkillInfo } from '../../lib/api/types';
import { skillLabApi } from '../../lib/api/skillLab';
import { toasts } from '../../lib/toast.svelte';
import PaneDivider from '../../lib/components/PaneDivider.svelte';
import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
import { confirmer } from '../../lib/confirm.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { registry } from '../../lib/commands.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { recallSelection, rememberSelection } from '../../lib/lastSelection';
import Icon from '../../lib/components/Icon.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import Skeleton from '../../lib/components/Skeleton.svelte';
import SkillDetail, { type DetailTab } from './SkillDetail.svelte';
import NewSkillModal from './NewSkillModal.svelte';
import {
    bodyKey,
    categoryCounts,
    filterGroups,
    groupSkills,
    namesNeedingBodies,
    sourceCounts,
    sourceLabel,
    type SkillGroup,
    type SourceFilter,
    type VariantSource,
  } from './skillGroups';
;

  interface Props {
    listCollapsed?: boolean;
    onreview?: (name: string, source: string) => void;
    onevaluate?: (name: string, source: string) => void;
    /** Tells the page whether the library is empty (it then drops its header
     *  primary: the empty state owns "New skill"). */
    onempty?: (empty: boolean) => void;
    /** Phone push-navigation: the page shows a back button in its header. */
    onphonedetail?: (open: boolean) => void;
    /** Open one evaluation run in the Evaluator. */
    onopenrun?: (id: string) => void;
    /** Open one existing review in the Review tab. */
    onopenreview?: (id: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { listCollapsed = false, onreview, onevaluate, onempty, onphonedetail, onopenrun, onopenreview }: Props = $props();

  // ---- Data -----------------------------------------------------------------
  let library = $state<LibrarySkill[]>([]);
  let bundled = $state<BundledSkillView[]>([]);
  let providerSkills = $state<ProviderSkillInfo[]>([]);
  let loading = $state(true);
  let loadError = $state<string | null>(null);
  // Some (not all) sources failed: the list still shows, with a note saying
  // which copies are missing so they don't read as "not installed".
  let partialError = $state<string | null>(null);
  // Only ever reassigned (never mutated): raw, no per-key proxy.
  let bodies = $state.raw<Record<string, string>>({});

  async function loadAll(): Promise<void> {
    loadError = null;
    const [lib, bun, prov] = await Promise.allSettled([
      skillLabApi.listLibrary(),
      skillLabApi.listBundled(),
      skillLabApi.listProvider(),
    ]);
    if (lib.status === 'rejected' && bun.status === 'rejected' && prov.status === 'rejected') {
      loadError = loadErrorText(lib.reason);
    }
    const failed = [
      lib.status === 'rejected' ? 'your library' : '',
      bun.status === 'rejected' ? 'the bundled catalog' : '',
      prov.status === 'rejected' ? 'the Claude / Codex / Antigravity skill folders' : '',
    ].filter(Boolean);
    partialError = !loadError && failed.length > 0 ? `Couldn’t read ${failed.join(' or ')} — those copies are missing from the list.` : null;
    library = lib.status === 'fulfilled' ? lib.value : [];
    bundled = bun.status === 'fulfilled' ? bun.value : [];
    providerSkills = prov.status === 'fulfilled' ? prov.value : [];
    loading = false;
  }
  let retrying = $state(false);
  async function retryLoad(): Promise<void> {
    retrying = true;
    try {
      await loadAll();
    } finally {
      retrying = false;
    }
  }
  let started = false;
  $effect(() => {
    if (started) return;
    started = true;
    void loadAll();
  });

  const groups = $derived(groupSkills(library, bundled, providerSkills, bodies));
  $effect(() => {
    if (!loading) onempty?.(groups.length === 0);
  });

  // Background drift check: fetch the provider bodies that have a copy to
  // compare against. Four in flight; each result lands in `bodies`.
  let inflight = new Set<string>();
  $effect(() => {
    if (loading) return;
    const need = namesNeedingBodies(groups, bodies).filter((n) => !inflight.has(bodyKey(n.source, n.name)));
    if (need.length === 0) return;
    const queue = [...need];
    for (const n of queue) inflight.add(bodyKey(n.source, n.name));
    const worker = async () => {
      for (let n = queue.shift(); n; n = queue.shift()) {
        try {
          const p = await skillLabApi.getProvider(n.source, n.name);
          landBody(bodyKey(n.source, n.name), p.body);
        } catch {
          // Unreadable copy: record an empty marker so the check settles.
          landBody(bodyKey(n.source, n.name), '');
        }
      }
    };
    void Promise.all([worker(), worker(), worker(), worker()]);
  });

  // Landed bodies are batched into ONE `bodies` write per ~150 ms: each write
  // regroups every skill and re-renders the list, so a per-body write (with an
  // O(N) object spread) made the drift check quadratic.
  let pendingBodies: Record<string, string> = {};
  let bodyFlush: ReturnType<typeof setTimeout> | null = null;
  function landBody(key: string, body: string): void {
    pendingBodies[key] = body;
    if (bodyFlush) return;
    bodyFlush = setTimeout(() => {
      bodyFlush = null;
      const batch = pendingBodies;
      pendingBodies = {};
      bodies = { ...bodies, ...batch };
    }, 150);
  }
  $effect(() => () => {
    if (bodyFlush) clearTimeout(bodyFlush);
  });

  // ---- Filters ----------------------------------------------------------------
  let query = $state('');
  let category = $state<string | null>(null);
  let source = $state<SourceFilter>('all');
  let driftedOnly = $state(false);
  const cats = $derived(categoryCounts(groups));
  // The four biggest categories as chips; the rest in a (clamped) ctxMenu so
  // the filter block stays two or three rows however many categories exist.
  const TOP_CATS = 4;
  const topCats = $derived(cats.slice(0, TOP_CATS));
  const moreCats = $derived(cats.slice(TOP_CATS));
  function moreCategories(e: MouseEvent): void {
    ctxMenu.show(
      e,
      moreCats.map(([c, n]) => ({
        label: `${c} · ${n}`,
        icon: category === c ? ('check' as const) : undefined,
        action: () => (category = category === c ? null : c),
      })),
      { filter: moreCats.length > 8 },
    );
  }
  const sources = $derived(sourceCounts(groups));
  const driftedCount = $derived(groups.filter((g) => g.sync === 'drifted').length);
  const shown = $derived(filterGroups(groups, { query, category, source, driftedOnly }));
  const shownByCat = $derived.by(() => {
    const m = new Map<string, SkillGroup[]>();
    for (const g of shown) {
      const arr = m.get(g.category) ?? [];
      arr.push(g);
      m.set(g.category, arr);
    }
    return [...m.entries()];
  });
  const filtering = $derived(!!query.trim() || !!category || source !== 'all' || driftedOnly);
  function clearFilters(): void {
    query = '';
    category = null;
    source = 'all';
    driftedOnly = false;
  }

  // ---- Selection -----------------------------------------------------------
  // The skills list's width — drag / ←→ on the divider, remembered across visits.
  let listW = $state(loadPaneWidth('skills.listW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
  let selName = $state<string | null>(null);
  let selSource = $state<VariantSource>('library');
  let tab = $state<DetailTab>('overview');
  let phoneDetail = $state(false);
  const selected = $derived(groups.find((g) => g.name === selName) ?? null);
  $effect(() => onphonedetail?.(viewport.isPhone && phoneDetail));
  /** Phone: back from the detail to the list (the page header's back button). */
   async function back(): Promise<void> {
    if (!(await confirmLeave())) return;
    phoneDetail = false;
  }

  // Unsaved edits in the open skill's editor (reported up by SkillDetail).
  let editorDirty = $state(false);
  /** Ask before an action that would unmount the editor with unsaved edits
   *  (another skill, the page's Review / Evaluator tabs). */
   async function confirmLeave(): Promise<boolean> {
    if (!editorDirty) return true;
    const ok = await confirmer.ask(`You have unsaved changes to ${selName ?? 'this skill'}. Leaving the editor discards them.`, { title: 'Discard unsaved changes?', danger: true, confirmLabel: 'Discard', cancelLabel: 'Keep editing' });
    if (ok) editorDirty = false;
    return ok;
  }

  function defaultSource(g: SkillGroup): VariantSource {
    return g.variants.find((v) => v.source === 'library')?.source ?? g.variants.find((v) => v.source !== 'bundled')?.source ?? g.variants[0].source;
  }
  async function pick(g: SkillGroup): Promise<void> {
    if (g.name === selName) {
      if (viewport.isPhone) phoneDetail = true;
      return;
    }
    if (!(await confirmLeave())) return;
    select(g);
  }
  function select(g: SkillGroup, src?: VariantSource): void {
    selName = g.name;
    selSource = src && g.variants.some((v) => v.source === src) ? src : defaultSource(g);
    if (viewport.isPhone) phoneDetail = true;
  }
  // Open on the last-viewed skill, else the first one listed.
  $effect(() => {
    if (loading || groups.length === 0) return;
    if (selName && groups.some((g) => g.name === selName)) return;
    const last = recallSelection('skills-lab');
    const g = (last && groups.find((x) => x.name === last)) || shown[0] || groups[0];
    select(g);
    phoneDetail = false;
  });
  // A search / filter that hides the open skill moves the detail to the first
  // skill still listed — never a "pick one" pane beside a non-empty list. Not
  // while the editor holds unsaved edits (that would unmount it), and not on a
  // phone (the list is the screen there; the detail opens on a tap).
  $effect(() => {
    const list = shown;
    if (loading || list.length === 0 || editorDirty || viewport.isPhone) return;
    if (selName && list.some((g) => g.name === selName)) return;
    untrack(() => {
      selName = list[0].name;
      selSource = defaultSource(list[0]);
    });
  });
  $effect(() => {
    if (selName) rememberSelection('skills-lab', selName);
  });
  // A copy that disappeared (deleted) falls back to the default copy.
  $effect(() => {
    if (selected && !selected.variants.some((v) => v.source === selSource)) selSource = defaultSource(selected);
  });

  // ---- New / import (called from the page header) --------------------------
  let newOpen = $state(false);
  let newTemplate = $state<'blank' | 'bundled' | 'import'>('blank');
   function openNew(): void {
    newTemplate = 'blank';
    newOpen = true;
  }
   function openImport(): void {
    newTemplate = 'import';
    newOpen = true;
  }
  async function created(s: LibrarySkill): Promise<void> {
    newOpen = false;
    await loadAll();
    const g = groups.find((x) => x.name === s.name);
    if (g) select(g, 'library');
    tab = 'edit';
    toasts.success('Skill created', `${s.name} is in your library`);
  }

  async function changed(next?: { name: string; source: VariantSource }): Promise<void> {
    await loadAll();
    if (next) {
      selName = next.name;
      selSource = next.source;
    }
  }
  async function deleted(): Promise<void> {
    // Reload FIRST, then clear: the open-on-a-skill effect above re-runs on the
    // cleared selection against the fresh list and lands on a neighbour instead
    // of re-picking the skill that was just deleted (or leaving a blank pane).
    await loadAll();
    selName = null;
    phoneDetail = false;
  }

  // ---- ⌘K ---------------------------------------------------------------------
  $effect(() => {
    const g = selected;
    return registry.register('skills-lab', [
      { id: 'skills.new', title: 'New skill…', group: 'Skills Lab', keywords: 'create skill template', run: openNew },
      { id: 'skills.import', title: 'Import a skill package…', group: 'Skills Lab', keywords: 'zip upload skill', run: openImport },
      ...(g
        ? [
            { id: 'skills.review', title: `Review skill ${g.name}`, group: 'Skills Lab', keywords: 'multi-agent review', run: () => onreview?.(g.name, selSource) },
            { id: 'skills.evaluate', title: `Evaluate skill ${g.name}`, group: 'Skills Lab', keywords: 'eval score', run: () => onevaluate?.(g.name, selSource) },
          ]
        : []),
    ]);
  });

  // ---- List keyboard: ↑/↓ move, Enter opens --------------------------------
  let listEl = $state<HTMLElement | null>(null);
  function onListKey(e: KeyboardEvent): void {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const i = shown.findIndex((g) => g.name === selName);
    const n = e.key === 'ArrowDown' ? Math.min(shown.length - 1, i + 1) : Math.max(0, i - 1);
    if (n === i || !shown[n]) return;
    e.preventDefault();
    const target = shown[n];
    void pick(target).then(() => (listEl?.querySelector(`[data-name="${CSS.escape(selName ?? target.name)}"]`) as HTMLElement | null)?.focus());
  }

  function syncTitle(g: SkillGroup): string {
    switch (g.sync) {
      case 'in_sync':
        return `In sync across ${g.variants.length} copies`;
      case 'drifted':
        return g.drift.join('\n');
      case 'single':
        return 'Only in one place';
      default:
        return 'Comparing copies…';
    }
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => { if (partialError) announce(partialError, { assertive: true }); });
;
async () => {

 { svelteHTML.createElement("div", {   "class":`browser`,...__sveltets_2_empty({"data-testid":`skills-browser`}),});
  if(loading){
     { svelteHTML.createElement("div", { "class":`split`,});
       { svelteHTML.createElement("aside", { "class":`list-pane`,}); { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {      "rows":10,"height":40,"label":`skills`,}});} } }
       { svelteHTML.createElement("section", { "class":`detail-pane hide-phone`,}); { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {      "rows":6,"height":28,"announce":false,}});} } }
     }
  } else if (loadError){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {            "what":`skills`,"variant":`page`,"loading":retrying,"error":loadError,"empty":true,"onretry":retryLoad,}});}
  } else if (groups.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {               children:() => { return __sveltets_2_any(0); },"variant":`page`,"icon":`zap`,"title":`No skills yet`,"body":`Skills are instruction packs that teach an agent a method — how to review a diff, write release notes, run a report. Create one, or import a package.`,"actionLabel":`New skill`,"actionIcon":`plus`,"onaction":openNew,}});
       { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":openImport,});   }
     EmptyState}
  }else{
     { svelteHTML.createElement("div", { "class":`split`,});
       { svelteHTML.createElement("aside", {         "id":`skills-list-pane`,"class":`list-pane`,"aria-label":`Skills`,"style":`--list-pane-w:${listW}px`,});listCollapsed && !viewport.isPhone;viewport.isPhone && phoneDetail;
         { svelteHTML.createElement("div", { "class":`list-tools`,});
           { svelteHTML.createElement("label", { "class":`search`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":14,}});}
             { svelteHTML.createElement("input", {          "dir":`ltr`,"type":`search`,"placeholder":`Filter skills…`,"bind:value":query,"aria-label":`Filter skills`,});/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           }
           { svelteHTML.createElement("div", {     "class":`chips`,"role":`group`,"aria-label":`Filter by source`,});
             { svelteHTML.createElement("button", {      "class":`fchip`,"aria-pressed":source === 'all',"onclick":() => (source = 'all'),});source === 'all';  { svelteHTML.createElement("span", { "class":`n`,});groups.length; } }
               for(let [s, n] of __sveltets_2_ensureArray(sources)){s;
               { svelteHTML.createElement("button", {        "class":`fchip`,"aria-pressed":source === s,"onclick":() => (source = source === s ? 'all' : s),"title":`Skills with a ${sourceLabel(s)} copy`,});source === s;
                if(s === 'library'){ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`book`,"size":12,}});}} else if (s === 'bundled'){ { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`box`,"size":12,}});}}else{ { const $$_nocIredivorP6C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP6C({ target: __sveltets_2_any(), props: {    "provider":s,"size":12,}});}}
                sourceLabel(s);  { svelteHTML.createElement("span", { "class":`n`,});n; }
               }
            }
            if(driftedCount > 0){
               { svelteHTML.createElement("button", {        "class":`fchip warn`,"aria-pressed":driftedOnly,"onclick":() => (driftedOnly = !driftedOnly),"title":`Skills whose copies differ`,});driftedOnly;
                 { svelteHTML.createElement("span", { "class":`sdot drifted`,}); }  { svelteHTML.createElement("span", { "class":`n`,});driftedCount; }
               }
            }
           }
           { svelteHTML.createElement("div", {     "class":`chips`,"role":`group`,"aria-label":`Filter by category`,});
               for(let [c, n] of __sveltets_2_ensureArray(topCats)){c;
               { svelteHTML.createElement("button", {      "class":`fchip`,"aria-pressed":category === c,"onclick":() => (category = category === c ? null : c),});category === c;c;  { svelteHTML.createElement("span", { "class":`n`,});n; } }
            }
            if(moreCats.length > 0){
               { svelteHTML.createElement("button", {        "class":`fchip`,"aria-haspopup":`menu`,"onclick":moreCategories,"title":`More categories`,});!!category && !topCats.some(([c]) => c === category);
                category && !topCats.some(([c]) => c === category) ? category : `${moreCats.length} more`;
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
               }
            }
           }
         }
        if(partialError){
           { svelteHTML.createElement("div", { "class":`partial`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
             { svelteHTML.createElement("span", {   "class":`grow`,"title":partialError,});partialError; }
             { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":loadAll,});  }
           }
        }
         { const $$_div3 = svelteHTML.createElement("div", {            "class":`list`,...__sveltets_2_empty({"data-testid":`skill-list`}),"role":`listbox`,"aria-label":`Skills`,"tabindex":-1,"onkeydown":onListKey,});listEl = $$_div3;
             for(let [cat, items] of __sveltets_2_ensureArray(shownByCat)){cat;
             { svelteHTML.createElement("div", {   "class":`cat`,"role":`presentation`,});cat;  { svelteHTML.createElement("span", { "class":`n`,});items.length; } }
               for(let g of __sveltets_2_ensureArray(items)){g.name;
               { svelteHTML.createElement("button", {                "class":`row`,"role":`option`,"aria-selected":g.name === selName,"tabindex":g.name === selName ? 0 : -1,...__sveltets_2_empty({"data-name":g.name}),...__sveltets_2_empty({"data-testid":`skill-row`}),"onclick":() => pick(g),});g.name === selName;
                 { svelteHTML.createElement("span", {   "class":`sdot ${g.sync}`,"title":syncTitle(g),}); }
                 { svelteHTML.createElement("span", { "class":`row-main`,});
                   { svelteHTML.createElement("span", {   "class":`row-name`,"title":g.name,});g.name; }
                  if(g.description){ { svelteHTML.createElement("span", {   "class":`row-desc`,"title":g.description,});g.description; }}
                 }
                
                 { svelteHTML.createElement("span", {   "class":`badges`,"aria-label":`Copies: ${g.variants.map((v) => sourceLabel(v.source)).join(', ')} · ${syncTitle(g).split('\n')[0]}`,});
                     for(let v of __sveltets_2_ensureArray(g.variants)){v.source;
                     { svelteHTML.createElement("span", {    "class":`src-tile`,"title":`${sourceLabel(v.source)}${g.driftedSources.includes(v.source) ? ' — differs' : ''}`,});g.driftedSources.includes(v.source);
                      if(v.source === 'library'){ { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`book`,"size":12,}});}} else if (v.source === 'bundled'){ { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`box`,"size":12,}});}}else{ { const $$_nocIredivorP7C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP7C({ target: __sveltets_2_any(), props: {    "provider":v.source,"size":12,}});}}
                     }
                  }
                 }
               }
            }
          }
             { svelteHTML.createElement("div", { "class":`no-match`,});
               { svelteHTML.createElement("p", { "class":`dim`,});  query.trim() ? ` “${query.trim()}”` : '';  }
              if(filtering){ { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":clearFilters,});  }}
             }
          
         }
         { svelteHTML.createElement("div", {   "class":`legend dim`,"aria-hidden":`true`,});
           { svelteHTML.createElement("span", {}); { svelteHTML.createElement("span", { "class":`sdot in_sync`,}); }  }
           { svelteHTML.createElement("span", {}); { svelteHTML.createElement("span", { "class":`sdot drifted`,}); }  }
           { svelteHTML.createElement("span", {}); { svelteHTML.createElement("span", { "class":`sdot single`,}); }  }
         }
       }
      if(!viewport.isMobile && !listCollapsed){
         { const $$_rediviDenaP2C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP2 = new $$_rediviDenaP2C({ target: __sveltets_2_any(), props: {       width:listW,"storageKey":`skills.listW`,"label":`Resize the skills list`,}});/*Ωignore_startΩ*/() => listW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP2.$$bindings = 'width';}
      }

       { svelteHTML.createElement("section", {  "class":`detail-pane`,});viewport.isPhone && !phoneDetail;
        if(selected){
           { const $$_liateDllikS3C = __sveltets_2_ensureComponent(SkillDetail); new $$_liateDllikS3C({ target: __sveltets_2_any(), props: {                              "group":selected,"source":selSource,tab,"wsId":ws.currentId ?? '',"libraryBody":library.find((l) => l.name === selected.name)?.body,"bodyOf":(s) => bodies[bodyKey(s, selected.name)],"onsource":(s) => (selSource = s),"ontab":(t) => (tab = t),"onchanged":changed,"ondeleted":deleted,"onbody":(s, b) => (bodies = { ...bodies, [bodyKey(s, selected.name)]: b }),"onreview":() => onreview?.(selected.name, selSource),"onevaluate":() => onevaluate?.(selected.name, selSource),"ondirty":(d) => (editorDirty = d),onopenrun,onopenreview,}});}
        }else{
          
          if(groups.length === 0){
             { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {            "title":`No skills yet`,"body":`Create a skill or import a package to see its method, files and history here.`,"icon":`zap`,"actionLabel":`New skill…`,"actionIcon":`plus`,"onaction":openNew,}});}
          }
        }
       }
     }
  }
 }

if(newOpen){
   { const $$_ladoMllikSweN0C = __sveltets_2_ensureComponent(NewSkillModal); new $$_ladoMllikSweN0C({ target: __sveltets_2_any(), props: {            bundled,"categories":cats.map(([c]) => c).filter((c) => c !== 'uncategorized'),"taken":new Set(library.map((l) => l.name)),"initial":newTemplate,"onclose":() => (newOpen = false),"oncreated":created,}});}
}


};
return { props: {} as any as Props, exports: {} as any as { back: typeof back,confirmLeave: typeof confirmLeave,openNew: typeof openNew,openImport: typeof openImport }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SkillsBrowser__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SkillsBrowser__SvelteComponent_ = ReturnType<typeof SkillsBrowser__SvelteComponent_>;
/*Ωignore_endΩ*/export default SkillsBrowser__SvelteComponent_;