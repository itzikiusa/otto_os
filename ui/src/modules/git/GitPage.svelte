<script lang="ts">
  // Git module page. Workspace-INDEPENDENT: shows GitKraken-style top-level repo
  // tabs (one per open repo) above the active repo's RepoView. With no tab open
  // it shows a full-width landing — the repo browser/list + "Add Repository"
  // flow + EmptyState. Open repos persist across restarts (global localStorage,
  // via the git store). PR detail (#/git/:id/pr/:n) is still routed; a plain
  // #/git/:id/:tab deep-link opens that repo as a tab.
  import { untrack } from 'svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import type { RepoStatusResp } from '../../lib/api/types';
  import type { MenuItem } from '../../lib/contextmenu.svelte';
  import { router } from '../../lib/router.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import GitTabs from './GitTabs.svelte';
  import GitAutoFetchToggle from './GitAutoFetchToggle.svelte';
  import { focusPath, parseGitRoute } from './gitRoute';
  import { prWorkspace } from './prWorkspaceStore.svelte';
  import RepoView from './RepoView.svelte';
  import GitToolbar from './GitToolbar.svelte';
  import { gitBridge } from '../../lib/diff/gitBridge.svelte';
  import { registry } from '../../lib/commands.svelte';
  import FocusView from './FocusView.svelte';
  import GitLanding from './GitLanding.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';

  // The PR detail (+ its review panel, comment threads and merge dialog) is a
  // deep-linked route most Git visits never open: its own chunk (perf R7).
  // Route awareness is limited to PR detail + deep-links into a repo tab.
  // The active repo is driven by the tab store, not the route.
  const activeRepo = $derived(git.allRepos.find((r) => r.id === git.activeRepoId) ?? null);
  const activeStatus = $derived(activeRepo ? (git.statusById[activeRepo.id] ?? null) : null);
  let repoView = $state<{ moreMenuItems: () => MenuItem[]; selectTab: (id: string) => void; viewState: () => { tab: string; resolving: boolean; changes: number; conflicts: number; conflictFiles: number } } | undefined>();
  let toolbar = $state<{ menuItems: () => MenuItem[] } | undefined>();
  // The ONE view switch, in the header (RepoView owns the views).
  const VIEWS = [
    { id: 'changes', label: 'Changes' },
    { id: 'graph', label: 'History' },
    { id: 'prs', label: 'Pull requests' },
    { id: 'review', label: 'Review' },
  ];
  // Header toolbar's stash browser lives here: in PageHeader's actions row the overflow measurer hid it.
  const StashBrowser = lazyComponent(() => import('./StashBrowser.svelte'));
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
        if (Number.isInteger(n) && n > 0) prWorkspace.open(r.repo!, n);
        router.replace('git');
      });
      return;
    }
    if (r.repo && !r.pr && known && (r.tab || r.legacyFocus)) {
      untrack(() => {
        git.openRepoTab(r.repo!, r.tab ?? undefined);
        if (r.legacyFocus) router.replace(focusPath(true));
      });
    }
  });

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

  function openRepo(repoId: string): void {
    git.openRepoTab(repoId);
    // Drop any lingering deep-link route so the tab UI fully owns navigation.
    if (router.parts[0] === 'git' && router.parts.length > 1) router.go('git');
  }

</script>

