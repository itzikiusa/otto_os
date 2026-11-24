<script lang="ts" module>
  /** The browse query and account outlive the sheet: reopening it searches again with them. */
  const remembered = { query: '', account: '' };
</script>

<script lang="ts">
  // The Add repository sheet (split out of GitPage.svelte, loaded on first
  // open): register a local folder, browse a git account's repositories, or
  // clone a URL. The clone destination is remembered across sessions.
  import { untrack } from 'svelte';
  import { api } from '../../lib/api/client';
  import type { GitAccount, Repo, RemoteRepoSummary } from '../../lib/api/types';
  import { onTabKey } from '../../lib/tabKeys';
  import { router } from '../../lib/router.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { loadErrorText } from '../../lib/loadError';
  import { lsGet, lsSet } from '../../lib/storage';
  import Modal from '../../lib/components/Modal.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';

  interface Props {
    mode: 'register' | 'clone' | 'browse';
    onclose: () => void;
    /** A registered (not async-cloning) repo: the page opens it as a tab. */
    onregistered: (repoId: string) => void;
  }
  let { mode, onclose, onregistered }: Props = $props();
  let addMode = $state(untrack(() => mode));

  let addPath = $state('');
  let addUrl = $state('');
  let addName = $state('');
  let addAccount = $state('');
  let accounts: GitAccount[] = $state([]);
  let busy = $state(false);
  let pickerOpen = $state(false);

  // Clone destination dir — REMEMBERED across sessions so the user never has to
  // re-pick it (persisted to localStorage; defaults to the last-used location).
  const CLONE_DIR_KEY = 'otto_git_clone_dir';
  let cloneDir = $state(lsGet(CLONE_DIR_KEY) ?? '');
  let cloneDirPickerOpen = $state(false);
  function rememberCloneDir(): void {
    const v = cloneDir.trim();
    if (v) lsSet(CLONE_DIR_KEY, v);
  }

  // "Browse remote" mode: search repos under a git account's namespace.
  let browseAccount = $state(remembered.account);
  let browseQuery = $state(remembered.query);
  $effect(() => {
    remembered.query = browseQuery;
    remembered.account = browseAccount;
  });
  let remoteRepos: RemoteRepoSummary[] = $state([]);
  let remoteLoading = $state(false);
  let remoteError = $state('');
  let remoteRevision = 0;
  let remoteResultsAccount = '';
  $effect(() => {
    if (untrack(() => addMode === 'browse')) untrack(() => void runRemoteSearch());
    return () => {
      remoteRevision++;
      if (searchTimer) clearTimeout(searchTimer);
    };
  });
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  const accountsWithNs = $derived(accounts.filter((a) => a.namespace));
  // The accounts (clone auth, browse namespaces), read when the sheet opens.
  $effect(() => {
    const ctl = new AbortController();
    void api.get<GitAccount[]>('/git/accounts', ctl.signal).then((a) => { if (!ctl.signal.aborted) accounts = a; }).catch(() => {});
    return () => ctl.abort();
  });

  $effect(() => {
    // default the browse account to the first one that has a namespace
    if (browseAccount === '' && accountsWithNs.length > 0) browseAccount = accountsWithNs[0].id;
  });

  function scheduleRemoteSearch(): void {
    remoteRevision++;
    remoteRepos = [];
    remoteError = '';
    remoteLoading = true;
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(() => void runRemoteSearch(), 350);
  }

  async function runRemoteSearch(): Promise<void> {
    if (searchTimer) clearTimeout(searchTimer);
    const revision = ++remoteRevision;
    const accountId = browseAccount;
    remoteRepos = [];
    if (accountId === '') { remoteLoading = false; return; }
    remoteLoading = true;
    remoteError = '';
    try {
      const q = browseQuery.trim() === '' ? '' : `?q=${encodeURIComponent(browseQuery.trim())}`;
      const next = await api.get<RemoteRepoSummary[]>(`/git/accounts/${accountId}/remote-repos${q}`);
      if (revision !== remoteRevision) return;
      remoteRepos = next;
      remoteResultsAccount = accountId;
    } catch (e) {
      if (revision !== remoteRevision) return;
      remoteError = loadErrorText(e);
      remoteRepos = [];
    } finally {
      if (revision === remoteRevision) remoteLoading = false;
    }
  }

  async function cloneRemote(repo: RemoteRepoSummary): Promise<void> {
    if (!ws.currentId || busy || remoteLoading || remoteResultsAccount !== browseAccount) return;
    busy = true;
    try {
      const created = await api.post<Repo>(`/workspaces/${ws.currentId}/repos`, {
        clone_url: repo.clone_url,
        name: repo.name || null,
        git_account_id: remoteResultsAccount || null,
        clone_dir: cloneDir.trim() || null,
      });
      rememberCloneDir();
      toasts.success('Clone started', created.name);
      onclose();
      await git.loadAllRepos(true);
    } catch (e) {
      toastError('Couldn’t clone', e);
    } finally {
      busy = false;
    }
  }

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
              clone_dir: cloneDir.trim() || null,
            };
      const repo = await api.post<Repo>(`/workspaces/${ws.currentId}/repos`, body);
      if (addMode === 'clone') rememberCloneDir();
      toasts.success(addMode === 'clone' ? 'Clone started' : 'Repo registered', repo.name);
      onclose();
      await git.loadAllRepos(true);
      // Newly registered (not async-cloning) repos open straight into a tab.
      if (addMode === 'register') onregistered(repo.id);
    } catch (e) {
      toastError('Couldn’t add it', e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="Add repository" onclose={onclose}>
  <div class="segmented add-mode" role="tablist" aria-label="How to add the repository">
    <button role="tab" onkeydown={onTabKey} tabindex={addMode === 'register' ? 0 : -1} aria-selected={addMode === 'register'} class:active={addMode === 'register'} onclick={() => (addMode = 'register')}>
      Local folder
    </button>
    <button role="tab" onkeydown={onTabKey} tabindex={addMode === 'browse' ? 0 : -1} aria-selected={addMode === 'browse'} class:active={addMode === 'browse'} onclick={() => { addMode = 'browse'; if (remoteRepos.length === 0) void runRemoteSearch(); }}>
      Browse remote
    </button>
    <button role="tab" onkeydown={onTabKey} tabindex={addMode === 'clone' ? 0 : -1} aria-selected={addMode === 'clone'} class:active={addMode === 'clone'} onclick={() => (addMode = 'clone')}>Clone URL</button>
  </div>

  {#if addMode === 'register'}
    <div class="field">
      <label for="ar-path">Path</label>
      <div class="path-row">
        <input dir="ltr" id="ar-path" class="input mono ar-path-input" bind:value={addPath} placeholder="/Users/you/code/repo" spellcheck="false" />
        <button class="btn ar-browse" type="button" onclick={() => (pickerOpen = true)}>Browse…</button>
      </div>
    </div>
  {:else if addMode === 'browse'}
    {#if accountsWithNs.length === 0}
      <EmptyState
        icon="key"
        title="No namespace configured"
        body="Add a Git account with an organization, workspace or group to browse its repositories."
        actionLabel="Open Git accounts"
        actionIcon="key"
        onaction={() => { onclose(); router.go('settings/git-accounts'); }}
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
        <input dir="auto" id="ar-bq" class="input" bind:value={browseQuery} oninput={scheduleRemoteSearch} placeholder="api-gateway" spellcheck="false" />
      </div>
      <div class="remote-list">
        {#if remoteLoading}
          <div class="dim pad">Searching repositories…</div>
        {:else if remoteError}
          <div class="pad">
            <LoadState variant="compact" what="the account’s repositories" errorTitle="Couldn’t search the account’s repositories" error={remoteError} empty onretry={() => void runRemoteSearch()} />
          </div>
        {:else if remoteRepos.length === 0}
          <div class="dim pad">{browseQuery.trim() ? `No repositories match “${browseQuery.trim()}”.` : 'This account has no repositories.'}</div>
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
              <button class="btn small ar-clone" disabled={busy} onclick={() => cloneRemote(r)} title="Clone {r.full_name}">Clone</button>
            </div>
          {/each}
        {/if}
      </div>
    {/if}
  {:else}
    <div class="field">
      <label for="ar-url">Clone URL</label>
      <input dir="ltr" id="ar-url" class="input mono" bind:value={addUrl} placeholder="git@github.com:org/repo.git" spellcheck="false" />
    </div>
    <div class="field">
      <label for="ar-acct">Git account <span class="dim">(for HTTPS auth)</span></label>
      <select id="ar-acct" class="input" bind:value={addAccount}>
        <option value="">None (public, or SSH agent)</option>
        {#each accounts as a (a.id)}
          <option value={a.id}>{a.label} ({a.provider})</option>
        {/each}
      </select>
    </div>
  {/if}

  {#if addMode !== 'register'}
    <div class="field">
      <label for="ar-clonedir">Clone into</label>
      <div class="path-row">
        <input dir="ltr"
          id="ar-clonedir"
          class="input mono ar-path-input"
          bind:value={cloneDir}
          placeholder="~/code"
          spellcheck="false"
        />
        <button class="btn ar-browse" type="button" onclick={() => (cloneDirPickerOpen = true)}>Browse…</button>
      </div>
      <span class="hint">Empty uses the workspace folder. Otto remembers the last folder you pick.</span>
    </div>
  {/if}

  {#if addMode !== 'browse'}
    <div class="field">
      <label for="ar-name">Name <span class="dim">(optional)</span></label>
      <input dir="auto" id="ar-name" class="input" bind:value={addName} />
    </div>
  {/if}

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    {#if addMode !== 'browse'}
      <button
        class="btn primary"
        disabled={busy || (addMode === 'register' ? addPath.trim() === '' : addUrl.trim() === '')}
        onclick={addRepo}
      >
        {busy ? (addMode === 'clone' ? 'Cloning…' : 'Adding…') : addMode === 'clone' ? 'Clone' : 'Add repository'}
      </button>
    {/if}
  {/snippet}
</Modal>

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

{#if cloneDirPickerOpen}
<FolderPicker
  title="Choose a clone destination"
  start={cloneDir}
  onpick={(p) => {
    cloneDir = p;
    rememberCloneDir();
    cloneDirPickerOpen = false;
  }}
  onclose={() => (cloneDirPickerOpen = false)}
/>
{/if}

<style>
  .add-mode {
    margin-bottom: 14px;
  }
  .path-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .ar-path-input {
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
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .remote-desc {
    font-size: var(--fs-xs);
    margin-top: 1px;
  }
  .remote-full {
    font-size: var(--fs-xs);
    margin-top: 1px;
  }
  .pad {
    padding: 14px;
    font-size: var(--fs-s);
  }
  /* ≤1024px: full-width segmented/path rows so nothing spills past the
     (already viewport-clamped) modal edge. */
  @media (max-width: 1024px) {
    .add-mode {
      display: flex;
      width: 100%;
    }
    .add-mode > button {
      flex: 1;
      height: 32px;
      white-space: nowrap;
    }
    .path-row {
      flex-wrap: wrap;
    }
    .ar-path-input {
      flex: 1 1 100%;
    }
    .ar-browse {
      min-height: 36px;
    }
    .remote-list {
      max-height: 50vh;
    }
    .remote-row {
      padding: 10px;
    }
    .ar-clone {
      min-height: 36px;
      flex-shrink: 0;
    }
  }
</style>
