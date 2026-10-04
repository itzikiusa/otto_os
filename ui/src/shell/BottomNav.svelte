<script lang="ts">
  // Phone bottom navigation bar. Shows the first few modules the current user
  // can view as large tap targets, plus a "More" affordance that opens an
  // overflow sheet for the rest (and Settings). Tapping a module routes via
  // router.go(); it draws from the same shared sidebar registry as the Rail and
  // Navigator and honours the user's saved order + hidden set, so all three
  // navigations stay in lockstep.
  //
  // Rendered only on phone (App.svelte gates it behind viewport.isPhone), so it
  // adds nothing to the desktop layout.
  import { untrack } from 'svelte';
  import Icon from '../lib/components/Icon.svelte';
  import { dialogFocus } from '../lib/dialogFocus';
  import { router } from '../lib/router.svelte';
  import { navPending } from '../lib/navPending.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { navBadge } from '../lib/navBadge';
  import { auth } from '../lib/stores/auth.svelte';
  import { plugins } from '../lib/stores/plugins.svelte';
  import {
    availableModules,
    activeNavId,
    resolveOrder,
    sidebarSections,
    visibleOrder,
  } from '../lib/sidebar';

  // How many primary tabs sit on the bar before everything spills into "More".
  const PRIMARY_COUNT = 4;

  const pluginEntries = $derived(
    plugins.list
      .filter((p) => auth.canPlugin(p.slug, 'view'))
      .map((p) => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name })),
  );
  // Flattened section by section, so the bar's order matches the sidebar's —
  // Favorites first, so a user's favorites become the phone's primary tabs.
  const modules = $derived(
    sidebarSections(
      visibleOrder(
        resolveOrder(
          availableModules((f) => auth.can(f, 'view'), pluginEntries),
          ui.sidebarOrder,
        ),
        ui.sidebarHidden,
      ),
      ui.sidebarFavorites,
      ui.sidebarGroupOrder,
    ).flatMap((s) => s.modules),
  );
  // The entry the current route highlights (plugin slug, '' → Agents, …).
  const current = $derived(activeNavId(router.parts));
  const primary = $derived(modules.slice(0, PRIMARY_COUNT));
  const overflow = $derived(modules.slice(PRIMARY_COUNT));

  // Badges on spilled modules (a session waiting on you must not hide in More).
  const moreBadge = $derived.by(() => {
    let count = 0;
    let needs = false;
    for (const m of overflow) {
      const b = navBadge(m.id);
      if (!b) continue;
      count += b.count;
      needs ||= b.tone === 'needs';
    }
    return count > 0 ? { count, needs } : null;
  });

  let moreOpen = $state(false);

  // The sheet is a modal dialog: register it as an open overlay (native webview
  // hides, global shortcuts stand down) exactly like Modal/Drawer do. untrack:
  // pushModal reads modalCount, so the effect may depend only on `moreOpen`.
  $effect(() => {
    if (!moreOpen) return;
    untrack(() => ui.pushModal());
    return () => untrack(() => ui.popModal());
  });

  /** Focus, Esc and Tab-trapping for the sheet (lib/dialogFocus). */
  function sheetFocus(node: HTMLElement) {
    return dialogFocus(node, () => (moreOpen = false));
  }

  function go(id: string): void {
    router.openModule(id);
    moreOpen = false;
  }

  // "More" is active when the current module lives in the overflow set (or
  // Settings), so the bar reflects where you are even for spilled modules.
  const moreActive = $derived(
    router.module === 'settings' || overflow.some((m) => m.id === current),
  );
</script>

