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
  /** The chunk a tab renders (null: Notes lives in this file). */
  function panelKeyOf(tab: string, browserVersion: string): PanelKey | null {
    if (tab === 'notes') return null;
    if (tab === 'browser') return browserVersion === 'v1' ? 'browserV1' : 'browserV2';
    return tab in PANEL_LOADERS ? (tab as PanelKey) : null;
  }
  // Warm the persisted tab with this chunk, so an open panel's first tab does
  // not wait for the mount → effect round trip before its import starts.
  {
    const k = uiStore.rightOpen ? panelKeyOf(uiStore.rightTab, uiStore.browserPanelVersion) : null;
    if (k) void loadPanel(k);
  }
</script>

<script lang="ts">
  // Collapsible right panel (⌘J): Git / Files / Notes / Activity / Outputs / Canvas / Info / Browser / API tabs ⇄ 36px icon strip.
  import Icon from '../lib/components/Icon.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';
  import { ui, type RightTab } from '../lib/stores/ui.svelte';
  import { RIGHT_TABS, SESSION_PANEL } from '../lib/rightTabs';
  import { startMouseDrag } from '../lib/dragCursor';
  import { ws } from '../lib/stores/workspace.svelte';
  import { getToken } from '../lib/api/client';
  import { ctxMenu, type MenuItem } from '../lib/contextmenu.svelte';
  import { onDestroy, untrack } from 'svelte';

  // `forceOpen` is set when the panel is hosted inside the mobile right drawer:
  // it then always renders the panel body, fills the drawer width (no fixed
  // 300px), and drops the desktop drag-resize handle + collapse-to-strip path.
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

  // A narrow panel can't fit all nine labels. The row used to scroll, so the
  // edge tab was cut mid-word ("Ou", "Brow"). Now the tabs that don't fit
  // fold into a "More" menu (the active tab always stays visible), the way
  // PageHeader folds its actions.
  let tabsEl = $state<HTMLDivElement | null>(null);
  let overflowIds = $state<RightTab[]>([]);
  const MORE_W = 34; // the "More" button (+ its gap)
  let measuring = false;
  function measureTabs(): void {
    const el = tabsEl;
    if (!el || measuring) return;
    measuring = true;
    try {
      const btns = Array.from(el.querySelectorAll<HTMLElement>('.rtab'));
      for (const b of btns) b.removeAttribute('data-rt-hidden');
      const widths = btns.map((b) => b.getBoundingClientRect().width + 2);
      const total = widths.reduce((a, w) => a + w, 0);
      const avail = el.clientWidth;
      if (total <= avail + 0.5) {
        if (overflowIds.length) overflowIds = [];
        return;
      }
      const activeIdx = tabs.findIndex((t) => t.id === ui.rightTab);
      // With the button already shown, clientWidth has room for it taken.
      let used = (overflowIds.length ? 0 : MORE_W) + (activeIdx >= 0 ? widths[activeIdx] : 0);
      // Keep the row in order: once one tab doesn't fit, every later one
      // folds too (a short later tab slipping in would reorder the strip).
      const hidden: RightTab[] = [];
      let full = false;
      tabs.forEach((t, i) => {
        if (i === activeIdx) return;
        if (!full && used + widths[i] <= avail) used += widths[i];
        else {
          full = true;
          hidden.push(t.id);
        }
      });
      btns.forEach((b, i) => {
        if (hidden.includes(tabs[i].id)) b.setAttribute('data-rt-hidden', '');
      });
      if (hidden.join() !== overflowIds.join()) overflowIds = hidden;
    } finally {
      measuring = false;
    }
  }
  let raf = 0;
  function scheduleMeasure(): void {
    if (raf) return;
    raf = requestAnimationFrame(() => {
      raf = 0;
      measureTabs();
    });
  }
  $effect(() => {
    const el = tabsEl;
    if (!el) return;
    const ro = new ResizeObserver(scheduleMeasure);
    ro.observe(el);
    scheduleMeasure();
    return () => {
      ro.disconnect();
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
    };
  });
  // Picking a folded tab makes it the active one — re-fit so it swaps in.
  $effect(() => {
    void ui.rightTab;
    untrack(scheduleMeasure);
  });
  function openMore(e: MouseEvent): void {
    const items: MenuItem[] = tabs
      .filter((t) => overflowIds.includes(t.id))
      .map((t) => ({ label: t.label, icon: t.icon, action: () => (ui.rightTab = t.id) }));
    ctxMenu.show(e, items);
  }

  // One list with the ⌘K "Open <tab> panel" commands (lib/rightTabs.ts).
  const tabs = RIGHT_TABS;

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
  const open = $derived(ui.rightOpen || forceOpen);
  let browserKept = $state(false);
  $effect(() => {
    if (open && ui.rightTab === 'browser' && ui.browserPanelVersion === 'v1') browserKept = true;
  });
  const browserShown = $derived(open && ui.rightTab === 'browser');

  // Load the visible tab's chunk, and the kept-alive v1 Browser's.
  $effect(() => {
    const k = open ? panelKeyOf(ui.rightTab, ui.browserPanelVersion) : null;
    if (k && !panels.has(k)) void loadPanel(k);
  });

  // Tablist keys: ←/→ (RTL-aware), Home/End move and select; focus follows.
  function onTabsKey(e: KeyboardEvent): void {
    const i = tabs.findIndex((t) => t.id === ui.rightTab);
    const rtl = getComputedStyle(e.currentTarget as Element).direction === 'rtl';
    let next = i;
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      const fwd = (e.key === 'ArrowRight') !== rtl;
      next = (i + (fwd ? 1 : -1) + tabs.length) % tabs.length;
    } else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = tabs.length - 1;
    else return;
    e.preventDefault();
    ui.rightTab = tabs[next].id;
    // After the re-fit (a folded tab swaps in on the next frame).
    requestAnimationFrame(() =>
      requestAnimationFrame(() => tabsEl?.querySelector<HTMLElement>('.rtab.active')?.focus()),
    );
  }
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

