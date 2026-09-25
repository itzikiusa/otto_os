<script lang="ts">
  // Collapsed 44px icon rail (⌘1 expands to the Navigator).
  import Icon from '../lib/components/Icon.svelte';
  import NotificationBell from './NotificationBell.svelte';
  import { router } from '../lib/router.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { ws } from '../lib/stores/workspace.svelte';
  import { assistant } from '../lib/stores/assistant.svelte';
  import { auth } from '../lib/stores/auth.svelte';
  import { plugins } from '../lib/stores/plugins.svelte';
  import { ctxMenu, type MenuItem } from '../lib/contextmenu.svelte';
  import { sidePane, splitMenuItems, navClick, SPLIT_HINT } from '../lib/stores/sidePane.svelte';
  import { tick } from 'svelte';
  import {
    activeNavId,
    availableModules,
    resolveOrder,
    sidebarSections,
    visibleOrder,
  } from '../lib/sidebar';

  // The collapsed rail mirrors the same resolved module list as the Navigator
  // (shared registry → RBAC filter + plugins → user's saved order, minus hidden
  // ones), section by section with a thin separator between sections (no
  // headers or folding — the Navigator owns those): Favorites first, then the
  // rest in the user's section order. It's read-only here: reordering /
  // show-hide / favoriting happens in the expanded Navigator (and Settings →
  // Appearance). See ui.svelte.ts for persistence.
  const pluginEntries = $derived(
    plugins.list
      .filter((p) => auth.canPlugin(p.slug, 'view'))
      .map((p) => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name })),
  );
  const sections = $derived(
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
    ),
  );

  // The module column scrolls when the window is short; keep the active icon
  // in view on every route change (same rule as the Navigator).
  let listEl = $state<HTMLDivElement>();
  $effect(() => {
    void router.parts.join('/');
    const el = listEl;
    if (!el) return;
    void tick().then(() => el.querySelector<HTMLElement>('.rail-btn.active')?.scrollIntoView({ block: 'nearest' }));
  });

  // Active when the route highlights the entry (plugin slug, default route →
  // Agents, database/brokers → Connections: see activeNavId).
  const activeId = $derived(activeNavId(router.parts));
  function isActive(id: string): boolean {
    return activeId === id;
  }

  // Same row menu as the Navigator (minus in-section moves): the side-by-side
  // entries, Favorites, and a way into Customize — which lives in the
  // expanded Navigator, so it expands the sidebar first.
  function moduleMenu(e: MouseEvent, id: string, label: string): void {
    const fav = ui.sidebarFavorites.includes(id);
    const split = splitMenuItems(id, label);
    const items: MenuItem[] = [
      ...split,
      ...(split.length ? [{ separator: true }] : []),
      fav
        ? { label: 'Remove from Favorites', icon: 'star', action: () => ui.removeSidebarFavorite(id) }
        : { label: 'Add to Favorites', icon: 'star', action: () => ui.addSidebarFavorite(id) },
      { separator: true },
      {
        label: 'Customize sidebar',
        icon: 'edit',
        action: () => {
          if (!ui.railExpanded) ui.toggleRail();
          ui.sidebarEditMode = true;
        },
      },
    ];
    ctxMenu.show(e, items);
  }

  // The account avatar was a button that did nothing — and the collapsed
  // rail had no way to sign out (only the expanded Navigator's footer did).
  function accountMenu(e: MouseEvent): void {
    const name = auth.me?.display_name ?? 'Account';
    const sub = auth.isRoot ? 'root' : auth.me?.username;
    ctxMenu.showAt(e.currentTarget as HTMLElement, [
      { label: sub && sub !== name ? `${name} (${sub})` : name, icon: 'user', disabled: true },
      { separator: true },
      { label: 'Help', icon: 'info', action: () => router.go('walkthroughs') },
      { label: 'Settings', icon: 'gear', action: () => router.go('settings/appearance') },
      { separator: true },
      { label: 'Sign out', icon: 'logout', action: () => auth.logout() },
    ]);
  }

  /** The badge, spoken: ", 2 waiting on you" / ", 3 working" (the badge
   *  itself is only a title for pointer users). */
  function railCount(id: string): string {
    if (id === 'agents' && ws.needsYouCount > 0) return `, ${ws.needsYouCount} waiting on you`;
    if (id === 'agents' && ws.workingCount > 0) return `, ${ws.workingCount} working`;
    if (id === 'assistant' && assistant.needsYouCount > 0) return `, ${assistant.needsYouCount} waiting on you`;
    return '';
  }
</script>

