<script lang="ts">
  // Git module page. Workspace-INDEPENDENT: shows GitKraken-style top-level repo
  // tabs (one per open repo) above the active repo's RepoView. With no tab open
  // it shows a full-width landing — the repo browser/list + "Add Repository"
  // flow + EmptyState. Open repos persist across restarts (global localStorage,
  // via the git store). PR detail (#/git/:id/pr/:n) is still routed; a plain
  // #/git/:id/:tab deep-link opens that repo as a tab.
  import { untrack } from 'svelte';
  import type { Repo, RepoStatusResp } from '../../lib/api/types';
  import { api } from '../../lib/api/client';
  import type { MenuItem } from '../../lib/contextmenu.svelte';
  import { router } from '../../lib/router.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import GitTabs from './GitTabs.svelte';
  import { focusPath, parseGitRoute } from './gitRoute';
  import { gitPhoneMenu } from './gitPhoneMenu.svelte';
  import { prWorkspace } from './prWorkspaceStore.svelte';
  import { gitBridge } from '../../lib/diff/gitBridge.svelte';
  import { registry } from '../../lib/commands.svelte';
  import GitLanding from './GitLanding.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { pageNav } from '../../lib/components/side/pageNav.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import Tabs, { type TabItem } from '../../lib/components/Tabs.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';

  // The PR detail (+ its review panel, comment threads and merge dialog) is a
  // deep-linked route most Git visits never open: its own chunk (perf R7).
  // Route awareness is limited to PR detail + deep-links into a repo tab.
  // The active repo is driven by the tab store, not the route.
  const activeRepo = $derived(git.allRepos.find((r) => r.id === git.activeRepoId) ?? null);
  const activeStatus = $derived(activeRepo ? (git.statusById[activeRepo.id] ?? null) : null);
  let repoView = $state<{ moreMenuItems: () => MenuItem[]; selectTab: (id: string) => void; viewState: () => { tab: string; resolving: boolean; conflictFiles: number } } | undefined>();
  let toolbar = $state<{ menuItems: () => MenuItem[] } | undefined>();
  // The ONE view switch (RepoView owns the views): the page's sections as the
  // shared underline tabs, one row under the header (design.md §7.14); while
  // conflicts are resolved, a "Resolve conflicts" tab is the current one.
  const VIEWS: TabItem[] = [
    { id: 'graph', label: 'History' },
    { id: 'prs', label: 'Pull requests' },
    { id: 'review', label: 'Review' },
  ];
  // Header toolbar's stash browser lives here: in PageHeader's actions row the overflow measurer hid it.
  const StashBrowser = lazyComponent(() => import('./StashBrowser.svelte'));
  // The repository view and Focus load with their first use (perf G6): the
  // landing's list of repositories doesn't pay for the graph, the diff
  // viewer and the commit card.
  const RepoViewLazy = lazyComponent(() => import('./RepoView.svelte'));
  // The verbs load with the repo view, not with the landing's list.
  const GitToolbarLazy = lazyComponent(() => import('./GitToolbar.svelte'));
  // The repo tabs' floor (share of the header row): they shrink (ellipsis,
  // then scroll in their strip) down to it before the verbs' labels give way.
  const TABS_MIN = 0.08;
  const FocusViewLazy = lazyComponent(() => import('./FocusView.svelte'));
  // The Add repository sheet loads on first open.
  const AddRepoSheetLazy = lazyComponent(() => import('./AddRepoSheet.svelte'));
  let stashesRepo = $state<string | null>(null);

  // Focus (my PRs + my Jira work across every repo) is a PAGE-level view, not
  // a repo sub-tab, and it lives in the URL (gitRoute.ts): the Focus entry
  // pinned before the repo tabs goes to #/git/focus; a repo tab, the Git
  // crumb, a repo link inside Focus or Back leave it. The legacy
  // #/git/<repo>/focus opens that repo, then Focus.
  const route = $derived(parseGitRoute(router.parts));
  const focusView = $derived(route.focus);
  function setFocus(on: boolean): void {
    router.go(focusPath(on));
  }

  // add repo sheet
  let addOpen = $state(false);
  let addMode: 'register' | 'clone' | 'browse' = $state('register');

  // ⌘K: adding a repository (the header no longer carries its own button —
  // the + tab and the empty state do). The open repo's verbs come from
  // GitToolbar.
  $effect(() =>
    registry.register('git', [
      { id: 'git.add-local', title: 'Add a local repository…', group: 'Git', keywords: 'register folder repo', run: () => { addMode = 'register'; addOpen = true; } },
      { id: 'git.clone', title: 'Clone a repository…', group: 'Git', keywords: 'remote url github bitbucket gitlab', run: () => { addMode = 'clone'; addOpen = true; } },
    ]),
  );

  // ── Load the GLOBAL repo list + restore open tabs (once, on mount) ─────────
  // Workspace-independent: the page does NOT re-load when the active workspace
  // changes. `restoreOpenTabs` prunes ids that no longer exist. Guarded so it
  // runs a single time even if the effect re-fires.
  let restored = false;
  // A failed list load is `git.allReposError` (the store keeps the last known
  // list), so the landing shows Retry instead of "No repositories yet" — an
  // empty state one click from registering a duplicate.
  async function retryRepos(): Promise<void> {
    await git.loadAllRepos(true);
    if (git.allReposLoaded) await git.initializeOpenTabs();
  }
  $effect(() => {
    if (restored) return;
    restored = true;
    void (async () => {
      await git.initializeOpenTabs();
      // A deep-link into a repo (#/git/:id or #/git/:id/:tab, non-PR) opens that
      // repo as a tab so the route still lands somewhere useful.
      const r = route;
      if (r.repo && !r.pr && git.allRepos.some((x) => x.id === r.repo)) {
        git.openRepoTab(r.repo, r.tab ?? undefined);
        if (r.legacyFocus) router.replace(focusPath(true));
      }
    })();
  });

  // CRITICAL: `openRepoTab` WRITES subTab/activeRepoId/openRepoIds (fresh refs via
  // spread) AND reads them — and this effect tracks those reads — so calling it
  // bare here is a read→write→re-run INFINITE LOOP that froze the whole app on the
  // back-from-PR navigation. `untrack` the mutation so the effect only re-runs when
  // the route (its real deps, read above the untrack) changes.
  $effect(() => {
    const r = route;
    const known = r.repo != null && git.allRepos.some((x) => x.id === r.repo);
    // A PR deep link opens that PR as a tab of the repo's PR workspace.
    if (r.repo && r.pr && known) {
      const n = Number(router.parts[3]);
      untrack(() => {
        git.openRepoTab(r.repo!, 'prs');
        if (Number.isInteger(n) && n > 0) prWorkspace.reveal(r.repo!, n);
        router.replace('git');
      });
      return;
    }
    if (r.repo && !r.pr && known) {
      untrack(() => {
        git.openRepoTab(r.repo!, r.tab ?? undefined);
        if (r.legacyFocus) router.replace(focusPath(true));
      });
    }
  });

  // A deep link to an id /git/repos doesn't list (a duplicate row it
  // dropped, a removed repo): open the registered repo at the same path,
  // else say so — never leave the previous repo on screen under that URL.
  let missingRepo = $state<string | null>(null);
  $effect(() => {
    const r = route;
    if (!r.repo || !git.allReposLoaded || git.allRepos.some((x) => x.id === r.repo)) {
      missingRepo = null;
      return;
    }
    const id = r.repo;
    untrack(() =>
      void api.get<Repo>(`/repos/${id}`).then(
        (dup) => {
          // The user may have moved on while this was in flight.
          if (router.parts[0] !== 'git' || router.parts[1] !== id) return;
          const same = git.allRepos.find((x) => x.path === dup.path);
          if (same) router.replace(['git', same.id, ...router.parts.slice(2)].join('/'));
          else missingRepo = id;
        },
        () => (missingRepo = id),
      ),
    );
  });
  const notFound = $derived(missingRepo !== null && route.repo === missingRepo);

  // Phone, a pull request / file / graph pushed from its list: the header
  // says "‹ Pull requests · #482 title" (the ListDetail's title) and the
  // repository switcher moves under ⋯, as on the list page.
  const pushed = $derived(viewport.navDrawer && !!pageNav.back);
  /** Phone: the title's repo switcher — Focus, the open repos, any other, add. */
  function phoneRepoMenu(e: MouseEvent): void {
    const open = git.openRepoIds.map((id) => git.allRepos.find((r) => r.id === id)).filter((r) => !!r);
    const rest = git.allRepos.filter((r) => !git.openRepoIds.includes(r.id));
    ctxMenu.show(e, [
      { label: 'Focus', icon: 'zap', checked: focusView, pinned: true, action: () => setFocus(true) },
      ...open.map((r) => ({ label: r.name, icon: 'branch', checked: !focusView && r.id === git.activeRepoId, pinned: true, action: () => openRepo(r.id) })),
      { label: 'Close this tab', icon: 'x', pinned: true, disabled: !activeRepo || focusView, action: () => activeRepo && git.closeRepoTab(activeRepo.id) },
      { label: 'Add or clone a repository…', icon: 'plus', pinned: true, action: () => { addMode = 'register'; addOpen = true; } },
      ...(rest.length ? [{ separator: true }] : []),
      ...rest.map((r) => ({ label: r.name, icon: 'branch', action: () => openRepo(r.id) })),
    ], rest.length > 8 ? { filter: true, filterPlaceholder: 'Open a repository…', maxVisible: 12 } : undefined);
  }

  // A phone list page draws the list pane's header in this one row: the
  // repository switcher (the title) moves under its ⋯ (gitPhoneMenu); the
  // view switch keeps its row under the header.
  $effect(() =>
    gitPhoneMenu.set(() => {
      const open = git.openRepoIds.map((id) => git.allRepos.find((r) => r.id === id)).filter((r) => !!r);
      return [
        {
          label: 'Repository',
          icon: 'branch',
          children: [
            { label: 'Focus', icon: 'zap', checked: focusView, action: () => setFocus(true) },
            ...open.map((r) => ({ label: r.name, icon: 'branch' as const, checked: !focusView && r.id === git.activeRepoId, action: () => openRepo(r.id) })),
            { separator: true },
            { label: 'Add or clone a repository…', icon: 'plus', action: () => { addMode = 'register'; addOpen = true; } },
          ],
        },
      ];
    }),
  );

  function openRepo(repoId: string): void {
    git.openRepoTab(repoId);
    // Drop any lingering deep-link route so the tab UI fully owns navigation.
    if (router.parts[0] === 'git' && router.parts.length > 1) router.go('git');
  }

