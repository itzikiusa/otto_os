<script lang="ts">
  // Review state of one discovered MCP tool. A server's tools are reviewed
  // when it is added; a tool that appears later starts disabled ("New"), and
  // a tool whose description / input schema changed since review needs
  // approval on every call ("Changed") until a person acknowledges it here —
  // a server can't quietly rewrite what a tool says it does.
  import Badge from '../../lib/components/Badge.svelte';
  import type { BadgeTone } from '../../lib/status';
  import type { McpToolView } from '../../lib/api/types';

  let {
    tool,
    busy = false,
    canConfigure = false,
    onacknowledge,
  }: {
    tool: McpToolView;
    busy?: boolean;
    canConfigure?: boolean;
    onacknowledge: () => void;
  } = $props();

  /** The badge for a tool's review state, or `null` when it is reviewed. */
  function reviewState(t: Pick<McpToolView, 'unreviewed' | 'changed_since_review'>): { label: string; tone: BadgeTone; hint: string } | null {
    if (t.changed_since_review) {
      return {
        label: 'Changed since reviewed',
        tone: 'warn',
        hint: 'The server changed this tool’s description or input schema after it was reviewed. Every call needs approval until you acknowledge the new version.',
      };
    }
    if (t.unreviewed) {
      return {
        label: 'New — not reviewed',
        tone: 'info',
        hint: 'This tool appeared after the server was reviewed, so it starts disabled. Enabling or acknowledging it marks it reviewed.',
      };
    }
    return null;
  }

  const review = $derived(reviewState(tool));
</script>

{#if review}
  <span class="review">
    <Badge tone={review.tone} label={review.label} title={review.hint} dot />
    {#if canConfigure}
      <button
        class="btn small"
        disabled={busy}
        onclick={onacknowledge}
        title={`Mark the current description and schema of ${tool.name} as reviewed`}
      >
        Acknowledge
      </button>
    {/if}
  </span>
{/if}

<style>
  .review {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin-block-start: 2px;
  }
</style>
