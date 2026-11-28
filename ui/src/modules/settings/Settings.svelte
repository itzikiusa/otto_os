<script lang="ts">
  import { rowMenu as rowMenuKeys } from '../../lib/rowMenu';
  // Settings layout: subnav + routed page (#/settings/<page>). The section
  // list itself lives in ./sections.ts (labels, groups, keywords, access) so
  // the nav, each page title and the ⌘K "Settings: …" commands can't drift.
  // Appearance is the default section, so it ships with the page; every other
  // section is its own chunk (see VIEWS below).
  import Appearance from './Appearance.svelte';
  import { router } from '../../lib/router.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent, type LazyComponent } from '../../lib/lazy-component.svelte';
  import { setContext, type Component, type Snippet } from 'svelte';
  import {
    SETTINGS_GROUPS,
    availableSections,
    canOpenSection,
    filterSections,
    findSection,
    groupLabel,
    matchedSetting,
    scopeDescription,
    SCOPE_LABEL,
    type SettingsSection,
    type SettingsSectionId,
  } from './sections';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import ListDetail from '../../lib/components/side/ListDetail.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import PhoneBack from '../../lib/components/side/PhoneBack.svelte';
  import { HEADER_HOST, pageNav, type HeaderHost } from '../../lib/components/side/pageNav.svelte';
  import type { MenuItem } from '../../lib/contextmenu.svelte';
  import { daemonStatus } from './daemonStatus.svelte';
  import { settingsUi } from './settingsUiState.svelte';

  // As a sheet over another page (Threads' footer ⚙), Settings keeps its own
  // section instead of routing, and ✕ closes the sheet.
  let { sheet = null }: { sheet?: { onclose: () => void } | null } = $props();

  // The section nav is the page's list pane (ListDetail, 240 px). On a phone
  // (and a phone on its side) it is two levels, like iOS Settings: the grouped
  // list (with the filter) is the page, a section is pushed with "‹ Settings"
  // in its page header. A deep link (#/settings/<id>) opens the section
  // pushed; bare #/settings is the list. Back keeps the section (marked).
  let phoneOpen = $state(false);

  // Explicit exit: go back to where you were, or home (Agents) if there's no
  // history — so you don't have to click another module to leave Settings.
  function exit(): void {
    if (sheet) sheet.onclose();
    else if (router.canBack) router.back();
    else router.go('agents');
  }
  function goPage(id: string): void {
    phoneOpen = true;
    if (sheet) settingsUi.sheetPage = id;
    else router.go(`settings/${id}`);
  }
  // Desktop / tablet: ONE header across the page (design.md §7.16) — the
  // section's own PageHeader is drawn above the nav and the section as
  // "Settings / <section>", with the nav's ⋯ rows in its ⋯; the nav starts
  // with its filter. A phone keeps the two levels (and a sheet its own head).
  let hostedBar = $state.raw<Snippet | null>(null);
  setContext<HeaderHost>(HEADER_HOST, {
    get active() {
      return !viewport.navDrawer && !sheet;
    },
    crumbs: [{ label: 'Settings', onclick: () => router.go('settings') }],
    more: () => [
      ...(pageNav.show ? [] : [{ label: 'Hide Settings', icon: 'sidebar' as const, hint: '⌘B', action: () => pageNav.toggle?.() }]),
      ...paneMenu(),
    ],
    claim(bar) {
      hostedBar = bar;
      return () => {
        if (hostedBar === bar) hostedBar = null;
      };
    },
  });
  /** The nav's ⋯ (after Hide Settings ⌘B). */
  function paneMenu(): MenuItem[] {
    return sheet ? [] : [{ label: 'Close settings', icon: 'x', hint: '⌘⇧←', action: exit }];
  }

  // Root sees a dot on Trust & safety while secrets sit unencrypted (one
  // cached read shared with the status bar chip; never polled).
  $effect(() => {
    daemonStatus.ensureSecrets(auth.me?.id, auth.isRoot);
  });
  const warnSecrets = (s: SettingsSection): boolean => s.id === 'trust-safety' && daemonStatus.secretsUnencrypted;

  // The section components, keyed by registry id — the only per-section thing
  // that needs Svelte. The Record type makes a new registry entry without a
  // view a compile error. Each section is a lazy chunk (one promise each, see
  // lib/lazy-component.svelte.ts): a static map dragged all 28 into the
  // Settings chunk — CodeMirror via Context Library, the whole Database store
  // via Appearance — for a page that shows one section at a time.
  const VIEWS: Record<SettingsSectionId, LazyComponent<Component>> = {
    appearance: lazyComponent(async () => ({ default: Appearance })),
    'session-names': lazyComponent(() => import('./SessionNames.svelte')),
    notifications: lazyComponent(() => import('./Notifications.svelte')),
    snipping: lazyComponent(() => import('./SnipSettings.svelte')),
    clipboard: lazyComponent(() => import('./ClipboardSettings.svelte')),
    'keep-awake': lazyComponent(() => import('./KeepAwakeSettings.svelte')),
    threads: lazyComponent(() => /* ui-guards: allow — Threads owns its prefs section (its stores) */ import('../threads/ThreadsSettings.svelte')),
    browser: lazyComponent(() => import('./BrowserSettings.svelte')),
    tokens: lazyComponent(() => import('./PersonalAccessTokens.svelte')),
    'git-accounts': lazyComponent(() => import('../git/GitAccounts.svelte')),
    jira: lazyComponent(() => import('./IssueAccounts.svelte')),
    channels: lazyComponent(() => import('./Channels.svelte')),
    'mcp-servers': lazyComponent(() => import('./McpServers.svelte')),
    'language-servers': lazyComponent(() => import('./LanguageServers.svelte')),
    'phone-access': lazyComponent(() => import('./PhoneAccess.svelte')),
    sharing: lazyComponent(() => import('./EmailSenderSetup.svelte')),
    providers: lazyComponent(() => import('./Providers.svelte')),
    'context-soul': lazyComponent(() => import('./ContextSoul.svelte')),
    'self-improvement': lazyComponent(() => import('./SelfImprovement.svelte')),
    insights: lazyComponent(() => import('./InsightsSettings.svelte')),
    skills: lazyComponent(() => import('./SkillsLibrary.svelte')),
    'skill-eval': lazyComponent(() => import('./SkillEvalSettings.svelte')),
    'context-library': lazyComponent(() => import('./ContextLibrary.svelte')),
    daemon: lazyComponent(() => import('./Daemon.svelte')),
    updates: lazyComponent(() => import('./UpdatesSettings.svelte')),
    plugins: lazyComponent(() => import('./PluginsSettings.svelte')),
    'trust-safety': lazyComponent(() => import('./TrustSafety.svelte')),
    logs: lazyComponent(() => import('./Logs.svelte')),
    backup: lazyComponent(() => import('./BackupRestore.svelte')),
    users: lazyComponent(() => import('./Users.svelte')),
    'access-groups': lazyComponent(() => import('./AccessGroups.svelte')),
    sessions: lazyComponent(() => import('./AdminSessions.svelte')),
  };

  const routedId = $derived(sheet ? settingsUi.sheetPage : (router.parts[1] ?? ''));
  const pageId = $derived(routedId || 'appearance');
  $effect(() => {
    phoneOpen = !!routedId;
  });
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
  let filterEl = $state<HTMLInputElement | null>(null);
  let listEl = $state<HTMLDivElement | null>(null);
  const results = $derived(settingsUi.query.trim() ? filterSections(sections, settingsUi.query) : null);

  function open(s: SettingsSection): void {
    goPage(s.id);
  }

  function navItems(): HTMLButtonElement[] {
    return listEl?.closest('.side-list') ? [...listEl.closest('.side-list')!.querySelectorAll<HTMLButtonElement>('.settings-nav-item')] : [];
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
      if (settingsUi.query) settingsUi.query = '';
      else filterEl?.blur();
    } else if (e.key === 'ArrowDown') {
      const first = navItems()[0];
      if (first) {
        e.preventDefault();
        first.focus();
      }
    }
  }

  // The rows walk with the list pane's keys (use:listNav); on top of that ↑
  // from the first row returns to the filter, and typing a character on a row
  // carries it into the filter.
  function onListKey(e: KeyboardEvent): void {
    const items = navItems();
    const i = items.indexOf(document.activeElement as HTMLButtonElement);
    if (i < 0) return;
    if (e.key === 'ArrowUp' && i === 0) {
      e.preventDefault();
      filterEl?.focus();
    } else if (e.key === 'Escape' && settingsUi.query) {
      e.preventDefault();
      e.stopPropagation();
      settingsUi.query = '';
      filterEl?.focus();
    } else if (e.key.length === 1 && e.key !== ' ' && !e.metaKey && !e.ctrlKey && !e.altKey) {
      e.preventDefault();
      settingsUi.query += e.key;
      filterEl?.focus();
    }
  }

  function rowMenu(e: MouseEvent, s: SettingsSection): void {
    ctxMenu.show(e, [{ label: `Open ${s.label}`, icon: 'gear', action: () => open(s) }]);
  }

  /** Plain-words names for the feature grants (never the raw snake_case id). */
  const FEATURE_LABEL: Record<string, string> = {
    agents: 'Agents',
    connections: 'Connections',
    database: 'the Database Explorer',
    git: 'Git',
    issues: 'Issues',
    product: 'Product',
    api_client: 'the API client',
    workflows: 'Workflows',
    channels: 'Channels',
    skill_eval: 'Skills evaluator',
    skills: 'Skills',
    insights: 'Insights',
    usage: 'Usage',
    self_improvement: 'Self-improvement',
    context: 'the Context library',
    settings: 'Settings',
    users: 'Users',
    canvas: 'Canvas',
    design: 'Design Hall',
    mcp: 'the MCP control plane',
    scheduled_tasks: 'Scheduled tasks',
    browser: 'the Browser',
    aws: 'AWS',
    aws_s3: 'AWS S3',
    aws_sqs: 'AWS SQS',
    aws_ec2: 'AWS EC2',
    aws_athena: 'AWS Athena',
    aws_eks: 'AWS EKS',
    aws_rds: 'AWS RDS',
    kubernetes: 'Kubernetes',
  };

  function gateHint(s: SettingsSection): string {
    const g = s.gate;
    if (g === 'root') return 'Only the root account can open this section.';
    if (g == null) return '';
    return `Ask a workspace admin for ${g.level} access to ${FEATURE_LABEL[g.feature] ?? g.feature.replace(/_/g, ' ')}.`;
  }
