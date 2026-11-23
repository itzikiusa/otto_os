<script module lang="ts">
  import type { BrowserCredential, BrowserTab, SimDevice } from '../../lib/api/types';
  import type { LiveNav } from './liveWebviews';
  import type { LiveViewState } from './live/protocol';

  export interface BrowserUrlBarLive {
    activeLive: BrowserTab | null;
    activeRemote: BrowserTab | null;
    remoteState: LiveViewState | null;
    remoteDown: boolean;
    liveCapable: boolean;
    pickMode: boolean;
    canAutofill: boolean;
    matchedCredential: BrowserCredential | null;
    matchedCredentialsCount?: number;
    filling: boolean;
  }

  export interface BrowserUrlBarTools {
    vaultSaving: boolean;
    emulationEnabled: boolean;
    inspectorOpen: boolean;
    mirroredSim: SimDevice | null;
    canShowMarksRail?: boolean;
    showMarksRail?: boolean;
  }

  export interface BrowserUrlBarActions {
    onUrlInput: (v: string) => void;
    onUrlFocus: (focused: boolean) => void;
    go: () => void;
    liveNav: (nav: LiveNav) => void;
    toggleMode: (mode: 'reader' | 'live') => void;
    chooseRenderer: (e: MouseEvent) => void;
    setGuard?: (on: boolean) => boolean;
    togglePick: () => void;
    autofill: (e: MouseEvent) => void;
    toggleDevice: () => void;
    toggleInspector: () => void;
    screenshot: () => void;
    simulators: () => void;
    summarize: () => void;
    vaultSave: (e: MouseEvent) => void;
    toggleMarksRail?: () => void;
  }
</script>

