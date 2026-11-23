<script lang="ts">
  // Old and new side by side for every piece a change really altered, then
  // ONE choice: keep the new one, or keep the old one (it comes back).
  import { listVersions } from '../../../lib/api/design';
  import type { ProductionItem, ProductionItemResult } from '../../../lib/api/types';
  import AudioLine from './AudioLine.svelte';
  import CreateModel from './CreateModel.svelte';

  interface Props {
    items: ProductionItem[];
    busy: boolean;
    onkeep: () => void;
    onback: () => void;
  }
  let { items, busy, onkeep, onback }: Props = $props();

  /** version id → its number, per artifact (CreateModel shows versions by number). */
  let seqs = $state<Record<string, number>>({});
  $effect(() => {
    const ids = new Set(items.flatMap((i) => [i.result?.artifact_id, i.previous_result?.artifact_id]).filter((x): x is string => !!x));
    for (const a of ids) {
      void listVersions(a, { limit: 200 })
        .then((vs) => {
          const next = { ...seqs };
          for (const v of vs) next[v.id] = v.seq;
          seqs = next;
        })
        .catch(() => {});
    }
  });
  const seqOf = (r: ProductionItemResult | null | undefined) => (r?.version_id ? (seqs[r.version_id] ?? null) : null);
</script>

<h1 class="cr-h1">Which one do you like?</h1>
<p class="cr-sub">Here is what changed: before on the left, new on the right.</p>

{#each items as i (i.id)}
  <section class="ci" data-testid="create-compare">
    <h2 class="cr-h2">{i.name}</h2>
    <div class="ci-pair">
      <figure>
        {#if i.previous_result?.artifact_id}
          <div class="ci-stage"><CreateModel artifactId={i.previous_result.artifact_id} seq={seqOf(i.previous_result)} label={`${i.name}, before`} /></div>
        {:else if i.previous_result?.clip_id}
          <AudioLine clipId={i.previous_result.clip_id} title="Before" />
        {/if}
        <figcaption>Before</figcaption>
      </figure>
      <figure class="ci-new">
        {#if i.result?.artifact_id}
          <div class="ci-stage"><CreateModel artifactId={i.result.artifact_id} seq={seqOf(i.result)} label={`${i.name}, new`} /></div>
        {:else if i.result?.clip_id}
          <AudioLine clipId={i.result.clip_id} title="New" />
        {/if}
        <figcaption>New</figcaption>
      </figure>
    </div>
  </section>
{/each}

<div class="cr-sticky ci-btns">
  <button type="button" class="cr-secondary" disabled={busy} onclick={onback} data-testid="create-go-back">Keep the old one</button>
  <button type="button" class="cr-secondary" disabled={busy} onclick={onkeep} data-testid="create-keep-new">Keep the new one</button>
</div>

<style>
  .ci-pair {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--sp-3);
  }
  figure {
    margin: 0;
  }
  figcaption {
    text-align: center;
    font-weight: 600;
    margin-block-start: var(--sp-2);
  }
  .ci-stage {
    aspect-ratio: 1;
  }
  .ci-btns {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--sp-3);
  }
</style>
