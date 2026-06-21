<script lang="ts">
  // Collapsed 44px icon rail (⌘1 expands to the Navigator).
  import Icon from '../lib/components/Icon.svelte';
  import { router } from '../lib/router.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { ws } from '../lib/stores/workspace.svelte';
  import { auth } from '../lib/stores/auth.svelte';
  import { plugins } from '../lib/stores/plugins.svelte';

  // Each entry carries the feature name used by auth.can(); all are gated at
  // 'view'. Root always passes (can() returns true for root). The memory-layer
  // "Vault" module and the "Message Brokers" module predate (resp. ship without)
  // an RBAC feature key, so they are appended unconditionally (visible to any
  // authenticated member) rather than filtered.
  const ALL_MODULES = [
    { id: 'agents',     icon: 'terminal', label: 'Agents',           feature: 'agents'      as const },
    { id: 'swarm',      icon: 'grid',     label: 'Swarm',            feature: 'swarm'       as const },
    { id: 'connections',icon: 'plug',     label: 'Connections',      feature: 'connections' as const },
    { id: 'git',        icon: 'branch',   label: 'Git',              feature: 'git'         as const },
    { id: 'product',    icon: 'note',     label: 'Product',          feature: 'product'     as const },
    { id: 'api',        icon: 'send',     label: 'API',              feature: 'api_client'  as const },
    { id: 'database',   icon: 'db',       label: 'Database',         feature: 'database'    as const },
    { id: 'workflows',  icon: 'split',    label: 'Workflows',        feature: 'workflows'   as const },
    { id: 'skills-eval',icon: 'zap',      label: 'Skills Evaluator', feature: 'skill_eval'  as const },
    { id: 'insights',   icon: 'gauge',    label: 'Insights',         feature: 'insights'    as const },
    { id: 'usage',      icon: 'chart',    label: 'Usage',            feature: 'usage'       as const },
  ];
  // Filter to only the modules the current user has at least view on, then
  // splice in the un-gated "Vault" (memory) module after Product (or at the end
  // when Product itself is not visible) and "Message Brokers" after Database
  // (or at the end when Database is not visible) so they are always reachable.
  const modules = $derived.by(() => {
    const gated = ALL_MODULES.filter(m => auth.can(m.feature, 'view')).map(m => ({
      id: m.id,
      icon: m.icon,
      label: m.label,
    }));
    const vault = { id: 'vault', icon: 'globe', label: 'Vault' };
    const vi = gated.findIndex(m => m.id === 'product');
    let out = vi === -1 ? [...gated, vault] : [...gated.slice(0, vi + 1), vault, ...gated.slice(vi + 1)];
    const brokers = { id: 'brokers', icon: 'box', label: 'Message Brokers' };
    const bi = out.findIndex(m => m.id === 'database');
    out = bi === -1 ? [...out, brokers] : [...out.slice(0, bi + 1), brokers, ...out.slice(bi + 1)];
    // Runtime custom plugins (RBAC-gated by slug) append after the built-ins,
    // each routing to `#/plugin/<slug>`.
    for (const p of plugins.list) {
      if (auth.canPlugin(p.slug, 'view')) {
        out.push({ id: `plugin/${p.slug}`, icon: p.icon, label: p.name });
      }
    }
    return out;
  });

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
