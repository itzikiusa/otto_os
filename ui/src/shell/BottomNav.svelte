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
  import Icon from '../lib/components/Icon.svelte';
  import { router } from '../lib/router.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { ws } from '../lib/stores/workspace.svelte';
  import { assistant } from '../lib/stores/assistant.svelte';
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

  let moreOpen = $state(false);

  function go(id: string): void {
    router.go(id);
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
    <button class="bn-btn" class:active={current === m.id} onclick={() => go(m.id)}>
      <span class="bn-icon">
        <Icon name={m.icon} size={20} />
        {#if m.id === 'agents' && ws.workingCount > 0}
          <span class="bn-badge">{ws.workingCount}</span>
        {/if}
        {#if m.id === 'assistant' && assistant.needsYouCount > 0}
          <span class="bn-badge needs">{assistant.needsYouCount}</span>
        {/if}
      </span>
      <span class="bn-label">{m.label}</span>
    </button>
  {/each}

  <!-- Always present: besides the spilled modules it holds Commands (the
       phone's only palette entry off the Agents page) and Settings. -->
  <button class="bn-btn" class:active={moreActive} onclick={() => (moreOpen = true)}>
    <span class="bn-icon"><Icon name="command" size={20} /></span>
    <span class="bn-label">More</span>
  </button>
</nav>

{#if moreOpen}
  <!-- Overflow sheet: the remaining modules + Settings as a bottom sheet. -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="sheet-backdrop" onclick={() => (moreOpen = false)}></div>
  <div class="more-sheet" role="dialog" aria-modal="true" aria-label="More modules">
    <div class="sheet-grip"></div>
    <div class="sheet-grid">
      <button
        class="sheet-item"
        onclick={() => {
          moreOpen = false;
          ui.openPalette('commands');
        }}
      >
        <Icon name="search" size={22} />
        <span>Commands</span>
      </button>
      {#each overflow as m (m.id)}
        <button class="sheet-item" class:active={current === m.id} onclick={() => go(m.id)}>
          <Icon name={m.icon} size={22} />
          <span>{m.label}</span>
        </button>
      {/each}
      <button
        class="sheet-item"
        class:active={router.module === 'settings'}
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
    min-width: 14px;
    height: 14px;
    padding: 0 3px;
    border-radius: 999px;
    background: var(--status-working);
    color: #fff;
    font-size: 9px;
    font-weight: 700;
    display: grid;
    place-items: center;
  }
  .bn-badge.needs {
    background: var(--warning-soft);
    color: var(--warning);
  }

  .sheet-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
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
  .sheet-grip {
    width: 36px;
    height: 4px;
    border-radius: 999px;
    background: var(--text-dim);
    opacity: 0.4;
    margin: 4px auto 12px;
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
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 12px 4px;
    border: none;
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text-dim);
    font-size: 11px;
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
