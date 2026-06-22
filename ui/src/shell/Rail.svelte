<script lang="ts">
  // Collapsed 44px icon rail (⌘1 expands to the Navigator).
  import Icon from '../lib/components/Icon.svelte';
  import { router } from '../lib/router.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { ws } from '../lib/stores/workspace.svelte';
  import { auth } from '../lib/stores/auth.svelte';
  import { plugins } from '../lib/stores/plugins.svelte';
  import { availableModules, resolveOrder, visibleOrder } from '../lib/sidebar';

  // The collapsed rail mirrors the same resolved module list as the Navigator
  // (shared registry → RBAC filter + plugins → user's saved order, minus hidden
  // ones). It's read-only here: reordering / show-hide happens in the expanded
  // Navigator (and Settings → Appearance). See ui.svelte.ts for persistence.
  const pluginEntries = $derived(
    plugins.list
      .filter((p) => auth.canPlugin(p.slug, 'view'))
      .map((p) => ({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name })),
  );
  const modules = $derived(
    visibleOrder(
      resolveOrder(
        availableModules((f) => auth.can(f, 'view'), pluginEntries),
        ui.sidebarOrder,
      ),
      ui.sidebarHidden,
    ),
  );

  // Active when the route matches the entry id. Plugin entries use a `plugin/<slug>`
  // id while `router.module` is just `plugin`, so compare the slug for those.
  function isActive(id: string): boolean {
    if (id.startsWith('plugin/')) {
      return router.module === 'plugin' && `plugin/${router.parts[1] ?? ''}` === id;
    }
    return router.module === id;
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

  <div class="rail-modules">
    {#each modules as m (m.id)}
      <button
        class="rail-btn"
        class:active={isActive(m.id)}
        onclick={() => router.go(m.id)}
        title={m.label}
        aria-label={m.label}
      >
        <Icon name={m.icon} />
        {#if m.id === 'agents' && ws.workingCount > 0}
          <span class="rail-badge">{ws.workingCount}</span>
        {/if}
      </button>
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
  .rail-modules {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 10px;
    flex: 1;
  }
  .rail-bottom {
    display: flex;
    flex-direction: column;
    gap: 4px;
    align-items: center;
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
    transition: background 130ms ease-out, color 130ms ease-out;
  }
  .rail-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text);
  }
  .rail-btn.active {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
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
