<script lang="ts">
  // The batch "Post N comments" confirm. Posting is outward-facing (one
  // notification per comment, under the person's git account), so the dialog
  // shows every comment IN FULL — grouped by file, with its line and severity —
  // and names the destination. Comments whose finding was dismissed or that
  // another review already posted are left out and counted; unverified claims
  // are left out unless the person includes them here.
  import type { ReviewComment } from '../../lib/api/types';
  import Modal from '../../lib/components/Modal.svelte';
  import { plural } from '../../lib/plural';
  import { groupByFile, UNVERIFIED_MARK, type PostAllPlan } from './reviewTriage';

  interface Props {
    plan: PostAllPlan;
    /** "acme/api · PR #42 on GitHub". */
    where: string;
    who: string;
    /** The comments to post, or null when cancelled. */
    onclose: (send: ReviewComment[] | null) => void;
  }
  let { plan, where, who, onclose }: Props = $props();

  let withUnverified = $state(false);
  const send = $derived(withUnverified ? [...plan.send, ...plan.unverified] : plan.send);
  const groups = $derived(groupByFile(send));
  const left = $derived(
    [
      plan.onPr > 0 ? `${plan.onPr} already on the PR` : '',
      plan.dismissed > 0 ? `${plan.dismissed} whose finding was dismissed` : '',
      !withUnverified && plan.unverified.length > 0 ? `${plan.unverified.length} unverified` : '',
    ].filter(Boolean),
  );
</script>

<Modal title={`Post ${plural(send.length, 'comment')}?`} width={620} onclose={() => onclose(null)}>
  {#snippet children()}
    <div class="pcc" data-testid="post-comments-confirm">
      <p><strong>Where:</strong> {where}</p>
      <p><strong>Who sees it:</strong> {who} Each comment is its own notification.</p>
      {#if left.length > 0}
        <p class="dim">Left out: {left.join(' · ')}.</p>
      {/if}
      {#if plan.unverified.length > 0}
        <label class="pcc-unverified">
          <input type="checkbox" bind:checked={withUnverified} data-testid="post-include-unverified" />
          <span>Also post {plural(plan.unverified.length, 'unverified claim')} — the final verifier did not check {plan.unverified.length === 1 ? 'it' : 'them'}, so {plan.unverified.length === 1 ? 'it is' : 'they are'} a reviewer’s guess, not a verified finding.</span>
        </label>
      {/if}
      <div class="pcc-list" role="list" aria-label="Comments to post">
        {#each groups as g (g.path)}
          <section role="listitem">
            <h4 class="mono pcc-file" dir="ltr">{g.path ?? 'General comments'}</h4>
            {#each g.comments as c (c.id)}
              <article class="pcc-comment">
                <div class="pcc-meta">
                  <span class="pcc-sev pcc-sev-{c.severity}">{c.severity}</span>
                  {#if c.line !== null}<span class="mono dim" dir="ltr">line {c.line}</span>{/if}
                  {#if c.body.startsWith(UNVERIFIED_MARK)}<span class="pcc-flag">unverified</span>{/if}
                </div>
                <p class="pcc-body">{c.body}</p>
              </article>
            {/each}
          </section>
        {:else}
          <p class="dim">Nothing to post.</p>
        {/each}
      </div>
    </div>
  {/snippet}
  {#snippet footer()}
    <button class="btn ghost" onclick={() => onclose(null)}>Cancel</button>
    <button class="btn primary" disabled={send.length === 0} data-testid="post-comments-confirm-go" onclick={() => onclose(send)}>
      Post {plural(send.length, 'comment')}
    </button>
  {/snippet}
</Modal>

<style>
  .pcc { display: flex; flex-direction: column; gap: 6px; font-size: var(--fs-s); overflow-wrap: anywhere; }
  .pcc p { margin: 0; }
  .pcc-unverified { display: flex; gap: 8px; align-items: flex-start; padding: 6px 8px; border-radius: var(--radius-s); background: var(--warning-soft); }
  .pcc-unverified input { margin-block-start: 2px; }
  .pcc-list { max-height: min(52vh, 480px); overflow-y: auto; border: 1px solid var(--border); border-radius: var(--radius-s); padding: 4px 10px; }
  .pcc-file { margin: 10px 0 4px; font-size: var(--fs-xs); color: var(--text-dim); font-weight: 600; }
  .pcc-comment { padding: 6px 0; border-block-start: 1px solid var(--border); }
  .pcc-meta { display: flex; gap: 8px; align-items: center; flex-wrap: wrap; font-size: var(--fs-xs); }
  .pcc-sev { font-weight: 600; text-transform: uppercase; }
  .pcc-sev-bug { color: var(--danger); }
  .pcc-sev-warn { color: var(--warning); }
  .pcc-sev-info { color: var(--info); }
  .pcc-flag { color: var(--warning); }
  .pcc-body { margin: 4px 0 0; white-space: pre-wrap; line-height: 1.45; }
  .mono { font-family: var(--font-mono); }
  .dim { color: var(--text-dim); }
</style>
