<script lang="ts">
  // The stacked (phone / short) IDE's pane switcher: Files · Editor · Tools ·
  // Agent. It sits in the PageHeader's tabs slot, replacing Project/Scratches
  // (those move to ⋯), so the stacked header stays one row plus this one.
  import { onTabKey } from '../../../lib/tabKeys';
  import { ideAgent } from '../store/ideAgentController.svelte';
  import { ideLayout, type IdeStackPane } from './ideLayout.svelte';

  interface Props {
    /** Show a pane (the page also syncs the agent panel's open state). */
    onshow: (pane: IdeStackPane) => void;
  }
  let { onshow }: Props = $props();

  const PANES: readonly { id: IdeStackPane; label: string }[] = [
    { id: 'files', label: 'Files' },
    { id: 'editor', label: 'Editor' },
    { id: 'tools', label: 'Tools' },
    { id: 'agent', label: 'Agent' },
  ];
</script>

<div class="segmented ide-stack-switcher" role="group" aria-label="IDE pane" data-testid="ide-stack-switcher">
  {#each PANES as p (p.id)}
    <button
      type="button"
      class:active={ideLayout.pane === p.id}
      aria-pressed={ideLayout.pane === p.id}
      onkeydown={onTabKey}
      onclick={() => onshow(p.id)}
      data-testid="ide-stack-tab-{p.id}"
    >
      {p.label}{#if p.id === 'agent' && ideAgent.contexts.length > 0}&nbsp;({ideAgent.contexts.length}){/if}
    </button>
  {/each}
</div>

<style>
  /* 44 px tap targets on touch (the segmented control's desktop size is 22 px). */
  .ide-stack-switcher { max-inline-size: 100%; }
  .ide-stack-switcher > button { padding-inline: 10px; min-inline-size: 56px; }
  @media (pointer: coarse) {
    .ide-stack-switcher > button { min-block-size: 40px; }
  }
</style>