<script lang="ts">
  // Address bar & page tools for BrowserView. Keeps a single compact row on
  // phone and short landscape viewports by collapsing secondary actions into
  // a `⋯` overflow menu while keeping 1-click icon buttons on desktop.
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { nativeBrowserAvailable } from '../../lib/nativeBrowser';
  import { browser } from '../../lib/stores/browser.svelte';
  import { browserLive } from '../../lib/stores/browserLive.svelte';
  import { browserBookmarks } from './bookmarks.svelte';
  import SpaGuardToggle from './live/SpaGuardToggle.svelte';

  interface Props {
    urlInput: string;
    live: BrowserUrlBarLive;
    tools: BrowserUrlBarTools;
    actions: BrowserUrlBarActions;
  }

  let { urlInput, live, tools, actions }: Props = $props();

  let urlEl = $state<HTMLInputElement | null>(null);
  let marksBtnEl = $state<HTMLButtonElement | null>(null);
  const isBookmarked = $derived(browserBookmarks.has(browser.activeTab?.url));

  export function focusUrl(): void {
    urlEl?.focus();
    urlEl?.select();
  }

  export function focusMarksToggle(): void {
    marksBtnEl?.focus();
  }

  function onkeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') actions.go();
  }

  function openBookmarksMenu(e: MouseEvent): void {
    const tab = browser.activeTab;
    const items: MenuItem[] = [];
    if (tab?.url) {
      items.push({
        label: isBookmarked ? 'Remove bookmark for this page' : 'Bookmark this page',
        icon: 'star',
        action: () => {
          const added = browserBookmarks.toggle(tab.url, tab.title);
          toasts.info(added ? 'Bookmarked' : 'Removed bookmark', tab.title || tab.url);
        },
      });
    }
    for (const bm of browserBookmarks.items) {
      items.push({
        label: bm.title,
        icon: 'globe',
        action: () => {
          actions.onUrlInput(bm.url);
          actions.go();
        },
      });
    }
    if (items.length === 0) {
      toasts.info('No bookmarks yet', 'Open a page and click the star to save quick dev URLs.');
      return;
    }
    ctxMenu.show(e, items, { filter: items.length > 8, filterPlaceholder: 'Filter bookmarks…' });
  }

  function openCompactTools(e: MouseEvent): void {
    const tab = browser.activeTab;
    const items: MenuItem[] = [];
    if ((live.activeRemote && browserLive.ready) || live.activeLive) {
      const remote = live.activeRemote ? live.remoteState?.session : undefined;
      items.push(
        {
          label: 'Back',
          icon: 'chevronLeft',
          disabled: live.remoteDown || (!!live.activeRemote && !remote?.can_go_back),
          action: () => actions.liveNav('back'),
        },
        {
          label: 'Forward',
          icon: 'chevronRight',
          disabled: live.remoteDown || (!!live.activeRemote && !remote?.can_go_forward),
          action: () => actions.liveNav('forward'),
        },
      );
      if (live.activeLive) {
        items.push({
          label: 'Stop loading',
          icon: 'stop',
          action: () => actions.liveNav('stop'),
        });
      }
    }
    if (live.activeRemote && browserLive.ready) {
      items.push(
        {
          label: tools.emulationEnabled ? 'Exit device preview' : 'Device preview (iPhone / Android / iPad)',
          icon: 'phone',
          action: actions.toggleDevice,
        },
        {
          label: tools.inspectorOpen ? 'Hide inspector' : 'Page inspector (Console / Network / Element)',
          icon: 'bug',
          action: actions.toggleInspector,
        },
        {
          label: 'Capture screenshot (PNG)',
          icon: 'image',
          action: actions.screenshot,
        },
      );
    }
    items.push({
      label: tools.mirroredSim ? `Simulators (${tools.mirroredSim.name})` : 'iOS Simulators & Android Emulators…',
      icon: 'phone',
      action: actions.simulators,
    });
    if (tab?.url) {
      items.push(
        {
          label: isBookmarked ? 'Remove bookmark' : 'Bookmark this page',
          icon: 'star',
          action: () => {
            const added = browserBookmarks.toggle(tab.url, tab.title);
            toasts.info(added ? 'Bookmarked' : 'Removed bookmark', tab.title || tab.url);
          },
        },
        {
          label: browser.summarizing ? 'Summarizing…' : 'Summarize this page',
          icon: 'sparkle',
          disabled: browser.summarizing,
          action: actions.summarize,
        },
        {
          label: 'Save to vault',
          icon: 'book',
          disabled: tools.vaultSaving,
          action: () => actions.vaultSave(e),
        },
      );
      if (tools.canShowMarksRail && actions.toggleMarksRail) {
        items.push({
          label: tools.showMarksRail ? 'Hide marks' : `Show marks (${browser.annotations.length})`,
          icon: 'note',
          action: actions.toggleMarksRail,
        });
      }
    }
    for (const bm of browserBookmarks.items.slice(0, 6)) {
      items.push({
        label: `Open: ${bm.title}`,
        icon: 'globe',
        action: () => {
          actions.onUrlInput(bm.url);
          actions.go();
        },
      });
    }
    ctxMenu.show(e, items);
  }
</script>