<nav class="bottomnav" aria-label="Primary">
  {#each primary as m (m.id)}
    <button class="bn-btn" class:active={current === m.id} aria-current={current === m.id ? 'page' : undefined} data-nav-id={m.id} aria-busy={navPending.id === m.id || undefined} aria-label="{m.label}{navBadge(m.id)?.spoken ?? ''}" onclick={() => go(m.id)}>
      <span class="bn-icon">
        <Icon name={m.icon} size={20} />
        {#if navBadge(m.id)}
          {@const b = navBadge(m.id)!}
          <span class="bn-badge" class:needs={b.tone === 'needs'} title={b.title}>{b.count}</span>
        {/if}
      </span>
      <span class="bn-label">{m.label}</span>
    </button>
  {/each}

  <!-- Always present: besides the spilled modules it holds Commands (the
       phone's only palette entry off the Agents page) and Settings. -->
  <button class="bn-btn" class:active={moreActive} aria-haspopup="dialog" aria-expanded={moreOpen} aria-label={moreBadge ? `More, ${moreBadge.count} ${moreBadge.needs ? 'need you' : 'active'}` : 'More'} onclick={() => (moreOpen = true)}>
    <span class="bn-icon">
      <Icon name="more" size={20} />
      {#if moreBadge}<span class="bn-badge" class:needs={moreBadge.needs}>{moreBadge.count}</span>{/if}
    </span>
    <span class="bn-label">More</span>
  </button>
</nav>

{#if moreOpen}
  <!-- Overflow sheet: the remaining modules + Settings as a bottom sheet. -->
  <div class="sheet-backdrop" role="presentation" onclick={() => (moreOpen = false)}></div>
  <div class="more-sheet" role="dialog" aria-modal="true" aria-label="More modules" use:sheetFocus>
    <div class="sheet-header">
      <div class="sheet-grip"></div>
      <button class="icon-btn sheet-close" onclick={() => (moreOpen = false)} aria-label="Close" title="Close (Esc)">
        <Icon name="x" size={14} />
      </button>
    </div>
    <div class="sheet-grid">
      <button
        class="sheet-item"
        onclick={() => {
          moreOpen = false;
          ui.openPalette('commands');
        }}
      >
        <Icon name="command" size={22} />
        <span>Commands</span>
      </button>
      {#each overflow as m (m.id)}
        {@const b = navBadge(m.id)}
        <button class="sheet-item" class:active={current === m.id} aria-current={current === m.id ? 'page' : undefined} data-nav-id={m.id} aria-busy={navPending.id === m.id || undefined} aria-label="{m.label}{b?.spoken ?? ''}" onclick={() => go(m.id)}>
          <Icon name={m.icon} size={22} />
          <span>{m.label}</span>
          {#if b}<span class="bn-badge sheet-badge" class:needs={b.tone === 'needs'} aria-hidden="true">{b.count}</span>{/if}
        </button>
      {/each}
      <button
        class="sheet-item"
        class:active={router.module === 'walkthroughs'}
        aria-current={router.module === 'walkthroughs' ? 'page' : undefined}
        onclick={() => {
          router.go('walkthroughs');
          moreOpen = false;
        }}
      >
        <Icon name="info" size={22} />
        <span>Help</span>
      </button>
      <button
        class="sheet-item"
        class:active={router.module === 'settings'}
        aria-current={router.module === 'settings' ? 'page' : undefined}
        onclick={() => {
          router.go('settings/appearance');
          moreOpen = false;
        }}
      >
        <Icon name="gear" size={22} />
        <span>Settings</span>
      </button>
    </div>
  </div>
{/if}

<style>
  .bottomnav {
    display: flex;
    align-items: stretch;
    height: var(--mobile-bottomnav-h);
    flex-shrink: 0;
    border-top: 1px solid var(--border);
    background: var(--bg-sidebar);
    /* iOS home-indicator safe area. */
    padding-bottom: env(safe-area-inset-bottom, 0);
    z-index: var(--z-mobile-nav);
  }
  .bn-btn {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 2px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 4px 2px;
    min-width: 0;
  }
  .bn-btn.active {
    color: var(--accent-text);
  }
  .bn-icon {
    position: relative;
    display: grid;
    place-items: center;
    height: 22px;
  }
  .bn-label {
    font-size: var(--fs-xs);
    font-weight: 500;
    line-height: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .bn-badge {
    position: absolute;
    top: -4px;
    inset-inline-end: -8px;
    min-width: 15px;
    height: 15px;
    padding: 0 3px;
    border-radius: 999px;
    /* Count-chip language (tint + semantic text), opaque over the bar —
       white on the bright working-green was ~2:1. */
    background: color-mix(in srgb, var(--success) 24%, var(--bg-sidebar));
    color: var(--success);
    font-size: var(--fs-xs);
    font-weight: 600;
    display: grid;
    place-items: center;
  }
  .bn-badge.needs {
    background: color-mix(in srgb, var(--warning) 24%, var(--bg-sidebar));
    color: var(--warning);
  }

  .sheet-badge {
    top: 6px;
    inset-inline-end: 8px;
    background: color-mix(in srgb, var(--success) 24%, var(--surface));
  }
  .sheet-badge.needs {
    background: color-mix(in srgb, var(--warning) 24%, var(--surface));
  }
  .sheet-backdrop {
    position: fixed;
    inset: 0;
    background: var(--scrim);
    z-index: calc(var(--z-drawer) + 2);
  }
  .more-sheet {
    position: fixed;
    inset-inline-start: 0;
    inset-inline-end: 0;
    bottom: 0;
    z-index: calc(var(--z-drawer) + 3);
    background: var(--bg);
    border-top: 1px solid var(--border);
    border-radius: 14px 14px 0 0;
    box-shadow: var(--shadow);
    padding: 8px 12px calc(16px + env(safe-area-inset-bottom, 0));
    /* The grid is data-driven (all overflow modules + every installed plugin):
       cap the sheet so it never grows past the top edge and scroll inside. */
    max-height: calc(100% - 48px); /* % of the window — vh is the screen's in WKWebView */
    display: flex;
    flex-direction: column;
  }
  .sheet-header {
    position: relative;
    flex-shrink: 0;
  }
  .sheet-grip {
    width: 36px;
    height: 4px;
    border-radius: 999px;
    background: var(--text-dim);
    opacity: 0.4;
    margin: 4px auto 12px;
  }
  .sheet-close {
    position: absolute;
    inset-block-start: -2px;
    inset-inline-end: 0;
  }
  .sheet-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 8px;
    overflow-y: auto;
    overscroll-behavior: contain;
    -webkit-overflow-scrolling: touch;
  }
  .sheet-item {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 12px 4px;
    border: none;
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .sheet-item.active {
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent) 12%, var(--surface));
  }
  .sheet-item span {
    line-height: 1;
    text-align: center;
  }
</style>
