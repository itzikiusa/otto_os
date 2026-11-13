<script lang="ts">
  // The review before a Product publish leaves this Mac: where it goes (site
  // + issue/page), who sees it, and the FULL text sent in a scrollable block.
  // Driven by `publishReview` (publishReviewStore.svelte.ts); each tab mounts
  // one with its own `owner`.
  import Modal from '../../lib/components/Modal.svelte';
  import { publishReview } from './publishReviewStore.svelte';
  import { storySite } from './publishReview';

  let { owner }: { owner: string } = $props();
  const req = $derived.by(() => (publishReview.current?.owner === owner ? publishReview.current : null));
  const site = $derived.by(() => storySite(req?.url));
</script>

{#if req}
  <Modal title={req.title} onclose={() => publishReview.answer(false)} width={560}>
    {#if req.note}<p class="pr-note" data-testid="publish-review-note">{req.note}</p>{/if}
    <dl class="pr-facts">
      <dt>Where</dt>
      <dd data-testid="publish-review-where">{req.where}{site ? ` · ${site}` : ''}</dd>
      <dt>Who sees it</dt>
      <dd>{req.who}</dd>
    </dl>
    <div class="pr-label" id="pr-what-{owner}">What is sent</div>
    <!-- Not capped here: the Modal body scrolls (and takes a keyboard stop
         when it overflows), so the whole text is reachable. -->
    <pre class="pr-body" aria-labelledby="pr-what-{owner}" data-testid="publish-review-body">{req.what}</pre>
    {#snippet footer()}
      <button class="btn" onclick={() => publishReview.answer(false)} data-autofocus={req?.danger ? '' : undefined}>Cancel</button>
      <button
        class="btn"
        class:primary={!req?.danger}
        class:danger-solid={!!req?.danger}
        data-autofocus={req?.danger ? undefined : ''}
        onclick={() => publishReview.answer(true)}
      >
        {req.verb}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .pr-note {
    margin: 0 0 10px;
    padding: 6px 10px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    color: var(--warning);
    font-size: var(--fs-s);
  }
  .pr-facts {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 4px 12px;
    margin: 0 0 10px;
    font-size: var(--fs-s);
  }
  .pr-facts dt {
    color: var(--text-dim);
  }
  .pr-facts dd {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .pr-label {
    color: var(--text-dim);
    font-size: var(--fs-s);
    margin-block-end: 4px;
  }
  .pr-body {
    margin: 0;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
