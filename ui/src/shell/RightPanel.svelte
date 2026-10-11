<script module lang="ts">
  // Each tab's panel is its own chunk, loaded the first time it is shown (perf
  // G1). The panel used to import all nine statically, so the Agents page —
  // the default landing — evaluated Git, Files, both Browsers, Canvas and the
  // API client (CodeMirror, xterm, marked, the git views) to render one tab.
  // One promise per panel, shared by every RightPanel instance (the desktop
  // aside and the mobile drawer); `panels` is reactive so a tab renders the
  // moment its chunk lands and a loaded tab switches in synchronously.
  import type { Component } from 'svelte';
  import { toastError } from '../lib/toastError';
  import { SvelteMap, SvelteSet } from 'svelte/reactivity';
  import { ui as uiStore } from '../lib/stores/ui.svelte';
  import { RIGHT_MIN, RIGHT_MAX } from '../lib/stores/ui.svelte';
  import { paneResizer, pxWide, RESIZE_TITLE } from '../lib/paneResizer';
  // Panels take different props (v1 Browser: `active`).
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  type PanelComponent = Component<any>;
  export type PanelKey = 'git' | 'files' | 'activity' | 'outputs' | 'canvas' | 'info' | 'browserV1' | 'browserV2' | 'api';
  export const PANEL_LOADERS: Record<PanelKey, () => Promise<{ default: PanelComponent }>> = {
    git: () => import('../modules/git/GitPanel.svelte'),
    files: () => import('../modules/panels/FilesPanel.svelte'),
    activity: () => import('../modules/panels/ActivityPanel.svelte'),
    outputs: () => import('../modules/panels/OutputsPanel.svelte'),
    canvas: () => import('../modules/panels/CanvasPanel.svelte'),
    info: () => import('../modules/panels/InfoPanel.svelte'),
    browserV1: () => import('../modules/panels/BrowserPanel.svelte'),
    browserV2: () => import('../modules/panels/BrowserPanelV2.svelte'),
    api: () => import('../modules/api/ApiPanel.svelte'),
  };
  const panels = new SvelteMap<PanelKey, PanelComponent>();
  const failedPanels = new SvelteSet<PanelKey>();
  const inflight = new Map<PanelKey, Promise<void>>();
  function loadPanel(key: PanelKey): Promise<void> {
    let p = inflight.get(key);
    if (!p) {
      failedPanels.delete(key);
      p = PANEL_LOADERS[key]().then(
        (m) => void panels.set(key, m.default),
        () => {
          inflight.delete(key); // let Retry re-import
          failedPanels.add(key);
        },
      );
      inflight.set(key, p);
    }
    return p;
  }
  /** The chunk a tab renders (null: Notes lives in this file, a page tab is
   *  the page's own component). */
  function panelKeyOf(tab: string | null, browserVersion: string): PanelKey | null {
    if (!tab || tab === 'notes') return null;
    if (tab === 'browser') return browserVersion === 'v1' ? 'browserV1' : 'browserV2';
    return tab in PANEL_LOADERS ? (tab as PanelKey) : null;
  }
  // Warm the persisted tab with this chunk, so an open panel's first tab does
  // not wait for the mount → effect round trip before its import starts.
  {
    const k = uiStore.rightOpen ? panelKeyOf(uiStore.rightTab, uiStore.browserPanelVersion) : null;
    if (k) void loadPanel(k);
  }
  /** Tabs whose content works on the panel's folder (they get the folder bar). */
  const FOLDER_TABS = new Set(['git', 'files']);
</script>

