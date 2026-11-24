<script lang="ts">
  // Commit over the thread (Thread details' git row): Git's commit card
  // (CommitCard — every change included, the message drafted, one primary
  // that commits and pushes) in a sheet. A commit closes the sheet and the
  // card's row moves on to Push / Create PR.
  import type { Repo } from '../../lib/api/types';
  import Modal from '../../lib/components/Modal.svelte';
  import { git } from '../../lib/stores/git.svelte';
  import { router } from '../../lib/router.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import CommitCard from '../git/CommitCard.svelte'; // ui-guards: allow — commits go through Git's one commit card

  interface Props {
    repo: Repo;
    onclose: () => void;
  }
  let { repo, onclose }: Props = $props();

  const branch = $derived(git.statusById[repo.id]?.branch ?? null);
  const title = $derived(branch ? `Commit to ${branch}` : 'Commit');
</script>

<Modal {title} width={viewport.isPhone ? undefined : 880} height={640} {onclose}>
  <div class="cs-body" data-testid="thread-commit-sheet">
    <p class="cs-where" dir="auto">{repo.name} · {repo.path}</p>
    <CommitCard {repo} layout="sheet" oncommitted={onclose} onopenpr={(n) => router.go(`git/${encodeURIComponent(repo.id)}/pr/${n}`)} />
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
