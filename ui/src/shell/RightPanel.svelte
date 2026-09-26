<script lang="ts">
  // Collapsible right panel (⌘J): Git / Files / Notes / Activity / Outputs / Canvas / Info / Browser / API tabs ⇄ 36px icon strip.
  import Icon, { type IconName } from '../lib/components/Icon.svelte';
  import EmptyState from '../lib/components/EmptyState.svelte';
  import GitPanel from '../modules/git/GitPanel.svelte';
  import InfoPanel from '../modules/panels/InfoPanel.svelte';
  import BrowserPanel from '../modules/panels/BrowserPanel.svelte';
  import BrowserPanelV2 from '../modules/panels/BrowserPanelV2.svelte';
  import FilesPanel from '../modules/panels/FilesPanel.svelte';
  import ActivityPanel from '../modules/panels/ActivityPanel.svelte';
  import OutputsPanel from '../modules/panels/OutputsPanel.svelte';
  import CanvasPanel from '../modules/panels/CanvasPanel.svelte';
  import ApiPanel from '../modules/api/ApiPanel.svelte';
  import { ui, type RightTab } from '../lib/stores/ui.svelte';
  import { ws } from '../lib/stores/workspace.svelte';
  import { getToken } from '../lib/api/client';
  import { toasts } from '../lib/toast.svelte';
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
  function startResize(e: MouseEvent): void {
    e.preventDefault();
    resizing = true;
    const startX = e.clientX;
    const startW = ui.rightWidth;
    const onMove = (ev: MouseEvent) => ui.setRightWidth(startW + (startX - ev.clientX));
    const onUp = () => {
      resizing = false;
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
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

  const tabs: { id: RightTab; icon: IconName; label: string }[] = [
    { id: 'git', icon: 'branch', label: 'Git' },
    { id: 'files', icon: 'file', label: 'Files' },
    { id: 'notes', icon: 'note', label: 'Notes' },
    { id: 'activity', icon: 'zap', label: 'Activity' },
    // Outputs — artifacts the focused agent produced, with sandboxed previews
    // (docs/design/conversation-view.md §5.6). Gated like the rest on an
    // active agent session by the shell.
    { id: 'outputs', icon: 'layers', label: 'Outputs' },
    { id: 'canvas', icon: 'shapes', label: 'Canvas' },
    { id: 'info', icon: 'info', label: 'Info' },
    { id: 'browser', icon: 'globe', label: 'Browser' },
    { id: 'api', icon: 'send', label: 'API' },
  ];

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
        toasts.error('Notes not saved', e instanceof Error ? e.message : String(e));
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

{#if open || browserKept}
  <aside
    class="rpanel"
    class:resizing
    class:embedded={forceOpen}
    hidden={!open}
    style={forceOpen ? undefined : `width:${ui.rightWidth}px`}
  >
    {#if !forceOpen}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="resize-handle"
        onmousedown={startResize}
        ondblclick={() => ui.setRightWidth(300)}
        title="Drag to resize · double-click to reset"
      ></div>
    {/if}
    <header class="rpanel-head">
      <div class="rpanel-tabs" role="tablist" tabindex="-1" aria-label="Session panel" bind:this={tabsEl} onkeydown={onTabsKey}>
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
        title="Collapse panel (⌘J)"
        aria-label="Collapse panel"
      >
        <Icon name="panel" size={13} />
      </button>
    </header>

    <div class="rpanel-body">
      {#if ui.rightTab === 'git'}
        <GitPanel />
      {:else if ui.rightTab === 'files'}
        <FilesPanel />
      {:else if ui.rightTab === 'activity'}
        <ActivityPanel />
      {:else if ui.rightTab === 'outputs'}
        <OutputsPanel />
      {:else if ui.rightTab === 'canvas'}
        <CanvasPanel />
      {:else if ui.rightTab === 'info'}
        <InfoPanel />
      {/if}
      {#if browserShown || (browserKept && ui.browserPanelVersion === 'v1')}
        <!-- Transitional v1/v2 switch: v1 is the original per-session panel,
             v2 embeds the Browser module (persisted tabs/marks + ask bar).
             Only here, in agent mode — the Browser page itself is always v2.
             v1 stays mounted while hidden (see `browserKept`). -->
        <div class="browser-host" hidden={!browserShown}>
          <div class="browser-ver" role="group" aria-label="Browser version">
            <span class="dim">Browser</span>
            <button
              class="ver"
              class:active={ui.browserPanelVersion === 'v1'}
              aria-pressed={ui.browserPanelVersion === 'v1'}
              onclick={() => ui.setBrowserPanelVersion('v1')}
              title="v1 — per-session browser: native tabs + take-over picker"
            >v1</button>
            <button
              class="ver"
              class:active={ui.browserPanelVersion === 'v2'}
              aria-pressed={ui.browserPanelVersion === 'v2'}
              onclick={() => ui.setBrowserPanelVersion('v2')}
              title="v2 — the Browser module: reader/live tabs, saved marks, ask the agent"
            >v2</button>
          </div>
          {#if ui.browserPanelVersion === 'v2'}
            <BrowserPanelV2 />
          {:else}
            <BrowserPanel active={browserShown} />
          {/if}
        </div>
      {/if}
      {#if ui.rightTab === 'api'}
        <ApiPanel />
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
  <aside class="rstrip" aria-label="Session panel">
    {#each tabs as t (t.id)}
      <button
        class="icon-btn strip-btn"
        onclick={() => ui.openRight(t.id)}
        title={t.label}
        aria-label={t.label}
      >
        <Icon name={t.icon} size={15} />
      </button>
    {/each}
  </aside>
{/if}

<style>
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
      color-mix(in srgb, var(--accent) 40%, transparent) 45%,
      color-mix(in srgb, var(--accent) 40%, transparent) 55%,
      transparent 100%
    );
  }
  .rpanel-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 7px 8px 6px;
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
    transition: background 120ms ease-out, color 120ms ease-out;
  }
  .rtab:hover {
    background: var(--surface-2);
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
  .browser-host > :global(:not(.browser-ver)) {
    flex: 1;
    min-height: 0;
  }
  .browser-ver {
    display: flex;
    align-items: center;
    gap: 0.3rem;
    padding: 0.25rem 0.6rem;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-xs);
  }
  .browser-ver .dim {
    color: var(--text-dim);
    margin-inline-end: auto;
  }
  .ver {
    height: 20px;
    padding: 0 0.5rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .ver.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent-text);
    border-color: var(--accent);
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
