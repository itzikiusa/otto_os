<!--
  A council in Threads (docs/features/threads-structured-runtime.md
  "Council"): the same question answered by 2–4 agents in columns (one at a
  time on a phone, picked with a segmented control). Then the person keeps
  talking to it in rounds: a follow-up to every seat or one ("Ask the
  others" quotes one seat's answer to the rest), and Synthesize with a judge
  on the CURRENT round, whose answer is the last, full-width column. Every
  round starts with a click; nothing re-asks by itself.
-->
<script lang="ts">
  import { onMount } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Tabs, { type TabItem } from '../../../lib/components/Tabs.svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import type { CouncilRound, RuntimeProvider } from '../../../lib/api/types';
  import { loadErrorText } from '../../../lib/loadError';
  import { agentProviders, harnessLabel, providerReadiness } from '../../../lib/providers';
  import type { RuntimeEmbedStatus } from '../../../lib/runtimeThreadPort';
  import { now } from '../../../lib/stores/now.svelte';
  import { toastError } from '../../../lib/toastError';
  import { councilMembers, councilOf, councilSyntheses } from './council';
  import { askOthersText, missingSeats, roundSynthBlocker, seatRounds, synthesisRound, type SeatRound, type SeatRoundState } from './councilRounds';
  import { council } from './councilStore.svelte';
  import CouncilComposer from './CouncilComposer.svelte';
  import CouncilRounds from './CouncilRounds.svelte';
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
  const seatNo = (i: number): number => councilOf(seats[i])?.seat ?? i + 1;
  const seatNumbers = $derived(seats.map((_, i) => seatNo(i)));
  const allSyntheses = $derived(councilSyntheses(Object.values(runtimeThreads.byId), sessionId));
  const title = $derived((seats[0]?.title ?? 'Council').replace(/ · [^·]*$/, ''));
  /** Each column's state, from its embed. */
  let statuses = $state<Record<string, RuntimeEmbedStatus>>({});
  const working = (id: string): boolean => statuses[id] === 'working' || runtimeThreads.get(id)?.status === 'working';
  /** The seat a phone shows (index into `seats`, or -1 = the synthesis). */
  let picked = $state(0);
  let judge = $state<RuntimeProvider | null>(null);
  let judgeEl = $state<HTMLButtonElement | null>(null);
  let synthesizing = $state(false);
  let synthError = $state<string | null>(null);
  let loadError = $state<string | null>(null);
  let roundsError = $state<string | null>(null);
  let attempt = $state(0);
  /** The synthesis shown (null = the newest). */
  let shownSynth = $state<string | null>(null);
  /** The follow-up composer: its text and who gets it (empty = every seat). */
  let followText = $state('');
  let followTargets = $state<number[]>([]);
  let sending = $state(false);
  let followError = $state<string | null>(null);
  let stopping = $state(false);

  const question = $derived.by(() => {
    const items = seats[0] ? (runtimeViews.projections[seats[0].id]?.items ?? []) : [];
    const first = items.find((i) => i.type === 'user_message' && !i.parent_item_id);
    return first && first.type === 'user_message' ? first.text : '';
  });
  /** The stored rounds; until they load (or a council from before rounds),
   *  one round: the first question to every seat. */
  const stored = $derived(members ? council.rounds[members.meta.id] : undefined);
  const rounds = $derived.by<CouncilRound[]>(() =>
    stored?.rounds.length
      ? stored.rounds
      : [{ n: 1, text: question, seats: seatNumbers, created_at: seats[0]?.created_at ?? '' }],
  );
  const maxRounds = $derived(stored?.max_rounds ?? 10);
  const current = $derived(rounds[rounds.length - 1]);
  /** Per seat number: its place in every round. */
  const states = $derived.by<Record<number, SeatRound[]>>(() => {
    const t = now();
    const out: Record<number, SeatRound[]> = {};
    const last = rounds.length - 1;
    seats.forEach((s, i) => {
      const p = runtimeViews.projections[s.id];
      const list = seatRounds(rounds, seatNo(i), p?.items ?? [], p?.turns ?? [], p?.queued ?? [], t);
      // A seat whose thread works is working on the current round (a re-ask,
      // or a turn its timeline has not caught up with yet).
      const cur = list[last];
      if (cur && working(s.id) && !['not-asked', 'refused', 'queued'].includes(cur.state)) list[last] = { ...cur, state: 'working' };
      out[seatNo(i)] = list;
    });
    return out;
  });
  const roundOf = (n: number): SeatRound | null => states[n]?.[rounds.length - 1] ?? null;
  const currentSeats = $derived(
    seatNumbers.map((n): { seat: number; state: SeatRoundState; answer: string } => {
      const r = roundOf(n);
      return { seat: n, state: r?.state ?? 'not-asked', answer: r?.answer ?? '' };
    }),
  );
  const synthesis = $derived(allSyntheses.find((x) => x.id === shownSynth) ?? allSyntheses[allSyntheses.length - 1] ?? null);
  const synthTimeline = $derived(allSyntheses.map((x) => ({ id: x.id, round: synthesisRound(rounds, x.id) })));
  const targets = $derived(followTargets.length ? followTargets : seatNumbers);
  const workingSeats = $derived(seats.filter((_, i) => roundOf(seatNo(i))?.state === 'working'));
  const missing = $derived(missingSeats(currentSeats));

  /** A phone shows one seat at a time. */
  const seatTabs = $derived<TabItem[]>([
    ...seats.map((_, i) => ({ id: String(i), label: `Seat ${seatNo(i)}`, testid: `council-tab-${seatNo(i)}` })),
    ...(synthesis ? [{ id: '-1', label: 'Synthesis', testid: 'council-tab-synthesis' }] : []),
  ]);
  const judgeProvider = $derived<RuntimeProvider>(judge ?? seats[0]?.provider ?? 'claude');
  const blocker = $derived(
    seats.length
      ? roundSynthBlocker(current.n, currentSeats, synthesis ? working(synthesis.id) : false, seats[0]?.status === 'closed')
      : 'The council has no seats left',
  );

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
  const councilId = $derived(members?.meta.id ?? null);
  $effect(() => {
    const cid = councilId;
    const id = sessionId;
    void attempt;
    if (cid) void refreshRounds(cid, id);
  });
  $effect(() => {
    if (picked >= seats.length) picked = 0;
  });

  async function refreshRounds(cid: string, id: string): Promise<void> {
    roundsError = null;
    try {
      await council.loadRounds(cid, id);
    } catch (e) {
      roundsError = loadErrorText(e);
    }
  }

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
      const j = await council.synthesize(seats[0].id, { provider: judgeProvider });
      shownSynth = j.id;
      picked = -1;
      if (councilId) void refreshRounds(councilId, seats[0].id);
    } catch (e) {
      synthError = loadErrorText(e);
    } finally {
      synthesizing = false;
    }
  }

  async function sendFollowUp(): Promise<void> {
    const text = followText.trim();
    if (sending || !text || !councilId || !seats[0]) return;
    sending = true;
    followError = null;
    try {
      const to = [...targets];
      const all = to.length === seatNumbers.length;
      await council.followUp(councilId, seats[0].id, { text, ...(all ? {} : { seats: to }) });
      followText = '';
      followTargets = [];
      // On a phone, the seat asked (or the first one) is in view.
      picked = Math.max(0, seatNumbers.indexOf(to[0]));
    } catch (e) {
      followError = loadErrorText(e);
    } finally {
      sending = false;
    }
  }

  async function stopAll(): Promise<void> {
    if (stopping) return;
    stopping = true;
    try {
      const r = await Promise.allSettled(workingSeats.map((s) => runtimeThreads.interrupt(s.id)));
      const bad = r.find((x): x is PromiseRejectedResult => x.status === 'rejected');
      if (bad) toastError('Couldn’t stop every seat', bad.reason);
    } finally {
      stopping = false;
    }
  }

  function askOthers(n: number, answer: string): void {
    followText = askOthersText(n, answer);
    followTargets = seatNumbers.filter((m) => m !== n);
    followError = null;
  }

  /** "Synthesize with Claude" / "Synthesize round 2 with Claude" / "… again". */
  function synthLabel(n: number, done: boolean, many: boolean, judgeName: string): string {
    const what = many ? `round ${n} ` : '';
    return done ? `Synthesize ${what}again with ${judgeName}` : `Synthesize ${what}with ${judgeName}`;
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

    {#if rounds.length > 1 || allSyntheses.length > 1 || roundsError}
      <CouncilRounds
        {rounds}
        {seatNumbers}
        {states}
        syntheses={synthTimeline}
        shownSynthesis={synthesis?.id ?? null}
        onsynthesis={(id) => {
          shownSynth = id;
          picked = -1;
        }}
        error={roundsError}
        onretry={() => {
          if (councilId) void refreshRounds(councilId, sessionId);
        }}
      />
    {/if}

    <div class="cv-seg">
      <Tabs label="Council seats" size="s" tabs={seatTabs} value={String(picked)} onchange={(id) => (picked = Number(id))} />
    </div>

    <div class="cv-cols" class:off={picked === -1} style:--cv-n={seats.length}>
      {#each seats as s, i (s.id)}
        {@const n = seatNo(i)}
        <CouncilSeatColumn
          session={s}
          label="Seat {n}"
          question={current.seats.includes(n) ? current.text : question}
          active={picked === i}
          onstatus={setStatus(s.id)}
          roundN={current.n}
          round={roundOf(n)}
          targeted={followTargets.length === 1 && followTargets[0] === n}
          ontarget={() => (followTargets = followTargets.length === 1 && followTargets[0] === n ? [] : [n])}
          onaskothers={seats.length > 1 ? (a) => askOthers(n, a) : undefined}
        />
      {/each}
    </div>

    {#if synthesis}
      {@const sr = synthesisRound(rounds, synthesis.id)}
      <div class="cv-synth" class:on={picked === -1}>
        <CouncilSeatColumn session={synthesis} label={rounds.length > 1 ? `Synthesis · round ${sr}` : 'Synthesis'} question="" active={picked === -1} wide onstatus={setStatus(synthesis.id)} />
      </div>
    {/if}

    <div class="cv-foot">
      {#if synthError}
        <p class="cv-error" use:announceShown={{ text: `Couldn’t synthesize: ${synthError}`, assertive: true }}>Couldn’t synthesize: {synthError}</p>
      {:else if blocker}
        <p class="cv-hint" data-testid="council-synth-blocker">{blocker}</p>
      {:else if missing.length}
        <p class="cv-hint" data-testid="council-synth-missing">The judge will be told: no answer from {missing.join(', ')}</p>
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
        title={blocker ?? `Compare the answers of round ${current.n}: agreement, disagreement, a pick or a merge, open questions`}
        data-testid="council-synthesize"
        onclick={() => void synthesize()}
      >
        {#if synthesizing}<span class="spinner" aria-hidden="true"></span>{/if}
        {synthLabel(current.n, synthTimeline.some((x) => x.round === current.n), rounds.length > 1, harnessLabel(judgeProvider))}
      </button>
    </div>

    <CouncilComposer
      {seatNumbers}
      {targets}
      text={followText}
      rounds={rounds.length}
      {maxRounds}
      {sending}
      error={followError}
      working={workingSeats.length}
      {stopping}
      ontargets={(t) => (followTargets = t.length === seatNumbers.length ? [] : t)}
      ontext={(t) => (followText = t)}
      onsend={() => void sendFollowUp()}
      onstop={() => void stopAll()}
    />
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
