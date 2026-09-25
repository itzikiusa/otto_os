<script lang="ts">
  // Settings layout: subnav + routed page (#/settings/<page>). The section
  // list itself lives in ./sections.ts (labels, groups, keywords, access) so
  // the nav, each page title and the ⌘K "Settings: …" commands can't drift.
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
  import BrowserSettings from './BrowserSettings.svelte';
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
  import AssistantSettings from './AssistantSettings.svelte';
  import { router } from '../../lib/router.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import type { Component } from 'svelte';
  import {
    SETTINGS_GROUPS,
    availableSections,
    canOpenSection,
    filterSections,
    findSection,
    groupLabel,
    type SettingsSection,
    type SettingsSectionId,
  } from './sections';

  // Explicit exit: go back to where you were, or home (Agents) if there's no
  // history — so you don't have to click another module to leave Settings.
  function exit(): void {
    if (router.canBack) router.back();
    else router.go('agents');
  }

  // The section components, keyed by registry id — the only per-section thing
  // that needs Svelte. The Record type makes a new registry entry without a
  // view a compile error.
  const VIEWS: Record<SettingsSectionId, Component> = {
    appearance: Appearance,
    'session-names': SessionNames,
    notifications: Notifications,
    snipping: SnipSettings,
    browser: BrowserSettings,
    tokens: PersonalAccessTokens,
    'git-accounts': GitAccounts,
    jira: IssueAccounts,
    channels: Channels,
    'mcp-servers': McpServers,
    'language-servers': LanguageServers,
    sharing: EmailSenderSetup,
    assistant: AssistantSettings,
    providers: Providers,
    'context-soul': ContextSoul,
    'self-improvement': SelfImprovement,
    insights: InsightsSettings,
    skills: SkillsLibrary,
    'skill-eval': SkillEvalSettings,
    'context-library': ContextLibrary,
    daemon: Daemon,
    plugins: PluginsSettings,
    'trust-safety': TrustSafety,
    logs: Logs,
    backup: BackupRestore,
    users: Users,
    'access-groups': AccessGroups,
    sessions: AdminSessions,
  };

  const pageId = $derived(router.parts[1] || 'appearance');
  const section = $derived(findSection(pageId));
  // A routed section is openable, known-but-forbidden (a stale link or a role
  // change) or unknown — the last two get an explicit empty state instead of
  // silently rendering Appearance with nothing selected in the nav.
  const View = $derived(section && canOpenSection(section, auth) ? VIEWS[section.id] : null);

  const sections = $derived(availableSections(auth));
  // Grouped like the main sidebar (a macOS source list); sections the role
  // can't open are simply absent.
  const groups = $derived(
    SETTINGS_GROUPS.map((g) => ({ ...g, items: sections.filter((s) => s.group === g.id) })).filter(
      (g) => g.items.length > 0,
    ),
  );

  // ---- filter ----
  // Typing narrows the nav to a flat, ranked result list (label matches
  // first, group as dim detail); Enter opens the first result, ↓ walks into
  // the results, Esc clears.
  let query = $state('');
  let filterEl = $state<HTMLInputElement | null>(null);
  let listEl = $state<HTMLDivElement | null>(null);
  const results = $derived(query.trim() ? filterSections(sections, query) : null);

  function open(s: SettingsSection): void {
    router.go(`settings/${s.id}`);
  }

  function navItems(): HTMLButtonElement[] {
    return listEl ? [...listEl.querySelectorAll<HTMLButtonElement>('.settings-nav-item')] : [];
  }

  function onFilterKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      const first = results?.[0];
      if (first) {
        e.preventDefault();
        open(first);
      }
    } else if (e.key === 'Escape') {
      // Esc clears first; a second Esc leaves the field.
      e.preventDefault();
      e.stopPropagation();
      if (query) query = '';
      else filterEl?.blur();
    } else if (e.key === 'ArrowDown') {
      const first = navItems()[0];
      if (first) {
        e.preventDefault();
        first.focus();
      }
    }
  }

  // Arrow keys move between rows; ↑ from the first row returns to the filter,
  // and typing a character on a row carries it into the filter.
  function onListKey(e: KeyboardEvent): void {
    const items = navItems();
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    if (i < 0) return;
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      items[Math.min(i + 1, items.length - 1)]?.focus();
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      if (i === 0) filterEl?.focus();
      else items[i - 1]?.focus();
    } else if (e.key === 'Escape' && query) {
      e.preventDefault();
      e.stopPropagation();
      query = '';
      filterEl?.focus();
    } else if (e.key.length === 1 && e.key !== ' ' && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      query += e.key;
      filterEl?.focus();
    }
  }

  function rowMenu(e: MouseEvent, s: SettingsSection): void {
    ctxMenu.show(e, [{ label: `Open ${s.label}`, icon: 'gear', action: () => open(s) }]);
  }

  function gateHint(s: SettingsSection): string {
    const g = s.gate;
    if (g === 'root') return 'Only the root account can open this section.';
    if (g == null) return '';
    return `Ask a workspace admin for ${g.level} access to ${g.feature.replace(/_/g, ' ')}.`;
  }
