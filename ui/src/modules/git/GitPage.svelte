<script lang="ts">
  import { toastError } from '../../lib/toastError';
  // Git module page. Workspace-INDEPENDENT: shows GitKraken-style top-level repo
  // tabs (one per open repo) above the active repo's RepoView. With no tab open
  // it shows a full-width landing — the repo browser/list + "Add Repository"
  // flow + EmptyState. Open repos persist across restarts (global localStorage,
  // via the git store). PR detail (#/git/:id/pr/:n) is still routed; a plain
  // #/git/:id/:tab deep-link opens that repo as a tab.
  import { untrack } from 'svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { GitAccount, GitProviderKind, Repo, RepoStatusResp } from '../../lib/api/types';
  import type { MenuItem } from '../../lib/contextmenu.svelte';
  import { router } from '../../lib/router.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import GitTabs from './GitTabs.svelte';
  import { focusPath, parseGitRoute } from './gitRoute';
  import { prWorkspace } from './prWorkspaceStore.svelte';
  import RepoView from './RepoView.svelte';
  import GitToolbar from './GitToolbar.svelte';
  import { gitBridge } from '../../lib/diff/gitBridge.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { graphCache } from './graph-cache';
  import FocusView from './FocusView.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
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
  let accounts: GitAccount[] = $state([]);

  /** Display name for the forge chip (the enum is lowercase: "bitbucket"). */
  const PROVIDER_LABEL: Record<string, string> = {
    github: 'GitHub',
    bitbucket: 'Bitbucket',
    gitlab: 'GitLab',
  };

  // Landing-hub repo filter: matches name, local path, or remote URL.
  let repoFilter = $state('');
  const filteredRepos = $derived.by(() => {
    const q = repoFilter.trim().toLowerCase();
    if (q === '') return git.allRepos;
    return git.allRepos.filter(
      (r) =>
        r.name.toLowerCase().includes(q) ||
        r.path.toLowerCase().includes(q) ||
        (r.remote_url ?? '').toLowerCase().includes(q),
    );
  });

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

  function openRepo(repoId: string): void {
    git.openRepoTab(repoId);
    // Drop any lingering deep-link route so the tab UI fully owns navigation.
    if (router.parts[0] === 'git' && router.parts.length > 1) router.go('git');
  }

  // The hosting accounts behind each card's account picker, read once.
  $effect(() => {
    const ctl = new AbortController();
    void api.get<GitAccount[]>('/git/accounts', ctl.signal).then((a) => { if (!ctl.signal.aborted) accounts = a; }).catch(() => {});
    return () => ctl.abort();
  });
  /** Accounts that could serve a repo on `provider` (a token is provider-bound). */
  function accountsFor(provider: GitProviderKind | null): GitAccount[] {
    return provider === null ? [] : accounts.filter((a) => a.provider === provider);
  }

  /** (Re)bind a repo to a hosting account. Registration resolves the account
   *  once, so a repo added before its account existed needs this — otherwise
   *  every provider call fails with "repo has no git account". */
  async function setRepoAccount(r: Repo, accountId: string): Promise<void> {
    try {
      const updated = await api.patch<Repo>(`/repos/${r.id}`, {
        git_account_id: accountId === '' ? null : accountId,
      });
      await git.loadAllRepos(true);
      toasts.success(
        updated.git_account_id ? 'Account linked' : 'Account unlinked',
        r.name,
      );
    } catch (e) {
      toastError('Couldn’t link account', e);
      // Snap the <select> back to the stored binding — the change never landed.
      await git.loadAllRepos(true);
    }
  }

  async function removeRepo(r: Repo): Promise<void> {
    if (!(await confirmer.ask(`Remove “${r.name}” from Otto? Its tab closes and its saved review findings are deleted. The folder and its git history on disk are not touched, so you can add it again later.`, { title: 'Remove repository', confirmLabel: 'Remove', danger: true }))) return;
    try {
      await api.del(`/repos/${r.id}`);
      git.closeRepoTab(r.id);
      graphCache.drop(r.id);
      await git.loadAllRepos(true);
      toasts.info('Repository removed', r.name);
    } catch (e) {
      toastError('Couldn’t remove the repository', e);
    }
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
      more={activeRepo && !focusView ? () => [...(toolbar?.menuItems() ?? []), ...(toolbar ? [{ separator: true }] : []), ...(repoView?.moreMenuItems() ?? [])] : undefined}
    >
      {#snippet tabs()}
        <GitTabs embedded focus={focusView} ontogglefocus={() => setFocus(!focusView)} onopen={openRepo} onadd={(mode) => { addMode = mode; addOpen = true; }} />
      {/snippet}
      {#snippet actions()}
        {#if activeRepo && !focusView}
          {@const repoId = activeRepo.id}
          <!-- The repo's verbs live in the header (not a second toolbar row under
               it); they collapse into ⋯ by data-overflow when the bar is tight. -->
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
              {#if repoView}
                {@const v = repoView.viewState()}
                <div class="rv-tabs segmented git-views" role="tablist" aria-label="{activeRepo.name} views" data-keep>
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
          <div class="landing-inner">
            <div class="landing-hint">
              <span class="section-title landing-count">Repositories · {git.allRepos.length}</span>
              <span class="dim">Open one to see its graph, pull requests and reviews.</span>
            </div>
            <div class="repo-search">
              <Icon name="search" size={13} />
              <input dir="ltr"
                class="repo-search-input"
                type="search"
                bind:value={repoFilter}
                placeholder="Filter repositories by name, path or remote…"
                aria-label="Filter repositories"
                spellcheck="false"
                onkeydown={(e) => {
                  if (e.key === 'Escape' && repoFilter !== '') {
                    e.stopPropagation();
                    repoFilter = '';
                  }
                }}
              />
              {#if repoFilter.trim() !== ''}
                <span class="repo-search-count dim">
                  {filteredRepos.length} / {git.allRepos.length}
                </span>
              {/if}
            </div>
            {#if filteredRepos.length === 0}
              <div class="repo-search-empty dim">
                No repositories match “{repoFilter.trim()}”.
              </div>
            {/if}
            <div class="repo-grid">
              {#each filteredRepos as r (r.id)}
              <div class="repo-card card">
                <button class="repo-main" onclick={() => openRepo(r.id)}>
                  <div class="repo-name">
                    <Icon name="branch" size={14} />
                    <span class="repo-name-text" title={r.name}>{r.name}</span>
                    {#if r.provider}<span class="chip repo-provider">{PROVIDER_LABEL[r.provider] ?? r.provider}</span>{/if}
                  </div>
                  <div class="repo-path mono" title={r.path}>{r.path}</div>
                  {#if r.remote_url}<div class="repo-remote mono dim" title={r.remote_url}>{r.remote_url}</div>{/if}
                </button>
                <div class="repo-actions">
                  <button class="btn small" onclick={() => git.openRepoTab(r.id, 'prs')} title="Open {r.name} on its pull requests">
                    <Icon name="pr" size={12} /> Pull requests
                  </button>
                  {#if r.provider}
                    <!-- Which hosting credential this repo talks to. Unbound is a
                         real failure mode (PRs/reviewers 400), so it reads as a
                         warning rather than a blank field. -->
                    <select
                      class="repo-account"
                      class:unbound={!r.git_account_id}
                      title={r.git_account_id
                        ? 'Git account used for pull requests'
                        : `No ${PROVIDER_LABEL[r.provider] ?? r.provider} account linked — pull requests will fail`}
                      aria-label="Git account for {r.name}"
                      value={r.git_account_id ?? ''}
                      onchange={(e) => setRepoAccount(r, e.currentTarget.value)}
                    >
                      <option value="">No account</option>
                      {#each accountsFor(r.provider) as a (a.id)}
                        <option value={a.id}>{a.label}</option>
                      {/each}
                    </select>
                  {/if}
                  <span class="grow"></span>
                  <button class="icon-btn" title="Remove {r.name} from Otto" aria-label="Remove {r.name} from Otto" onclick={() => removeRepo(r)}>
                    <Icon name="trash" size={14} />
                  </button>
                </div>
              </div>
            {/each}
            </div>
          </div>
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
  /* The repo hub follows the one page-width rule: full width, left-aligned
     with the header title. */
  .landing-inner {
    width: 100%;
  }
  .landing-hint {
    display: flex;
    align-items: baseline;
    flex-wrap: wrap;
    gap: 4px 10px;
    font-size: var(--fs-s);
    margin-bottom: 10px;
  }
  .landing-count {
    margin: 0;
  }
  /* Filter row above the repo grid — styled like an input but with an inline
     leading icon + live match count, so it reads as part of the hub chrome. */
  .repo-search {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 10px;
    margin-bottom: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text-dim);
    transition: border-color var(--dur-fast) ease-out;
  }
  .repo-search:focus-within {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .repo-search-input {
    flex: 1;
    min-width: 0;
    height: 34px;
    border: none;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    outline: none;
  }
  .repo-search-count {
    font-size: var(--fs-xs);
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
  }
  .repo-search-empty {
    font-size: var(--fs-s);
    padding: 18px 4px;
    text-align: center;
  }
  .repo-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 12px;
  }
  .repo-card {
    display: flex;
    flex-direction: column;
    transition: border-color var(--dur-fast) ease-out;
  }
  .repo-card:hover {
    border-color: color-mix(in srgb, var(--accent) 35%, var(--border));
  }
  /* Grows so every card's action row sits on the same baseline, whether or
     not the repo has a remote line. */
  .repo-main {
    flex: 1;
    text-align: start;
    border: none;
    background: transparent;
    padding: 14px 14px 6px;
    cursor: pointer;
    color: var(--text);
  }
  .repo-name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-m);
    font-weight: 600;
    min-width: 0;
  }
  .repo-name > :global(svg),
  .repo-provider {
    flex-shrink: 0;
  }
  /* Long repo names clip with an ellipsis instead of widening the card (on the
     text span: a bare text node in a flex row is cut mid-character). */
  .repo-name-text {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .repo-path {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-top: 4px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .repo-remote {
    font-size: var(--fs-xs);
    margin-top: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .repo-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px 10px;
    padding-inline-start: 14px;
  }

  /* Account picker sits inline with the card actions; capped so a long label
     can't stretch the (grid-sized) card. */
  .repo-account {
    max-width: 140px;
    height: 24px;
    padding: 0 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }

  .repo-account.unbound {
    color: var(--warning);
    border-color: var(--warning);
    background: var(--warning-soft);
  }

  /* ── Mobile + tablet (≤1024px): tighten the landing hub so the repo cards +
     add-flow fit a phone without horizontal overflow — single-column grid,
     stacked header, lighter padding, and full-width segmented/path rows inside
     the Add Repository modal. The modal box itself is already viewport-clamped
     (Modal.svelte: min(width, 100vw-24px)). ── */
  @media (max-width: 1024px) {
    /* One column on phone — minmax(320px) would otherwise force a track wider
       than the content box on a 375px screen. */
    .repo-grid {
      grid-template-columns: 1fr;
    }
    .repo-search-input {
      min-height: 38px;
    }
    .repo-actions .btn,
    .repo-actions .icon-btn {
      min-height: 36px;
    }
  }
</style>
