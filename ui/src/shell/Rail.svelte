<script lang="ts">
  // Collapsed 44px icon rail (⌘1 expands to the Navigator).
  import Icon from '../lib/components/Icon.svelte';
  import { router } from '../lib/router.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { ws } from '../lib/stores/workspace.svelte';
  import { auth } from '../lib/stores/auth.svelte';

  const modules = $derived([
    { id: 'agents', icon: 'terminal', label: 'Agents' },
    { id: 'connections', icon: 'plug', label: 'Connections' },
    { id: 'git', icon: 'branch', label: 'Git' },
    { id: 'api', icon: 'send', label: 'API' },
    { id: 'database', icon: 'db', label: 'Database' },
    { id: 'workflows', icon: 'split', label: 'Workflows' },
    { id: 'skills-eval', icon: 'zap', label: 'Skills Evaluator' },
    // Usage analytics aggregate across all workspaces — root only.
    ...(auth.isRoot ? [{ id: 'usage', icon: 'chart', label: 'Usage' }] : []),
  ]);
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
        class:active={router.module === m.id}
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
    border-right: 1px solid var(--border);
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
    right: -3px;
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