<nav class="rail sidebar-material" aria-label="Modules">
  <button
    class="rail-btn"
    onclick={() => ui.toggleRail()}
    title="Expand sidebar (⌘1)"
    aria-label="Expand sidebar"
  >
    <Icon name="sidebar" />
  </button>
  <NotificationBell />

  <div class="rail-modules" bind:this={listEl}>
    {#each sections as sec, si (sec.group.id)}
      {#if si > 0}
        <div class="rail-sep" role="separator" aria-label={sec.group.label} data-testid="rail-sep"></div>
      {/if}
      {#each sec.modules as m (m.id)}
        {@const inSide = sidePane.showing && sidePane.key === m.id}
        <button
          class="rail-btn"
          class:active={isActive(m.id)}
          class:side={inSide}
          onclick={(e) => navClick(e, m.id, m.label)}
          oncontextmenu={(e) => moduleMenu(e, m.id, m.label)}
          title={`${m.label} · ${sec.group.label}${inSide ? ' · in the side pane' : sidePane.supported ? ` — ${SPLIT_HINT}` : ''}`}
          aria-label={`${m.label}${inSide ? ' (in the side pane)' : ''}${railCount(m.id)}`}
          data-testid={`rail-${m.id}`}
        >
          <Icon name={m.icon} />
          <!-- A session waiting on you outranks "working": the expanded sidebar
               shows it as the Needs-you pill, the rail as a warning badge. -->
          {#if m.id === 'agents' && ws.needsYouCount > 0}
            <span class="rail-badge needs" title={`${ws.needsYouCount} waiting on you`}>{ws.needsYouCount}</span>
          {:else if m.id === 'agents' && ws.workingCount > 0}
            <span class="rail-badge" title={`${ws.workingCount} working`}>{ws.workingCount}</span>
          {/if}
          {#if m.id === 'assistant' && assistant.needsYouCount > 0}
            <span class="rail-badge needs" title={`${assistant.needsYouCount} waiting on you`}>{assistant.needsYouCount}</span>
          {/if}
        </button>
      {/each}
    {/each}
  </div>

  <div class="rail-bottom">
    <button
      class="rail-btn"
      class:active={router.module === 'settings'}
      onclick={() => router.go('settings/appearance')}
      title="Settings"
      aria-label="Settings"
    >
      <Icon name="gear" />
    </button>
    <button
      class="rail-btn user"
      onclick={accountMenu}
      title={auth.me?.display_name ?? 'Account'}
      aria-label="Account"
      aria-haspopup="menu"
    >
      <span class="avatar">{(auth.me?.display_name ?? '?').slice(0, 1).toUpperCase()}</span>
    </button>
  </div>
</nav>

<style>
  .rail {
    width: 44px;
    height: 100%;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 10px 0;
    gap: 4px;
    border-inline-end: 1px solid var(--separator);
  }
  /* Scrolls (scrollbar hidden) when the window is shorter than the icon
     column, so the bottom Settings/account buttons are never pushed under the
     status bar. */
  .rail-modules {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    margin-top: 10px;
    flex: 1;
    min-height: 0;
    width: 100%;
    overflow-y: auto;
    scrollbar-width: none;
    /* A scroller clips: room for the first/last icon's focus ring and the
       count badge that overhangs the icon's top edge. */
    padding-block: 3px;
  }
  .rail-modules::-webkit-scrollbar {
    display: none;
  }
  .rail-sep {
    flex-shrink: 0;
    width: 18px;
    height: 1px;
    margin: 4px 0;
    background: var(--border);
  }
  .rail-bottom {
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: center;
    padding-top: 6px;
    border-top: 1px solid var(--border);
    width: 26px;
  }
  .rail-btn {
    position: relative;
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    color: var(--text-dim);
    cursor: pointer;
    flex-shrink: 0;
    transition: background 130ms ease-out, color 130ms ease-out;
  }
  .rail-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text);
  }
  /* Same selection language as the Navigator: accent tint + accent glyph + a
     short accent bar at the rail's inline-start edge. */
  .rail-btn.active {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  /* The module shown in the side-by-side pane: open, but not the page — a
     hairline ring instead of the selection tint. */
  .rail-btn.side {
    color: var(--text);
    box-shadow: inset 0 0 0 1px var(--border-strong);
  }
  .rail-modules .rail-btn.active::before {
    content: '';
    position: absolute;
    inset-inline-start: -7px;
    inset-block: 7px;
    width: 3px;
    border-radius: 0 2px 2px 0;
    background: var(--accent);
  }
  :global([dir='rtl']) .rail-modules .rail-btn.active::before {
    border-radius: 2px 0 0 2px;
  }
  .rail-badge {
    position: absolute;
    top: -2px;
    inset-inline-end: -3px;
    min-width: 15px;
    height: 15px;
    padding: 0 3px;
    border-radius: 999px;
    /* The Navigator's count-chip language (tinted fill, semantic text), made
       opaque over the sidebar so the icon under the badge doesn't show
       through. White on the bright working-green was ~2:1. */
    background: color-mix(in srgb, var(--success) 24%, var(--bg-sidebar));
    color: var(--success);
    font-size: var(--fs-xs);
    font-weight: 700;
    line-height: 1;
    display: grid;
    place-items: center;
  }
  .rail-badge.needs {
    background: color-mix(in srgb, var(--warning) 24%, var(--bg-sidebar));
    color: var(--warning);
  }
  .avatar {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--accent) 28%, transparent);
    color: var(--accent-text);
    font-size: var(--fs-xs);
    font-weight: 600;
    display: grid;
    place-items: center;
  }
</style>
