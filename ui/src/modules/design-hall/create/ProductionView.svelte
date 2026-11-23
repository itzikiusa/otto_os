<script lang="ts">
  // One Smart production, whatever state it is in: thinking about the idea →
  // the plan (a short picture list; changed by saying so) → making, piece by
  // piece → the finished scene, changed the same way. A change comes back as
  // one sentence (with the pinned notes), then "Make the changes", then the
  // changed pieces before and after — only when they really differ.
  import { untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import * as papi from '../../../lib/api/productions';
  import type { Production, ProductionItem, ProductionReuse } from '../../../lib/api/types';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { router } from '../../../lib/router.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { createStore } from './createStore.svelte';
  import PieceList from './PieceList.svelte';
  import SceneView from './SceneView.svelte';
  import CompareItems from './CompareItems.svelte';
  import SteerBox, { type SteerPin } from './SteerBox.svelte';
  import PieceLook from './PieceLook.svelte';
  import { createHref, itemStep, readPlan, remainingSecs, reuseLine, steerExample, theName, timeText } from './plan';
  import { clockText, etaLeft, sceneSoFar, type LiveItem, type LiveProduction } from './progress';
  import { now } from '../../../lib/stores/now.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';

  interface Props {
    id: string;
    /** The scene so far, while the rest is still being made (its own step, so Back returns to the list). */
    peek?: boolean;
  }
  let { id, peek = false }: Props = $props();

  let error = $state<string | null>(null);
  let busy = $state(false);
  let look = $state<ProductionItem | null>(null);
  const p = $derived<LiveProduction | undefined>(createStore.productions[id]);
  const items = $derived(p?.plan?.items ?? []);
  const sound = $derived(createStore.soundReady === true);
  const plan = $derived(readPlan(items, p?.plan?.notes ?? [], sound));
  const revision = $derived(p?.revisions?.at(-1) ?? null);
  const revItems = $derived(revision ? new Set([...revision.changed, ...revision.added]) : new Set<string>());
  const anyDone = $derived(items.some((i) => i.status === 'done'));
  /** A change asked for after things were made, waiting for "Make the changes". */
  const revisionPending = $derived(p?.status === 'ready' && !!revision && anyDone && items.some((i) => revItems.has(i.id) && i.status === 'waiting'));
  const seenKey = $derived(`otto.create.seen.${id}`);
  let seen = $state<string | null>(null);
  /** Only the pieces whose new result really is different from the old one. */
  const changedPieces = $derived(
    items.filter((i) => revItems.has(i.id) && i.previous_result && (i.previous_result.artifact_id !== i.result?.artifact_id || i.previous_result.version_id !== i.result?.version_id || i.previous_result.clip_id !== i.result?.clip_id)),
  );
  const showCompare = $derived(p?.status === 'done' && !!revision && seen !== revision.id && changedPieces.length > 0);
  /** Pieces made again that came back exactly as they were (the same version). */
  const samePieces = $derived(
    items.filter((i) => revItems.has(i.id) && i.status === 'done' && i.previous_result && i.previous_result.artifact_id === i.result?.artifact_id && i.previous_result.version_id === i.result?.version_id && i.previous_result.clip_id === i.result?.clip_id),
  );
  /** What the last change did, said once on the scene, part by part: what already shows (a size or
   *  place change), and a piece made again that came out the same. */
  const said = $derived.by(() => {
    if (!revision || seen === revision.id || !anyDone || p?.status === 'ready') return null;
    const done = revision.done?.length ? `Done: ${revision.done.join(' ')}` : null;
    const looks = samePieces.map((i) => `${i.name}${i.name.endsWith('s') ? '’' : '’s'} new look`).join(' and ');
    if (samePieces.length) return { same: true, done, text: `${looks} came out the same. Try saying it differently.` };
    return { same: false, text: done ?? `Done: “${revision.comment}”` };
  });
  // One estimate, the same everywhere: the server's, counting down between its updates
  // (without one, the pieces still to make).
  const eta = $derived(createStore.eta[id]);
  const leftSecs = $derived(eta ? (p?.status === 'running' ? etaLeft(eta, now()) : eta.secs) : remainingSecs(plan.made));
  const doneCount = $derived(plan.made.filter((i) => i.status === 'done').length);
  /** "9 of 12 pieces reused · about 12 minutes instead of about an hour". */
  const reused = $derived(reuseLine(items, leftSecs, p?.eta_unreused_secs));
  const queued = $derived(p?.status === 'running' ? (p.queue_position ?? 0) : 0);
  /** What is being made right now, in plain words. */
  const nowText = $derived.by(() => {
    const busyNow = plan.made.filter((i) => i.status === 'working' && (i as LiveItem).waiting !== 'engine');
    if (busyNow.length) return `Now making: ${busyNow.map((i) => i.name).join(', ')}`;
    if (plan.made.some((i) => (i as LiveItem).waiting === 'engine')) return 'Waiting for its turn';
    return 'Getting ready…';
  });
  /** Other creations being made now: this one takes its turn after them. */
  const others = $derived(createStore.inProgress().filter((x) => x.id !== id && x.status === 'running').length);
  const soFar = $derived(!!p && sceneSoFar(p, sound));
  // Thinking: a clock from when it started (the first plan: when it was asked for; a change: when this screen saw it start).
  let planSeen = $state<number | null>(null);
  $effect(() => {
    if (p && p.status !== 'planning') planSeen = null;
    else if (planSeen === null) planSeen = Date.now();
  });
  const planFrom = $derived(p && !p.revisions?.length ? Math.min(Date.parse(p.created_at), planSeen ?? Infinity) : planSeen);
  const planSecs = $derived(planFrom ? Math.max(0, (now() - planFrom) / 1000) : 0);
  /** The open piece as it is now (its look changes when its skeleton and moves arrive). */
  const lookItem = $derived(look ? (items.find((i) => i.id === look!.id) ?? look) : null);
  /** What the open piece is waiting for: its skeleton or moves still being added, or that didn't work. */
  const lookRig = $derived.by((): 'adding' | 'failed' | null => {
    const it = lookItem;
    if (!it || it.kind !== 'character') return null;
    if (it.status === 'failed') return 'failed';
    const step = (it as LiveItem).step?.label ?? '';
    return it.status === 'working' || it.status === 'waiting' || /skeleton|move/i.test(step) || ['skeleton', 'moves'].includes(itemStep(it) ?? '') ? 'adding' : null;
  });
  let laterNote = $state('');
  function keepNote(): void {
    const text = laterNote.trim();
    if (!text || !look) return;
    createStore.laterNotes[id] = [...(createStore.laterNotes[id] ?? []), `${look.name}: ${text}`];
    laterNote = '';
    toasts.success('Note kept', 'When everything is made, it is waiting for you in the change box.');
  }
  $effect(() => {
    if (p?.status === 'done') createStore.markLooked(id);
  });
  const changeSecs = $derived(remainingSecs(items.filter((i) => revItems.has(i.id))));
  let steerTyped = $state(false);
  const notes = $derived(createStore.lastNotes[id] ?? []);

  $effect(() => {
    const pid = id;
    untrack(() => {
      try {
        seen = localStorage.getItem(seenKey);
      } catch {
        seen = null;
      }
      if (createStore.soundReady === null) void createStore.loadSoundEngine();
      void load(pid);
    });
  });

  async function load(pid: string): Promise<void> {
    error = null;
    try {
      await createStore.refreshProduction(pid);
    } catch (e) {
      error = loadErrorText(e);
    }
  }

  async function act(run: () => Promise<Production>, failTitle: string): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      createStore.watchProduction(await run());
      await createStore.refreshProduction(id).catch(() => undefined);
    } catch {
      toasts.warn(failTitle, 'Please try again in a moment.');
    } finally {
      busy = false;
    }
  }

  function start(): Promise<void> {
    // A change to the plan before anything was made needs no word on the scene later.
    if (!anyDone) keepNew();
    return act(() => papi.startProduction(id), 'I couldn’t start making');
  }
  const stop = () => act(() => papi.cancelProduction(id), 'I couldn’t stop it');

  /** A piece switched to one the project has (picked from them), or to a new one. */
  async function switchPiece(i: ProductionItem, to: 'new' | 'pick'): Promise<void> {
    let reuse: ProductionReuse | null = null;
    if (to === 'pick') {
      const offered = (p?.project_pieces ?? []).filter((x) => x.kind === i.kind && x.details.reuse);
      const options = offered.map((x) => ({ label: `${x.name} (from “${x.details.reuse?.from ?? 'an earlier scene'}”)`, value: x.details.reuse!.artifact_id }));
      const got = await confirmer.choose(`Which one is ${theName(i.name)}?`, { title: 'Use one from this project', options });
      const picked = offered.find((x) => x.details.reuse!.artifact_id === got.value)?.details.reuse;
      if (!picked) return;
      reuse = { artifact_id: picked.artifact_id, version_id: picked.version_id };
    }
    const details = { ...i.details };
    delete details.reuse;
    await act(() => papi.updateProduction(id, { items: [{ id: i.id, details: reuse ? { ...details, reuse } : details }] }), 'I couldn’t change that');
  }

  /** The one way to change things: words, plus the notes pinned on the scene. */
  function steer(comment: string, pins: (SteerPin & { item_id?: string; artifact_id?: string; point?: [number, number, number] })[]): void {
    createStore.lastNotes[id] = pins.map((x) => (x.on ? `${x.note} (on ${x.on})` : x.note));
    delete createStore.laterNotes[id];
    const words = comment || pins.map((x) => x.note).join('. ');
    void act(() => papi.steerProduction(id, { comment: words, pins: pins.map((x) => ({ item_id: x.item_id, artifact_id: x.artifact_id, point: x.point, note: x.note })) }), 'I couldn’t work out the change');
  }

  function keepNew(): void {
    if (!revision) return;
    seen = revision.id;
    try {
      localStorage.setItem(seenKey, revision.id);
    } catch {
      /* private mode */
    }
  }
  async function keepOld(): Promise<void> {
    if (!revision) return;
    const rid = revision.id;
    await act(() => papi.revertProduction(id, rid), 'I couldn’t go back');
    keepNew();
  }
