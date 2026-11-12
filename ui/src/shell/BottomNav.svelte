<script lang="ts">
  // Phone bottom navigation bar. Shows the first few modules the current user
  // can view as large tap targets, plus a "More" affordance that opens the
  // shared shell Drawer with the rest (and Settings). Tapping a module routes
  // via router.go(); it draws from the same shared sidebar registry as the Rail and
  // Navigator and honours the user's saved order + hidden set, so all three
  // navigations stay in lockstep.
  //
  // Rendered only on phone (App.svelte gates it behind viewport.isPhone), so it
  // adds nothing to the desktop layout.
  //
  // The light version (lib/lightMode.svelte.ts, opt-in from the top of the
  // More sheet) shows exactly its five modules as tabs, in a fixed order, and
  // shrinks "More" to a small ⋯ that holds the way back to the full app,
  // notifications, Commands, Settings, sign out and the machine row.
  import Icon from '../lib/components/Icon.svelte';
  import Drawer from './Drawer.svelte';
  import MachineChip from './MachineChip.svelte';
  import NotificationBell from './NotificationBell.svelte';
  import { notifications } from '../lib/stores/notifications.svelte';
  import { plural } from '../lib/plural';
  import { LIGHT_MODULES, lightMode } from '../lib/lightMode.svelte';
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
  const light = $derived(lightMode.active);
  // Flattened section by section, so the bar's order matches the sidebar's —
  // Favorites first, so a user's favorites become the phone's primary tabs.
  // The light version is a fixed set instead (still role-filtered: the
  // registry's own gate, inside the light scope), never the saved order.
  const modules = $derived.by(() => {
    const available = availableModules((f) => auth.can(f, 'view'), light ? [] : pluginEntries);
    if (light) return LIGHT_MODULES.flatMap((id) => available.filter((m) => m.id === id));
    return sidebarSections(
      visibleOrder(resolveOrder(available, ui.sidebarOrder), ui.sidebarHidden),
      ui.sidebarFavorites,
      ui.sidebarGroupOrder,
    ).flatMap((s) => s.modules);
  });
  // The entry the current route highlights (plugin slug, '' → Agents, …).
  const current = $derived(activeNavId(router.parts));
  const primary = $derived(light ? modules : modules.slice(0, PRIMARY_COUNT));
  const overflow = $derived(light ? [] : modules.slice(PRIMARY_COUNT));
  // Light: the ⋯ carries the unread notifications (the bell lives in its sheet).
  const unread = $derived(light ? notifications.unreadRows : 0);

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

  // The overflow opens in the shell Drawer, which owns the modal plumbing
  // (pushModal, focus trap, Esc, backdrop, ✕).
  let moreOpen = $state(false);

  function go(id: string): void {
    router.openModule(id);
    moreOpen = false;
  }

  /** Switch between the light version and the full app (one tap, either way). */
  function setLight(on: boolean): void {
    moreOpen = false;
    lightMode.set(on);
  }

  // "More" is active when the current module lives in the overflow set (or
  // Settings), so the bar reflects where you are even for spilled modules.
  const moreActive = $derived(
    router.module === 'settings' || overflow.some((m) => m.id === current),
  );
  const moreLabel = $derived(
    moreBadge
      ? `More, ${moreBadge.count} ${moreBadge.needs ? 'need you' : 'active'}`
      : unread > 0
        ? `More, ${plural(unread, 'unread notification')}`
        : 'More',
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
       phone's only palette entry off the Agents page) and Settings. In the
       light version it is a small ⋯ beside the five tabs, not a sixth tab. -->
  <button class="bn-btn" class:bn-compact={light} class:active={moreActive} aria-haspopup="dialog" aria-expanded={moreOpen} aria-label={moreLabel} title="More" onclick={() => (moreOpen = true)}>
    <span class="bn-icon">
      <Icon name="more" size={20} />
      {#if moreBadge}<span class="bn-badge" class:needs={moreBadge.needs}>{moreBadge.count}</span>
      {:else if unread > 0}<span class="bn-badge needs" aria-hidden="true">{unread > 99 ? '99+' : unread}</span>{/if}
    </span>
    {#if !light}<span class="bn-label">More</span>{/if}
  </button>
</nav>

<Drawer bind:open={moreOpen} side="right" title="More" width="min(86vw, 360px)">
    <div class="sheet-grid">
      <!-- One tap either way between the full app and the light version
           (remembered on this device; Settings ▸ Appearance has it too). -->
      <button class="sheet-row" data-testid="light-toggle" onclick={() => setLight(!light)}>
        <Icon name={light ? 'maximize' : 'minimize'} size={20} />
        <span class="sr-text">
          <span class="sr-title">{light ? 'Full app' : 'Light version'}</span>
          <span class="sr-hint">{light ? 'Show every module again' : 'Threads, Connections, Git, API and Usage only'}</span>
        </span>
      </button>
      {#if light}
        <!-- The light version's top bar may be absent (Threads draws its own
             header), so the bell lives here too. -->
        <div class="sheet-row sheet-bell" data-testid="more-notifications">
          <span class="sr-title">Notifications</span>
          <NotificationBell placement="below" />
        </div>
      {/if}
      <button
        class="sheet-item"
        onclick={() => {
          moreOpen = false;
          ui.openPalette('commands');
        }}
      >
        <Icon name="command" size={20} />
        <span>Commands</span>
      </button>
      {#each overflow as m (m.id)}
        {@const b = navBadge(m.id)}
        <button class="sheet-item" class:active={current === m.id} aria-current={current === m.id ? 'page' : undefined} data-nav-id={m.id} aria-busy={navPending.id === m.id || undefined} aria-label="{m.label}{b?.spoken ?? ''}" onclick={() => go(m.id)}>
          <Icon name={m.icon} size={20} />
          <span>{m.label}</span>
          {#if b}<span class="bn-badge sheet-badge" class:needs={b.tone === 'needs'} aria-hidden="true">{b.count}</span>{/if}
        </button>
      {/each}
      {#if !light}
        <button
          class="sheet-item"
          class:active={router.module === 'walkthroughs'}
          aria-current={router.module === 'walkthroughs' ? 'page' : undefined}
          onclick={() => {
            router.go('walkthroughs');
            moreOpen = false;
          }}
        >
          <Icon name="info" size={20} />
          <span>Help</span>
        </button>
      {/if}
      <button
        class="sheet-item"
        class:active={router.module === 'settings'}
        aria-current={router.module === 'settings' ? 'page' : undefined}
        onclick={() => {
          router.go('settings/appearance');
          moreOpen = false;
        }}
      >
        <Icon name="gear" size={20} />
        <span>Settings</span>
      </button>
      {#if light}
        <!-- The full app signs out from the Navigator; the light version has
             no Navigator on its landing page, so the sheet carries it. -->
        <button
          class="sheet-item"
          onclick={() => {
            moreOpen = false;
            void auth.logout();
          }}
        >
          <Icon name="logout" size={20} />
          <span>Sign out</span>
        </button>
      {/if}
      <!-- The status bar (and its machine chip) is hidden on a phone: the
           chip lives here as a full-width row; the whole row opens its
           details. Hidden until the machine status has loaded. -->
      <div class="sheet-machine" data-testid="more-machine">
        <span class="sm-label">Machine</span>
        <MachineChip />
      </div>
    </div>
</Drawer>

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
  /* Light version: "More" is a narrow ⋯ beside the five tabs, not a sixth. */
  .bn-btn.bn-compact {
    flex: 0 0 48px;
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

  .sheet-badge {
    top: 6px;
    inset-inline-end: 8px;
    background: color-mix(in srgb, var(--success) 24%, var(--surface));
  }
  .sheet-badge.needs {
    background: color-mix(in srgb, var(--warning) 24%, var(--surface));
  }
  .sheet-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 8px;
    padding: 12px 12px calc(16px + env(safe-area-inset-bottom, 0));
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
    /* --accent-soft over the item's own surface (the tint is translucent). */
    background: linear-gradient(var(--accent-soft), var(--accent-soft)), var(--surface);
  }
  .sheet-item span {
    line-height: 1;
    text-align: center;
  }
  /* Full-width rows: the light/full switch and (light) the bell. */
  .sheet-row {
    grid-column: 1 / -1;
    display: flex;
    align-items: center;
    gap: 12px;
    min-height: 48px;
    padding: 8px 12px;
    border: none;
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-s);
    text-align: start;
    cursor: pointer;
  }
  .sr-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .sr-title {
    font-weight: 500;
  }
  .sr-hint {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .sheet-bell {
    justify-content: space-between;
    cursor: default;
  }
  .sheet-machine {
    grid-column: 1 / -1;
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-height: 44px;
    padding-inline: 12px;
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .sheet-machine:not(:has(:global(.machine-chip))) {
    display: none;
  }
  /* The chip's button covers the whole row (one tap target), and its status
     text shows here even at phone width, where the status bar hides it. */
  .sheet-grid .sheet-machine :global(.machine-chip) {
    position: static;
    font-size: var(--fs-s);
  }
  .sheet-grid .sheet-machine :global(.machine-chip)::after {
    content: '';
    position: absolute;
    inset: 0;
    border-radius: var(--radius-m);
  }
  .sheet-grid .sheet-machine :global(.mc-text) {
    display: inline;
  }
</style>