{#if open || browserKept}
  <aside
    class="rpanel"
    class:resizing
    class:embedded={forceOpen}
    hidden={!open}
    style={forceOpen ? undefined : `width:${ui.rightWidth}px`}
  >
    {#if !forceOpen}
      <!-- The window splitter (paneResizer: focus, keys, drag, double-click reset). -->
      <div
        class="resize-handle"
        role="separator"
        aria-label="Resize the {SESSION_PANEL}"
        title={RESIZE_TITLE}
        use:paneResizer={{ value: ui.rightWidth, min: RIGHT_MIN, max: Math.max(RIGHT_MIN, Math.min(RIGHT_MAX, (typeof window === 'undefined' ? RIGHT_MAX : window.innerWidth) - 360)), invert: true, onChange: (w) => ui.setRightWidth(w), onReset: () => ui.setRightWidth(300), onDragStart: startResize, text: pxWide }}
      ></div>
    {/if}
    <header class="rpanel-head">
      <div class="rpanel-tabs" role="tablist" tabindex="-1" aria-label={SESSION_PANEL} bind:this={tabsEl} onkeydown={onTabsKey}>
        {#each tabs as t (t.id)}
          <button
            class="rtab"
            class:active={ui.rightTab === t.id}
            role="tab"
            aria-selected={ui.rightTab === t.id}
            tabindex={ui.rightTab === t.id ? 0 : -1}
            onclick={() => (ui.rightTab = t.id)}
          >
            {t.label}
          </button>
        {/each}
      </div>
      {#if overflowIds.length > 0}
        <button
          class="icon-btn rtab-more"
          onclick={openMore}
          title="More panels: {tabs.filter((t) => overflowIds.includes(t.id)).map((t) => t.label).join(', ')}"
          aria-label="More panels"
          aria-haspopup="menu"
        >
          <Icon name="chevronDown" size={13} />
        </button>
      {/if}
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
      {/if}
      <button
        class="icon-btn"
        onclick={() => ui.toggleRight()}
        title="Collapse panel" aria-keyshortcuts="Meta+J"
        aria-label="Collapse panel"
      >
        <Icon name="panel" size={13} />
      </button>
    </header>

    <div class="rpanel-body">
      {#if ui.rightTab === 'git' || ui.rightTab === 'files' || ui.rightTab === 'activity' || ui.rightTab === 'outputs' || ui.rightTab === 'canvas' || ui.rightTab === 'info'}
        {@render lazyPanel(ui.rightTab as PanelKey)}
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
      {#if ui.rightTab === 'api'}
        {@render lazyPanel('api')}
      {:else if ui.rightTab === 'notes'}
        <div class="notes-wrap">
          <textarea
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
{#if !open}
  <aside class="rstrip" aria-label={SESSION_PANEL}>
    {#each tabs as t (t.id)}
      <button
        class="icon-btn strip-btn"
        onclick={() => ui.openRight(t.id)}
        title={t.label}
        aria-label={t.label}
      >
        <Icon name={t.icon} size={14} />
      </button>
    {/each}
  </aside>
{/if}

<style>
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
  /* Hosted inside the mobile right drawer: fill the drawer, no left border
     (the drawer supplies it), no fixed width. */
  .rpanel.embedded {
    width: 100%;
    border-inline-start: none;
  }
  /* …and leave the drawer's floating close button (32px + 8px inset) its own
     corner instead of sitting over the tab row's "More" control. */
  .rpanel.embedded .rpanel-head {
    padding-inline-end: 48px;
    min-height: 48px;
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
  .rpanel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 6px 8px 6px;
    border-bottom: 1px solid var(--border);
  }
  .rpanel-tabs {
    display: flex;
    gap: 2px;
    /* Narrow panel: tabs that don't fit fold into the "More" menu (measured
       in JS) — never a half-visible label cut at the edge. */
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }
  .rtab:global([data-rt-hidden]) {
    display: none;
  }
  .rtab-more {
    flex-shrink: 0;
    margin-inline-end: 4px;
  }
  .rpanel-head > :global(.icon-btn) {
    flex-shrink: 0;
  }
  .rtab {
    flex-shrink: 0;
    height: 24px;
    padding: 0 10px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-s);
    font-weight: 500;
    cursor: pointer;
    transition: background var(--dur-fast) ease-out, color var(--dur-fast) ease-out;
  }
  .rtab:hover {
    background: var(--hover);
  }
  .rtab.active {
    background: var(--surface-2);
    color: var(--text);
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
    padding-top: 10px;
    border-inline-start: 1px solid var(--border);
    background: var(--bg);
    flex-shrink: 0;
  }
  .strip-btn {
    width: 28px;
    height: 28px;
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
</style>
