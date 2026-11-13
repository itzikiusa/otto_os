<!--
  The council's follow-up composer (docs/features/threads-structured-runtime.md
  "Council"): one text box under the columns, a target chooser ("All seats"
  or one seat — a seat's header picks it too), the round counter, and
  "Stop all" while seats of the round still work. Nothing is sent without a
  click: every round costs the person tokens.
-->
<script lang="ts">
  import { announceShown } from '../../../lib/announce.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { followUpBlocker, targetLabel } from './councilRounds';

  interface Props {
    /** The council's seat numbers. */
    seatNumbers: number[];
    /** Seats the next follow-up goes to (all of them = "All seats"). */
    targets: number[];
    text: string;
    rounds: number;
    maxRounds: number;
    sending: boolean;
    error: string | null;
    /** Seats of the current round still working ("Stop all"). */
    working: number;
    stopping: boolean;
    ontargets: (targets: number[]) => void;
    ontext: (text: string) => void;
    onsend: () => void;
    onstop: () => void;
  }
  let { seatNumbers, targets, text, rounds, maxRounds, sending, error, working, stopping, ontargets, ontext, onsend, onstop }: Props = $props();

  let chooserEl = $state<HTMLButtonElement | null>(null);
  const blocker = $derived(followUpBlocker(rounds, maxRounds, targets.length));
  const atCap = $derived(rounds >= maxRounds);
  const canSend = $derived(!blocker && !sending && !!text.trim());
  const label = $derived(targetLabel(targets, seatNumbers.length));

  function choose(): void {
    if (!chooserEl) return;
    const all = targets.length === seatNumbers.length;
    ctxMenu.showAt(
      chooserEl,
      [
        { label: 'All seats', checked: all, action: () => ontargets([...seatNumbers]) },
        ...seatNumbers.map((n) => ({
          label: `Seat ${n} only`,
          checked: !all && targets.length === 1 && targets[0] === n,
          action: () => ontargets([n]),
        })),
      ],
      { align: 'start' },
    );
  }

  function keydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      if (canSend) onsend();
    }
  }
</script>

<div class="cc" data-testid="council-composer">
  {#if atCap}
    <p class="cc-cap" data-testid="council-round-cap" use:announceShown={blocker ?? ''}>
      <Icon name="info" size={12} /><span>{blocker}</span>
    </p>
  {:else}
    <textarea
      class="cc-text"
      rows={2}
      dir="auto"
      value={text}
      placeholder={`Ask ${label === 'All seats' ? 'the council' : label.toLowerCase()} a follow-up — each seat answers in its own thread`}
      aria-label="Follow-up for the council"
      data-testid="council-followup"
      oninput={(e) => ontext((e.currentTarget as HTMLTextAreaElement).value)}
      onkeydown={keydown}
    ></textarea>
  {/if}
  {#if error}
    <p class="cc-error" use:announceShown={{ text: `Couldn’t ask the council: ${error}`, assertive: true }}>Couldn’t ask the council: {error}</p>
  {/if}
  <div class="cc-row">
    <span class="cc-count" data-testid="council-round-count" title="Each follow-up is a round; a council runs at most {maxRounds}">Round {Math.min(rounds, maxRounds)} of {maxRounds}</span>
    <span class="cc-spacer"></span>
    {#if working > 0}
      <button
        type="button"
        class="btn small ghost"
        disabled={stopping}
        title="Stop every seat still working on this round"
        data-testid="council-stop-all"
        onclick={onstop}
      >
        <Icon name="stop" size={12} /> Stop all
      </button>
    {/if}
    {#if !atCap}
      <button
        bind:this={chooserEl}
        type="button"
        class="btn small ghost"
        aria-haspopup="menu"
        title="Who gets the follow-up"
        data-testid="council-target"
        onclick={choose}
      >To: {label}</button>
      <button
        type="button"
        class="btn small primary"
        disabled={!canSend}
        title={blocker ?? 'Send this follow-up as the next round (⌘↩)'}
        data-testid="council-send-followup"
        onclick={onsend}
      >
        {#if sending}<span class="spinner" aria-hidden="true"></span>{/if}
        Ask round {rounds + 1}
      </button>
    {/if}
  </div>
</div>

<style>
  .cc {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 12px;
    border-block-start: 1px solid var(--border);
    flex: none;
  }
  .cc-text {
    inline-size: 100%;
    box-sizing: border-box;
    resize: vertical;
    min-block-size: 44px;
    max-block-size: 30vh;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
  }
  .cc-row {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .cc-count {
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .cc-spacer {
    flex: 1;
  }
  .cc-cap,
  .cc-error {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .cc-error {
    color: var(--danger);
  }
  .cc .spinner {
    --spinner-size: 12px;
  }
</style>
