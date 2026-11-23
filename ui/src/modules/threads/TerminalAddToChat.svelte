<script lang="ts">
  // "Add to chat" over a thread terminal while text is selected in it: the
  // selection joins the thread's draft as a context chip ("Terminal · N
  // lines", composerContext.ts). It never sends and never clears the draft.
  import { tick } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { runtimeDrafts } from '../../lib/stores/runtimeDrafts.svelte';
  import { addComposerContext, terminalContext } from './runtime/composerContext';

  interface Props {
    /** The selected text ('' = none). */
    text: string;
    /** The runtime thread whose draft takes it; null = no button. */
    threadId: string | null;
  }
  let { text, threadId }: Props = $props();

  async function add(): Promise<void> {
    if (!threadId || !text.trim()) return;
    const draft = runtimeDrafts.for(threadId);
    draft.context = addComposerContext(draft.context, terminalContext(text));
    await tick();
    document.querySelector<HTMLTextAreaElement>(`[data-testid="runtime-thread-${CSS.escape(threadId)}"] [data-testid="runtime-composer"]`)?.focus({ preventScroll: true });
  }
</script>

{#if threadId && text.trim()}
  <button
    type="button"
    class="btn small tac"
    title="Add the selected terminal text to this thread’s message"
    data-testid="terminal-add-to-chat"
    onmousedown={(e) => e.preventDefault()}
    onclick={() => void add()}
  >
    <Icon name="comment" size={12} />Add to chat
  </button>
{/if}

<style>
  .tac {
    position: absolute;
    inset-block-start: 6px;
    inset-inline-end: 12px;
    z-index: 2;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    box-shadow: var(--shadow-card);
  }
  @media (pointer: coarse) {
    .tac {
      min-block-size: var(--hit-min);
    }
  }
</style>
