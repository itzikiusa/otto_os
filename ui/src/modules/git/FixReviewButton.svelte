<script lang="ts">
  import type { GoalLoop, Review } from '../../lib/api/types';
  import { api } from '../../lib/api/client';
  import { router } from '../../lib/router.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Modal from '../../lib/components/Modal.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { reviewFixError } from './reviewFix';
  import { onDestroy } from 'svelte';
  let { review }: { review: Review } = $props();
  let open = $state(false);
  let target = $state(9.5);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let alive = true;
  onDestroy(() => { alive = false; });
  const unavailable = $derived(reviewFixError(review, target));
  const ineligible = $derived(reviewFixError(review, 9.5));
  async function start(): Promise<void> {
    if (busy || unavailable) return;
    const owner = review.id;
    busy = true;
    error = null;
    try {
      const goal = await api.post<GoalLoop>(`/reviews/${owner}/fix-goal`, { target_score: target });
      if (!alive || review.id !== owner) return;
      open = false;
      router.go(`loops/${goal.id}`);
    } catch (e) {
      if (alive && review.id === owner) error = loadErrorText(e);
    } finally { if (alive) busy = false; }
  }
</script>

<button class="btn small primary" disabled={!!ineligible} title={ineligible ?? 'Fix and independently verify findings in an isolated local worktree'} onclick={() => { open = true; error = null; }}>Run agent to fix…</button>
{#if open}
  <Modal title="Fix review findings" width={460} dismissable={!busy} onclose={() => { if (!busy) open = false; }}>
    {#snippet children()}
      <p>Batch related fixes, verify every previous finding, then run a fresh review.</p>
      <label class="fix-target">Reviewer target (out of 10)<input type="number" min="1" max="10" step="0.1" bind:value={target} /></label>
      <p class="dim">The score is an agent assessment backed by evidence. Unresolved important findings prevent success. Work runs locally from reviewed commit {review.scope_plan?.head_sha.slice(0, 12)}; nothing is committed or published.</p>
      <p class="dim">Up to 6 cohesive iterations or 2 hours. Reaching a limit stays incomplete. Progress, evidence, Stop and Resume are available in Goal Loops.</p>
      {#if error}<LoadState what="fix goal" errorTitle="Couldn’t start fixes" empty {error} onretry={() => void start()} variant="compact" />{/if}
      {#if unavailable}<p class="dim">{unavailable}</p>{/if}
    {/snippet}
    {#snippet footer()}
      <button class="btn" disabled={busy} onclick={() => { open = false; }}>Cancel</button>
      <button class="btn primary" disabled={busy || !!unavailable} onclick={() => void start()}>{busy ? 'Starting…' : 'Run agent to fix'}</button>
    {/snippet}
  </Modal>
{/if}

<style>
  .fix-target { display: flex; gap: 0.75rem; align-items: center; justify-content: space-between; }
  input { width: 5rem; }
  p { font-size: var(--fs-s); line-height: 1.5; }
</style>