</script>

{#snippet navRow(s: SettingsSection, detail?: string)}
  <button
    class="settings-nav-item"
    class:active={pageId === s.id}
    aria-current={pageId === s.id ? 'page' : undefined}
    onclick={() => open(s)}
    oncontextmenu={(e) => rowMenu(e, s)}
  >
    <span class="settings-nav-label">{s.label}</span>
    {#if detail}<span class="settings-nav-detail">{detail}</span>{/if}
  </button>
{/snippet}

<div class="settings">
  <nav class="settings-nav" aria-label="Settings sections">
    <div class="settings-nav-title">
      <span>Settings</span>
      <button class="icon-btn" title="Close settings (⌘⇧←)" aria-label="Close settings" onclick={exit}>
        <Icon name="x" size={14} />
      </button>
    </div>
    <label class="settings-nav-filter">
      <Icon name="search" size={12} />
      <input
        bind:this={filterEl}
        bind:value={query}
        class="settings-filter-input"
        type="search"
        placeholder="Filter settings"
        aria-label="Filter settings"
        aria-controls="settings-nav-list"
        autocomplete="off"
        spellcheck="false"
        onkeydown={onFilterKey}
      />
    </label>
    <!-- Arrow-key roving between the row buttons (each row is a real button). -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="settings-nav-list" id="settings-nav-list" bind:this={listEl} onkeydown={onListKey}>
      {#if results}
        {#each results as s (s.id)}
          {@render navRow(s, groupLabel(s.group))}
        {:else}
          <p class="settings-nav-empty">No sections match “{query.trim()}”</p>
        {/each}
      {:else}
        {#each groups as g (g.id)}
          <div class="settings-nav-group" role="group" aria-label={g.label}>
            <div class="settings-nav-heading" aria-hidden="true">{g.label}</div>
            {#each g.items as s (s.id)}
              {@render navRow(s)}
            {/each}
          </div>
        {/each}
      {/if}
    </div>
  </nav>

  <div class="settings-body">
    {#if View}
      <View />
    {:else if section}
      <PageHeader title={section.label} />
      <PageBody>
        <EmptyState
          variant="page"
          icon="lock"
          title={`You don't have access to ${section.label}`}
          body={gateHint(section)}
          actionLabel="Open Appearance"
          onaction={() => router.go('settings/appearance')}
        >
          {#if router.canBack}
            <button class="btn ghost" onclick={() => router.back()}>Go back</button>
          {/if}
        </EmptyState>
      </PageBody>
    {:else}
      <PageHeader title="Settings" />
      <PageBody>
        <EmptyState
          variant="page"
          icon="gear"
          title={`No settings section called “${pageId}”`}
          body="The link may be out of date. Pick a section from the list, or filter it by name."
          actionLabel="Open Appearance"
          onaction={() => router.go('settings/appearance')}
        />
      </PageBody>
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
    width: 200px;
    flex-shrink: 0;
    border-inline-end: 1px solid var(--separator);
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
    padding-block: 0;
    padding-inline: 18px 10px;
    border-bottom: 1px solid var(--separator);
    font-size: var(--fs-l);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text);
  }
  /* Filter field: a quiet search box pinned above the (scrolling) list. */
  .settings-nav-filter {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 10px 10px 2px;
    padding-inline: 8px;
    height: 27px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    transition: border-color 130ms ease-out, box-shadow 130ms ease-out;
  }
  .settings-nav-filter:focus-within {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .settings-filter-input {
    flex: 1;
    min-width: 0;
    height: 100%;
    border: none;
    outline: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-m);
  }
  .settings-filter-input::placeholder {
    color: var(--text-dim);
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
  .settings-nav-item {
    height: 28px;
    flex-shrink: 0;
    padding: 0 10px;
    display: flex;
    align-items: center;
    gap: 8px;
    text-align: start;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    font-size: var(--fs-m);
    color: var(--text);
    cursor: pointer;
    transition: background 120ms ease-out;
  }
  .settings-nav-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .settings-nav-detail {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .settings-nav-empty {
    margin: 0;
    padding: 8px 10px;
    font-size: var(--fs-s);
    color: var(--text-dim);
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

  @media (prefers-reduced-motion: reduce) {
    .settings-nav-item,
    .settings-nav-filter {
      transition: none;
    }
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
    .settings-nav-filter {
      margin: 8px 8px 0;
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
    .settings-nav-heading,
    .settings-nav-detail {
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
