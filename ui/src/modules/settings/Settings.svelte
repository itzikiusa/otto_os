<script lang="ts">
  // Settings layout: subnav + routed page (#/settings/<page>).
  import Appearance from './Appearance.svelte';
  import SessionNames from './SessionNames.svelte';
  import AdminSessions from './AdminSessions.svelte';
  import Daemon from './Daemon.svelte';
  import PluginsSettings from './PluginsSettings.svelte';
  import Providers from './Providers.svelte';
  import Users from './Users.svelte';
  import AccessGroups from './AccessGroups.svelte';
  import GitAccounts from '../git/GitAccounts.svelte';
  import IssueAccounts from './IssueAccounts.svelte';
  import Channels from './Channels.svelte';
  import Notifications from './Notifications.svelte';
  import SelfImprovement from './SelfImprovement.svelte';
  import McpServers from './McpServers.svelte';
  import InsightsSettings from './InsightsSettings.svelte';
  import SnipSettings from './SnipSettings.svelte';
  import SkillEvalSettings from './SkillEvalSettings.svelte';
  import ContextSoul from './ContextSoul.svelte';
  import ContextLibrary from './ContextLibrary.svelte';
  import SkillsLibrary from './SkillsLibrary.svelte';
  import Logs from './Logs.svelte';
  import LanguageServers from './LanguageServers.svelte';
  import TrustSafety from './TrustSafety.svelte';
  import EmailSenderSetup from './EmailSenderSetup.svelte';
  import PersonalAccessTokens from './PersonalAccessTokens.svelte';
  import BackupRestore from './BackupRestore.svelte';
  import { router } from '../../lib/router.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  // Explicit exit: go back to where you were, or home (Agents) if there's no
  // history — so you don't have to click another module to leave Settings.
  function exit(): void {
    if (router.canBack) router.back();
    else router.go('agents');
  }

  const page = $derived(router.parts[1] ?? 'appearance');

  type NavItem = { id: string; label: string };
  // Grouped like the main sidebar (a macOS source list): ~25 flat rows were a
  // wall to scan. Admin-only sections appear only for roles that can open them.
  const groups = $derived<{ label: string; items: NavItem[] }[]>(
    [
      {
        label: 'General',
        items: [
          { id: 'appearance', label: 'Appearance' },
          { id: 'session-names', label: 'Session Names' },
          { id: 'notifications', label: 'Notifications' },
          { id: 'snipping', label: 'Snipping' },
          { id: 'tokens', label: 'API Tokens' },
        ],
      },
      {
        label: 'Integrations',
        items: [
          { id: 'git-accounts', label: 'Git Accounts' },
          { id: 'jira', label: 'Jira' },
          { id: 'channels', label: 'Channels' },
          { id: 'mcp-servers', label: 'MCP Servers' },
          { id: 'language-servers', label: 'Language Servers' },
          { id: 'sharing', label: 'Sharing' },
        ],
      },
      {
        label: 'Agents',
        items: [
          ...(auth.can('settings', 'admin') ? [{ id: 'providers', label: 'Providers' }] : []),
          { id: 'context-soul', label: 'Workspace context' },
          { id: 'self-improvement', label: 'Self-Improvement' },
          { id: 'insights', label: 'Insights' },
          // skills + skill-eval + context-library: settings:admin covers root-managed items.
          ...(auth.can('settings', 'admin')
            ? [
                { id: 'skills', label: 'Skills' },
                { id: 'skill-eval', label: 'Skills Evaluator' },
                { id: 'context-library', label: 'Context Library' },
              ]
            : []),
        ],
      },
      {
        label: 'System',
        items: auth.can('settings', 'admin')
          ? [
              { id: 'daemon', label: 'Daemon' },
              { id: 'plugins', label: 'Plugins' },
              { id: 'trust-safety', label: 'Trust & Safety' },
              { id: 'logs', label: 'Logs' },
              { id: 'backup', label: 'Backup & Restore' },
            ]
          : [],
      },
      {
        label: 'People',
        items: [
          // users management + admin session overview: users:admin gate.
          ...(auth.can('users', 'admin') ? [{ id: 'users', label: 'Users' }] : []),
          ...(auth.isRoot ? [{ id: 'access-groups', label: 'Groups & Access' }] : []),
          ...(auth.can('users', 'admin') ? [{ id: 'sessions', label: 'Sessions' }] : []),
        ],
      },
    ].filter((g) => g.items.length > 0),
  );
</script>

