<script lang="ts">
  // Sync collections with a git repo connected to this workspace, as Postman
  // collection files under collections/. Pull imports them; Commit & push
  // exports every top-level collection. Outward-facing (a commit + push), so
  // the sheet says where it goes and what is written before the button.
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { api } from '../../lib/api/client';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { Repo } from '../../lib/api/types';

  interface Props { onclose: () => void }
  let { onclose }: Props = $props();

  let repos = $state<Repo[]>([]);
  let loading = $state(true);
  let loadError = $state('');
  let repoId = $state('');
  let commitMsg = $state('Update API collections');
  let branch = $state('');
  let busy = $state(false);

  const roots = $derived(apiClient.collections.filter((c) => (c.parent_id ?? null) === null));
  const repo = $derived(repos.find((r) => r.id === repoId) ?? null);

  async function load(): Promise<void> {
    if (!ws.currentId) return;
    loading = true;
    loadError = '';
    try {
      repos = await api.get<Repo[]>(`/workspaces/${ws.currentId}/repos`);
      if (repos.length && !repoId) repoId = repos[0].id;
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => { void load(); });

  async function pull(): Promise<void> {
    busy = true;
    await apiClient.gitPullCollections(repoId);
    busy = false;
    onclose();
  }
  async function push(): Promise<void> {
    busy = true;
    const ok = await apiClient.gitPushCollections(repoId, commitMsg, branch.trim() || null);
    busy = false;
    if (ok) onclose();
  }
</script>

<Modal title="Sync collections with Git" width={500} {onclose}>
  {#if loading}
    <p class="dim" role="status">Loading repositories…</p>
  {:else if loadError}
    <div class="err" role="alert">
      <Icon name="warning" size={14} /><span>Couldn’t load this workspace’s repositories. {loadError}</span>
      <button class="btn small" onclick={load}>Retry</button>
    </div>
  {:else if repos.length === 0}
    <p class="dim">No git repositories are connected to this workspace. Add one on the Git page, then come back.</p>
  {:else}
    <div class="field">
      <label for="git-repo">Repository</label>
      <select id="git-repo" class="input" bind:value={repoId}>
        {#each repos as r (r.id)}<option value={r.id}>{r.name}</option>{/each}
      </select>
    </div>
    <div class="field">
      <label for="git-msg">Commit message</label>
      <input id="git-msg" class="input" bind:value={commitMsg} />
    </div>
    <div class="field">
      <label for="git-branch">Branch (optional)</label>
      <input id="git-branch" class="input mono" bind:value={branch} placeholder="api-collections-update" />
      <span class="hint">Push to a new branch to open a pull request from the Git page afterwards.</span>
    </div>
    <p class="summary">
      <strong>Commit &amp; push</strong> writes {roots.length} collection file{roots.length === 1 ? '' : 's'}
      to <code>collections/</code> in <strong>{repo?.name}</strong>{branch.trim() ? ` on ${branch.trim()}` : ''} and pushes to its remote,
      where everyone with access to the repository can read them. Stored secrets are exported as <code>***</code>.
      <strong>Pull</strong> imports the collection files found there.
    </p>
  {/if}
  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    {#if repos.length > 0}
      <button class="btn" onclick={pull} disabled={busy || !repoId}><Icon name="download" size={13} />Pull</button>
      <button class="btn primary" onclick={push} disabled={busy || !repoId || roots.length === 0} title={roots.length === 0 ? 'No collections to push yet — save a request into a collection first' : undefined}>{busy ? 'Working…' : 'Commit & push'}</button>
    {/if}
  {/snippet}
</Modal>

<style>
  .dim {
    margin: 0;
    color: var(--text-dim);
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    color: var(--text);
  }
  .err :global(svg) {
    color: var(--danger);
    flex-shrink: 0;
  }
  .summary {
    margin: 4px 0 0;
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
  }
  code {
    font-family: var(--font-mono);
  }
</style>