</script>

{#if error && !p}
  <EmptyState variant="page" tone="error" icon="warning" title="I couldn’t open this" body={error} actionLabel="Try again" actionIcon="refresh" onaction={() => void load(id)} />
{:else if !p || p.status === 'planning'}
  <div class="cr-card pv-calm" data-testid="create-planning">
    <span class="spinner pv-spin" role="img" aria-label="Thinking"></span>
    <h1 class="cr-h1" use:announceShown={'Thinking about your idea'}>{p?.revisions?.length ? 'Thinking about your changes…' : 'Thinking about your idea…'}</h1>
    <p class="cr-sub">I’m working out who and what is in it. This takes about a minute.</p>
    <p class="pv-clock" data-testid="create-planning-clock">{clockText(planSecs)}</p>
    {#if planSecs > 120}
      <p class="cr-note pv-slow" data-testid="create-planning-slow">
        <Icon name="info" size={16} />
        <span>This is taking longer than usual. It keeps going if you go Back: you will find it on the first Create page under “In progress”, and in My creations.</span>
      </p>
    {/if}
  </div>
{:else if p.status === 'failed' && !anyDone}
  <EmptyState variant="page" tone="error" icon="warning" title="That didn’t work" body={p.message || 'Something went wrong while I was planning.'} actionLabel="Try different words" onaction={() => router.go(createHref())} />
{:else if revisionPending && revision}
  <h1 class="cr-h1">Here’s what I’ll change</h1>
  <div class="pv-say" data-testid="create-revision">
    <p>{revision.summary}</p>
    {#if notes.length}
      <p class="pv-notes-h">Your notes:</p>
      <ol>{#each notes as n (n)}<li>{n}</li>{/each}</ol>
    {/if}
  </div>
  <p class="cr-sub">{timeText(changeSecs)}. Everything else stays as it is.</p>
  <div class="cr-sticky">
    <button type="button" class="cr-primary" disabled={busy} onclick={() => void start()} data-testid="create-make-changes">Make the changes</button>
  </div>
  <button type="button" class="cr-quiet" disabled={busy} onclick={() => void keepOld()}>Never mind</button>
{:else if p.status === 'ready'}
  <h1 class="cr-h1">Here’s my plan</h1>
  <p class="cr-sub">{p.plan?.title}</p>
  {#if reused}<p class="cr-note pv-top" data-testid="create-reused"><Icon name="layers" size={16} /> {reused}</p>{/if}
  {#if revision && !anyDone}
    <p class="cr-note pv-top"><Icon name="check" size={16} /> {revision.summary}</p>
  {/if}
  {#if plan.notYet.length}
    <div class="cr-note pv-notyet" data-testid="create-not-yet">
      <Icon name="info" size={16} />
      <div><b>Not yet</b><ul>{#each plan.notYet as n (n)}<li>{n}</li>{/each}</ul></div>
    </div>
  {/if}
  <!-- The time and the button come first, so both show without scrolling. -->
  <div class="pv-go">
    <p class="pv-total" data-testid="create-total"><Icon name="clock" size={16} /> This will take {timeText(leftSecs).toLowerCase()}. You can leave while I work.</p>
    {#if others}<p class="cr-sub pv-turn" data-testid="create-takes-turn">Something else is being made now. This one will take its turn after it.</p>{/if}
    <button type="button" class={steerTyped ? 'cr-secondary' : 'cr-primary'} disabled={busy || !plan.made.length} onclick={() => void start()} data-testid="create-make">Make it</button>
  </div>
  <PieceList items={plan.made} mode="plan" pieces={p.project_pieces ?? []} onswitch={(i, to) => void switchPiece(i, to)} />
  <SteerBox {busy} collapsed bind:hasText={steerTyped} placeholder={steerExample(items)} onsend={(c) => steer(c, [])} />
{:else if p.status === 'running' && peek && soFar}
  <SceneView production={p} busy soundReady={sound} making onsteer={() => {}} onretry={() => {}} onlook={(i) => (look = i)} />
{:else if p.status === 'running'}
  <h1 class="cr-h1" use:announceShown={'Making your scene'}>Making your scene…</h1>
  {#if queued}
    <p class="pv-now" data-testid="create-now">Waiting for its turn</p>
    <p class="cr-sub" data-testid="create-left">{queued === 1 ? 'One other creation is' : `${queued} other creations are`} being made first. This one starts by itself. You can leave.</p>
  {:else}
    <p class="pv-now" data-testid="create-now">{nowText}</p>
    <p class="cr-sub" data-testid="create-left">{doneCount} of {plan.made.length} ready · {timeText(leftSecs).toLowerCase()} left. You can leave. It keeps going, and it will be here when you come back.</p>
  {/if}
  {#if soFar}
    <button type="button" class="cr-primary" onclick={() => router.go(createHref('p', id, 'scene'))} data-testid="create-so-far"><Icon name="layers" size={16} /> See the scene so far</button>
    <p class="pv-coming" data-testid="create-still-coming">Still coming: {plan.made.filter((i) => (i.status === 'waiting' && i.made !== 'mesh') || i.status === 'working').map((i) => i.name).join(', ')}</p>
  {/if}
  <PieceList items={plan.made} mode="run" productionId={id} onopen={(i) => (look = i)} />
  <div class="cr-sticky">
    <button type="button" class="cr-secondary pv-stop" disabled={busy} onclick={() => void stop()} data-testid="create-stop"><Icon name="stop" size={16} /> Stop</button>
  </div>
{:else if showCompare && revision}
  <CompareItems items={changedPieces} {busy} onkeep={keepNew} onback={() => void keepOld()} />
{:else}
  {#if p.status === 'cancelled'}
    <p class="cr-note pv-top"><Icon name="info" size={16} /> You stopped it. What was finished is kept.</p>
  {/if}
  <SceneView production={p} {busy} soundReady={sound} {said} onsteer={steer} onretry={() => void start()} onlook={(i) => (look = i)} initialSteer={(createStore.laterNotes[id] ?? []).join('\n')} />
{/if}

{#if lookItem?.result?.artifact_id}
  <Modal title={lookItem.name} width={720} onclose={() => (look = null)}>
    <div class="pv-look" data-testid="create-look">
      {#if p?.status === 'running'}<p class="pv-keeps"><Icon name="info" size={16} /> The rest keeps going while you look.</p>{/if}
      {#key `${lookItem.result.artifact_id}@${lookItem.result.version_id ?? ''}`}
        <PieceLook artifactId={lookItem.result.artifact_id} label={lookItem.name} thumbnail person={lookItem.kind === 'character'} rigState={lookRig} />
      {/key}
      {#if p?.status === 'running'}
        <label class="pv-later-h" for="create-later">Want it different? Write it here, and you can change it when everything is made.</label>
        <textarea id="create-later" class="cr-field pv-later" dir="auto" maxlength="600" placeholder="Make him older…" bind:value={laterNote} data-testid="create-later-note"></textarea>
        <button type="button" class="cr-secondary pv-keep" disabled={!laterNote.trim()} onclick={keepNote} data-testid="create-keep-note">Keep this note</button>
      {/if}
    </div>
  </Modal>
{/if}

<style>
  .pv-calm {
    text-align: center;
    display: grid;
    justify-items: center;
    gap: var(--sp-2);
    padding-block: var(--sp-7);
    margin-block-start: var(--sp-5);
  }
  .pv-spin {
    inline-size: 36px;
    block-size: 36px;
  }
  .pv-say {
    font-size: var(--fs-l);
    line-height: 1.45;
    background: var(--accent-soft);
    border-radius: var(--radius-l);
    padding: var(--sp-4);
    margin: var(--sp-3) 0;
  }
  .pv-say p {
    margin: 0;
  }
  .pv-say .pv-notes-h {
    margin-block-start: var(--sp-3);
    font-weight: 600;
  }
  .pv-say ol {
    margin: var(--sp-1) 0 0;
    padding-inline-start: var(--sp-6);
  }
  .pv-notyet {
    margin-block-start: var(--sp-4);
  }
  .pv-notyet ul {
    margin: var(--sp-1) 0 0;
    padding-inline-start: var(--sp-5);
  }
  .pv-go {
    display: grid;
    gap: var(--sp-2);
    margin-block: var(--sp-3) var(--sp-2);
  }
  .pv-total {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    font-size: var(--fs-l);
    margin: 0;
  }
  .pv-now {
    font-size: var(--fs-l);
    font-weight: 600;
    margin: 0 0 var(--sp-1);
  }
  .pv-stop {
    inline-size: 100%;
  }
  .pv-top {
    margin-block: var(--sp-3);
  }
  .pv-look :global(.cr-stage) {
    block-size: min(55vh, 460px);
    aspect-ratio: auto;
  }
  .pv-keeps {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin: 0 0 var(--sp-3);
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .pv-later-h {
    display: block;
    margin-block: var(--sp-4) var(--sp-2);
    font-size: var(--fs-m);
  }
  .pv-later {
    min-block-size: 64px;
  }
  .pv-keep {
    inline-size: 100%;
    margin-block-start: var(--sp-2);
  }
  .pv-clock {
    margin: 0;
    font-size: var(--fs-2xl);
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
  }
  .pv-slow {
    max-inline-size: 520px;
    text-align: start;
  }
  .pv-turn {
    margin: 0;
  }
  .pv-coming {
    margin: var(--sp-2) 0 0;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
</style>