<script lang="ts">
  // The Side panel (⌘J), on every page. One component in both states: the
  // vertical icon rail is always there (VS Code activity bar); clicking an
  // icon opens that tab's content beside the rail, clicking the shown tab
  // again collapses back to the rail. The content has ONE header (the tab's
  // title + expand + collapse) whatever the tab. Which tabs exist and which
  // folder Git / Files use comes from stores/rightPanel.svelte.ts: the
  // session's cwd in an agent session, a workspace folder anywhere else.
  import Icon from '../lib/components/Icon.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import FolderPicker from '../lib/components/FolderPicker.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { SIDE_PANEL, railClick } from '../lib/rightTabs';
  import { rightPanel } from '../lib/stores/rightPanel.svelte';
  import { startMouseDrag } from '../lib/dragCursor';
  import { ws } from '../lib/stores/workspace.svelte';
  import { git } from '../lib/stores/git.svelte';
  import { getToken } from '../lib/api/client';
  import { ctxMenu, type MenuItem } from '../lib/contextmenu.svelte';
  import { onDestroy, untrack } from 'svelte';

  // `forceOpen` is set when the panel is hosted inside the mobile right drawer:
  // it then always shows content, fills the drawer width (no fixed width), and
  // drops the desktop drag-resize handle, expand and collapse (the drawer's
  // own ✕ closes it).
  interface Props {
    forceOpen?: boolean;
  }
  let { forceOpen = false }: Props = $props();

  // Drag-to-resize: the panel is anchored right, so dragging the left edge
  // leftwards (smaller clientX) widens it.
  let resizing = $state(false);
  // Overlay cursor + one width write per frame + one localStorage write on
  // release (lib/dragCursor.ts) — no body.style restyle of the whole app.
  function startResize(e: MouseEvent): void {
    resizing = true;
    const startX = e.clientX;
    const startW = ui.rightWidth;
    startMouseDrag(e, {
      onMove: (ev) => ui.setRightWidth(startW + (startX - ev.clientX), false),
      onEnd: () => {
        resizing = false;
        ui.setRightWidth(ui.rightWidth);
      },
    });
  }

  const tabs = $derived(rightPanel.tabs);
  const shown = $derived(rightPanel.shown);
  const shownTab = $derived(tabs.find((t) => t.id === shown) ?? null);
  const pageTab = $derived(rightPanel.pageTabs.find((t) => t.id === shown) ?? null);
  const open = $derived((ui.rightOpen || forceOpen) && shown !== null);

  function onRail(id: string): void {
    // In the drawer the content is always showing: a click only switches.
    const next = forceOpen ? { open: true, tab: id } : railClick(ui.rightOpen, shown, id);
    ui.setRight(next.open, next.tab as typeof ui.rightTab);
  }

  // Rail keys: ↑/↓ (Home/End) move focus between the icons; Enter/Space click.
  let railEl = $state<HTMLElement | null>(null);
  function onRailKey(e: KeyboardEvent): void {
    const btns = Array.from(railEl?.querySelectorAll<HTMLButtonElement>('.strip-btn') ?? []);
    const i = btns.indexOf(document.activeElement as HTMLButtonElement);
    let next = -1;
    if (e.key === 'ArrowDown') next = (i + 1) % btns.length;
    else if (e.key === 'ArrowUp') next = (i - 1 + btns.length) % btns.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = btns.length - 1;
    if (next < 0 || !btns.length) return;
    e.preventDefault();
    btns[next].focus();
  }

  // ── folder bar (Git / Files) ─────────────────────────────────────────────
  const folder = $derived(rightPanel.folder);
  const folderName = $derived(folder.split('/').filter(Boolean).at(-1) ?? folder);
  let pickerOpen = $state(false);
  function folderMenu(e: MouseEvent): void {
    const root = ws.current?.root_path ?? '';
    const items: MenuItem[] = [
      {
        label: root ? `Workspace folder · ${root.split('/').filter(Boolean).at(-1) ?? root}` : 'Workspace folder',
        icon: 'folder',
        checked: !rightPanel.picked,
        disabled: !root,
        title: root || 'This workspace has no folder. Choose one below.',
        pinned: true,
        action: () => rightPanel.pickFolder(null),
      },
      { label: 'Choose folder…', icon: 'folder', pinned: true, action: () => (pickerOpen = true) },
    ];
    const repos = git.repos.filter((r) => r.path && r.path !== root);
    if (repos.length) {
      items.push({ separator: true, pinned: true });
      for (const r of repos) {
        items.push({ label: r.name, icon: 'branch', hint: 'repository', title: r.path, checked: folder === r.path, action: () => rightPanel.pickFolder(r.path) });
      }
    }
    const others = ws.workspaces.filter((w) => w.id !== ws.currentId && w.root_path);
    if (others.length) {
      items.push({ separator: true, pinned: true });
      for (const w of others) {
        items.push({ label: w.name, icon: 'folder', hint: 'workspace', title: w.root_path, checked: folder === w.root_path, action: () => rightPanel.pickFolder(w.root_path) });
      }
    }
    ctxMenu.show(e, items, { filter: repos.length + others.length > 8, filterPlaceholder: 'Find a repository or workspace' });
  }

  // ── notes ────────────────────────────────────────────────────────────────
  let notes = $state('');
  let notesLoadedFor: string | null = $state(null);
  let saveTimer: ReturnType<typeof setTimeout> | null = null;
  let saveState: 'idle' | 'saving' | 'saved' | 'error' = $state('idle');
  let notesError = $state('');
  let notesRevision = 0;
  let notesQueue: Promise<void> = Promise.resolve();

  /** Text typed but not yet saved, bound to the workspace it was typed in:
   *  the debounce used to save whatever workspace was current WHEN IT FIRED,
   *  so switching within 600 ms wrote B's text back to B and lost A's edit. */
  let pendingNotes: { wsId: string; text: string; token: string | null } | null = null;

  $effect(() => {
    // (re)load notes when workspace changes
    const w = ws.current;
    if (w && notesLoadedFor !== w.id) {
      untrack(() => void flushNotes());
      notesLoadedFor = w.id;
      notes = typeof w.settings?.notes === 'string' ? (w.settings.notes as string) : '';
      notesRevision++;
      notesError = '';
      saveState = 'idle';
    }
  });

  async function flushNotes(): Promise<void> {
    if (saveTimer) clearTimeout(saveTimer);
    saveTimer = null;
    const p = pendingNotes;
    pendingNotes = null;
    if (!p || p.token !== getToken()) return;
    const revision = notesRevision;
    saveState = 'saving';
    notesError = '';
    // Serialize writes so an older, slow save cannot overwrite newer text.
    const saving = notesQueue.then(() => {
      if (p.token === getToken()) return ws.saveNotes(p.text, p.wsId);
    });
    notesQueue = saving.catch(() => {});
    try {
      await saving;
      if (p.token !== getToken() || p.wsId !== notesLoadedFor || revision !== notesRevision) return;
      saveState = 'saved';
    } catch (e) {
      if (p.token !== getToken()) return;
      if (p.wsId !== notesLoadedFor) {
        toastError('Couldn’t save the notes', e);
        return;
      }
      if (revision !== notesRevision) return;
      pendingNotes = p;
      notesError = e instanceof Error ? e.message : String(e);
      saveState = 'error';
    }
  }

  function onNotesInput(): void {
    if (!notesLoadedFor) return;
    if (saveTimer) clearTimeout(saveTimer);
    pendingNotes = { wsId: notesLoadedFor, text: notes, token: getToken() };
    notesRevision++;
    notesError = '';
    saveState = 'saving';
    saveTimer = setTimeout(() => void flushNotes(), 600);
  }

  onDestroy(() => void flushNotes());

  // Keep-alive for the v1 Browser: it holds live pages (native webviews),
  // several tabs and unsent take-over annotations in component state, so
  // unmounting it on every tab switch or ⌘J collapse threw all of that away.
  // Once opened it stays mounted (hidden) until the panel itself goes away;
  // every other tab still mounts on demand (their state lives in stores).
  let browserKept = $state(false);
  $effect(() => {
    if (open && shown === 'browser' && ui.browserPanelVersion === 'v1') browserKept = true;
  });
  // Leaving the session drops the v1 tab from the rail: release its webviews.
  $effect(() => {
    if (browserKept && !tabs.some((t) => t.id === 'browser')) browserKept = false;
  });
  const browserShown = $derived(open && shown === 'browser');

  // Load the visible tab's chunk, and the kept-alive v1 Browser's.
  $effect(() => {
    const k = open ? panelKeyOf(shown, ui.browserPanelVersion) : null;
    if (k && !panels.has(k)) void loadPanel(k);
  });
