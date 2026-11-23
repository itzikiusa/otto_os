<script lang="ts">
  // One made piece, large — turn it with a finger — and, for a person, what
  // shows it can move: its moves as plain buttons ("Stand", "Walk", "Run",
  // from the clips the model really carries), the first one playing as soon
  // as it opens, a press on the playing one stops it, and "Show skeleton"
  // beside them (the bones drawn over the model, following the move). The
  // first time a person with a skeleton is opened, the skeleton shows for a
  // moment so it is plain that it was added. A person without moves says why.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { RigInfo } from '../../../lib/design/scene3d';
  import CreateModel from './CreateModel.svelte';
  import { moveLabel } from './plan';
  import { orderMoves, rigWords } from './rig';

  interface Props {
    artifactId: string;
    label: string;
    seq?: number | null;
    thumbnail?: boolean;
    /** It is a person (so no moves is worth saying). */
    person?: boolean;
    /** Its skeleton or moves are still being added, or adding them didn't work. */
    rigState?: 'adding' | 'failed' | null;
    onrig?: (info: RigInfo) => void;
  }
  let { artifactId, label, seq = null, thumbnail = false, person = false, rigState = null, onrig }: Props = $props();

  const SEEN_KEY = 'otto.create.skeletonSeen';
  let rig = $state<RigInfo | null>(null);
  /** The clip playing (its index in the file), or -1. */
  let clip = $state(-1);
  let started = false;
  let skeleton = $state(false);
  const moves = $derived(orderMoves(rig?.clips ?? []));
  const words = $derived(rig ? rigWords({ skeleton: rig.hasSkeleton, clips: rig.clips.map((c) => c.name) }) : null);
  const still = $derived.by(() => {
    if (!rig || !(person || rig.hasSkeleton) || moves.length) return null;
    if (rigState === 'adding') return rig.hasSkeleton ? 'Moves are still being added.' : 'Moves are still being added. It stands still until then.';
    if (rigState === 'failed') return 'It stands still: I couldn’t give it moves this time.';
    // (No skeleton and nothing coming: said by nothing here, since the picture may still be loading.)
    return rig.hasSkeleton ? 'It has a skeleton, but no moves yet.' : null;
  });

  function got(info: RigInfo): void {
    rig = info;
    onrig?.(info);
    if (started || !info.clips.length) return;
    started = true;
    clip = orderMoves(info.clips)[0].index;
    // The first look at a new skeleton: show it for a moment.
    if (!info.hasSkeleton) return;
    let seen: string[] = [];
    try {
      seen = JSON.parse(localStorage.getItem(SEEN_KEY) ?? '[]') as string[];
    } catch {
      /* none seen */
    }
    if (seen.includes(artifactId)) return;
    try {
      localStorage.setItem(SEEN_KEY, JSON.stringify([...seen, artifactId].slice(-200)));
    } catch {
      /* private mode */
    }
    skeleton = true;
    setTimeout(() => (skeleton = false), 4000);
  }
</script>

<div class="cr-stage">
  <CreateModel {artifactId} {label} {seq} {thumbnail} clipIndex={Math.max(0, clip)} playing={clip >= 0} {skeleton} onrig={got} />
</div>
{#if words}<p class="lk-words" data-testid="create-rig-words"><Icon name="user" size={16} /> {words}</p>{/if}
{#if still}<p class="cr-note lk-still" data-testid="create-rig-still"><Icon name="info" size={16} /> {still}</p>{/if}
{#if moves.length || rig?.hasSkeleton}
  <div class="lk-moves" role="group" aria-label="Moves">
    {#each moves as m (m.index)}
      <button type="button" class="cr-chip lk-move" aria-pressed={clip === m.index} onclick={() => (clip = clip === m.index ? -1 : m.index)} data-testid="create-move">
        <Icon name={clip === m.index ? 'stop' : 'play'} size={16} /> {moveLabel(m.name)}
      </button>
    {/each}
    {#if rig?.hasSkeleton}
      <button type="button" class="cr-chip lk-move" aria-pressed={skeleton} onclick={() => (skeleton = !skeleton)} data-testid="create-skeleton">
        <Icon name={skeleton ? 'eyeOff' : 'eye'} size={16} /> {skeleton ? 'Hide skeleton' : 'Show skeleton'}
      </button>
    {/if}
  </div>
{/if}

<style>
  .lk-words {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    margin: var(--sp-3) 0 0;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .lk-still {
    margin-block: var(--sp-3) 0;
  }
  .lk-moves {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-2);
    margin-block-start: var(--sp-3);
  }
  .lk-move {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
    min-inline-size: 96px;
    justify-content: center;
  }
</style>
