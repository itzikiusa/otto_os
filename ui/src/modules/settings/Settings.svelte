<script lang="ts">
  // Settings layout: subnav + routed page (#/settings/<page>).
  import Appearance from './Appearance.svelte';
  import Daemon from './Daemon.svelte';
  import Providers from './Providers.svelte';
  import Users from './Users.svelte';
  import GitAccounts from '../git/GitAccounts.svelte';
  import IssueAccounts from './IssueAccounts.svelte';
  import Channels from './Channels.svelte';
  import Notifications from './Notifications.svelte';
  import SelfImprovement from './SelfImprovement.svelte';
  import InsightsSettings from './InsightsSettings.svelte';
  import SkillEvalSettings from './SkillEvalSettings.svelte';
  import ContextSoul from './ContextSoul.svelte';
  import ContextLibrary from './ContextLibrary.svelte';
  import SkillsLibrary from './SkillsLibrary.svelte';
  import Logs from './Logs.svelte';
  import LanguageServers from './LanguageServers.svelte';
  import { router } from '../../lib/router.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';

  const page = $derived(router.parts[1] ?? 'appearance');

  const items = $derived([
    { id: 'appearance', label: 'Appearance' },
    { id: 'git-accounts', label: 'Git Accounts' },
    { id: 'jira', label: 'Jira' },
    { id: 'channels', label: 'Channels' },
    { id: 'notifications', label: 'Notifications' },
    { id: 'self-improvement', label: 'Self-Improvement' },
    { id: 'insights', label: 'Insights' },
    { id: 'context-soul', label: 'Context & Soul' },
    { id: 'language-servers', label: 'Language Servers' },
    ...(auth.isRoot
      ? [
          { id: 'skills', label: 'Skills' },
          { id: 'skill-eval', label: 'Skills Evaluator' },
          { id: 'context-library', label: 'Context Library' },
          { id: 'providers', label: 'Providers' },
          { id: 'users', label: 'Users' },
          { id: 'daemon', label: 'Daemon' },
          { id: 'logs', label: 'Logs' },
        ]
      : []),
  ]);
</script>

<div class="settings">
  <nav class="settings-nav">
    <div class="settings-nav-title">Settings</div>
    {#each items as it (it.id)}
      <button
        class="settings-nav-item"
        class:active={page === it.id}
        onclick={() => router.go(`settings/${it.id}`)}
        oncontextmenu={(e) => ctxMenu.show(e, [
          { label: `Open ${it.label}`, icon: 'gear', action: () => router.go(`settings/${it.id}`) },
        ])}
      >
        {it.label}
      </button>
    {/each}
  </nav>

  <div class="settings-body">
    {#if page === 'appearance'}
      <Appearance />
    {:else if page === 'git-accounts'}
      <GitAccounts />
    {:else if page === 'jira'}
      <IssueAccounts />
    {:else if page === 'channels'}
      <Channels />
    {:else if page === 'notifications'}
      <Notifications />
    {:else if page === 'self-improvement'}
      <SelfImprovement />
    {:else if page === 'insights'}
      <InsightsSettings />
    {:else if page === 'skills' && auth.isRoot}
      <SkillsLibrary />
    {:else if page === 'skill-eval' && auth.isRoot}
      <SkillEvalSettings />
    {:else if page === 'context-soul'}
      <ContextSoul />
    {:else if page === 'language-servers'}
      <LanguageServers />
    {:else if page === 'context-library' && auth.isRoot}
      <ContextLibrary />
    {:else if page === 'providers' && auth.isRoot}
      <Providers />
    {:else if page === 'users' && auth.isRoot}
      <Users />
    {:else if page === 'daemon' && auth.isRoot}
      <Daemon />
    {:else if page === 'logs' && auth.isRoot}
      <Logs />
    {:else}
      <Appearance />
    {/if}
  </div>
</div>

<style>
  .settings {
    display: flex;
    height: 100%;
  }
  .settings-nav {
    width: 180px;
    flex-shrink: 0;
    border-right: 1px solid var(--border);
    padding: 16px 10px;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .settings-nav-title {
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-dim);
    padding: 0 8px 8px;
  }
  .settings-nav-item {
    height: 28px;
    padding: 0 10px;
    text-align: left;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    font-size: 12.5px;
    color: var(--text);
    cursor: pointer;
    transition: background 120ms ease-out;
  }
  .settings-nav-item:hover {
    background: var(--surface-2);
  }
  .settings-nav-item.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
    font-weight: 500;
  }
  .settings-body {
    flex: 1;
    min-width: 0;
    overflow-y: auto;
  }
</style>
