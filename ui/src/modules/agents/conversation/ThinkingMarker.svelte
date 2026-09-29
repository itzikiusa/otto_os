<script lang="ts">
  // Where the agent reasoned. Claude persists thinking blocks EMPTY (a marker
  // only) and Codex drops reasoning items (counted per turn), so there is no
  // text to show — but "it thought here, this much" must never be confused
  // with the answer: its own violet tint, italic, a sparkle mark, and the
  // turn's thinking-token count when the transcript has usage. Expanding says
  // why the text is not there.
  import Icon from '../../../lib/components/Icon.svelte';
  import { fmtTokens } from './format';

  interface Props {
    /** Thinking blocks / Codex reasoning items this marker stands for. */
    count?: number;
    /** Thinking tokens of the response (null when not recorded). */
    tokens?: number | null;
    agentName: string;
    /** Codex: reasoning items are dropped by the CLI, not blanked. */
    codex?: boolean;
  }
  let { count = 1, tokens = null, agentName, codex = false }: Props = $props();
  let open = $state(false);
  const label = $derived(codex ? `Reasoned${count > 1 ? ` · ${count} steps` : ''}` : 'Thought');
</script>

<div class="think" class:open data-thinking={count}>
  <button class="think-row" onclick={() => (open = !open)} aria-expanded={open} title="Reasoning — not part of the answer">
    <span class="think-mark" aria-hidden="true"><Icon name="sparkle" size={12} /></span>
    <span class="think-label">{label}</span>
    {#if tokens}<span class="think-tok">{fmtTokens(tokens)} thinking tokens</span>{/if}
    <span class="think-caret" aria-hidden="true"><Icon name={open ? 'chevronDown' : 'chevronRight'} size={11} /></span>
  </button>
  {#if open}
    <p class="think-note">
      {agentName} reasoned here before acting.
      {codex ? 'Codex does not save its reasoning to the rollout — only that it happened.' : 'Claude Code does not save the reasoning text to the transcript — only that it happened.'}
      {#if tokens}This response spent {tokens.toLocaleString()} tokens thinking.{/if}
    </p>
  {/if}
</div>

<style>
  .think {
    --think: color-mix(in srgb, var(--cat-4) 72%, var(--text));
    min-width: 0;
    margin: 2px 0;
  }
  .think-row {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    padding: 2px 9px 2px 7px;
    border: 1px dashed color-mix(in srgb, var(--cat-4) 45%, transparent);
    border-radius: 99px;
    background: color-mix(in srgb, var(--cat-4) 9%, transparent);
    color: var(--think);
    font: inherit;
    font-size: var(--fs-s);
    font-style: italic;
    cursor: pointer;
    min-width: 0;
  }
  .think-row:hover {
    background: color-mix(in srgb, var(--cat-4) 15%, transparent);
  }
  .think-row:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .think-mark {
    display: inline-flex;
    flex-shrink: 0;
  }
  .think-label {
    font-weight: 500;
  }
  .think-tok {
    font-style: normal;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    opacity: 0.85;
    white-space: nowrap;
  }
  .think-caret {
    display: inline-flex;
    opacity: 0.8;
  }
  .think-note {
    margin: 4px 0 2px;
    padding-inline-start: 10px;
    border-inline-start: 2px solid color-mix(in srgb, var(--cat-4) 45%, transparent);
    font-size: var(--fs-xs);
    font-style: italic;
    color: var(--text-dim);
    max-width: var(--prose-measure, none);
  }
</style>
