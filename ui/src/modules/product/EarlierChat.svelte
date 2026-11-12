<script lang="ts">
  // EarlierChat — a story's earlier Discovery Chat, read-only. The Chat tab now
  // runs on one engine (RefineChat); these transcripts stay readable as they
  // were, with a one-line note of what each reply proposed. Nothing here
  // sends or applies. Props: { cid } (the discovery chat id).
  import { onDestroy } from 'svelte';
  import { product } from '../../lib/stores/product.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import AgentByline from '../../lib/components/AgentByline.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { renderMarkdown } from '../../lib/md';
  import { proposalSummary } from './chatActions';
  import type { DiscoveryChatDetail } from './types';

  let { cid, oncontinue }: { cid: string; oncontinue: () => void } = $props();

  let detail = $state<DiscoveryChatDetail | null>(null);
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  let loadSeq = 0;
  onDestroy(() => ++loadSeq);

  $effect(() => {
    void load(cid);
  });

  async function load(id: string): Promise<void> {
    const my = ++loadSeq;
    loading = true;
    loadError = null;
    detail = null;
    try {
      const d = await product.getDiscoveryChat(id);
      if (my === loadSeq) detail = d;
    } catch (e) {
      if (my === loadSeq) loadError = loadErrorText(e);
    } finally {
      if (my === loadSeq) loading = false;
    }
  }
</script>

<div class="earlier-chat">
  <div class="read-only-bar">
    <span class="ro-text"><Icon name="clock" size={12} /> Earlier chat · read-only</span>
    <button class="btn small" type="button" onclick={oncontinue} title="Start a new conversation in Chat">
      Continue in a new chat
    </button>
  </div>
  <div class="messages-area">
    {#if loading || loadError || !detail}
      <LoadState what="this chat" loading={loading || (!detail && !loadError)} error={loadError} onretry={() => void load(cid)} />
    {:else if detail.messages.length === 0}
      <EmptyState icon="comment" title="Nothing was said in this chat" body="It was started but never used." />
    {:else}
      {#each detail.messages as m (m.id)}
        {@const proposed = m.role === 'agent' ? proposalSummary(m.actions_json) : ''}
        <div class="bubble-row" class:row-user={m.role === 'user'}>
          <div class="bubble" class:bubble-user={m.role === 'user'} class:bubble-agent={m.role === 'agent'}>
            <div class="bubble-header">
              {#if m.role === 'agent'}
                <AgentByline model={detail.chat.model} at={m.created_at} />
              {:else}
                <span class="bubble-role">You</span>
                <RelTime iso={m.created_at} class="bubble-time" />
              {/if}
            </div>
            {#if m.role === 'agent'}
              <div class="bubble-body md-body">{@html renderMarkdown(m.body)}</div>
            {:else}
              <div class="bubble-body plain">{m.body}</div>
            {/if}
            {#if proposed}
              <p class="proposed">{proposed}</p>
            {/if}
          </div>
        </div>
      {/each}
    {/if}
  </div>
</div>

<style>
  .earlier-chat {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .read-only-bar {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 8px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border);
  }
  .ro-text {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .messages-area {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px 8px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .bubble-row {
    display: flex;
    justify-content: flex-start;
  }
  .bubble-row.row-user {
    justify-content: flex-end;
  }
  .bubble {
    max-width: 80%;
    min-width: 0;
    border-radius: var(--radius-s);
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .bubble-user {
    background: var(--accent-soft-strong);
    border: 1px solid var(--accent-line);
  }
  .bubble-agent {
    background: var(--surface);
    border: 1px solid var(--border);
    border-inline-start: 2px solid var(--accent-line-strong);
  }
  .bubble-header {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .bubble-role {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .bubble-header :global(.bubble-time) {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .bubble-body {
    font-size: var(--fs-m);
    line-height: 1.55;
    color: var(--text);
    overflow-wrap: break-word;
  }
  .bubble-body.plain {
    white-space: pre-wrap;
  }
  .proposed {
    margin: 4px 0 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-style: italic;
  }
</style>
