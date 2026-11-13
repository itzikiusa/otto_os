<!--
  The council's rounds (docs/features/threads-structured-runtime.md
  "Council"): "Round 2 · your question", each seat's state and time in that
  round (a seat that sat a round out, never got it or did not finish is
  named), and the syntheses judged on it. Newest round last, kept in view.
  The answers themselves are in the seat columns below, under each round's
  question in that seat's own thread.
-->
<script lang="ts">
  import { tick } from 'svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import type { CouncilRound } from '../../../lib/api/types';
  import { formatElapsed } from './council';
  import { roundHeading, roundStateLabel, type SeatRound } from './councilRounds';

  interface Props {
    rounds: CouncilRound[];
    seatNumbers: number[];
    /** Per seat number: its place in every round (aligned with `rounds`). */
    states: Record<number, SeatRound[]>;
    /** Every synthesis of the council with the round it judged, oldest first. */
    syntheses: { id: string; round: number }[];
    shownSynthesis: string | null;
    onsynthesis: (id: string) => void;
    /** The rounds could not be loaded (the view falls back to round 1). */
    error: string | null;
    onretry: () => void;
  }
  let { rounds, seatNumbers, states, syntheses, shownSynthesis, onsynthesis, error, onretry }: Props = $props();

  let open = $state(true);
  let listEl = $state<HTMLOListElement | null>(null);
  const current = $derived(rounds[rounds.length - 1] ?? null);
  const answered = $derived.by(() => {
    if (!current) return '';
    const k = rounds.length - 1;
    const asked = seatNumbers.filter((n) => states[n]?.[k] && states[n][k].state !== 'not-asked');
    const done = asked.filter((n) => states[n][k].state === 'done').length;
    return `${done} of ${asked.length} answered`;
  });

  // The newest round stays in view as rounds are added.
  $effect(() => {
    void rounds.length;
    void tick().then(() => listEl?.scrollTo({ top: listEl.scrollHeight }));
  });
</script>

<details class="cr" bind:open data-testid="council-rounds">
  <summary class="cr-sum">
    <span class="cr-title">Rounds · {rounds.length}</span>
    {#if current}<span class="cr-now ellipsis" dir="auto">Round {current.n} · {roundHeading(current, 60)} — {answered}</span>{/if}
  </summary>
  {#if error}
    <LoadState what="the rounds" error={error} empty variant="compact" {onretry} />
  {/if}
  <ol class="cr-list" bind:this={listEl}>
    {#each rounds as r, k (r.n)}
      <li class="cr-round" data-testid="council-round-{r.n}">
        <div class="cr-head" dir="auto"><span class="cr-n">Round {r.n}</span> · <span class="cr-q">{roundHeading(r)}</span></div>
        <div class="cr-seats">
          {#each seatNumbers as n (n)}
            {@const st = states[n]?.[k]}
            {#if st && st.state !== 'not-asked'}
              <span class="cr-seat {st.state}" title={st.reason ?? ''} data-testid="council-round-{r.n}-seat-{n}">
                Seat {n} · {roundStateLabel(st.state)}{#if st.elapsedMs !== null} · {formatElapsed(st.elapsedMs)}{/if}
              </span>
            {/if}
          {/each}
          {#each syntheses.filter((x) => x.round === r.n) as x, i (x.id)}
            <button
              type="button"
              class="btn small ghost cr-synth"
              aria-pressed={shownSynthesis === x.id}
              title="Show this synthesis of round {r.n}"
              data-testid="council-round-{r.n}-synthesis"
              onclick={() => onsynthesis(x.id)}
            >Synthesis{i > 0 ? ` ${i + 1}` : ''}</button>
          {/each}
        </div>
      </li>
    {/each}
  </ol>
</details>

<style>
  .cr {
    flex: none;
    padding: 4px 12px;
    border-block-end: 1px solid var(--border);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .cr-sum {
    display: flex;
    align-items: center;
    gap: 8px;
    min-inline-size: 0;
    cursor: pointer;
    padding-block: 2px;
  }
  .cr-title {
    flex: none;
    font-weight: 600;
    color: var(--text);
  }
  .cr-now {
    flex: 1;
    min-inline-size: 0;
  }
  .cr[open] .cr-now {
    display: none;
  }
  .cr-list {
    margin: 4px 0 0;
    padding: 0;
    list-style: none;
    max-block-size: 132px;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .cr-round {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-inline-size: 0;
  }
  .cr-head {
    overflow-wrap: anywhere;
  }
  .cr-n {
    font-weight: 600;
    color: var(--text);
  }
  .cr-q {
    color: var(--text);
  }
  .cr-seats {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
  }
  .cr-seat {
    padding: 0 6px;
    border-radius: var(--radius-s);
    background: var(--surface);
    border: 1px solid var(--border);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .cr-seat.working,
  .cr-seat.queued {
    color: var(--info);
    background: var(--info-soft);
  }
  .cr-seat.done {
    color: var(--success);
    background: var(--success-soft);
  }
  .cr-seat.interrupted,
  .cr-seat.missing,
  .cr-seat.refused {
    color: var(--warning);
    background: var(--warning-soft);
  }
  .cr-seat.failed {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .cr-synth[aria-pressed='true'] {
    color: var(--accent-text);
  }
</style>
