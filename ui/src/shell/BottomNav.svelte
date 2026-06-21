<script lang="ts">
  // Phone bottom navigation bar. Shows the first few modules the current user
  // can view as large tap targets, plus a "More" affordance that opens an
  // overflow sheet for the rest (and Settings). Tapping a module routes via
  // router.go(); this mirrors Rail.svelte's ALL_MODULES + auth.can() filtering
  // so the two navigations stay in lockstep.
  //
  // Rendered only on phone (App.svelte gates it behind viewport.isPhone), so it
  // adds nothing to the desktop layout.
  import Icon from '../lib/components/Icon.svelte';
  import { router } from '../lib/router.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { ws } from '../lib/stores/workspace.svelte';
  import { auth } from '../lib/stores/auth.svelte';

  // Same source-of-truth list as Rail.svelte. Each entry carries the feature
  // name used by auth.can(); all gated at 'view' (root always passes).
  const ALL_MODULES = [
    { id: 'agents', icon: 'terminal', label: 'Agents', feature: 'agents' as const },
    { id: 'swarm', icon: 'grid', label: 'Swarm', feature: 'swarm' as const },
    { id: 'connections', icon: 'plug', label: 'Connections', feature: 'connections' as const },
    { id: 'git', icon: 'branch', label: 'Git', feature: 'git' as const },
    { id: 'product', icon: 'note', label: 'Product', feature: 'product' as const },
    { id: 'api', icon: 'send', label: 'API', feature: 'api_client' as const },
    { id: 'database', icon: 'db', label: 'Database', feature: 'database' as const },
    { id: 'workflows', icon: 'split', label: 'Workflows', feature: 'workflows' as const },
    { id: 'skills-eval', icon: 'zap', label: 'Skills', feature: 'skill_eval' as const },
    { id: 'insights', icon: 'gauge', label: 'Insights', feature: 'insights' as const },
    { id: 'usage', icon: 'chart', label: 'Usage', feature: 'usage' as const },
  ];

  // How many primary tabs sit on the bar before everything spills into "More".
  const PRIMARY_COUNT = 4;

  const modules = $derived(ALL_MODULES.filter((m) => auth.can(m.feature, 'view')));
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
    router.module === 'settings' || overflow.some((m) => m.id === router.module),
  );
</script>

<nav class="bottomnav" aria-label="Primary">
  {#each primary as m (m.id)}
    <button class="bn-btn" class:active={router.module === m.id} onclick={() => go(m.id)}>
      <span class="bn-icon">
        <Icon name={m.icon} size={20} />
        {#if m.id === 'agents' && ws.workingCount > 0}
          <span class="bn-badge">{ws.workingCount}</span>
        {/if}
      </span>
      <span class="bn-label">{m.label}</span>
    </button>
  {/each}

  {#if overflow.length > 0}
    <button class="bn-btn" class:active={moreActive} onclick={() => (moreOpen = true)}>
      <span class="bn-icon"><Icon name="command" size={20} /></span>
      <span class="bn-label">More</span>
    </button>
  {/if}
</nav>

{#if moreOpen}
  <!-- Overflow sheet: the remaining modules + Settings as a bottom sheet. -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="sheet-backdrop" onclick={() => (moreOpen = false)}></div>
  <div class="more-sheet" role="dialog" aria-modal="true" aria-label="More modules">
    <div class="sheet-grip"></div>
    <div class="sheet-grid">
      {#each overflow as m (m.id)}
        <button class="sheet-item" class:active={router.module === m.id} onclick={() => go(m.id)}>
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
    z-index: 60;
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
    color: var(--accent);
  }
  .bn-icon {
    position: relative;
    display: grid;
    place-items: center;
    height: 22px;
  }
  .bn-label {
    font-size: 10.5px;
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

  .sheet-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    z-index: 92;
  }
  .more-sheet {
    position: fixed;
    inset-inline-start: 0;
    inset-inline-end: 0;
    bottom: 0;
    z-index: 93;
    background: var(--bg);
    border-top: 1px solid var(--border);
    border-radius: 14px 14px 0 0;
    box-shadow: var(--shadow);
    padding: 8px 12px calc(16px + env(safe-area-inset-bottom, 0));
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
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 12%, var(--surface));
  }
  .sheet-item span {
    line-height: 1;
    text-align: center;
  }
</style>
