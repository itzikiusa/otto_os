<!--
  A council in Threads (docs/features/threads-structured-runtime.md
  "Council"): the same question answered by 2–4 agents in columns (one at a
  time on a phone, picked with a segmented control), then ONE action —
  Synthesize with a judge — whose answer is the last, full-width column.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Tabs, { type TabItem } from '../../../lib/components/Tabs.svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import type { RuntimeProvider } from '../../../lib/api/types';
  import { loadErrorText } from '../../../lib/loadError';
  import { agentProviders, harnessLabel, providerReadiness } from '../../../lib/providers';
  import type { RuntimeEmbedStatus } from '../../../lib/runtimeThreadPort';
  import { councilMembers, synthesizeBlocker } from './council';
  import { council } from './councilStore.svelte';
  import CouncilSeatColumn from './CouncilSeatColumn.svelte';
  import { isRuntimeProvider, runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';

  interface Props {
    /** Any thread of the council (seat 1 when opened from the sidebar). */
    sessionId: string;
  }
  let { sessionId }: Props = $props();

  const members = $derived(councilMembers(Object.values(runtimeThreads.byId), sessionId));
  const seats = $derived(members?.seats ?? []);
  const synthesis = $derived(members?.synthesis ?? null);
  const title = $derived((seats[0]?.title ?? 'Council').replace(/ · [^·]*$/, ''));
  /** Each column's state, from its embed. */
  let statuses = $state<Record<string, RuntimeEmbedStatus>>({});
  /** The seat a phone shows (index into `seats`, or -1 = the synthesis). */
  let picked = $state(0);
  let judge = $state<RuntimeProvider | null>(null);
  let judgeEl = $state<HTMLButtonElement | null>(null);
  let synthesizing = $state(false);
  let synthError = $state<string | null>(null);
  let loadError = $state<string | null>(null);
  let attempt = $state(0);

  /** A phone shows one seat at a time. */
  const seatTabs = $derived<TabItem[]>([
    ...seats.map((_, i) => ({ id: String(i), label: `Seat ${i + 1}`, testid: `council-tab-${i + 1}` })),
    ...(synthesis ? [{ id: '-1', label: 'Synthesis', testid: 'council-tab-synthesis' }] : []),
  ]);
  const judgeProvider = $derived<RuntimeProvider>(judge ?? seats[0]?.provider ?? 'claude');
  const working = (id: string): boolean => statuses[id] === 'working' || runtimeThreads.get(id)?.status === 'working';
  const blocker = $derived(
    synthesizeBlocker(
      seats.map((s, i) => ({ seat: i + 1, working: working(s.id) })),
      synthesis ? working(synthesis.id) : false,
    ),
  );
  const question = $derived.by(() => {
    const items = seats[0] ? (runtimeViews.projections[seats[0].id]?.items ?? []) : [];
    const first = items.find((i) => i.type === 'user_message' && !i.parent_item_id);
    return first && first.type === 'user_message' ? first.text : '';
  });

  onMount(() => runtimeThreads.mount());
  $effect(() => {
    const id = sessionId;
    void attempt;
    loadError = null;
    // The seats are this thread's linked group (an agent-opened peer is not).
    void runtimeThreads.loadOne(id).then(
      () => runtimeThreads.loadLinked(id),
      (e: unknown) => (loadError = loadErrorText(e)),
    ).catch((e: unknown) => (loadError = loadErrorText(e)));
  });
  $effect(() => {
    if (picked >= seats.length) picked = 0;
  });

  function pickJudge(): void {
    if (!judgeEl) return;
    const options = agentProviders().filter(isRuntimeProvider);
    ctxMenu.showAt(
      judgeEl,
      options.map((p) => ({
        label: harnessLabel(p),
        checked: p === judgeProvider,
        disabled: !providerReadiness(p).available,
        action: () => (judge = p),
      })),
      { align: 'end' },
    );
  }

  async function synthesize(): Promise<void> {
    if (synthesizing || blocker || !seats[0]) return;
    synthesizing = true;
    synthError = null;
    try {
      await council.synthesize(seats[0].id, { provider: judgeProvider });
      picked = -1;
    } catch (e) {
      synthError = loadErrorText(e);
    } finally {
      synthesizing = false;
    }
  }

  const setStatus = (id: string) => (st: RuntimeEmbedStatus): void => {
    if (statuses[id] !== st) statuses = { ...statuses, [id]: st };
  };
</script>

<div class="cv" data-testid="council-view">
  {#if !members}
    <LoadState what="the council" loading={!loadError} error={loadError} empty onretry={() => (attempt += 1)} />
  {:else}
    <div class="cv-bar">
      <Icon name="people" size={14} />
      <h2 class="cv-title ellipsis" dir="auto" title={title}>{title}</h2>
      <span class="cv-count">Council · {seats.length}</span>
    </div>

    <div class="cv-seg">
      <Tabs label="Council seats" size="s" tabs={seatTabs} value={String(picked)} onchange={(id) => (picked = Number(id))} />
    </div>

    <div class="cv-cols" class:off={picked === -1} style:--cv-n={seats.length}>
      {#each seats as s, i (s.id)}
        <CouncilSeatColumn session={s} label="Seat {i + 1}" {question} active={picked === i} onstatus={setStatus(s.id)} />
      {/each}
    </div>

    {#if synthesis}
      <div class="cv-synth" class:on={picked === -1}>
        <CouncilSeatColumn session={synthesis} label="Synthesis" question="" active={picked === -1} wide onstatus={setStatus(synthesis.id)} />
      </div>
    {/if}

    <div class="cv-foot">
      {#if synthError}
        <p class="cv-error" use:announceShown={{ text: `Couldn’t synthesize: ${synthError}`, assertive: true }}>Couldn’t synthesize: {synthError}</p>
      {:else if blocker}
        <p class="cv-hint" data-testid="council-synth-blocker">{blocker}</p>
      {:else}
        <span class="cv-spacer"></span>
      {/if}
      <button
        bind:this={judgeEl}
        type="button"
        class="btn small ghost"
        aria-haspopup="menu"
        title="Choose the judge"
        data-testid="council-judge"
        onclick={pickJudge}
      >
        {#if hasProviderIcon(judgeProvider)}<ProviderIcon provider={judgeProvider} size={12} />{/if}
        Judge: {harnessLabel(judgeProvider)}
      </button>
      <button
        type="button"
        class="btn small primary"
        disabled={!!blocker || synthesizing}
        title={blocker ?? 'Compare the answers: agreement, disagreement, a pick or a merge, open questions'}
        data-testid="council-synthesize"
        onclick={() => void synthesize()}
      >
        {#if synthesizing}<span class="spinner" aria-hidden="true"></span>{/if}
        {synthesis ? `Synthesize again with ${harnessLabel(judgeProvider)}` : `Synthesize with ${harnessLabel(judgeProvider)}`}
      </button>
    </div>
  {/if}
</div>

<style>
  .cv {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-inline-size: 0;
    min-block-size: 0;
    block-size: 100%;
    background: var(--bg);
  }
  .cv-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-block-end: 1px solid var(--border);
    color: var(--text-dim);
    flex: none;
  }
  .cv-title {
    flex: 1;
    min-inline-size: 0;
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
  }
  .cv-count {
    flex: none;
    font-size: var(--fs-s);
  }
  .cv-seg {
    display: none;
  }
  .cv-cols {
    flex: 1;
    min-block-size: 0;
    display: grid;
    grid-template-columns: repeat(var(--cv-n), minmax(280px, 1fr));
    gap: 8px;
    padding: 8px;
    overflow-x: auto;
  }
  .cv-synth {
    display: flex;
    flex: 1;
    min-block-size: 240px;
    padding: 0 8px 8px;
  }
  .cv-synth > :global(*) {
    flex: 1;
  }
  .cv-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-block-start: 1px solid var(--border);
    flex: none;
  }
  .cv-hint,
  .cv-error {
    flex: 1;
    min-inline-size: 0;
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .cv-error {
    color: var(--danger);
  }
  .cv-spacer {
    flex: 1;
  }
  .cv-foot .spinner {
    --spinner-size: 12px;
  }
  @media (max-width: 640px) {
    .cv-seg {
      display: block;
      padding-inline: 8px;
      flex: none;
    }
    .cv-cols {
      grid-template-columns: minmax(0, 1fr);
      overflow-x: hidden;
    }
    .cv-cols.off,
    .cv-synth:not(.on) {
      display: none;
    }
    .cv-synth {
      padding: 8px;
    }
    .cv-foot {
      flex-wrap: wrap;
    }
    .cv-hint,
    .cv-error {
      flex-basis: 100%;
    }
  }
</style>
