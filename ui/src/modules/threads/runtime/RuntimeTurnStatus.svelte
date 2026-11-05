<script lang="ts">
  // The status line under a runtime thread's latest turn: "Working for 12s"
  // (or "Continuing on its own" for a turn the agent started itself), then —
  // once it ended — "Stopped" (neutral, never an error), "Stopped · no
  // progress" (the stall watchdog), or "Partly done" when it answered but a
  // step failed, with Details (the failed steps) and Send again.
  import Icon from '../../../lib/components/Icon.svelte';
  import { workLabel, type TurnOutcome } from './timeline';

  interface Props {
    /** "12s" while the turn runs, else null. */
    workingFor: string | null;
    /** The running turn is the agent's own. */
    ownTurn: boolean;
    outcome: TurnOutcome | null;
    disabled?: boolean;
    onresend: (text: string) => void;
  }
  let { workingFor, ownTurn, outcome, disabled = false, onresend }: Props = $props();
  let showFailed = $state(false);
</script>

{#if workingFor}
  <p class="ts" data-testid="runtime-working">
    <span class="spinner" aria-hidden="true"></span>{ownTurn ? `Continuing on its own · ${workingFor}` : `Working for ${workingFor}`}
  </p>
{:else if outcome?.kind === 'stopped'}
  <p class="ts">Stopped</p>
{:else if outcome?.kind === 'stalled'}
  <p class="ts" title="No output for a long while, so Otto stopped the turn">Stopped · no progress</p>
{:else if outcome?.kind === 'partial'}
  <div class="ts" data-testid="runtime-partial">
    <Icon name="info" size={12} />Partly done
    <button type="button" class="btn small" aria-expanded={showFailed} onclick={() => (showFailed = !showFailed)}>Details</button>
    {#if outcome.retryText}
      {@const again = outcome.retryText}
      <button type="button" class="btn small" {disabled} title="Send the same message again" onclick={() => onresend(again)}>Send again</button>
    {/if}
  </div>
  {#if showFailed}
    <ul class="ts-failed" aria-label="Steps that failed">
      {#each outcome.failed as f (f.id)}<li dir="auto">{workLabel(f)}</li>{/each}
    </ul>
  {/if}
{/if}

<style>
  .ts {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 4px 0 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .ts .spinner {
    --spinner-size: 11px;
  }
  .ts-failed {
    margin: 2px 0 0;
    padding-inline-start: 22px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
</style>
