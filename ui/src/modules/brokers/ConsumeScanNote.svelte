<script lang="ts">
  // The footnote under a peek's message list: how much of the range a
  // FILTERED peek actually searched (the daemon scans past non-matches, up to
  // its 50,000-message cap), so "no matches" never reads as "not in the topic"
  // when only part of it was searched — plus the plain "limit reached" note.
  import type { ConsumeResp } from '../../lib/api/types';

  interface Props {
    result: ConsumeResp | null;
    /** The peek searched from the oldest offset ("From start"). */
    fromStart: boolean;
  }
  let { result, fromStart }: Props = $props();

  const fmt = (n: number) => n.toLocaleString();
</script>

{#if result?.scanned !== undefined && result.scanned !== null}
  <p class="muted pad small">
    Scanned {fmt(result.scanned)} {result.scanned === 1 ? 'message' : 'messages'}, {fmt(result.messages.length)}
    {result.messages.length === 1 ? 'match' : 'matches'}.
    {#if result.scan_capped}
      The scan stopped at its 50,000-message cap — narrow the start (offset or time) or pick a partition to search further.
    {:else if !fromStart}
      Only the latest messages were searched — tick “From start” to search the whole topic.
    {:else if result.truncated}
      Showing the first {fmt(result.messages.length)} — increase the limit for more.
    {/if}
  </p>
{:else if result?.truncated}
  <p class="muted pad small">Showing first {result.messages.length} — increase the limit for more.</p>
{/if}

<style>
  .muted {
    color: var(--text-dim);
  }
  .pad {
    padding: 14px;
  }
  .small {
    font-size: var(--fs-xs);
  }
</style>
