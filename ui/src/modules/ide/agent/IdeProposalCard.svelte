<script lang="ts">
  // 1-click Diff Preview + Apply / Undo proposal card for code blocks targeting
  // IDE workspace files inside `IdeRuntimeTimeline.svelte`.
  import Badge from '../../../lib/components/Badge.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { copyWithToast } from '../../../lib/clipboard';
  import { ide } from '../ideStore.svelte';
  import { ideAgent, type IdeCodeProposal } from '../store/ideAgentController.svelte';

  interface Props {
    proposal: IdeCodeProposal;
  }
  let { proposal }: Props = $props();

  let diffOverride = $state<boolean | null>(null);
  const diffOpen = $derived(diffOverride ?? proposal.diffLines.length <= 40);
  const hasUndo = $derived(ideAgent.appliedUndoByProposalId[proposal.id] !== undefined);
  const readOnly = $derived(!ide.canEditCollab || Boolean(ide.tabs.find((t) => t.path === proposal.path)?.readOnly));

  function jumpToFile(): void {
    void ide.openFile(proposal.path, { line: proposal.range?.startLine ?? 1, preview: false });
  }
</script>

<section class="ia-proposal" data-testid="ide-agent-proposal">
  <div class="ia-prop-head">
    <button
      type="button"
      class="ia-prop-file"
      title="Open {proposal.relPath} in editor"
      onclick={jumpToFile}
    >
      <Icon name="file" size={12} />
      <span class="mono ellipsis" dir="ltr">{proposal.relPath}</span>
    </button>
    <Badge tone="ok" label="+{proposal.addedCount}" />
    <Badge tone="bad" label="-{proposal.removedCount}" />

    <div class="ia-prop-actions">
      <button
        type="button"
        class="btn small ghost"
        aria-expanded={diffOpen}
        onclick={() => (diffOverride = !diffOpen)}
        data-testid="ide-agent-toggle-diff"
      >
        Diff
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="Copy proposed code for {proposal.relPath}"
        title="Copy proposed code"
        onclick={() => void copyWithToast(proposal.code, proposal.relPath)}
      >
        <Icon name="copy" size={12} />
      </button>
      {#if hasUndo}
        <button
          type="button"
          class="btn small"
          title="Revert applied edit in {proposal.relPath}"
          onclick={() => void ideAgent.undoProposal(proposal.id)}
          data-testid="ide-agent-undo-proposal"
        >
          <Icon name="undo" size={12} /> Undo
        </button>
      {:else}
        <button
          type="button"
          class="btn small ia-cta"
          disabled={readOnly}
          title={readOnly ? 'File is read-only' : `Apply changes to ${proposal.relPath}`}
          onclick={() => void ideAgent.applyProposal(proposal)}
          data-testid="ide-agent-apply-proposal"
        >
          <Icon name="check" size={12} /> Apply
        </button>
      {/if}
    </div>
  </div>

  {#if diffOpen}
    <div class="ia-diff scroll-thin" dir="ltr" data-testid="ide-agent-diff-preview">
      {#each proposal.diffLines as dl, idx (`${idx}:${dl.kind}:${dl.oldLine ?? ''}:${dl.newLine ?? ''}`)}
        <div class="ia-diff-row" class:is-add={dl.kind === 'add'} class:is-del={dl.kind === 'del'}>
          <span class="ia-ln">{dl.newLine ?? dl.oldLine ?? ''}</span>
          <span class="ia-sign">{dl.kind === 'add' ? '+' : dl.kind === 'del' ? '-' : ' '}</span>
          <span class="ia-code">{dl.text}</span>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .ia-proposal {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
    overflow: hidden;
  }
  .ia-prop-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    padding-block: 6px;
    padding-inline: 8px;
    background: var(--surface-2);
    border-block-end: 1px solid var(--border);
  }
  .ia-prop-file {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-inline-size: 0;
    max-inline-size: 100%;
    padding-block: 2px;
    padding-inline: 4px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .ia-prop-file:hover {
    background: var(--hover);
    color: var(--accent-text);
  }
  .ia-prop-actions {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-inline-start: auto;
    flex-shrink: 0;
  }
  .ia-cta {
    background: var(--accent-soft);
    border-color: var(--accent-line);
    color: var(--accent-text);
  }
  .ia-cta:hover:not(:disabled) {
    background: var(--accent-soft-strong);
  }
  .ia-diff {
    max-block-size: 240px;
    overflow: auto;
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.45;
  }
  .ia-diff-row {
    display: flex;
    min-inline-size: max-content;
    padding-inline: 8px;
  }
  .ia-diff-row.is-add {
    background: var(--success-soft);
  }
  .ia-diff-row.is-del {
    background: var(--danger-soft);
  }
  .ia-ln {
    inline-size: 32px;
    padding-inline-end: 8px;
    text-align: end;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    user-select: none;
    flex-shrink: 0;
  }
  .ia-sign {
    inline-size: 14px;
    flex-shrink: 0;
    user-select: none;
    color: var(--text-dim);
  }
  .ia-diff-row.is-add .ia-sign {
    color: var(--success);
  }
  .ia-diff-row.is-del .ia-sign {
    color: var(--danger);
  }
  .ia-code {
    white-space: pre;
    color: var(--text);
  }
  @media (pointer: coarse) {
    .ia-prop-file {
      min-block-size: var(--hit-min);
    }
  }
</style>
