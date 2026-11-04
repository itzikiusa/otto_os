<script lang="ts">
  // The results grid's "Add row" entry points (MongoDB: "Insert document"),
  // all opening `flow.insert` — the schema-derived form on SQL engines, the
  // JSON insert editor on Mongo. Renders nothing unless the result can take an
  // insert (flow.insert.target). Variants:
  //   toolbar — the button in the results toolbar
  //   empty   — under the "No rows returned" check of a column-less result
  //             (a Mongo find that matched nothing)
  //   cta     — the banner over an empty grid (columns, zero rows)
  import Icon from '../../lib/components/Icon.svelte';
  import type { EditFlow } from './EditFlow.svelte';

  interface Props {
    flow: EditFlow;
    variant: 'toolbar' | 'empty' | 'cta';
  }
  let { flow, variant }: Props = $props();

  const target = $derived(flow.insert.target);
  const mongo = $derived(flow.engine === 'mongodb');
  const label = $derived(mongo ? 'Insert document' : 'Add row');
  const title = $derived(
    target
      ? mongo
        ? `Insert a document into ${target.table} (you review the insertOne before it runs)`
        : `Add a row to ${target.table} (you review the INSERT before it runs)`
      : '',
  );
  const open = (): void => void flow.insert.open();
</script>

{#if target}
  {#if variant === 'cta'}
    <div class="add-cta" data-testid="db-empty-add-row">
      <Icon name="grid" size={14} />
      <span class="add-cta-text">No rows — {mongo ? 'insert a document' : 'add the first row'} without writing {mongo ? 'insertOne' : 'an INSERT'} by hand.</span>
      <button class="btn small primary" onclick={open} {title}><Icon name="plus" size={12} />{label}</button>
    </div>
  {:else}
    <button class="btn small" onclick={open} {title} aria-label={label} data-testid="db-add-row">
      <Icon name="plus" size={12} /><span class:add-label={variant === 'toolbar'}>{label}</span>
    </button>
  {/if}
{/if}

<style>
  .add-cta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    margin: 8px 10px 0;
    padding: 8px 10px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .add-cta-text {
    flex: 1;
    min-width: 0;
  }
  /* The results toolbar hides its button labels on a narrow pane (tb-label);
     keep parity here so the toolbar wraps the same way. */
  @container (max-width: 560px) {
    .add-label {
      display: none;
    }
  }
</style>
