<script lang="ts">
  // Phone bottom navigation bar. Shows the first few modules the current user
  // can view as large tap targets, plus the "Menu" tab that opens the ONE phone
  // drawer (PhoneNavDrawer: the Navigator with every module, workspaces,
  // Commands, Light / Full, Machine, Help, Settings, account — design.md §7.2). Tapping a module routes
  // via router.go(); it draws from the same shared sidebar registry as the Rail and
  // Navigator and honours the user's saved order + hidden set, so all three
  // navigations stay in lockstep.
  //
  // Rendered only on phone (App.svelte gates it behind viewport.isPhone), so it
  // adds nothing to the desktop layout.
  //
  // The light version (lib/lightMode.svelte.ts) shows its first five modules
  // as tabs, in a fixed order (the sixth, Usage, in the drawer), and narrows
  // "Menu" to its glyph.
  import Icon from '../lib/components/Icon.svelte';
  import { notifications } from '../lib/stores/notifications.svelte';
  import { plural } from '../lib/plural';
  import { LIGHT_BAR_TABS, LIGHT_MODULES, lightMode } from '../lib/lightMode.svelte';
  import { router } from '../lib/router.svelte';
  import { navPending } from '../lib/navPending.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { navBadge } from '../lib/navBadge';
  import { navUsage } from '../lib/stores/navUsage.svelte';
  import { barOrder } from '../lib/sidebarMore';
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
  const light = $derived(lightMode.active);
  // Flattened section by section, so the bar's order matches the sidebar's —
  // Favorites first, so a user's favorites become the phone's primary tabs.
  // The light version is a fixed set instead (still role-filtered: the
  // registry's own gate, inside the light scope), never the saved order.
  const modules = $derived.by(() => {
    const available = availableModules((f) => auth.can(f, 'view'), light ? [] : pluginEntries);
    if (light) return LIGHT_MODULES.flatMap((id) => available.filter((m) => m.id === id));
    // What he uses most first (sidebarMore.ts `barOrder`: pinned, then by use), so the tabs are his; the rest in More.
    const split = navUsage.split(
      sidebarSections(visibleOrder(resolveOrder(available, ui.sidebarOrder), ui.sidebarHidden), ui.sidebarFavorites, ui.sidebarGroupOrder),
      current,
    );
    return barOrder([...split.sections.flatMap((s) => s.modules), ...split.more], ui.sidebarFavorites, navUsage.used);
  });
  // The entry the current route highlights (plugin slug, '' → Agents, …).
  const current = $derived(activeNavId(router.parts));
  const primary = $derived(modules.slice(0, light ? LIGHT_BAR_TABS : PRIMARY_COUNT));
  const overflow = $derived(modules.slice(light ? LIGHT_BAR_TABS : PRIMARY_COUNT));
  // The bell lives at the top of the drawer; this tab carries its unread dot.
  const unread = $derived(notifications.unreadRows);

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

  function go(id: string): void {
    router.openModule(id);
  }

  // "Menu" is the active tab while the drawer is open, or when the current
  // module has no tab of its own (a spilled module, Settings, Help).
  const menuActive = $derived(ui.navDrawerOpen || !primary.some((m) => m.id === current));
  const menuLabel = $derived(
    moreBadge
      ? `Menu, ${moreBadge.count} ${moreBadge.needs ? 'need you' : 'active'}`
      : unread > 0
        ? `Menu, ${plural(unread, 'unread notification')}`
        : 'Menu',
  );
</script>

<nav class="bottomnav" aria-label="Primary">
  {#each primary as m (m.id)}
    <button class="bn-btn" class:active={current === m.id && !ui.navDrawerOpen} aria-current={current === m.id ? 'page' : undefined} data-nav-id={m.id} aria-busy={navPending.id === m.id || undefined} aria-label="{m.label}{navBadge(m.id)?.spoken ?? ''}" onclick={() => go(m.id)}>
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

  <!-- Always present: the drawer holds every other module, the workspaces,
       Commands (the phone's palette entry), the bell, Help and Settings. In
       the light version it is a narrow glyph beside the five tabs. -->
  <button class="bn-btn" class:bn-compact={light} class:active={menuActive} aria-haspopup="dialog" aria-expanded={ui.navDrawerOpen} aria-label={menuLabel} title="Menu" data-testid="bottomnav-menu" onclick={() => (ui.navDrawerOpen = true)}>
    <span class="bn-icon">
      <Icon name="sidebar" size={20} />
      {#if moreBadge}<span class="bn-badge" class:needs={moreBadge.needs}>{moreBadge.count}</span>
      {:else if unread > 0}<span class="bn-dot" aria-hidden="true"></span>{/if}
    </span>
    {#if !light}<span class="bn-label">Menu</span>{/if}
  </button>
</nav>

<style>
  .bottomnav {
    display: flex;
    align-items: stretch;
    /* The tabs keep their 56 px; the home indicator's safe area is added
       under them (box-sizing would otherwise take it out of the tabs). */
    height: calc(var(--mobile-bottomnav-h) + var(--safe-bottom));
    flex-shrink: 0;
    border-top: 1px solid var(--border);
    background: var(--bg-sidebar);
    padding-block-end: var(--safe-bottom);
    z-index: var(--z-mobile-nav);
  }
  /* The on-screen keyboard is up (lib/viewportFit.ts): a tab bar pinned above
     it is never used mid-sentence and costs 56 px of what you are replying
     to — on every page (iOS Messages, Slack and GitHub do the same). Toasts
     drop to the keyboard's edge with it. */
  :global(html[data-kb='open']) {
    --mobile-bottomnav-h: 0px;
  }
  :global(html[data-kb='open']) .bottomnav {
    display: none;
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
    padding: 4px 0;
    min-width: 0;
    position: relative;
  }
  /* The Rail / Navigator selection language: an accent tint behind the glyph,
     accent text, and a short accent bar — here along the bar's top edge. */
  .bn-btn.active {
    color: var(--accent-text);
  }
  .bn-btn.active::before {
    content: '';
    position: absolute;
    inset-block-start: 0;
    inset-inline: 30%;
    block-size: 3px;
    border-radius: 0 0 2px 2px;
    background: var(--accent);
  }
  .bn-icon {
    position: relative;
    display: grid;
    place-items: center;
    height: 24px;
    padding-inline: 12px;
    border-radius: 999px;
  }
  .bn-btn.active .bn-icon {
    background: var(--accent-soft);
  }
  /* Light version: "More" is a narrow ⋯ beside the five tabs, not a sixth
     (narrow enough that "Connections" fits its tab at 360 px wide). */
  .bn-btn.bn-compact {
    flex: 0 0 44px;
  }
  .bn-label {
    font-size: var(--fs-xs);
    font-weight: 500;
    letter-spacing: -0.01em;
    line-height: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 100%;
  }
  .bn-badge {
    position: absolute;
    top: -4px;
    /* 8 px past the glyph's edge (the pill pads 12 px around it). */
    inset-inline-end: 4px;
    min-width: 15px;
    height: 15px;
    padding: 0 2px;
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

  /* Unread notifications: a dot on the Menu glyph (the count lives in the drawer's bell). */
  .bn-dot {
    position: absolute;
    inset-block-start: 0;
    inset-inline-end: 8px;
    inline-size: 8px;
    block-size: 8px;
    border-radius: 50%;
    background: var(--warning);
    box-shadow: 0 0 0 2px var(--bg-sidebar);
  }
</style>