</script>

  <div class="gitpage">
    <!-- Unified toolbar: the open-repo tabs sit inline in the page header
         (like a document window's tab strip), so an open repo costs no extra
         chrome row. -->
    <PageHeader
      title={focusView ? 'Focus' : 'Git'}
      crumbs={focusView ? [{ label: 'Git', onclick: () => setFocus(false) }] : []}
      class="git-header"
      tabsMin={TABS_MIN}
      more={activeRepo && !focusView && !notFound ? () => [...(toolbar?.menuItems() ?? []), ...(toolbar ? [{ separator: true }] : []), ...(repoView?.moreMenuItems() ?? []), ...(pushed ? gitPhoneMenu.items() : [])] : undefined}
    >
      {#snippet titleContent()}
        {#if pushed}
          <!-- Phone, an item pushed from a list: the repository switcher is under ⋯. -->
          {activeRepo?.name ?? 'Git'}
        {:else if viewport.isPhone}
          <!-- Phone: the repo is the title (a switcher), so the header stays one row. -->
          <button type="button" class="git-phone-repo" aria-haspopup="menu" data-testid="git-phone-repo" onclick={phoneRepoMenu}>
            <span class="git-phone-repo-name">{focusView ? 'Focus' : (activeRepo?.name ?? 'Git')}</span><Icon name="chevronDown" size={14} />
          </button>
        {:else}
          {focusView ? 'Focus' : 'Git'}
        {/if}
      {/snippet}
      {#snippet tabs()}
        {#if !viewport.isPhone}
          <GitTabs embedded focus={focusView} ontogglefocus={() => setFocus(!focusView)} onswitch={phoneRepoMenu} onopen={openRepo} onadd={(mode) => { addMode = mode; addOpen = true; }} />
        {/if}
      {/snippet}
      {#snippet actions()}
        {#if activeRepo && !focusView && !notFound}
          {@const repoId = activeRepo.id}
          <!-- The repo's verbs live in the header (not a second toolbar row under
               it); they collapse into ⋯ by data-overflow when the bar is tight. -->
          {#if activeStatus && GitToolbarLazy.component}
            {@const GitToolbar = GitToolbarLazy.component}
            <GitToolbar
              variant="header"
              tabsMin={TABS_MIN}
              bind:this={toolbar}
              {repoId}
              status={activeStatus}
              accent
              onstatus={(s) => git.setStatus(repoId, s)}
              onrefresh={() => gitBridge.refreshGraph(repoId)}
              onbrowsestashes={() => (stashesRepo = repoId)}
            />
          {/if}
        {/if}
      {/snippet}
    </PageHeader>

    {#if activeRepo && !focusView && !notFound && repoView}
      {@const v = repoView?.viewState() ?? { tab: 'graph', resolving: false, conflictFiles: 0 }}
      <div class="rv-tabs git-views chrome-material">
        <Tabs label="{activeRepo.name} views" tabs={v.resolving ? [...VIEWS, { id: 'resolve', label: 'Resolve conflicts', count: v.conflictFiles || undefined }] : VIEWS}
          value={v.resolving ? 'resolve' : v.tab} onchange={(id) => id !== 'resolve' && repoView?.selectTab(id)} />
      </div>
    {/if}

    <!-- Phone: the everyday verbs are a row of their own under the header (History). -->
    {#if viewport.isPhone && activeRepo && !focusView && !notFound && activeStatus && repoView && GitToolbarLazy.component && repoView.viewState().tab === 'graph' && !repoView.viewState().resolving}
      {@const repoId = activeRepo.id}
      {@const GitToolbar = GitToolbarLazy.component}
      <GitToolbar {repoId} status={activeStatus} onstatus={(s) => git.setStatus(repoId, s)} onrefresh={() => gitBridge.refreshGraph(repoId)} onbrowsestashes={() => (stashesRepo = repoId)} />
    {/if}

    {#if stashesRepo}{@const sid = stashesRepo}
      <LazyMount lazy={StashBrowser} what="the stash browser" quiet onerror={() => (stashesRepo = null)} props={{ repoId: sid, onclose: () => (stashesRepo = null), onchanged: (st: RepoStatusResp) => { git.setStatus(sid, st); gitBridge.refreshGraph(sid); } }} />
    {/if}

    {#if notFound}
      <PageBody>
        <EmptyState
          variant="page"
          icon="branch"
          title="Repository not found"
          body="This link names a repository that isn’t registered — it may have been removed."
          actionLabel="Show repositories"
          onaction={() => router.go('git')}
        />
      </PageBody>
    {:else if focusView}
      <PageBody fill padded={false}>
        <div class="landing-focus">
          <LazyMount lazy={FocusViewLazy} what="Focus" />
        </div>
      </PageBody>
    {:else if activeRepo}
      {#key activeRepo.id}
        <PageBody fill padded={false}>
        <div class="gitpage-body">
          {#if RepoViewLazy.component}
          {@const RepoView = RepoViewLazy.component}
          <RepoView
            repo={activeRepo}
            tab={git.subTabFor(activeRepo.id)}
            bind:this={repoView}
            onTab={(t) => git.setSubTab(activeRepo.id, t)}
          />
          {:else}
            <LoadState what={activeRepo.name} variant="page" loading={!RepoViewLazy.error} error={RepoViewLazy.error} rows={5} onretry={() => RepoViewLazy.retry()} />
          {/if}
        </div>
        </PageBody>
      {/key}
    {:else}
      <!-- ── Compact, centered hub (no tab open). Adding/opening also live in
           the + tab, so this is just a tidy launcher, not a separate screen. ── -->
      <PageBody>
        <LoadState
          what="repositories"
          variant="page"
          loading={!git.allReposLoaded && (git.loading || !git.allReposError)}
          error={git.allReposError}
          empty={git.allRepos.length === 0}
          rows={3}
          onretry={() => void retryRepos()}
        >
          {#snippet emptyView()}
            <EmptyState
              variant="page"
              icon="branch"
              title="No repositories yet"
              body="Register an existing local repo or clone one from GitHub, Bitbucket or GitLab."
              actionLabel="Add repository"
              actionIcon="plus"
              onaction={() => (addOpen = true)}
            />
          {/snippet}
          <GitLanding onopen={(id, sub) => { git.openRepoTab(id, sub); if (router.parts[0] === 'git' && router.parts.length > 1) router.go('git'); }} />
        </LoadState>
      </PageBody>
    {/if}
  </div>

{#if addOpen}
  <LazyMount lazy={AddRepoSheetLazy} what="the Add repository sheet" quiet onerror={() => (addOpen = false)} props={{ mode: addMode, onclose: () => (addOpen = false), onregistered: openRepo }} />
{/if}

<style>
  /* Full-bleed: tabs + active repo / landing fill the whole center column.
     No `.page` max-width or reserved right column. */
  .gitpage {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  /* The page's sections row (History · Pull requests · Review). */
  .git-views {
    flex-shrink: 0;
  }
  /* Touch: the header's ⋯ is finger-sized (44 px). */
  @media (pointer: coarse) {
    .gitpage :global(.ph-more) { min-inline-size: 44px; min-block-size: 44px; }
  }
  .git-phone-repo {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    min-height: 44px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }
  .git-phone-repo-name {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .gitpage-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  /* Focus view opened from the landing (no repo tab): slim header + the view. */
  .landing-focus {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
