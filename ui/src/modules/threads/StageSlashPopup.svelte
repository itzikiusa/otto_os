<script lang="ts">
  // The composer's `/` command list (listbox; the textarea keeps focus and
  // drives the active row via aria-activedescendant). Items carrying a
  // `group` show under its heading (Otto's commands, then the agent's own —
  // hostCommands.ts slashSuggestions). The list scrolls (60 skills fit), and
  // the active row is kept in view.
  import type { ComposerSlashCommand } from '../../lib/slashActions';

  interface Props {
    id: string;
    items: readonly ComposerSlashCommand[];
    active: number;
    onaccept: (c: ComposerSlashCommand) => void;
    onhover: (i: number) => void;
  }
  let { id, items, active, onaccept, onhover }: Props = $props();
  let listEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    listEl?.querySelector(`#${CSS.escape(`${id}-opt-${active}`)}`)?.scrollIntoView?.({ block: 'nearest' });
  });
</script>

<div class="ssp-pop" id="{id}-list" role="listbox" aria-label="Slash commands" data-slash-pop bind:this={listEl}>
  {#each items as c, i (`${c.group ?? ''}/${c.name}`)}
    {#if c.group && c.group !== items[i - 1]?.group}
      <div class="ssp-group" role="presentation" data-testid="slash-group">{c.group}</div>
    {/if}
    <div
      id="{id}-opt-{i}"
      class="ssp-row"
      class:active={i === active}
      role="option"
      tabindex="-1"
      aria-selected={i === active}
      data-source={c.source}
      onmousedown={(ev) => {
        ev.preventDefault();
        onaccept(c);
      }}
      onmouseenter={() => onhover(i)}
    >
      <span class="ssp-name mono" dir="ltr">/{c.name}{#if c.hint}<span class="ssp-hint"> {c.hint}</span>{/if}</span>
      <span class="ssp-desc" dir="auto">{c.description}</span>
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
  .ssp-group {
    padding: 6px 8px 2px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
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
    max-inline-size: 22ch;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .ssp-hint {
    color: var(--text-dim);
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