</script>

{#snippet navRow(s: SettingsSection, detail?: string)}
  {@const match = settingsUi.query.trim() ? matchedSetting(s, settingsUi.query) : null}
  {@const current = pageId === s.id}
  <!-- The shared row look (.side-row); the scope tag and warning dot are
       aria-hidden so the row's name stays its label; both are in the title
       (the row's description). -->
  <div class="side-row" class:two-line={!!match} class:selected={current}>
    <button use:rowMenuKeys
      class="side-row-main settings-nav-item"
      data-side-row
      aria-current={current ? 'page' : undefined}
      title={warnSecrets(s) ? `${scopeDescription(s)} Secrets are not encrypted.` : scopeDescription(s)}
      onclick={() => open(s)}
      onkeydown={onListKey}
      onpointerenter={() => VIEWS[s.id].prefetch()}
      onfocus={() => VIEWS[s.id].prefetch()}
      oncontextmenu={(e) => rowMenu(e, s)}
    >
      <span class="side-row-text">
        <span class="side-row-title settings-nav-label">{s.label}</span>
        {#if match}<span class="side-row-meta settings-nav-match">{match}</span>{/if}
      </span>
      <span class="side-row-trailing">
        {#if warnSecrets(s)}<span class="settings-nav-dot" aria-hidden="true" data-testid="settings-nav-secrets-dot"></span>{/if}
        {#if detail}
          <span class="settings-nav-detail">{detail}</span>
        {:else}
          <span class="settings-nav-scope" aria-hidden="true">{SCOPE_LABEL[s.scope]}</span>
        {/if}
      </span>
    </button>
  </div>
{/snippet}

<svelte:window
  onkeydown={(e) => {
    if (!sheet || e.key !== '/' || e.metaKey || e.ctrlKey || e.altKey) return;
    const t = e.target as HTMLElement | null;
    if (t?.closest('input, textarea, [contenteditable="true"]')) return;
    e.preventDefault();
    filterEl?.focus();
  }}
/>

<div class="settings" class:sheet>
  {#if viewport.navDrawer && !phoneOpen && !sheet}
    <!-- Phone, the list is the page: this header carries ☰ · Settings · ⋯ ·
         Side panel (the section's own PageHeader is hidden with it). -->
    <PageHeader title="Settings" />
  {/if}
  {#if hostedBar && !viewport.navDrawer && !sheet}{@render hostedBar()}{/if}
  <ListDetail module="settings" name="Settings" open={phoneOpen} onback={() => (phoneOpen = false)} defaultWidth={240}>
    {#snippet list()}
      <div class="settings-nav">
        <ListPane as="nav" menu={paneMenu} head={viewport.navDrawer || !!sheet} testid="settings-nav">
          {#snippet searchRow()}
            <label class="side-search settings-nav-filter">
              <Icon name="search" size={14} />
              <input dir="ltr"
                bind:this={filterEl}
                bind:value={settingsUi.query}
                type="search"
                placeholder={sheet ? 'Search settings  /' : 'Filter settings…'}
                aria-label="Filter settings"
                aria-controls="settings-nav-list"
                autocomplete="off"
                spellcheck="false"
                onkeydown={onFilterKey}
              />
            </label>
          {/snippet}
          <div id="settings-nav-list" bind:this={listEl}>
            {#if results}
              <div class="side-section">
                {#each results as s (s.id)}
                  {@render navRow(s, groupLabel(s.group))}
                {:else}
                  <p class="settings-nav-empty">No sections match “{settingsUi.query.trim()}”</p>
                {/each}
              </div>
            {:else}
              {#each groups as g (g.id)}
                <div class="side-section" role="group" aria-label={g.label}>
                  <div class="side-section-head"><span class="ssh-title settings-nav-heading" aria-hidden="true">{g.label}</span></div>
                  {#each g.items as s (s.id)}
                    {@render navRow(s)}
                  {/each}
                </div>
              {/each}
            {/if}
          </div>
        </ListPane>
      </div>
    {/snippet}

    <div class="settings-body">
      {#if sheet && viewport.navDrawer}
        <!-- A sheet has no page header of its own: the one Back sits on top. -->
        <div class="settings-back"><PhoneBack label="Settings" onclick={() => (phoneOpen = false)} /></div>
      {/if}
      {#if View}
        <LazyMount lazy={View} what={`${section?.label ?? 'these'} settings`} />
      {:else if section}
        <PageHeader title={section.label} />
        <PageBody>
          <EmptyState
            variant="page"
            icon="lock"
            title={`You don’t have access to ${section.label}`}
            body={gateHint(section)}
            actionLabel="Open Appearance"
            onaction={() => goPage('appearance')}
          >
            {#if router.canBack && !sheet}
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
            onaction={() => goPage('appearance')}
          />
        </PageBody>
      {/if}
    </div>
  </ListDetail>
</div>

<style>
  .settings {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .settings-nav {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  /* Filter field: a quiet search box pinned above the (scrolling) list. */
  .settings-nav-detail,
  .settings-nav-scope {
    flex-shrink: 0;
  }
  /* Root: Trust & safety while the secret store is unencrypted. */
  .settings-nav-dot {
    flex-shrink: 0;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--warning);
  }
  .settings-nav-empty {
    margin: 0;
    padding: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .settings-back {
    flex: none;
    padding: 6px 8px 0;
  }
  /* Each section owns its chrome (PageHeader) and scroll (PageBody).
     --settings-col: the ONE column width every Settings page caps its form
     cards, intros and hints at, so all 28 pages line up on the same right
     edge (full-width data tables — Sessions, audit log, logs — ignore it). */
  .settings-body {
    --settings-col: 760px;
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  /* app.css gives .section-title an 18px top margin; a page that opens on
     one (Appearance, Notifications, Daemon…) would start 18px lower than a
     page that opens on a SectionIntro. Every page starts on the same line. */
  .settings-body :global(.page-body-inner > .section-title:first-child),
  .settings-body :global(.page-body-inner > :first-child > .section-title:first-child) {
    margin-top: 0;
  }

</style>
