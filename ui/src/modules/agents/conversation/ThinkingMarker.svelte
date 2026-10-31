<script lang="ts">
  // Where the agent reasoned. Providers that persist reasoning prose (e.g.
  // Antigravity `agy`) expand to a collapsible violet-tinted Markdown pane;
  // Claude (empty markers) and Codex (dropped rollout items) keep the clear
  // explanation note with token counts when recorded.
  import Icon from '../../../lib/components/Icon.svelte';
  import Markdown from './Markdown.svelte';
  import { fmtTokens } from './format';

  interface Props {
    /** Thinking blocks / Codex reasoning items this marker stands for. */
    count?: number;
    /** Thinking tokens of the response (null when not recorded). */
    tokens?: number | null;
    /** Persisted reasoning prose when available (e.g. Antigravity `agy`). */
    text?: string | null;
    agentName: string;
    /** Codex: reasoning items are dropped by the CLI, not blanked. */
    codex?: boolean;
  }
  let { count = 1, tokens = null, text = null, agentName, codex = false }: Props = $props();
  let open = $state(false);
  const cleanText = $derived(text?.trim() ?? '');
  const hasText = $derived(cleanText.length > 0);
  const label = $derived(
    codex
      ? `Reasoned${count > 1 ? ` · ${count} steps` : ''}`
      : `Thought${!hasText && count > 1 ? ` · ${count} steps` : ''}`,
  );
</script>

<div class="think" class:open class:has-text={hasText} data-thinking={count}>
  <button
    type="button"
    class="think-row"
    onclick={() => (open = !open)}
    aria-expanded={open}
    title={hasText ? 'Reasoning — click to expand' : 'Reasoning — not part of the answer'}
  >
    <span class="think-mark" aria-hidden="true"><Icon name="sparkle" size={12} /></span>
    <span class="think-label">{label}</span>
    {#if tokens}<span class="think-tok">{fmtTokens(tokens)} thinking tokens</span>{/if}
    <span class="think-caret" aria-hidden="true"><Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} /></span>
  </button>
  {#if open}
    {#if hasText}
      <div class="think-prose" data-thinking-prose>
        <Markdown md={cleanText} small />
      </div>
    {:else}
      <p class="think-note">
        {agentName} reasoned here before acting.
        {codex
          ? 'Codex does not save its reasoning to the rollout — only that it happened.'
          : `${agentName === 'Claude' ? 'Claude Code' : agentName} does not save the reasoning text to the transcript — only that it happened.`}
        {#if tokens}This response spent {tokens.toLocaleString()} tokens thinking.{/if}
      </p>
    {/if}
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
    padding-block: 2px;
    padding-inline: 6px 8px;
    border: 1px dashed color-mix(in srgb, var(--cat-4) 45%, transparent);
    border-radius: 999px;
    background: color-mix(in srgb, var(--cat-4) 9%, transparent);
    color: var(--think);
    font: inherit;
    font-size: var(--fs-s);
    font-style: italic;
    cursor: pointer;
    min-width: 0;
  }
  .think.has-text .think-row {
    border-style: solid;
    border-color: color-mix(in srgb, var(--cat-4) 38%, var(--border));
  }
  .think-row:hover {
    background: color-mix(in srgb, var(--cat-4) 15%, transparent);
  }
  .think-row:focus-visible {
    outline: 2px solid var(--accent-text);
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
  .think-prose {
    margin: 6px 0 4px;
    padding: 8px 12px;
    border: 1px solid color-mix(in srgb, var(--cat-4) 26%, var(--border));
    border-inline-start: 3px solid color-mix(in srgb, var(--cat-4) 62%, var(--border-strong));
    border-start-start-radius: var(--radius-s);
    border-end-start-radius: var(--radius-s);
    border-start-end-radius: var(--radius-m);
    border-end-end-radius: var(--radius-m);
    background: color-mix(in srgb, var(--cat-4) 6%, var(--surface));
    max-height: 280px;
    overflow-y: auto;
    max-width: var(--prose-measure, none);
  }
  .think-prose :global(.md) {
    color: color-mix(in srgb, var(--text) 88%, var(--text-dim));
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
