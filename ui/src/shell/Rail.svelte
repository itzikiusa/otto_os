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
  import { tick } from 'svelte';
  import {
    activeNavId,
    availableModules,
    groupModules,
    resolveOrder,
    visibleOrder,
  } from '../lib/sidebar';

  // The collapsed rail mirrors the same resolved module list as the Navigator
  // (shared registry → RBAC filter + plugins → user's saved order, minus hidden
  // ones), section by section with a thin separator between sections (no
  // headers or folding — the Navigator owns those). It's read-only here:
  // reordering / show-hide happens in the expanded Navigator (and Settings →
  // Appearance). See ui.svelte.ts for persistence.
  const pluginEntries = $derived(
    plugins.list
      .filter((p) => auth.canPlugin(p.slug, 'view'))
      .map((p) => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name })),
  );
  const sections = $derived(
    groupModules(
      visibleOrder(
        resolveOrder(
          availableModules((f) => auth.can(f, 'view'), pluginEntries),
          ui.sidebarOrder,
        ),
        ui.sidebarHidden,
      ),
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
        <button
          class="rail-btn"
          class:active={isActive(m.id)}
          onclick={() => router.go(m.id)}
          title={`${m.label} · ${sec.group.label}`}
          aria-label={m.label}
        >
          <Icon name={m.icon} />
          {#if m.id === 'agents' && ws.workingCount > 0}
            <span class="rail-badge">{ws.workingCount}</span>
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
    <button class="rail-btn user" title={auth.me?.display_name ?? 'Account'} aria-label="Account">
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
    border-inline-end: 1px solid var(--border);
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
  .rail-badge.needs {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .avatar {
    width: 22px;
    height: 22px;
    border-radius: 50%;
    background: color-mix(in srgb, var(--accent) 28%, transparent);
    color: var(--accent);
    font-size: 11px;
    font-weight: 600;
    display: grid;
    place-items: center;
  }
</style>
