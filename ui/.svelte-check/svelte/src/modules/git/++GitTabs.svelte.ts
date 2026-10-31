///<reference types="svelte" />
;// GitKraken-style top-level repo tab strip for the Git page. Each tab is one
// OPEN repo (name + a status dot + a close ✕, and its current branch when
// the strip has room — see `fitBranches`; the branch is always in the
// tab's tooltip and accessible name). Click to
// activate, middle-click to close, drag to reorder (mirrors shell/TabBar). The
// trailing + button opens a picker of repos not already open. Styled like
// ApiPage's `.req-tabs`. Workspace-independent — driven entirely by the git
// store's open-tabs state, which persists across restarts.

import { tick, untrack } from 'svelte';
import type { Repo } from '../../lib/api/types';
import { git } from '../../lib/stores/git.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import Icon from '../../lib/components/Icon.svelte';
import { onTabKey } from '../../lib/tabKeys';
import { branchMode, repoTabLabel } from './repoTabFit';
;

  interface Props {
    /** Open a repo as a tab (parent loads/wires the active RepoView). */
    onopen: (repoId: string) => void;
    /** Open the "Add repository" flow in a given mode (parent owns the modal). */
    onadd: (mode: 'register' | 'clone' | 'browse') => void;
    /** Rendered inside the PageHeader bar (pill tabs, no own border row). */
    embedded?: boolean;
    /** The page-level Focus view is showing: its entry is pressed and no
     *  repo tab reads as selected. */
    focus?: boolean;
    /** Toggle Focus. Without it the Focus entry isn't rendered. */
    ontogglefocus?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onopen, onadd, embedded = false, focus = false, ontogglefocus }: Props = $props();
  const selectedId = $derived(focus ? null : git.activeRepoId);

  const byId = $derived(new Map(git.allRepos.map((r) => [r.id, r])));
  const openRepos = $derived(
    git.openRepoIds.map((id) => byId.get(id)).filter((r): r is Repo => r != null),
  );
  const closedRepos = $derived(git.allRepos.filter((r) => !git.openRepoIds.includes(r.id)));

  // ── Per-repo status (branch + dirty dot) ─────────────────────────────────
  // Status is owned by the git store (shared with RepoView + the auto-fetch
  // loop). Lazily load it once per open repo so the strip shows the live branch
  // + a clean/dirty indicator; the auto-fetch loop keeps it fresh thereafter.
  // `untrack` the loads so this effect re-runs only when the OPEN set changes,
  // not on every status write the loop makes.
  $effect(() => {
    const ids = git.openRepoIds;
    untrack(() => {
      for (const id of ids) git.ensureStatus(id);
    });
  });

  function branchOf(id: string): string | null {
    return git.statusById[id]?.branch ?? null;
  }
  function isDirty(id: string): boolean {
    return (git.statusById[id]?.changes.length ?? 0) > 0;
  }

  function openPicker(e: MouseEvent): void {
    // ALWAYS offer ways to add a new repo (clone or local) — even when every
    // registered repo is already open — then list any not-yet-open repos to
    // open. Picking one that's already open just activates it (openRepoTab is
    // idempotent), so there's no duplicate-tab risk.
    // The repo list is data-driven and can be long — searchable, and capped at
    // a dozen visible rows (the search narrows into the rest). The three add
    // actions stay pinned above the list.
    ctxMenu.show(
      e,
      [
        { label: 'Clone a repository…', icon: 'download', pinned: true, action: () => onadd('clone') },
        { label: 'Browse remote to clone…', icon: 'globe', pinned: true, action: () => onadd('browse') },
        { label: 'Add a local repository…', icon: 'folder', pinned: true, action: () => onadd('register') },
        ...(closedRepos.length > 0 ? [{ separator: true }] : []),
        ...closedRepos.map((r) => ({
          label: r.name,
          icon: 'branch',
          action: () => onopen(r.id),
        })),
      ],
      { filter: true, filterPlaceholder: 'Filter repositories…', maxVisible: 12 },
    );
  }

  /** Tablist keys: Enter/Space open, Delete or Backspace closes (focus moves to
   *  the neighbour); ←/→ (Home/End, RTL-aware) move + open via the shared
   *  roving-tabindex handler; ⌥⇧←/→ moves the tab itself (the keyboard path
   *  for drag-to-reorder, as in shell/TabBar). */
  function onRepoTabKey(e: KeyboardEvent, id: string): void {
    if (e.altKey && e.shiftKey && !e.metaKey && !e.ctrlKey && (e.key === 'ArrowLeft' || e.key === 'ArrowRight')) {
      e.preventDefault();
      const rtl = getComputedStyle(e.currentTarget as Element).direction === 'rtl';
      moveTab(id, (e.key === 'ArrowRight') !== rtl ? 1 : -1);
    } else if (e.key === 'Enter' || e.key === ' ') {
      e.preventDefault();
      onopen(id);
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      const ids = openRepos.map((r) => r.id);
      const i = ids.indexOf(id);
      const next = ids[i + 1] ?? ids[i - 1];
      git.closeRepoTab(id);
      if (next) {
        onopen(next);
        queueMicrotask(() => listEl?.querySelector<HTMLElement>(`[data-repo-id="${CSS.escape(next)}"]`)?.focus());
      }
    } else {
      onTabKey(e);
    }
  }

  /** Move a repo tab one place (in strip order) and keep focus on it. */
  function moveTab(id: string, dir: 1 | -1): void {
    const from = git.openRepoIds.indexOf(id);
    const to = from + dir;
    if (from < 0 || to < 0 || to >= git.openRepoIds.length) return;
    git.reorderRepoTab(id, to);
    void tick().then(() => listEl?.querySelector<HTMLElement>(`[data-repo-id="${CSS.escape(id)}"]`)?.focus());
  }

  /** The tab's context menu: "Move left / right" are the pointer-free way to
   *  reorder (also ⌥⇧←/→); they name the VISUAL direction, so in RTL "Move
   *  left" moves toward the end of the strip. */
  function tabMenu(e: MouseEvent, id: string): void {
    const rtl = getComputedStyle(e.currentTarget as Element).direction === 'rtl';
    const i = git.openRepoIds.indexOf(id);
    const canBack = i > 0;
    const canFwd = i >= 0 && i < git.openRepoIds.length - 1;
    const leftOk = rtl ? canFwd : canBack;
    const rightOk = rtl ? canBack : canFwd;
    ctxMenu.show(e, [
      ...(leftOk ? [{ label: 'Move left', hint: '⌥⇧←', action: () => moveTab(id, rtl ? 1 : -1) } as MenuItem] : []),
      ...(rightOk ? [{ label: 'Move right', hint: '⌥⇧→', action: () => moveTab(id, rtl ? -1 : 1) } as MenuItem] : []),
      ...(leftOk || rightOk ? [{ separator: true } as MenuItem] : []),
      { label: 'Close tab', icon: 'x', action: () => git.closeRepoTab(id) },
    ]);
  }

  // ── Drag-to-reorder (mirrors shell/TabBar) ───────────────────────────────
  let dragId = $state<string | null>(null);
  let dragOverId = $state<string | null>(null);

  function onDragStart(e: DragEvent, id: string): void {
    dragId = id;
    e.dataTransfer?.setData('text/plain', id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function onDragOver(e: DragEvent, id: string): void {
    if (!dragId || id === dragId) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    dragOverId = id;
  }
  function onDragLeave(id: string): void {
    if (dragOverId === id) dragOverId = null;
  }
  function onDrop(e: DragEvent, id: string): void {
    e.preventDefault();
    if (!dragId || id === dragId) return;
    git.reorderRepoTab(dragId, git.openRepoIds.indexOf(id));
    dragId = null;
    dragOverId = null;
  }
  function onDragEnd(): void {
    dragId = null;
    dragOverId = null;
  }

  // ── Branch chips only while they fit ─────────────────────────────────────
  // Measured, not counted: show every branch, read the widths, and if the row
  // would overflow keep only the active tab's branch (if THAT fits) or none
  // (repoTabFit.ts). Written straight to `data-branches` on the list so the
  // measure → apply round trip happens inside one frame, without a re-render.
  // Re-measured when the strip or its header resizes, and when the tabs,
  // their branches or the active tab change.
  const GAP = 2;
  function fitBranches(): void {
    const el = listEl;
    if (!el) return;
    el.removeAttribute('data-branches');
    const tabs = Array.from(el.querySelectorAll<HTMLElement>('.git-tab'));
    const measures = tabs.map((t) => {
      const chip = t.querySelector<HTMLElement>('.git-tab-branch');
      return { full: t.getBoundingClientRect().width + GAP, branch: chip ? chip.getBoundingClientRect().width + 6 : 0 };
    });
    const overflowing = el.scrollWidth > el.clientWidth + 0.5;
    const active = tabs.findIndex((t) => t.classList.contains('active'));
    el.setAttribute('data-branches', overflowing ? branchMode(measures, el.clientWidth + GAP, active) : 'all');
  }
  let fitFrame = 0;
  function scheduleFit(): void {
    if (fitFrame) return;
    fitFrame = requestAnimationFrame(() => {
      fitFrame = 0;
      fitBranches();
    });
  }
  $effect(() => {
    const el = listEl;
    if (!el) return;
    const ro = new ResizeObserver(scheduleFit);
    ro.observe(el);
    // The row is content-sized, so its own size never says "there is room
    // again": watch what bounds it too — the header row and its siblings (the
    // actions shrinking frees room) and the header's style, where PageHeader
    // writes the tab cap (--ph-tabs-max).
    const header = el.closest('header');
    const row = el.closest('.ph-tabs-inline')?.parentElement ?? el.parentElement;
    if (row) {
      ro.observe(row);
      for (const child of Array.from(row.children)) ro.observe(child);
    }
    const mo = new MutationObserver(scheduleFit);
    if (header) mo.observe(header, { attributes: true, attributeFilter: ['style'] });
    scheduleFit();
    return () => {
      ro.disconnect();
      mo.disconnect();
      if (fitFrame) cancelAnimationFrame(fitFrame);
      fitFrame = 0;
    };
  });
  $effect(() => {
    void openRepos.map((r) => `${r.name}:${branchOf(r.id)}`).join();
    void selectedId;
    untrack(scheduleFit);
  });

  // Only the tab row scrolls; + and auto-fetch sit outside it so they are
  // never clipped. Keep the active tab in view when it changes (opened from
  // the picker, restored on load) — it may be off the scroller's edge.
  let listEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const id = git.activeRepoId;
    if (!listEl || !id) return;
    const el = listEl.querySelector<HTMLElement>(`[data-repo-id="${CSS.escape(id)}"]`);
    el?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  });
