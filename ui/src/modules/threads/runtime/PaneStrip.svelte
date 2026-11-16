<!--
  One thin row above Split / Grid panes (T2-04, T2-14): a chip per pane — its
  state in one word, click to focus it (amber when it needs you) — and
  "Message every pane…". When every pane is one council it is the council's
  row: its seats and synthesis, Ask every seat, Synthesize (disabled with the
  reason until a seat answered and none works), Stop all, and "Continue with
  this answer" once the synthesis answered. On a phone (one seat at a time)
  it is a seat switcher with one "Council" menu. The model is paneStrip.ts.
-->
<script lang="ts">
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { harnessLabel, providerReadiness } from '../../../lib/providers';
  import { sessionState } from '../../../lib/status';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import type { RuntimeProviderOrSlug } from '../../../lib/api/types';
  import { messagePanes } from '../threadRelay';
  import { threads } from '../threadsStore.svelte';
  import { answerItems, askEverySeat, continueWithAnswer, judgeChoices, seatNamer, stopSeats, synthesizeWith } from './councilStore.svelte';
  import { PANE_STATE_LABEL, stoppable, stripModel, synthGate, type StripChip } from './paneStrip';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { formatTokens } from './turnUsage';

  interface Props {
    paneIds: readonly string[];
    focusedId: string | null;
    /** One pane at a time: a council's seat switcher, nothing otherwise. */
    phone?: boolean;
  }
  let { paneIds, focusedId, phone = false }: Props = $props();

  const model = $derived(
    stripModel({
      paneIds,
      focusedId,
      all: Object.values(runtimeThreads.byId),
      projections: runtimeViews.projections,
      needsYou: (id) => ws.needsYou[id] === true,
      legacy: (id) => {
        const s = ws.getSession(id);
        return s ? { title: s.title, provider: s.provider, stateKey: sessionState(s, ws.statusMap[id] ?? s.status, ws.needsYou[id] === true).key } : null;
      },
      name: seatNamer,
    }),
  );
  const council = $derived(model.council);
  const gate = $derived(synthGate(model.chips));
  const busy = $derived(stoppable(model.chips));
  const synth = $derived(model.chips.find((c) => c.seat === 0 && c.id === council?.synthesis?.id) ?? null);
  const answered = $derived(synth?.state === 'answered');
  const anchorId = $derived(focusedId ?? paneIds[0] ?? null);
  const visible = $derived(phone ? !!council : paneIds.length >= 2);
  let stopping = $state(false);

  function pick(c: StripChip): void {
    if (c.shown) threads.focusPane(c.id);
    else if (phone) threads.selectThread(c.id);
    else threads.openBeside(c.id);
  }

  function chipMenu(e: MouseEvent, c: StripChip): void {
    if (!council) return;
    e.preventDefault();
    ctxMenu.show(e, answerItems(c.id));
  }

  async function stopAll(): Promise<void> {
    if (stopping) return;
    stopping = true;
    try {
      await stopSeats(busy.map((c) => c.id));
    } finally {
      stopping = false;
    }
  }

  function judgeItems(id: string): MenuItem[] {
    return judgeChoices(id).map((p) => ({
      label: harnessLabel(p),
      disabled: !providerReadiness(p).available,
      action: () => void synthesizeWith(id, p as RuntimeProviderOrSlug),
    }));
  }

  function synthesize(e: MouseEvent): void {
    if (!gate.enabled || !anchorId) return;
    const judges = judgeItems(anchorId).filter((j) => !j.disabled);
    if (judges.length) judges[0].action?.();
    else ctxMenu.show(e, judgeItems(anchorId));
  }

  function councilMenu(e: MouseEvent): void {
    if (!anchorId) return;
    ctxMenu.show(e, [
      { label: 'Ask every seat…', icon: 'send', action: () => void askEverySeat(anchorId) },
      { label: 'Synthesize with', icon: 'sparkle', disabled: !gate.enabled, title: gate.reason || undefined, children: judgeItems(anchorId) },
      ...(busy.length ? [{ label: `Stop ${busy.length === 1 ? 'the working seat' : `${busy.length} working seats`}`, icon: 'stop', action: () => void stopAll() }] : []),
      { separator: true },
      ...answerItems(anchorId),
    ]);
  }

  const chipTitle = (c: StripChip): string =>
    `${c.name} · ${PANE_STATE_LABEL[c.state]}${c.tokens ? ` · ${formatTokens(c.tokens)}` : ''}${c.shown ? '' : ' · not shown — click to open'}`;
