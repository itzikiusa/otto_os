<script lang="ts">
  // Git module page. Workspace-INDEPENDENT: shows GitKraken-style top-level repo
  // tabs (one per open repo) above the active repo's RepoView. With no tab open
  // it shows a full-width landing — the repo browser/list + "Add Repository"
  // flow + EmptyState. Open repos persist across restarts (global localStorage,
  // via the git store). PR detail (#/git/:id/pr/:n) is still routed; a plain
  // #/git/:id/:tab deep-link opens that repo as a tab.
  import { api } from '../../lib/api/client';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { GitAccount, Repo, RemoteRepoSummary } from '../../lib/api/types';
  import { router } from '../../lib/router.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import GitTabs from './GitTabs.svelte';
  import RepoView from './RepoView.svelte';
  import PrDetail from './PrDetail.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import Icon from '../../lib/components/Icon.svelte';

  // Route awareness is now limited to PR detail + deep-links into a repo tab.
  const routeRepoId = $derived(router.parts[1] ?? null);
  const isPr = $derived(router.parts[2] === 'pr' && router.parts[3] !== undefined);
  const routeTab = $derived(router.parts[2] ?? null);

  // The active repo is driven by the tab store, not the route.
  const activeRepo = $derived(git.allRepos.find((r) => r.id === git.activeRepoId) ?? null);
  const prRepo = $derived(git.allRepos.find((r) => r.id === routeRepoId) ?? null);

  // add repo sheet
  let addOpen = $state(false);
  let addMode: 'register' | 'clone' | 'browse' = $state('register');
  let addPath = $state('');
  let addUrl = $state('');
  let addName = $state('');
  let addAccount = $state('');
  let accounts: GitAccount[] = $state([]);
  let busy = $state(false);
  let pickerOpen = $state(false);

  // "Browse remote" mode: search repos under a git account's namespace.
  let browseAccount = $state('');
  let browseQuery = $state('');
  let remoteRepos: RemoteRepoSummary[] = $state([]);
  let remoteLoading = $state(false);
  let remoteError = $state('');
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  const accountsWithNs = $derived(accounts.filter((a) => a.namespace));

  $effect(() => {
    // default the browse account to the first one that has a namespace
    if (browseAccount === '' && accountsWithNs.length > 0) browseAccount = accountsWithNs[0].id;
  });

  // ── Load the GLOBAL repo list + restore open tabs (once, on mount) ─────────
  // Workspace-independent: the page does NOT re-load when the active workspace
  // changes. `restoreOpenTabs` prunes ids that no longer exist. Guarded so it
  // runs a single time even if the effect re-fires.
  let restored = false;
  $effect(() => {
    if (restored) return;
    restored = true;
    void (async () => {
      await git.loadAllRepos();
      git.restoreOpenTabs();
      // A deep-link into a repo (#/git/:id or #/git/:id/:tab, non-PR) opens that
      // repo as a tab so the route still lands somewhere useful.
      if (routeRepoId && !isPr && git.allRepos.some((r) => r.id === routeRepoId)) {
        git.openRepoTab(routeRepoId, routeTab ?? undefined);
      }
    })();
  });

  // PrDetail's "back to PRs" routes to #/git/:id/prs; honour that as a tab open.
  $effect(() => {
    if (routeRepoId && !isPr && routeTab && git.allRepos.some((r) => r.id === routeRepoId)) {
      git.openRepoTab(routeRepoId, routeTab);
    }
  });

  function openRepo(repoId: string): void {
    git.openRepoTab(repoId);
    // Drop any lingering deep-link route so the tab UI fully owns navigation.
    if (router.parts[0] === 'git' && router.parts.length > 1) router.go('git');
  }

  function scheduleRemoteSearch(): void {
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void runRemoteSearch(), 350);
  }

  async function runRemoteSearch(): Promise<void> {
    if (browseAccount === '') return;
    remoteLoading = true;
    remoteError = '';
    try {
      const q = browseQuery.trim() === '' ? '' : `?q=${encodeURIComponent(browseQuery.trim())}`;
      remoteRepos = await api.get<RemoteRepoSummary[]>(`/git/accounts/${browseAccount}/remote-repos${q}`);
    } catch (e) {
      remoteError = e instanceof Error ? e.message : String(e);
      remoteRepos = [];
    } finally {
      remoteLoading = false;
    }
  }

  async function cloneRemote(repo: RemoteRepoSummary): Promise<void> {
    if (!ws.currentId || busy) return;
    busy = true;
    try {
      const created = await api.post<Repo>(`/workspaces/${ws.currentId}/repos`, {
        clone_url: repo.clone_url,
        name: repo.name || null,
        git_account_id: browseAccount || null,
      });
      toasts.success('Clone started', created.name);
      addOpen = false;
      await git.loadAllRepos(true);
    } catch (e) {
      toasts.error('Clone failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  $effect(() => {
    if (addOpen) {
      void api
        .get<GitAccount[]>('/git/accounts')
        .then((a) => (accounts = a))
        .catch(() => (accounts = []));
    }
  });

  async function addRepo(): Promise<void> {
    if (!ws.currentId) return;
    busy = true;
    try {
      const body =
        addMode === 'register'
          ? { path: addPath.trim(), name: addName.trim() || null }
          : {
              clone_url: addUrl.trim(),
              name: addName.trim() || null,
              git_account_id: addAccount === '' ? null : addAccount,
            };
      const repo = await api.post<Repo>(`/workspaces/${ws.currentId}/repos`, body);
      toasts.success(addMode === 'clone' ? 'Clone started' : 'Repo registered', repo.name);
      addOpen = false;
      addPath = addUrl = addName = '';
      await git.loadAllRepos(true);
      // Newly registered (not async-cloning) repos open straight into a tab.
      if (addMode === 'register') openRepo(repo.id);
    } catch (e) {
      toasts.error('Add failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  async function removeRepo(r: Repo): Promise<void> {
    if (!(await confirmer.ask(`Unregister “${r.name}”? Files on disk are not touched.`, { title: 'Unregister repo', confirmLabel: 'Unregister' }))) return;
    try {
      await api.del(`/repos/${r.id}`);
      git.closeRepoTab(r.id);
      await git.loadAllRepos(true);
      toasts.info('Repo unregistered', r.name);
    } catch (e) {
      toasts.error('Remove failed', e instanceof Error ? e.message : String(e));
    }
  }
</script>

{#if prRepo && isPr}
  <!-- PR detail is still routed (deep-linkable). -->
  <PrDetail repoId={prRepo.id} number={Number(router.parts[3])} />
{:else}
  <div class="gitpage">
    <GitTabs onopen={openRepo} onadd={() => (addOpen = true)} />

    {#if activeRepo}
      {#key activeRepo.id}
        <div class="gitpage-body">
          <RepoView
            repo={activeRepo}
            tab={git.subTabFor(activeRepo.id)}
            embedded
            onTab={(t) => git.setSubTab(activeRepo.id, t)}
          />
        </div>
      {/key}
    {:else}
      <!-- ── Compact, centered hub (no tab open). Adding/opening also live in
           the + tab, so this is just a tidy launcher, not a separate screen. ── -->
      <div class="landing">
        {#if git.loading && !git.allReposLoaded}
          <Skeleton rows={3} height={56} />
        {:else if git.allRepos.length === 0}
          <EmptyState
            icon="branch"
            title="No repositories yet"
            body="Register an existing local repo or clone one from GitHub, Bitbucket or GitLab."
            actionLabel="Add Repository"
            onaction={() => (addOpen = true)}
          />
        {:else}
          <div class="landing-inner">
            <div class="landing-head">
              <div>
                <h2 class="landing-title">No repository open</h2>
                <div class="landing-hint">
                  Open one below, or use the <strong>+</strong> tab — you can also add a
                  new repository there.
                </div>
              </div>
              <button class="btn primary" onclick={() => (addOpen = true)}>
                <Icon name="plus" size={12} /> Add Repository
              </button>
            </div>
            <div class="repo-grid">
              {#each git.allRepos as r (r.id)}
              <div class="repo-card card">
                <button class="repo-main" onclick={() => openRepo(r.id)}>
                  <div class="repo-name">
                    <Icon name="branch" size={14} />
                    {r.name}
                    {#if r.provider}<span class="chip">{r.provider}</span>{/if}
                  </div>
                  <div class="repo-path mono">{r.path}</div>
                  {#if r.remote_url}<div class="repo-remote mono dim">{r.remote_url}</div>{/if}
                </button>
                <div class="repo-actions">
                  <button class="btn small" onclick={() => git.openRepoTab(r.id, 'prs')}>
                    <Icon name="pr" size={11} /> PRs
                  </button>
                  <span class="grow"></span>
                  <button class="icon-btn" title="Unregister" onclick={() => removeRepo(r)}>
                    <Icon name="trash" size={13} />
                  </button>
                </div>
              </div>
            {/each}
            </div>
          </div>
        {/if}
      </div>
    {/if}
  </div>
{/if}

{#if addOpen}
  <Modal title="Add Repository" onclose={() => (addOpen = false)}>
    <div class="segmented" style="margin-bottom: 14px">
      <button class:active={addMode === 'register'} onclick={() => (addMode = 'register')}>
        Register local path
      </button>
      <button class:active={addMode === 'browse'} onclick={() => { addMode = 'browse'; if (remoteRepos.length === 0) void runRemoteSearch(); }}>
        Browse remote
      </button>
      <button class:active={addMode === 'clone'} onclick={() => (addMode = 'clone')}>Clone URL</button>
    </div>

    {#if addMode === 'register'}
      <div class="field">
        <label for="ar-path">Path</label>
        <div class="path-row">
          <input id="ar-path" class="input mono" bind:value={addPath} placeholder="/Users/you/code/repo" spellcheck="false" />
          <button class="btn" type="button" onclick={() => (pickerOpen = true)}>Browse…</button>
        </div>
      </div>
    {:else if addMode === 'browse'}
      {#if accountsWithNs.length === 0}
        <EmptyState
          icon="key"
          title="No namespace configured"
          body="Add a git account with an organisation / workspace / group in Settings → Git Accounts to browse its repositories."
        />
      {:else}
        <div class="field">
          <label for="ar-bacct">Account</label>
          <select id="ar-bacct" class="input" bind:value={browseAccount} onchange={() => { remoteRepos = []; void runRemoteSearch(); }}>
            {#each accountsWithNs as a (a.id)}
              <option value={a.id}>{a.label} · {a.namespace} ({a.provider})</option>
            {/each}
          </select>
        </div>
        <div class="field">
          <label for="ar-bq">Search repositories</label>
          <input id="ar-bq" class="input" bind:value={browseQuery} oninput={scheduleRemoteSearch} placeholder="filter by name…" spellcheck="false" />
        </div>
        <div class="remote-list">
          {#if remoteLoading}
            <div class="dim pad">Searching…</div>
          {:else if remoteError}
            <div class="err pad">{remoteError}</div>
          {:else if remoteRepos.length === 0}
            <div class="dim pad">No repositories found.</div>
          {:else}
            {#each remoteRepos as r (r.full_name)}
              <div class="remote-row">
                <div class="grow min0">
                  <div class="remote-name">
                    {r.name}
                    {#if r.private}<span class="chip">private</span>{/if}
                  </div>
                  {#if r.description}<div class="remote-desc dim ellipsis">{r.description}</div>{/if}
                  <div class="remote-full mono dim ellipsis">{r.full_name}</div>
                </div>
                <button class="btn small" disabled={busy} onclick={() => cloneRemote(r)}>Clone</button>
              </div>
            {/each}
          {/if}
        </div>
      {/if}
    {:else}
      <div class="field">
        <label for="ar-url">Clone URL</label>
        <input id="ar-url" class="input mono" bind:value={addUrl} placeholder="git@github.com:org/repo.git" spellcheck="false" />
      </div>
      <div class="field">
        <label for="ar-acct">Git account <span class="dim">(for https auth)</span></label>
        <select id="ar-acct" class="input" bind:value={addAccount}>
          <option value="">none (public / ssh agent)</option>
          {#each accounts as a (a.id)}
            <option value={a.id}>{a.label} ({a.provider})</option>
          {/each}
        </select>
      </div>
    {/if}

    {#if addMode !== 'browse'}
      <div class="field">
        <label for="ar-name">Name <span class="dim">(optional)</span></label>
        <input id="ar-name" class="input" bind:value={addName} />
      </div>
    {/if}

    {#snippet footer()}
      <button class="btn" onclick={() => (addOpen = false)}>Cancel</button>
      {#if addMode !== 'browse'}
        <button
          class="btn primary"
          disabled={busy || (addMode === 'register' ? addPath.trim() === '' : addUrl.trim() === '')}
          onclick={addRepo}
        >
          {busy ? 'Working…' : addMode === 'clone' ? 'Clone' : 'Register'}
        </button>
      {/if}
    {/snippet}
  </Modal>
{/if}

{#if pickerOpen}
  <FolderPicker
    title="Choose a git repository"
    start={addPath}
    gitOnly
    onpick={(p) => {
      addPath = p;
      if (addName.trim() === '') addName = p.split('/').filter(Boolean).pop() ?? '';
      pickerOpen = false;
    }}
    onclose={() => (pickerOpen = false)}
  />
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
  .gitpage-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .landing {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 28px 24px 40px;
  }
  /* Centered, constrained hub so the no-tab state isn't a sprawling near-empty
     page — the + tab is the primary add/open entry point. */
  .landing-inner {
    max-width: 880px;
    margin: 0 auto;
    width: 100%;
  }
  .landing-head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    margin-bottom: 16px;
  }
  .landing-title {
    font-size: 16px;
    font-weight: 650;
    margin: 0;
  }
  .landing-hint {
    font-size: 12px;
    color: var(--text-dim);
    margin-top: 3px;
    max-width: 520px;
  }
  .path-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .path-row .input {
    flex: 1;
  }
  .remote-list {
    max-height: 280px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .remote-row {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
  }
  .remote-row:last-child {
    border-bottom: none;
  }
  .min0 {
    min-width: 0;
  }
  .remote-name {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 13px;
    font-weight: 600;
  }
  .remote-desc {
    font-size: 11px;
    margin-top: 1px;
  }
  .remote-full {
    font-size: 10.5px;
    margin-top: 1px;
  }
  .pad {
    padding: 14px;
    font-size: 12px;
  }
  .err {
    padding: 14px;
    font-size: 12px;
    color: var(--danger, #e5534b);
  }
  .repo-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
    gap: 12px;
  }
  .repo-card {
    display: flex;
    flex-direction: column;
    transition: border-color 130ms ease-out;
  }
  .repo-card:hover {
    border-color: color-mix(in srgb, var(--accent) 35%, var(--border));
  }
  .repo-main {
    text-align: left;
    border: none;
    background: transparent;
    padding: 14px 14px 6px;
    cursor: pointer;
    color: var(--text);
  }
  .repo-name {
    display: flex;
    align-items: center;
    gap: 7px;
    font-size: 13.5px;
    font-weight: 600;
  }
  .repo-path {
    font-size: 11px;
    color: var(--text-dim);
    margin-top: 4px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .repo-remote {
    font-size: 10.5px;
    margin-top: 2px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .repo-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px 10px 14px;
  }
</style>