</script>

<!-- One tab's panel: its chunk, a skeleton while it loads, Retry if it failed. -->
{#snippet lazyPanel(key: PanelKey, props: Record<string, unknown> = {})}
  {@const Panel = panels.get(key)}
  {#if Panel}
    <Panel {...props} />
  {:else if failedPanels.has(key)}
    <div class="rp-fail" role="alert">
      <span>Couldn’t load this panel.</span>
      <button class="btn small" onclick={() => void loadPanel(key)}>Retry</button>
    </div>
  {:else}
    <div class="rp-loading"><Skeleton rows={4} /></div>
  {/if}
{/snippet}

{#if pickerOpen}
  <FolderPicker
    title="Folder for the {SIDE_PANEL}"
    start={folder}
    onpick={(p) => {
      pickerOpen = false;
      rightPanel.pickFolder(p);
    }}
    onclose={() => (pickerOpen = false)}
  />
{/if}

<div class="rside" class:embedded={forceOpen}>
  {#if open || browserKept}
    <aside
      class="rpanel"
      id="side-panel-content"
      class:resizing
      class:embedded={forceOpen}
      hidden={!open}
      aria-label={shownTab ? `${shownTab.label} · ${SIDE_PANEL}` : SIDE_PANEL}
      style={forceOpen ? undefined : `width:${ui.rightWidth}px`}
    >
      {#if !forceOpen}
        <!-- The window splitter (paneResizer: focus, keys, drag, double-click reset). -->
        <div
          class="resize-handle"
          role="separator"
          aria-label="Resize the {SIDE_PANEL}"
          title={RESIZE_TITLE}
          use:paneResizer={{ value: ui.rightWidth, min: RIGHT_MIN, max: Math.max(RIGHT_MIN, Math.min(RIGHT_MAX, (typeof window === 'undefined' ? RIGHT_MAX : window.innerWidth) - 360)), invert: true, onChange: (w) => ui.setRightWidth(w), onReset: () => ui.setRightWidth(300), onDragStart: startResize, text: pxWide }}
        ></div>
      {/if}
      <header class="rpanel-head">
        {#if shownTab}<Icon name={shownTab.icon} size={13} />{/if}
        <h2 class="rpanel-title">{shownTab?.label ?? SIDE_PANEL}</h2>
        {#if !forceOpen}
          <button
            class="icon-btn"
            onclick={() => ui.toggleRightWide()}
            title={ui.rightWide ? 'Restore panel width' : 'Expand panel — wider view'}
            aria-label={ui.rightWide ? 'Restore panel width' : 'Expand panel'}
            aria-pressed={ui.rightWide}
          >
            <Icon name={ui.rightWide ? 'minimize' : 'maximize'} size={13} />
          </button>
          <button
            class="icon-btn"
            onclick={() => ui.toggleRight()}
            title="Collapse panel (⌘J)"
            aria-keyshortcuts="Meta+J"
            aria-label="Collapse panel"
          >
            <Icon name="panel" size={13} />
          </button>
        {/if}
      </header>

      {#if shown && FOLDER_TABS.has(shown)}
        <!-- Which folder Git / Files work in: the session's, or a pickable one. -->
        <div class="rpanel-folder">
          {#if rightPanel.session}
            <span class="folder-label" title={folder}>
              <Icon name="folder" size={12} />
              <span class="folder-name">{folderName || 'No folder'}</span>
              <span class="dim folder-kind">session</span>
            </span>
          {:else}
            <button class="folder-btn" title={folder ? `${folder} · change the folder` : 'Choose a folder'} aria-haspopup="menu" aria-label="Folder: {folderName || 'none'}. Change folder" onclick={folderMenu}>
              <Icon name="folder" size={12} />
              <span class="folder-name">{folderName || 'Choose a folder'}</span>
              {#if folder}<span class="dim folder-kind">{rightPanel.picked ? 'picked' : 'workspace'}</span>{/if}
              <Icon name="chevronDown" size={12} />
            </button>
          {/if}
        </div>
      {/if}

      <div class="rpanel-body">
        {#if pageTab}
          {@const PagePanel = pageTab.component}
          <PagePanel {...pageTab.props ?? {}} />
        {:else if shown === 'git' || shown === 'files' || shown === 'activity' || shown === 'outputs' || shown === 'canvas' || shown === 'info'}
          {@render lazyPanel(shown as PanelKey)}
        {/if}
        {#if browserShown || (browserKept && ui.browserPanelVersion === 'v1')}
          <!-- v2 embeds the Browser module (persisted tabs/marks + ask bar); v1 is
               the classic per-session panel, chosen in Settings → Browser and
               kept mounted while hidden (see `browserKept`). -->
          <div class="browser-host" hidden={!browserShown}>
            {#if ui.browserPanelVersion === 'v2'}
              {@render lazyPanel('browserV2')}
            {:else}
              {@render lazyPanel('browserV1', { active: browserShown })}
            {/if}
          </div>
        {/if}
        {#if shown === 'api'}
          {@render lazyPanel('api')}
        {:else if shown === 'notes'}
          <div class="notes-wrap">
            <textarea dir="auto"
              class="notes"
              aria-label="Workspace notes"
              bind:value={notes}
              oninput={onNotesInput}
              placeholder="Workspace notes (markdown)…"
              spellcheck="false"
            ></textarea>
            <div class="notes-foot" aria-live="polite">
              {#if saveState === 'error'}
                <div class="notes-error" role="alert"><span>Notes not saved. {notesError}</span>
                  <button class="btn small" onclick={() => void flushNotes()}>Retry</button>
                </div>
              {:else if saveState === 'saving'}<span class="dim">Saving…</span>
              {:else if saveState === 'saved'}<span class="dim">Saved</span>
              {:else}<span class="dim">Saved to this workspace as you type</span>{/if}
            </div>
          </div>
        {/if}
      </div>
    </aside>
  {/if}

  <!-- The rail: the same in both states. Icons only; the label is the tooltip
       and the accessible name. A vertical toolbar: ↑/↓ move between icons. -->
  <div class="rstrip" role="toolbar" aria-orientation="vertical" tabindex="-1" aria-label={SIDE_PANEL} bind:this={railEl} onkeydown={onRailKey}>
    {#each tabs as t, i (t.id)}
      {#if i > 0 && tabs[i - 1].page && !t.page}<span class="strip-sep" aria-hidden="true"></span>{/if}
      <button
        class="icon-btn strip-btn"
        class:active={open && shown === t.id}
        onclick={() => onRail(t.id)}
        title={open && shown === t.id && !forceOpen ? `${t.label} · click to collapse` : t.label}
        aria-label={t.label}
        aria-pressed={open && shown === t.id}
        aria-controls={open ? 'side-panel-content' : undefined}
      >
        <Icon name={t.icon} size={14} />
      </button>
    {/each}
  </div>
</div>

<style>
  .rside {
    height: 100%;
    display: flex;
    flex-shrink: 0;
    min-height: 0;
  }
  /* In the mobile drawer: fill it. */
  .rside.embedded {
    flex: 1;
    width: 100%;
    min-width: 0;
  }
  .rp-fail {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 24px 12px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rp-loading {
    padding: 12px;
  }
  .rpanel {
    /* width is set inline from ui.rightWidth (drag-resizable) */
    height: 100%;
    display: flex;
    flex-direction: column;
    border-inline-start: 1px solid var(--border);
    background: var(--bg);
    flex-shrink: 0;
    min-width: 0;
    position: relative;
  }
  /* `hidden` must beat the display:flex above (kept-alive browser). */
  .rpanel[hidden],
  .browser-host[hidden] {
    display: none;
  }
  .rpanel.resizing {
    /* no transition while dragging for 1:1 tracking */
    user-select: none;
  }
  /* Hosted inside the mobile right drawer: fill the drawer beside the rail, no
     left border (the drawer supplies it), no fixed width. */
  .rpanel.embedded {
    flex: 1;
    width: auto;
    border-inline-start: none;
  }
  .resize-handle {
    position: absolute;
    inset-inline-start: -3px;
    top: 0;
    bottom: 0;
    width: 7px;
    cursor: col-resize;
    z-index: 5;
  }
  .resize-handle:hover,
  .rpanel.resizing .resize-handle {
    background: linear-gradient(
      to right,
      transparent 0,
      var(--accent-line) 45%,
      var(--accent-line) 55%,
      transparent 100%
    );
  }
  /* One header for every tab: icon + title, then expand + collapse. Same
     height as the rail's first slot so the two read as one component. */
  .rpanel-head {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 37px;
    padding-block: 6px;
    padding-inline: 12px 8px;
    border-bottom: 1px solid var(--border);
    color: var(--text-dim);
  }
  .rpanel-title {
    flex: 1;
    min-width: 0;
    margin: 0;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .rpanel-head > :global(.icon-btn) {
    flex-shrink: 0;
  }
  .rpanel-folder {
    display: flex;
    align-items: center;
    min-width: 0;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
  }
  .folder-btn,
  .folder-label {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    max-width: 100%;
    padding: 2px 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    text-align: start;
  }
  .folder-btn {
    cursor: pointer;
  }
  .folder-btn:hover {
    background: var(--hover);
  }
  .folder-btn:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .folder-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
  }
  .folder-kind {
    flex-shrink: 0;
    font-size: var(--fs-xs);
  }
  .rpanel-body {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
  }
  .browser-host {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .browser-host > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .rstrip {
    width: 36px;
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding-top: 6px;
    border-inline-start: 1px solid var(--border);
    background: var(--bg);
    flex-shrink: 0;
    overflow-x: hidden;
    overflow-y: auto;
  }
  /* The drawer floats its ✕ over the top corner: start the icons below it. */
  .rside.embedded .rstrip {
    padding-top: 44px;
  }
  .strip-btn {
    width: 28px;
    height: 28px;
    flex-shrink: 0;
    position: relative;
  }
  /* The shown tab: accent tint + a bar on the edge it opens toward. */
  .strip-btn.active {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .strip-btn.active::before {
    content: '';
    position: absolute;
    inset-block: 6px;
    inset-inline-start: -4px;
    width: 2px;
    border-radius: 1px;
    background: var(--accent-text);
  }
  .strip-sep {
    width: 16px;
    height: 1px;
    margin-block: 2px;
    background: var(--border);
    flex-shrink: 0;
  }
  .notes-wrap {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .notes {
    flex: 1;
    border: none;
    resize: none;
    background: transparent;
    padding: 12px;
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    line-height: 1.6;
    color: var(--text);
    outline: none;
  }
  .notes-error {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--danger);
    overflow-wrap: anywhere;
  }
  .notes-error span { flex: 1; min-width: 0; }
  .notes:focus-visible { outline: 2px solid var(--accent-text); outline-offset: -2px; }
  .notes-foot {
    padding: 4px 12px 8px;
    font-size: var(--fs-xs);
  }
  @media (prefers-reduced-motion: no-preference) {
    .strip-btn {
      transition: background var(--dur-fast) ease-out, color var(--dur-fast) ease-out;
    }
  }
</style>