<div class="settings">
  <nav class="settings-nav">
    <div class="settings-nav-title">
      <span>Settings</span>
      <button class="settings-close" title="Close settings (⌘⇧←)" aria-label="Close settings" onclick={exit}>
        <Icon name="x" size={14} />
      </button>
    </div>
    <div class="settings-nav-list">
    {#each groups as g (g.label)}
      <div class="settings-nav-group" role="group" aria-label={g.label}>
        <div class="settings-nav-heading" aria-hidden="true">{g.label}</div>
        {#each g.items as it (it.id)}
          <button
            class="settings-nav-item"
            class:active={page === it.id}
            aria-current={page === it.id ? 'page' : undefined}
            onclick={() => router.go(`settings/${it.id}`)}
            oncontextmenu={(e) => ctxMenu.show(e, [
              { label: `Open ${it.label}`, icon: 'gear', action: () => router.go(`settings/${it.id}`) },
            ])}
          >
            {it.label}
          </button>
        {/each}
      </div>
    {/each}
    </div>
  </nav>

  <div class="settings-body">
    {#if page === 'session-names'}
      <SessionNames />
    {:else if page === 'appearance'}
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
    {:else if page === 'mcp-servers'}
      <McpServers />
    {:else if page === 'insights'}
      <InsightsSettings />
    {:else if page === 'snipping'}
      <SnipSettings />
    {:else if page === 'skills' && auth.can('settings', 'admin')}
      <SkillsLibrary />
    {:else if page === 'skill-eval' && auth.can('settings', 'admin')}
      <SkillEvalSettings />
    {:else if page === 'context-soul'}
      <ContextSoul />
    {:else if page === 'language-servers'}
      <LanguageServers />
    {:else if page === 'context-library' && auth.can('settings', 'admin')}
      <ContextLibrary />
    {:else if page === 'providers' && auth.can('settings', 'admin')}
      <Providers />
    {:else if page === 'users' && auth.can('users', 'admin')}
      <Users />
    {:else if page === 'access-groups' && auth.isRoot}
      <AccessGroups />
    {:else if page === 'sessions' && auth.can('users', 'admin')}
      <AdminSessions />
    {:else if page === 'sharing'}
      <EmailSenderSetup />
    {:else if page === 'tokens'}
      <PersonalAccessTokens />
    {:else if page === 'daemon' && auth.can('settings', 'admin')}
      <Daemon />
    {:else if page === 'plugins' && auth.can('settings', 'admin')}
      <PluginsSettings />
    {:else if page === 'trust-safety' && auth.can('settings', 'admin')}
      <TrustSafety />
    {:else if page === 'logs' && auth.can('settings', 'admin')}
      <Logs />
    {:else if page === 'backup' && auth.can('settings', 'admin')}
      <BackupRestore />
    {:else}
      <Appearance />
    {/if}
  </div>
</div>

<style>
  .settings {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .settings-nav {
    width: 180px;
    flex-shrink: 0;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  /* Same 46px bar + bottom border as the section's PageHeader, so the two
     read as one toolbar row across the split. */
  .settings-nav-title {
    height: 46px;
    flex-shrink: 0;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 10px 0 18px;
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-l);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text);
  }
  .settings-nav-list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 6px 10px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .settings-nav-group {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  /* Same section-label treatment as the main sidebar's groups. */
  .settings-nav-heading {
    padding: 8px 10px 3px;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .settings-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 22px;
    height: 22px;
    border-radius: 6px;
    color: var(--text-dim);
    background: none;
    border: none;
    cursor: pointer;
  }
  .settings-close:hover {
    background: var(--hover);
    color: var(--text);
  }
  .settings-nav-item {
    height: 28px;
    padding: 0 10px;
    text-align: start;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    font-size: var(--fs-m);
    color: var(--text);
    cursor: pointer;
    transition: background 120ms ease-out;
  }
  .settings-nav-item:hover {
    background: var(--hover);
  }
  /* The sidebar's active row: accent tint + --text at 600 (accent is never
     text — it fails contrast in several themes). */
  .settings-nav-item.active {
    background: var(--accent-soft);
    color: var(--text);
    font-weight: 600;
  }
  /* Each section owns its chrome (PageHeader) and scroll (PageBody). */
  .settings-body {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  @media (max-width: 640px) {
    .settings {
      flex-direction: column;
    }
    .settings-nav {
      width: 100%;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
    }
    /* One horizontally scrolling row of sections (the wrapped grid of ~25
       buttons took a third of the phone screen). */
    .settings-nav-list {
      flex-direction: row;
      flex-wrap: nowrap;
      gap: 4px;
      padding: 8px;
      overflow-x: auto;
      scrollbar-width: none;
    }
    .settings-nav-list::-webkit-scrollbar {
      display: none;
    }
    .settings-nav-group {
      display: contents;
    }
    .settings-nav-heading {
      display: none;
    }
    .settings-nav-item {
      flex: none;
      white-space: nowrap;
    }
    .settings-nav-title {
      display: none;
    }
  }
</style>
