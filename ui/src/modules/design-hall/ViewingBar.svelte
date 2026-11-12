<script lang="ts" module>
  import type { DesignVersion as V } from '../../lib/api/types';
  /** One older version shown alone in the stage (read-only). */
  export interface Viewing {
    v: V;
    text: string | null;
    blobUrl: string | null;
    loading: boolean;
    error: string | null;
  }
</script>

<script lang="ts">
  // The stage toolbar line while an older version is shown on its own:
  // "Viewing v5 · read-only" with Back to current, Compare with current,
  // Restore and Approve — the same verbs as its row in Versions.
  import Icon from '../../lib/components/Icon.svelte';
  import { announceShown } from '../../lib/announce.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';

  interface Props {
    viewing: Viewing;
    canEdit: boolean;
    approved: boolean;
    onback: () => void;
    oncompare: () => void;
    onrestore: () => void;
    onapprove: () => void;
    onretry: () => void;
  }
  let { viewing, canEdit, approved, onback, oncompare, onrestore, onapprove, onretry }: Props = $props();
</script>

<span class="viewing" data-testid="design-viewing" use:announceShown={`Viewing v${viewing.v.seq}, read-only.`}>
  <Icon name="eye" size={12} />
  <strong>Viewing v{viewing.v.seq}</strong> · read-only
  {#if viewing.loading}<span class="spinner" role="img" aria-label={`Loading v${viewing.v.seq}`}></span>{/if}
  {#if viewing.error}
    <LoadState what={`v${viewing.v.seq}`} variant="compact" error={viewing.error} empty {onretry} />
  {/if}
  <button class="btn small primary" onclick={onback} data-testid="design-back-to-current">Back to current</button>
  <button class="btn small ghost" onclick={oncompare}><Icon name="columns" size={12} /> Compare</button>
  {#if canEdit}
    <button class="btn small ghost" onclick={onrestore}><Icon name="refresh" size={12} /> Restore</button>
    {#if !approved}<button class="btn small ghost" onclick={onapprove}><Icon name="check" size={12} /> Approve</button>{/if}
  {/if}
</span>

<style>
  .viewing {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--fs-s);
  }
</style>