;
async () => {

 { svelteHTML.createElement("div", {  "class":`git-tabs`,});embedded;
  if(ontogglefocus && git.allRepos.length > 0){
    
     { svelteHTML.createElement("button", {          "class":`git-focus`,"aria-pressed":focus,"title":focus ? 'Back to the open repository' : 'Focus: your pull requests and Jira work across repositories',"onclick":ontogglefocus,});focus;
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":13,}});}
     }
     { svelteHTML.createElement("span", {   "class":`git-focus-sep`,"aria-hidden":`true`,}); }
  }
  
   { const $$_div1 = svelteHTML.createElement("div", {      "class":`git-tablist`,"role":`tablist`,"aria-label":`Open repositories`,});listEl = $$_div1;
     for(let r of __sveltets_2_ensureArray(openRepos)){r.id;
    const branch = branchOf(r.id);
    const label = repoTabLabel(r.name, branch);
    const dirty = isDirty(r.id);
    
     { svelteHTML.createElement("div", {                   "class":`git-tab`,"role":`presentation`,"draggable":`true`,"ondragstart":(e) => onDragStart(e, r.id),"ondragover":(e) => onDragOver(e, r.id),"ondragleave":() => onDragLeave(r.id),"ondrop":(e) => onDrop(e, r.id),"ondragend":onDragEnd,});selectedId === r.id;dragOverId === r.id;
       { svelteHTML.createElement("button", {                         "type":`button`,"class":`git-tab-main`,"role":`tab`,"tabindex":git.activeRepoId === r.id || (!git.activeRepoId && r.id === openRepos[0]?.id) ? 0 : -1,"aria-selected":selectedId === r.id,"aria-label":dirty ? `${label}, uncommitted changes` : label,...__sveltets_2_empty({"data-repo-id":r.id}),"title":`${label}${dirty ? ' · uncommitted changes' : ''}\n${r.path}`,"onclick":() => onopen(r.id),"onkeydown":(e) => onRepoTabKey(e, r.id),"oncontextmenu":(e) => tabMenu(e, r.id),"onauxclick":(e) => {
          if (e.button === 1) {
            e.preventDefault();
            git.closeRepoTab(r.id);
          }
        },});
         { svelteHTML.createElement("span", {   "class":`git-tab-dot`,"aria-hidden":`true`,});dirty; }
         { svelteHTML.createElement("span", { "class":`git-tab-name`,});r.name; }
        if(branch){
           { svelteHTML.createElement("span", {   "class":`git-tab-branch mono`,"aria-hidden":`true`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":12,}});} { svelteHTML.createElement("span", { "class":`git-tab-branch-name`,});branch; }
           }
        }
       }
       { svelteHTML.createElement("button", {            "type":`button`,"class":`git-tab-close`,"tabindex":-1,"title":`Close ${r.name}`,"aria-label":`Close ${r.name}`,"onclick":(e) => {
          e.stopPropagation();
          git.closeRepoTab(r.id);
        },}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
     }
  }
   }
   { svelteHTML.createElement("button", {            "class":`git-autofetch`,"title":git.autoFetchEnabled
      ? `Auto-fetch on — selected repo every 30s, other open repos every ${git.autoFetchIntervalSec}s while Git is open and this window is focused. Click to pause.`
      : 'Auto-fetch paused. Click to fetch open repos automatically.',"aria-label":`Toggle auto-fetch for open repositories`,"aria-pressed":git.autoFetchEnabled,"onclick":() => git.setAutoFetch(!git.autoFetchEnabled),});git.autoFetchEnabled;
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`fetch`,"size":13,}});}
   }
   { svelteHTML.createElement("button", {       "class":`git-tab-new`,"title":`Open a repository`,"aria-label":`Open a repository`,"onclick":openPicker,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":14,}});}
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GitTabs__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GitTabs__SvelteComponent_ = ReturnType<typeof GitTabs__SvelteComponent_>;
/*Ωignore_endΩ*/export default GitTabs__SvelteComponent_;