</script>

{#if visible}
  <div
    class="ps"
    class:phone
    role="toolbar"
    aria-label={council ? `Council ${council.title}` : 'Panes'}
    data-testid="threads-pane-strip"
    data-council={council ? council.id : undefined}
  >
    {#if council && !phone}
      <span class="ps-title ellipsis" title="Council · {council.title}" data-testid="pane-strip-title">
        <Icon name="people" size={13} /><span class="ellipsis">Council · {council.title}</span>
      </span>
    {/if}
    <div class="ps-chips" role="group" aria-label={council ? 'Seats' : 'Panes'}>
      {#each model.chips as c (c.id)}
        <button
          type="button"
          class="ps-chip"
          class:focused={c.focused}
          class:hidden-pane={!c.shown}
          data-state={c.state}
          data-testid="pane-chip-{c.id}"
          aria-pressed={c.focused}
          aria-label="{c.name}, {PANE_STATE_LABEL[c.state]}"
          title={chipTitle(c)}
          onclick={() => pick(c)}
          oncontextmenu={(e) => chipMenu(e, c)}
        >
          <span class="ps-dot" aria-hidden="true"></span>
          {#if hasProviderIcon(c.provider)}<ProviderIcon provider={c.provider} size={12} />{/if}
          <span class="ps-label ellipsis" dir="auto">{c.label}</span>
          <span class="ps-state">{PANE_STATE_LABEL[c.state]}</span>
        </button>
      {/each}
    </div>
    {#if model.tokens && !phone}
      <span class="ps-total" title="Tokens reported by the loaded turns" data-testid="pane-strip-tokens">{formatTokens(model.tokens)}</span>
    {/if}
    <div class="ps-actions">
      {#if phone}
        <button type="button" class="btn small" aria-haspopup="menu" data-testid="pane-strip-council-menu" onclick={councilMenu}>
          Council <Icon name="chevronDown" size={12} />
        </button>
      {:else if !council}
        <button type="button" class="btn small ghost" data-testid="pane-strip-message" onclick={() => void messagePanes(paneIds)}>
          <Icon name="send" size={12} />Message every pane…
        </button>
      {:else}
        <button type="button" class="btn small ghost" data-testid="pane-strip-ask" onclick={() => anchorId && void askEverySeat(anchorId)}>
          <Icon name="send" size={12} />Ask every seat…
        </button>
        {#if busy.length}
          <button type="button" class="btn small ghost" disabled={stopping} data-testid="pane-strip-stop" onclick={() => void stopAll()}>
            <Icon name="stop" size={12} />{stopping ? 'Stopping…' : 'Stop all'}
          </button>
        {/if}
        {#if !gate.enabled}<span class="ps-why" data-testid="pane-strip-why">{gate.reason}</span>{/if}
        <span class="ps-split">
          <button
            type="button"
            class="btn small"
            class:primary={gate.enabled && !answered}
            aria-disabled={!gate.enabled}
            title={gate.enabled ? 'Compare the seats’ answers in a new thread' : gate.reason}
            data-testid="pane-strip-synthesize"
            onclick={synthesize}
          >
            <Icon name="sparkle" size={12} />{synth ? 'Synthesize again' : 'Synthesize'}
          </button>
          <button
            type="button"
            class="btn small ps-caret"
            aria-haspopup="menu"
            aria-label="Synthesize with…"
            title={gate.enabled ? 'Synthesize with…' : gate.reason}
            aria-disabled={!gate.enabled}
            data-testid="pane-strip-synthesize-with"
            onclick={(e) => gate.enabled && anchorId && ctxMenu.show(e, judgeItems(anchorId))}
          >
            <Icon name="chevronDown" size={12} />
          </button>
        </span>
        {#if synth && answered}
          <button type="button" class="btn small primary" data-testid="pane-strip-continue" onclick={() => void continueWithAnswer(synth.id)}>
            Continue with this answer
          </button>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
  .ps {
    container-type: inline-size;
    display: flex;
    align-items: center;
    gap: 8px;
    min-block-size: 32px;
    padding-inline: 8px;
    padding-block: 2px;
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    font-size: var(--fs-s);
    flex-shrink: 0;
    min-width: 0;
  }
  .ps-title {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-inline-size: 180px;
    font-weight: 600;
    color: var(--text);
    flex-shrink: 1;
  }
  .ps-chips {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    flex: 1 1 auto;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .ps-chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    max-inline-size: 240px;
    block-size: 24px;
    padding-inline: 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: var(--bg);
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
    /* Every chip stays on screen: they shrink (the name ellipsizes) before
       the row scrolls. */
    flex: 0 1 auto;
    min-inline-size: 72px;
  }
  .ps-chip:hover {
    background: var(--hover);
  }
  .ps-chip.focused {
    border-color: var(--accent-line-strong);
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .ps-chip.hidden-pane {
    border-style: dashed;
  }
  .ps-chip[data-state='needs-you'] {
    border-color: var(--warning);
    background: var(--warning-soft);
    color: var(--text);
  }
  .ps-label {
    min-width: 4.5em;
    color: var(--text);
    font-weight: 500;
  }
  .ps-state {
    flex-shrink: 0;
  }
  .ps-dot {
    inline-size: 7px;
    block-size: 7px;
    border-radius: 50%;
    background: var(--text-dim);
    flex-shrink: 0;
  }
  [data-state='working'] .ps-dot {
    background: var(--success);
  }
  [data-state='needs-you'] .ps-dot {
    background: var(--warning);
  }
  [data-state='failed'] .ps-dot {
    background: var(--danger);
  }
  [data-state='failed'] .ps-state {
    color: var(--danger);
  }
  [data-state='answered'] .ps-dot {
    background: var(--accent-text);
  }
  .ps-total {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    flex-shrink: 0;
    white-space: nowrap;
  }
  .ps-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .ps-why {
    color: var(--text-dim);
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .ps-split {
    display: inline-flex;
  }
  .ps-split > :first-child {
    border-start-end-radius: 0;
    border-end-end-radius: 0;
  }
  .ps-caret {
    border-start-start-radius: 0;
    border-end-start-radius: 0;
    border-inline-start: 0;
    padding-inline: 4px;
  }
  .ps-actions [aria-disabled='true'] {
    opacity: var(--disabled-opacity);
    cursor: default;
  }
  /* Narrow canvas: chips keep their names; the state word and the reason go
     to the chip's / button's title. */
  /* The chip's name outlasts its state word: the dot keeps the colour, the
     title the word — except "Needs you" and "Failed", which stay. */
  @container (max-width: 1180px) {
    .ps-chip:not([data-state='needs-you'], [data-state='failed']) .ps-state {
      display: none;
    }
  }
  @container (max-width: 1100px) {
    .ps-why,
    .ps-total {
      display: none;
    }
  }
  @container (max-width: 820px) {
    .ps-title,
    .ps-state {
      display: none;
    }
  }
  .ps.phone {
    gap: 6px;
    padding-block: 4px;
  }
  .ps.phone .ps-chip,
  .ps.phone .ps-actions :global(.btn) {
    min-block-size: 44px;
  }
  .ps.phone .ps-chip {
    flex-shrink: 0;
    max-inline-size: 200px;
    padding-inline: 12px;
  }
  .ps.phone .ps-state {
    display: inline;
  }
</style>
