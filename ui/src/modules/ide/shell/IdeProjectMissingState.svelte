<script lang="ts">
  // The project's folder is gone, or its SSH host can't be reached (review I3-12).
  // One sentence of what is wrong, the full path (wrapping, never cut), and next
  // steps that can work: Retry only where a retry can succeed (SSH), "Choose another
  // folder…" (the project switcher), "Remove from recents" when it is listed there.
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import { ide } from '../ideStore.svelte';
  import { ideProjectHealth, type IdeRootIssue } from '../store/ideProjectHealth.svelte';
  import { ideProjectRecents, openIdeProjectSwitcher } from './ideProjectRecents.svelte';

  interface Props {
    issue: IdeRootIssue;
  }

  let { issue }: Props = $props();

  const ssh = $derived(issue.kind === 'unreachable');
  const host = $derived(ide.target.connection_name || 'the SSH host');
  const listed = $derived(ideProjectRecents.has({ kind: ide.target.kind, root: issue.root, connection_id: ide.target.connection_id ?? null }));

  function chooseAnother(): void {
    openIdeProjectSwitcher();
  }

  function removeFromRecents(): void {
    ideProjectRecents.remove({ kind: ide.target.kind, root: issue.root, connection_id: ide.target.connection_id ?? null });
    openIdeProjectSwitcher();
  }

  function retry(): void {
    ideProjectHealth.clear();
    void ide.loadTree();
  }
</script>

<div class="pm-wrap" data-testid="ide-project-missing">
  <EmptyState
    headingLevel={2}
    tone="error"
    icon={ssh ? 'server' : 'folder'}
    title={ssh ? `Can’t reach ${host}` : 'This folder no longer exists'}
    body={ssh ? 'The project’s files are on a host that isn’t answering right now.' : 'It was moved, renamed or deleted after you opened it.'}
    actionLabel={ssh ? 'Retry' : undefined}
    actionKind="secondary"
    onaction={ssh ? retry : undefined}
  >
    <code class="pm-path" dir="ltr" data-testid="ide-project-missing-path">{issue.root}</code>
    <div class="pm-actions">
      <button type="button" class="btn small" onclick={chooseAnother} data-testid="ide-project-missing-choose">{ssh ? 'Choose another project…' : 'Choose another folder…'}</button>
      {#if listed}
        <button type="button" class="btn small ghost" onclick={removeFromRecents} data-testid="ide-project-missing-remove">Remove from recents</button>
      {/if}
    </div>
  </EmptyState>
</div>

<style>
  .pm-wrap { padding-block: 12px; padding-inline: 8px; }
  .pm-path {
    display: block; max-inline-size: 100%; margin-block: 4px 8px; padding-block: 4px; padding-inline: 6px;
    font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text); background: var(--surface-2);
    border-radius: var(--radius-s); overflow-wrap: anywhere; white-space: normal; text-align: start;
  }
  .pm-actions { display: flex; flex-wrap: wrap; gap: 6px; justify-content: center; }
</style>