<div class="urlbar">
  {#if (live.activeRemote && browserLive.ready) || live.activeLive}
    {@const remote = live.activeRemote ? live.remoteState?.session : undefined}
    <button class="icon-btn tool desktop-tool" onclick={() => actions.liveNav('back')} disabled={live.remoteDown || (!!live.activeRemote && !remote?.can_go_back)} aria-label="Back" title="Back">
      <Icon name="chevronLeft" size={14} />
    </button>
    <button class="icon-btn tool desktop-tool" onclick={() => actions.liveNav('forward')} disabled={live.remoteDown || (!!live.activeRemote && !remote?.can_go_forward)} aria-label="Forward" title="Forward">
      <Icon name="chevronRight" size={14} />
    </button>
    <button class="icon-btn tool" onclick={() => actions.liveNav('reload')} disabled={live.remoteDown} aria-label="Reload page" title="Reload page (⌘R)">
      <Icon name="refresh" size={14} />
    </button>
    {#if live.activeLive}
      <button class="icon-btn tool desktop-tool" onclick={() => actions.liveNav('stop')} aria-label="Stop loading" title="Stop loading">
        <Icon name="stop" size={14} />
      </button>
    {/if}
  {/if}
  <input
    type="text"
    dir="ltr"
    placeholder="e.g. https://example.com"
    aria-label="Address"
    enterkeyhint="go"
    inputmode="url"
    autocapitalize="off"
    spellcheck="false"
    bind:this={urlEl}
    value={urlInput}
    oninput={(e) => actions.onUrlInput((e.currentTarget as HTMLInputElement).value)}
    {onkeydown}
    onfocus={() => actions.onUrlFocus(true)}
    onblur={() => actions.onUrlFocus(false)}
  />
  <button class="icon-btn tool go-btn" onclick={actions.go} title="Go" aria-label="Go" disabled={!urlInput.trim()}>
    <Icon name="chevronRight" size={14} />
  </button>
  {#if browser.activeTab}
    <div class="mode-toggle" role="group" aria-label="Tab mode">
      <button
        class="seg"
        class:active={browser.activeTab.mode === 'reader'}
        onclick={() => actions.toggleMode('reader')}
        aria-label="Reader mode"
        aria-pressed={browser.activeTab.mode === 'reader'}
        title="Reader mode — fetched and rendered as clean markdown"
      >
        <Icon name="file" size={13} />
      </button>
      <button
        class="seg"
        class:active={live.liveCapable && browser.activeTab.mode === 'live'}
        onclick={() => actions.toggleMode('live')}
        aria-label={live.liveCapable ? 'Live mode' : 'Open in a new browser tab'}
        aria-pressed={live.liveCapable && browser.activeTab.mode === 'live'}
        title={live.liveCapable
          ? 'Live mode — a real browser'
          : 'Open this page in a new browser tab. This Otto daemon has no live browser.'}
      >
        <Icon name="globe" size={13} />
      </button>
    </div>
    {#if nativeBrowserAvailable && browser.activeTab.mode === 'live'}
      <button
        class="icon-btn tool"
        onclick={actions.chooseRenderer}
        aria-label="Live engine"
        title={browserLive.renderer === 'native'
          ? 'Live engine: this Mac’s web view'
          : 'Live engine: Otto’s Chromium'}
      >
        <Icon name="chevronDown" size={14} />
      </button>
    {/if}
  {/if}
  {#if live.activeRemote && browserLive.ready && actions.setGuard}
    <SpaGuardToggle tabId={live.activeRemote.id} session={live.remoteState?.session} disabled={live.remoteDown} onsetguard={actions.setGuard} />
  {/if}
  {#if live.activeLive}
    <button
      class="icon-btn tool"
      class:active={live.pickMode}
      onclick={actions.togglePick}
      title={live.pickMode ? 'Picking… click an element in the page' : 'Pick an element to mark'}
      aria-label="Pick an element to mark"
      aria-pressed={live.pickMode}
    >
      <Icon name="target" size={14} />
    </button>
  {/if}
  {#if live.canAutofill}
    <button
      class="icon-btn tool"
      onclick={actions.autofill}
      disabled={live.filling}
      aria-label="Autofill saved credentials"
      aria-haspopup={(live.matchedCredentialsCount ?? 1) > 1 ? 'menu' : undefined}
      title={(live.matchedCredentialsCount ?? 1) > 1
        ? `Autofill saved credentials (${live.matchedCredentialsCount} accounts)`
        : `Autofill ${live.matchedCredential?.username ?? ''} for ${live.matchedCredential?.domain ?? ''}`}
    >
      <Icon name="key" size={14} />
    </button>
  {/if}
  {#if browser.activeTab}
    {#if tools.canShowMarksRail && actions.toggleMarksRail}
      <button
        class="icon-btn tool desktop-tool marks-toggle"
        class:active={!!tools.showMarksRail}
        class:has-count={browser.annotations.length > 0}
        bind:this={marksBtnEl}
        onclick={actions.toggleMarksRail}
        aria-label={tools.showMarksRail ? 'Hide marks' : 'Show marks'}
        aria-pressed={!!tools.showMarksRail}
        title={tools.showMarksRail ? 'Hide marks' : `Show marks (${browser.annotations.length})`}
      >
        <Icon name="note" size={14} />
        {#if browser.annotations.length > 0}<span class="marks-count">{browser.annotations.length}</span>{/if}
      </button>
    {/if}
    {#if live.activeRemote && browserLive.ready}
      <button
        class="icon-btn tool desktop-tool"
        class:active={tools.emulationEnabled}
        onclick={actions.toggleDevice}
        aria-label="Toggle device preview"
        aria-pressed={tools.emulationEnabled}
        title={tools.emulationEnabled ? 'Exit device preview' : 'Device preview (iPhone, iPad, Android, Responsive)'}
      >
        <Icon name="phone" size={14} />
      </button>
      <button
        class="icon-btn tool desktop-tool"
        class:active={tools.inspectorOpen}
        onclick={actions.toggleInspector}
        aria-label="Toggle live page inspector"
        aria-pressed={tools.inspectorOpen}
        title={tools.inspectorOpen ? 'Hide page inspector' : 'Page inspector (Console, Network, Element)'}
      >
        <Icon name="bug" size={14} />
      </button>
    {/if}
    <button
      class="icon-btn tool desktop-tool"
      class:active={isBookmarked}
      onclick={openBookmarksMenu}
      aria-label="Bookmarks"
      title={isBookmarked ? 'Bookmarks (page saved)' : 'Bookmarks & quick URLs'}
    >
      <Icon name="star" size={14} />
    </button>
    <button
      class="icon-btn tool desktop-tool"
      onclick={actions.summarize}
      disabled={browser.summarizing}
      aria-label="Summarize this page"
      title={browser.summarizing ? 'Summarizing…' : 'Summarize this page'}
    >
      <Icon name="sparkle" size={14} />
    </button>
    <button
      class="icon-btn tool desktop-tool"
      onclick={actions.vaultSave}
      disabled={tools.vaultSaving}
      aria-label="Save to vault"
      title={tools.vaultSaving ? 'Saving to vault…' : 'Save to vault'}
    >
      <Icon name="book" size={14} />
    </button>
  {:else if browserBookmarks.items.length > 0}
    <button
      class="icon-btn tool desktop-tool"
      onclick={openBookmarksMenu}
      aria-label="Bookmarks"
      title="Bookmarks & quick URLs"
    >
      <Icon name="star" size={14} />
    </button>
  {/if}
  <button
    class="icon-btn tool compact-more"
    onclick={openCompactTools}
    aria-label="More browser tools"
    aria-haspopup="menu"
    title="More browser tools"
  >
    <Icon name="more" size={14} />
  </button>
</div>

<style>
  .urlbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    min-width: 0;
  }
  .urlbar input {
    flex: 1;
    min-width: 0;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 10px;
    font: inherit;
    font-size: var(--fs-m);
  }
  .mode-toggle {
    display: flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
    flex-shrink: 0;
  }
  .seg {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 30px;
    border: none;
    background: var(--surface);
    color: var(--text-dim);
    cursor: pointer;
  }
  .seg + .seg {
    border-inline-start: 1px solid var(--border);
  }
  .seg:hover {
    color: var(--text);
  }
  .seg.active {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .tool {
    flex: none;
    width: 30px;
    height: 30px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
  }
  .tool:disabled {
    opacity: var(--disabled-opacity);
    cursor: not-allowed;
  }
  .tool.active {
    background: var(--accent-soft);
    color: var(--accent-text);
    border-color: var(--accent);
  }
  .marks-toggle.has-count {
    width: auto;
    padding-inline: 8px;
    gap: 4px;
  }
  .marks-count {
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .compact-more {
    display: none;
  }
  @media (max-width: 640px), (max-height: 500px) {
    .urlbar {
      padding: 6px 8px;
      gap: 4px;
    }
    .desktop-tool,
    .go-btn {
      display: none;
    }
    .compact-more {
      display: flex;
    }
    .tool {
      width: 36px;
      height: 36px;
    }
  }
  @container (max-width: 560px) {
    .desktop-tool { display: none; }
    .compact-more { display: flex; }
  }
  @media (pointer: coarse) {
    .urlbar input {
      font-size: 16px; /* ui-guards: allow — iOS Safari no-zoom input floor */
    }
  }
  @media (pointer: coarse), (max-width: 640px) {
    .seg { min-width: 36px; min-height: 36px; }
  }
</style>
