<script lang="ts">
  // The PR header's "something changed" pill. Two cases (pr-head-follow.ts):
  //   · activity only (comments, state, title) → "Updated · Refresh": nothing
  //     is swapped under the reader until they ask;
  //   · a push moved the head → the files and diff already followed it, so
  //     the pill SAYS so ("3 new commits · files updated") and only dismisses.
  import Icon from '../../lib/components/Icon.svelte';
  import { announceShown } from '../../lib/announce.svelte';

  interface Props {
    /** A fresher copy is waiting (activity only). */
    pending: boolean;
    /** Set after the files followed a new head; null = nothing to tell. */
    headNote: string | null;
    onapply: () => void;
    ondismiss: () => void;
  }
  let { pending, headNote, onapply, ondismiss }: Props = $props();
</script>

{#if headNote}
  <button class="btn small prd-pending" data-testid="pr-head-moved" onclick={ondismiss} use:announceShown={{ text: `Pull request updated: ${headNote}` }}
    title="New commits were pushed: the file list and diff now show the latest head. Click to dismiss.">
    <Icon name="refresh" size={12} /> {headNote}
  </button>
{:else if pending}
  <button class="btn small prd-pending" onclick={onapply} title="Load the latest version of this pull request">
    <Icon name="refresh" size={12} /> Updated · Refresh
  </button>
{/if}

<style>
  .prd-pending { color: var(--info); border-color: color-mix(in srgb, var(--info) 40%, var(--border)); }
</style>