{#snippet viewSwitch(name: string)}
  {#if repoView}
    {@const v = repoView.viewState()}
    <div class="rv-tabs segmented git-views" role="tablist" aria-label="{name} views" data-keep>
      {#each VIEWS as t (t.id)}
        {@const on = v.tab === t.id && !v.resolving}
        <button class="rv-tab" role="tab" aria-selected={on} tabindex={on ? 0 : -1} class:active={on} onclick={() => repoView?.selectTab(t.id)} onkeydown={onTabKey}>
          {t.label}
          {#if t.id === 'changes' && v.changes > 0}<span class="git-views-count" title="{v.changes} uncommitted {v.changes === 1 ? 'change' : 'changes'}">{v.changes}</span>{/if}
        </button>
      {/each}
      {#if v.resolving}
        <button class="rv-tab active git-views-conflict" role="tab" aria-selected="true" tabindex="0" onkeydown={onTabKey}>Resolve conflicts{#if v.conflictFiles > 0}&nbsp;· {v.conflictFiles}{/if}</button>
      {/if}
    </div>
  {/if}
{/snippet}

  <div class="gitpage">
    <!-- Unified toolbar: the open-repo tabs sit inline in the page header
         (like a document window's tab strip), so an open repo costs no extra
         chrome row. -->
    <PageHeader
      title={focusView ? 'Focus' : 'Git'}
      crumbs={focusView ? [{ label: 'Git', onclick: () => setFocus(false) }] : []}
      class="git-header"
      more={activeRepo && !focusView ? () => [...(toolbar?.menuItems() ?? []), ...(toolbar ? [{ separator: true }] : []), ...(repoView?.moreMenuItems() ?? [])] : undefined}
    >
      {#snippet titleContent()}
        {#if viewport.isPhone}
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
          <GitTabs embedded focus={focusView} ontogglefocus={() => setFocus(!focusView)} onopen={openRepo} onadd={(mode) => { addMode = mode; addOpen = true; }} />
        {:else if activeRepo && !focusView && repoView}
          {@render viewSwitch(activeRepo.name)}
        {/if}
      {/snippet}
      {#snippet actions()}
        {#if activeRepo && !focusView}
          {@const repoId = activeRepo.id}
          <!-- The repo's verbs live in the header (not a second toolbar row under
               it); they collapse into ⋯ by data-overflow when the bar is tight. -->
          {#if viewport.isPhone}<GitAutoFetchToggle />{/if}
          {#if activeStatus}
            <GitToolbar
              inHeader
              bind:this={toolbar}
              {repoId}
              status={activeStatus}
              onstatus={(s) => git.setStatus(repoId, s)}
              onrefresh={() => gitBridge.refreshGraph(repoId)}
              onbrowsestashes={() => (stashesRepo = repoId)}
            >
              {#if repoView && !viewport.isPhone}{@render viewSwitch(activeRepo.name)}{/if}
            </GitToolbar>
          {/if}
        {/if}
      {/snippet}
    </PageHeader>

    {#if stashesRepo}{@const sid = stashesRepo}
      <LazyMount lazy={StashBrowser} what="the stash browser" quiet onerror={() => (stashesRepo = null)} props={{ repoId: sid, onclose: () => (stashesRepo = null), onchanged: (st: RepoStatusResp) => { git.setStatus(sid, st); gitBridge.refreshGraph(sid); } }} />
    {/if}

    {#if focusView}
      <PageBody fill padded={false}>
        <div class="landing-focus">
          <FocusView />
        </div>
      </PageBody>
    {:else if activeRepo}
      {#key activeRepo.id}
        <PageBody fill padded={false}>
        <div class="gitpage-body">
          <RepoView
            repo={activeRepo}
            tab={git.subTabFor(activeRepo.id)}
            embedded
            hostActions
            bind:this={repoView}
            onTab={(t) => git.setSubTab(activeRepo.id, t)}
            onopenrepo={openRepo}
            onaddrepo={(mode) => { addMode = mode; addOpen = true; }}
          />
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
  /* The header's view switch (Changes · History · Pull requests · Review). */
  .git-views {
    flex-shrink: 0;
  }
  .git-views > .rv-tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 24px;
    white-space: nowrap;
  }
  .git-views-count {
    min-width: 16px;
    padding: 0 4px;
    border-radius: 999px;
    background: var(--accent-soft-strong);
    color: var(--accent-text);
    font-size: var(--fs-xs);
    font-weight: 600;
    text-align: center;
  }
  .git-views > .git-views-conflict {
    color: var(--warning);
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
