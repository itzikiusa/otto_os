<script lang="ts">
  // Two things an unattended loop must not hide: that a restart (not a
  // person) paused it, with the way back, and Otto's own per-run time cap.
  import type { GoalLoop } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { formatSeconds } from '../../lib/metric-format';
  import { RUN_CAP_SECS, pausedByRestart, runCapBinds } from './loopSafety';

  let { loop, elapsedSecs, busy = false, blocked = false, onresume }: {
    loop: GoalLoop;
    /** Active seconds used so far (banked + the open window). */
    elapsedSecs: number;
    /** A lifecycle call is in flight. */
    busy?: boolean;
    /** An open question keeps Resume off. */
    blocked?: boolean;
    onresume: () => void;
  } = $props();

  const restart = $derived(pausedByRestart(loop));
  const binds = $derived(runCapBinds(loop, elapsedSecs));
  const cap = formatSeconds(RUN_CAP_SECS);
</script>

{#if restart}
  <div class="restart">
    <Icon name="warning" size={13} />
    <p>Otto restarted while this loop was running, so it was paused. Its work and progress are kept.</p>
    <button class="btn small" disabled={busy || blocked} onclick={onresume}
      title={blocked ? 'Resume stays off until this answer is recorded.' : 'Continue iterating toward the goal'}>
      <Icon name="play" size={12} /> Resume
    </button>
  </div>
{/if}
<p class="cap-note" class:binds>
  {#if binds}
    This loop’s runtime limit is longer than Otto’s run limit: a run stops after {cap} of active time (the loop shows Exhausted) and Resume starts another.
  {:else}
    Otto stops any run after {cap} of active time, whatever the runtime limit.
  {/if}
</p>

<style>
  .restart {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin: 12px 0 0;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    font-size: var(--fs-m);
  }
  .restart :global(svg) {
    color: var(--warning);
    flex-shrink: 0;
  }
  .restart p {
    flex: 1 1 220px;
    margin: 0;
    min-width: 0;
  }
  .cap-note {
    margin: 8px 0 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .cap-note.binds {
    color: var(--text);
  }
</style>
