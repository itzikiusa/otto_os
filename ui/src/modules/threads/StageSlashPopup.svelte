<script lang="ts">
  // The new-thread composer's `/` command list (listbox; the textarea keeps
  // focus and drives the active row via aria-activedescendant).
  import type { ComposerSlashCommand } from '../../lib/slashActions';

  interface Props {
    id: string;
    items: readonly ComposerSlashCommand[];
    active: number;
    onaccept: (c: ComposerSlashCommand) => void;
    onhover: (i: number) => void;
  }
  let { id, items, active, onaccept, onhover }: Props = $props();
</script>

<div class="ssp-pop" id="{id}-list" role="listbox" aria-label="Slash commands" data-slash-pop>
  {#each items as c, i (c.name)}
    <div
      id="{id}-opt-{i}"
      class="ssp-row"
      class:active={i === active}
      role="option"
      tabindex="-1"
      aria-selected={i === active}
      onmousedown={(ev) => {
        ev.preventDefault();
        onaccept(c);
      }}
      onmouseenter={() => onhover(i)}
    >
      <span class="ssp-name mono" dir="ltr">/{c.name}</span>
      <span class="ssp-desc">{c.description}</span>
    </div>
  {/each}
</div>

<style>
  .ssp-pop {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: min(220px, 35vh);
    overflow-y: auto;
    padding: 4px;
    border-radius: var(--radius-m);
    border: 1px solid var(--border);
    background: var(--surface-2);
  }
  .ssp-row {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
    font-size: var(--fs-s);
    min-width: 0;
  }
  .ssp-row.active {
    background: var(--accent-soft);
  }
  .ssp-name {
    color: var(--accent-text);
    white-space: nowrap;
  }
  .ssp-desc {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
