<script lang="ts">
  // Emoji reactions under one PR comment: a pill per reaction (yours
  // highlighted, tap to toggle) and an "add reaction" button whose eight
  // choices open in the shared, viewport-clamped context menu.
  import type { PrComment } from '../../lib/api/types';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { plural } from '../../lib/plural';
  import type { PrCommentActions } from './pr-comment-ctx';
  import { REACTIONS, reactedByMe, reactionEmoji } from './pr-comments';

  interface Props {
    comment: PrComment;
    actions: PrCommentActions;
  }
  let { comment, actions }: Props = $props();
  const list = $derived(comment.reactions ?? []);
  const label = (key: string): string => REACTIONS.find((r) => r.key === key)?.label ?? key;

  function pick(e: MouseEvent): void {
    ctxMenu.show(
      e,
      REACTIONS.map((r) => ({
        label: `${r.emoji}  ${r.label}`,
        checked: reactedByMe(list, r.key),
        action: () => void actions.react(comment, r.key),
      })),
    );
  }
</script>

<div class="rx" role="group" aria-label="Reactions">
  {#each list as r (r.key)}
    <button class="rx-pill" class:mine={r.me} aria-pressed={r.me}
      aria-label="{label(r.key)}: {plural(r.count, 'reaction')}{r.me ? ', including yours' : ''}"
      title={r.me ? `Remove your ${label(r.key)} reaction` : `React with ${label(r.key)}`}
      onclick={() => void actions.react(comment, r.key)}>
      <span aria-hidden="true">{reactionEmoji(r.key)}</span><span class="rx-n">{r.count}</span>
    </button>
  {/each}
  <button class="rx-pill rx-add" aria-haspopup="menu" aria-label="Add a reaction" title="Add a reaction" onclick={pick}>
    <span aria-hidden="true">☺︎+</span>
  </button>
</div>

<style>
  .rx {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin-block: 4px 2px;
  }
  .rx-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-height: 24px;
    padding-inline: 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .rx-pill:hover { border-color: var(--accent-text); }
  .rx-pill.mine {
    border-color: var(--accent-text);
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .rx-n { font-variant-numeric: tabular-nums; font-size: var(--fs-xs); }
  .rx-add { color: var(--text-dim); }
  @media (max-width: 1024px), (pointer: coarse) {
    /* Touch targets: 44px tall, never smaller than a fingertip. */
    .rx-pill {
      min-height: 44px;
      min-width: 44px;
      justify-content: center;
    }
  }
</style>
