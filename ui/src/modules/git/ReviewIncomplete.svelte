<script lang="ts">
  // A review in `error`. When its reviewers finished but the final verifier
  // did not (or a lens is missing), the daemon keeps their findings as draft
  // comments marked `[unverified] ` and says "Review incomplete — not a
  // verdict: …". Show that — and the findings — instead of a bare "failed":
  // they are a reviewer's claims, not verified, so nothing here posts them.
  import type { Review } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  let { review }: { review: Review } = $props();

  const INCOMPLETE = 'Review incomplete — not a verdict: ';
  const UNVERIFIED = '[unverified] ';
  const cause = $derived(review.error?.split(INCOMPLETE)[1] ?? null);
  const unverified = $derived(review.comments.filter(c => c.body.startsWith(UNVERIFIED)));
</script>

{#if cause !== null}
  <div class="ri card" data-testid="review-incomplete" use:announceShown={{ text: 'Review incomplete — not a verdict', assertive: true }}>
    <div class="ri-head"><Icon name="warning" size={14} /> <strong>Review incomplete — not a verdict</strong></div>
    <p class="ri-msg">{cause}</p>
    {#if unverified.length}
      <details>
        <summary>{unverified.length} unverified reviewer {unverified.length === 1 ? 'finding' : 'findings'}</summary>
        <ul>
          {#each unverified as c (c.id)}
            <li><span class="dim">[{c.severity}] {c.path ?? 'general'}{c.line ? `:${c.line}` : ''}</span> — {c.body.slice(UNVERIFIED.length)}</li>
          {/each}
        </ul>
      </details>
    {/if}
  </div>
{:else}
  <div class="ri ri-failed card" use:announceShown={{ text: 'The review failed', assertive: true }}>
    <Icon name="warning" size={14} />
    <span class="ri-msg">The review failed. <span class="dim">{review.error ?? 'No reason was reported — check Settings → Logs.'}</span></span>
  </div>
{/if}

<style>
  .ri { padding: 12px 14px; margin-block-start: 8px; font-size: var(--fs-s); overflow-wrap: anywhere; color: var(--warning); }
  .ri-failed { display: flex; align-items: center; gap: 10px; color: var(--danger); }
  .ri-head { display: flex; align-items: center; gap: 8px; }
  .ri-msg { flex: 1; min-width: 0; margin-block: 6px; color: var(--text); }
  .ri-failed .ri-msg { margin-block: 0; color: inherit; }
  summary { cursor: pointer; color: var(--text); }
  ul { padding-inline-start: 1.25rem; margin-block: 6px; color: var(--text); }
  li + li { margin-block-start: 6px; }
</style>
