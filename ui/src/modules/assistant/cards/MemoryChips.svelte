<script lang="ts">
  // Memory chips on a response (the `memory` turns the daemon posts when the
  // agent remembers or forgets). "Remembered" has Undo (forget it again);
  // "Forgot" has Undo (restore it); "Suggested" waits for review on the Memory
  // tab when memory approval is on. Every change stays visible in the thread.
  import Icon from '../../../lib/components/Icon.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { assistant, describeError } from '../../../lib/stores/assistant.svelte';
  import type { ChatCard } from '../model';

  type MemoryCard = Extract<ChatCard, { kind: 'memory' }>;
  interface Props {
    cards: MemoryCard[];
    onreview?: () => void;
  }
  let { cards, onreview }: Props = $props();

  let busy = $state<string | null>(null);
  /** chip id → what this window did to it ('undone' | 'restored'). */
  let local = $state<Record<string, 'undone' | 'restored'>>({});

  /** "Remembered: x" → "x" (the daemon's text carries its own verb). */
  function body(text: string): string {
    return text.replace(/^(remembered|forgot|forgotten|suggested)\s*:\s*/i, '');
  }

  async function undo(c: MemoryCard): Promise<void> {
    const u = c.chip?.undo;
    if (!u) return;
    busy = c.id;
    try {
      if (u.kind === 'delete') await assistant.forgetMemory(u.memory_id);
      else await assistant.restoreMemory(u.undo_tokens);
      local = { ...local, [c.id]: u.kind === 'delete' ? 'undone' : 'restored' };
    } catch (e) {
      toasts.error(u.kind === 'delete' ? 'Couldn’t forget that memory' : 'Couldn’t restore that memory', describeError(e));
    } finally {
      busy = null;
    }
  }

  async function redo(c: MemoryCard): Promise<void> {
    const u = c.chip?.undo;
    if (!u || u.kind !== 'delete') return;
    const token = assistant.forgotten[u.memory_id];
    if (!token) return;
    busy = c.id;
    try {
      await assistant.restoreMemory([token]);
      const { [c.id]: _gone, ...rest } = local;
      local = rest;
    } catch (e) {
      toasts.error('Couldn’t restore that memory', describeError(e));
    } finally {
      busy = null;
    }
  }
</script>

{#if cards.length}
  <ul class="chips" aria-label="Memory">
    {#each cards as c (c.id)}
      {@const action = c.chip?.action ?? 'remembered'}
      {@const state = local[c.id]}
      {#if state === 'undone'}
        <li class="chip-m gone" data-testid="memory-undone">
          <Icon name="x" size={12} /><span class="t">Forgot: {body(c.turn.text)}</span>
          <button class="act" onclick={() => void redo(c)} disabled={busy === c.id} title="Remember it again">Restore</button>
        </li>
      {:else if action === 'remembered' || state === 'restored'}
        <li class="chip-m new" data-testid="memory-new">
          <Icon name="plus" size={12} /><span class="t">Remembered: {body(c.turn.text)}</span>
          {#if c.chip?.undo?.kind === 'delete'}
            <button class="act" onclick={() => void undo(c)} disabled={busy === c.id} aria-label={`Undo: forget “${body(c.turn.text)}”`} title="Forget this memory">
              {busy === c.id ? 'Forgetting…' : 'Undo'}
            </button>
          {/if}
        </li>
      {:else if action === 'forgot'}
        <li class="chip-m gone">
          <Icon name="x" size={12} /><span class="t">Forgot: {body(c.turn.text)}</span>
          {#if c.chip?.undo?.kind === 'restore'}
            <button class="act" onclick={() => void undo(c)} disabled={busy === c.id} title="Restore what was forgotten">{busy === c.id ? 'Restoring…' : 'Undo'}</button>
          {/if}
        </li>
      {:else}
        <li class="chip-m pending" title="Memory approval is on — nothing is saved until you accept it">
          <Icon name="clock" size={12} /><span class="t">Suggested: {body(c.turn.text)}</span>
          {#if onreview}<button class="act" onclick={onreview}>Review</button>{/if}
        </li>
      {/if}
    {/each}
  </ul>
{/if}

<style>
  .chips {
    list-style: none;
    margin: 0 0 8px;
    padding: 0;
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip-m {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-height: 22px;
    max-width: 100%;
    padding: 0 10px;
    border-radius: 999px;
    font-size: var(--fs-s);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
  }
  .chip-m .t {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip-m :global(svg) {
    color: var(--text-dim);
  }
  .new {
    border-color: transparent;
    background: var(--info-soft);
    color: var(--info);
  }
  .new :global(svg) {
    color: var(--info);
  }
  .pending {
    border-style: dashed;
    border-color: var(--border-strong);
  }
  .gone .t {
    color: var(--text-dim);
    text-decoration: line-through;
  }
  .act {
    border: 0;
    background: transparent;
    padding: 0 2px;
    font: inherit;
    font-weight: 600;
    color: var(--accent-text);
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .new .act {
    color: var(--info);
  }
  .act:hover:not(:disabled) {
    text-decoration: underline;
  }
  .act:disabled {
    color: var(--text-dim);
    cursor: default;
  }
</style>
