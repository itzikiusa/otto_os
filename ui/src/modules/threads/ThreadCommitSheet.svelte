<script lang="ts">
  // Commit over the thread (Thread details' git row): Git's own changes panel
  // (WipPanel — stage toggles, the drafted message, Amend / Sign, Commit) in a
  // sheet, with every change staged on open when nothing is yet. A commit
  // closes the sheet and the card's row moves on to Push / Create PR.
  import { api } from '../../lib/api/client';
  import type { Repo, RepoStatusResp } from '../../lib/api/types';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { git } from '../../lib/stores/git.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import WipPanel from '../git/WipPanel.svelte'; // ui-guards: allow — commits go through Git's own changes panel

  interface Props {
    repo: Repo;
    onclose: () => void;
  }
  let { repo, onclose }: Props = $props();

  let status = $state<RepoStatusResp | null>(null);
  let error = $state<string | null>(null);
  const title = $derived(status?.branch ? `Commit to ${status.branch}` : 'Commit');

  function onstatus(s: RepoStatusResp): void {
    status = s;
    git.setStatus(repo.id, s);
  }

  async function load(): Promise<void> {
    error = null;
    try {
      let s = await api.get<RepoStatusResp>(`/repos/${encodeURIComponent(repo.id)}/status`);
      const open = s.changes.filter((c) => c.kind !== 'conflicted');
      if (!open.some((c) => c.staged) && open.length > 0) {
        s = await api.post<RepoStatusResp>(`/repos/${encodeURIComponent(repo.id)}/stage`, { paths: open.map((c) => c.path) });
      }
      onstatus(s);
    } catch (e) {
      error = loadErrorText(e);
    }
  }
  $effect(() => {
    void load();
  });
</script>

<Modal {title} width={viewport.isPhone ? undefined : 880} height={640} {onclose}>
  <div class="cs-body" data-testid="thread-commit-sheet">
    <p class="cs-where" dir="auto">{repo.name} · {repo.path}</p>
    {#if status}
      <WipPanel repoId={repo.id} {status} {onstatus} oncommitted={onclose} />
    {:else}
      <LoadState loading={!error} {error} what="changes" onretry={() => void load()} />
    {/if}
  </div>
</Modal>

<style>
  .cs-body {
    display: flex;
    flex-direction: column;
    gap: 8px;
    block-size: 100%;
    min-block-size: 0;
  }
  .cs-where {
    margin: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
    overflow-wrap: anywhere;
  }
  .cs-body > :global(:last-child) {
    flex: 1;
    min-block-size: 0;
  }
</style>
