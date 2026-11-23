<script lang="ts">
  // The pieces of a scene as a short picture list a child can read, grouped
  // People · Things · Voices · Sounds and music: a picture (or a plain icon
  // until there is one) and one sentence. No switches, no pencils — changes
  // are said in words (SteerBox). While making, every row says where it is:
  // its picture as soon as one is drawn, the step in plain words and which
  // step of how many, a clock that ticks and a bar that keeps moving inside a
  // long step; a made piece's whole row opens it ("Ready · Look or change it").
  // On the plan a piece the project already has shows its picture and
  // "From …", with one quiet switch: "Make a new one", or for a new piece
  // "Use one from this project".
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import type { ProductionItem, ProductionItemKind, ProductionPiece } from '../../../lib/api/types';
  import { now } from '../../../lib/stores/now.svelte';
  import AudioLine from './AudioLine.svelte';
  import ItemThumb from './ItemThumb.svelte';
  import RigLine from './RigLine.svelte';
  import { STEP_LABEL, groupItems, itemStep } from './plan';
  import { StepClock, barPct, clockText, type LiveItem } from './progress';

  interface Props {
    items: ProductionItem[];
    mode: 'plan' | 'run';
    /** The production (for the pieces' pictures). */
    productionId?: string;
    onopen?: (item: ProductionItem) => void;
    /** The plan: the project's made pieces an item can be switched to, and the switch. */
    pieces?: ProductionPiece[];
    onswitch?: (item: ProductionItem, to: 'new' | 'pick') => void;
  }
  let { items, mode, productionId = '', onopen, pieces = [], onswitch }: Props = $props();

  const KIND_ICON: Record<ProductionItemKind, IconName> = { character: 'user', object: 'box', voice_line: 'mic', sound: 'volume', music: 'volume' };
  const GROUP_ICON: Record<string, IconName> = { people: 'people', things: 'box', voices: 'mic', sounds: 'volume' };
  const groups = $derived(groupItems(items));
  const clock = new StepClock();
  /** The furthest each bar has been, so it never goes back. */
  const furthest = new Map<string, number>();

  const live = (i: ProductionItem) => i as LiveItem;
  const isModel = (i: ProductionItem) => i.kind === 'character' || i.kind === 'object';
  // A person whose model stands in the scene (`made: "mesh"`) waits for its skeleton, and can be looked at.
  const openable = (i: ProductionItem) => mode === 'run' && isModel(i) && !!i.result?.artifact_id && (i.status === 'done' || i.status === 'working' || i.made === 'mesh');
  const previewKey = (i: ProductionItem) => (live(i).has_preview && productionId ? `${productionId}/${i.id}/${live(i).preview_rev ?? 0}` : null);
  /** Seconds since this piece (or its waiting) started, when the server says. */
  const since = (i: ProductionItem) => {
    const t = live(i).working_since ? Date.parse(live(i).working_since!) : NaN;
    return Number.isFinite(t) ? Math.max(0, (now() - t) / 1000) : null;
  };

  function stepWords(i: ProductionItem): string {
    const s = live(i).step;
    if (s) return s.label;
    const old = itemStep(i);
    return old ? STEP_LABEL[old] : i.kind === 'voice_line' ? 'Recording' : 'Making it';
  }
  function bar(i: ProductionItem): number {
    const t = now();
    const shown = Math.max(furthest.get(i.id) ?? 0, barPct(live(i), (t - clock.start(live(i), t)) / 1000));
    furthest.set(i.id, shown);
    return shown;
  }
  const from = (i: ProductionItem) => i.details?.reuse?.from ?? 'an earlier scene';
  const reusePreview = (i: ProductionItem) => {
    const r = i.details?.reuse;
    return r?.production_id && r.item_id ? `${r.production_id}/${r.item_id}/1` : null;
  };
  const canSwitch = (i: ProductionItem) => !!onswitch && isModel(i) && (!!i.details?.reuse || pieces.some((x) => x.kind === i.kind));
  function stateText(i: ProductionItem): string {
    if (i.status === 'waiting') return i.made === 'mesh' ? 'In the scene · its skeleton comes next' : (i.message ?? 'Waiting its turn');
    if (i.status === 'done' && i.details?.reuse) return `From “${from(i)}” · Look at it`;
    if (i.status === 'done') return isModel(i) ? 'Ready · Look or change it' : 'Ready';
    if (i.status === 'skipped') return 'Not made';
    return 'I couldn’t make this one';
  }
</script>

{#snippet row(i: ProductionItem)}
  <span class="pl-pic">
    {#if i.details?.reuse}
      <!-- The picture it has in the creation it was made in. -->
      <ItemThumb artifactId={i.details.reuse.artifact_id} preview={reusePreview(i)} icon={KIND_ICON[i.kind]} />
    {:else if mode === 'run' && isModel(i) && (previewKey(i) || i.result?.artifact_id)}
      <ItemThumb artifactId={i.result?.artifact_id ?? null} preview={previewKey(i)} icon={KIND_ICON[i.kind]} />
    {:else}
      <span class="cr-tile pl-tile" data-kind={i.kind} aria-hidden="true"><Icon name={KIND_ICON[i.kind]} size={24} /></span>
    {/if}
  </span>
  <span class="pl-body">
    <b>{i.name}</b>
    {#if mode === 'plan'}
      <span class="pl-line">{i.description}</span>
      {#if i.details?.reuse}<span class="pl-from" data-testid="create-item-from">From “{from(i)}”</span>{/if}
    {:else if i.status === 'working' && live(i).waiting === 'engine'}
      <span class="pl-line" data-testid="create-item-state">Waiting for its turn{#if since(i) !== null}<span class="pl-clock" data-testid="create-item-clock">{clockText(since(i)!)}</span>{/if}</span>
    {:else if i.status === 'working'}
      {@const s = live(i).step}
      <span class="pl-line now" data-testid="create-item-state">{stepWords(i)}…{#if since(i) !== null}<span class="pl-clock" data-testid="create-item-clock">{clockText(since(i)!)}</span>{/if}</span>
      {#if s}<span class="pl-of">Step {s.n} of {s.of}</span>{/if}
      <span class="pl-bar" role="progressbar" aria-label={`${i.name}: how far along`} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(bar(i) * 10) / 10} data-testid="create-item-bar">
        <i style:inline-size={`${Math.max(4, bar(i))}%`}></i>
      </span>
    {:else}
      <span class="pl-line" class:bad={i.status === 'failed'} class:ok={i.status === 'done'} data-testid="create-item-state">{#if i.status === 'done'}<Icon name="check" size={14} /> {/if}{stateText(i)}</span>
      {#if i.status === 'done' && i.kind === 'character' && i.result?.artifact_id}<RigLine artifactId={i.result.artifact_id} versionId={i.result.version_id} />{/if}
    {/if}
  </span>
  {#if openable(i)}<span class="pl-go" aria-hidden="true"><Icon name="chevronRight" size={16} /></span>{/if}
{/snippet}

{#each groups as g (g.id)}
  <section class="pl-group" aria-labelledby={`pl-${g.id}`}>
    <h2 class="cr-h2" id={`pl-${g.id}`}><Icon name={GROUP_ICON[g.id]} size={16} /> {g.label}</h2>
    <ul class="pl-list">
      {#each g.items as i (i.id)}
        <li class="pl-item" data-testid="create-plan-item" data-kind={i.kind} data-status={i.status}>
          {#if openable(i)}
            <button type="button" class="pl-row pl-open" class:done={i.status === 'done'} aria-label={`${i.name}: look at it`} onclick={() => onopen?.(i)} data-testid="create-open-piece">{@render row(i)}</button>
          {:else}
            <div class="pl-row" class:done={i.status === 'done'} class:working={i.status === 'working'}>
              {@render row(i)}
              {#if mode === 'plan' && canSwitch(i)}
                <button type="button" class="pl-switch" onclick={() => onswitch?.(i, i.details?.reuse ? 'new' : 'pick')} data-testid="create-item-switch">{i.details?.reuse ? 'Make a new one' : 'Use one from this project'}</button>
              {/if}
            </div>
          {/if}
          {#if mode === 'run' && i.status === 'done' && i.result?.clip_id}
            <AudioLine clipId={i.result.clip_id} title={`Play ${i.name}`} loop={!!i.details?.loop} />
          {/if}
        </li>
      {/each}
    </ul>
  </section>
{/each}

<style>
  .pl-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: var(--sp-2);
  }
  .pl-item {
    display: grid;
    gap: var(--sp-1);
  }
  .pl-row {
    display: flex;
    gap: var(--sp-3);
    align-items: center;
    inline-size: 100%;
    padding: var(--sp-2) var(--sp-3);
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    color: var(--text);
    font: inherit;
    text-align: start;
  }
  .pl-row.done {
    border-color: var(--success);
  }
  .pl-row.working {
    border-color: var(--accent-line-strong);
  }
  .pl-open {
    cursor: pointer;
  }
  .pl-open:hover {
    background: var(--hover);
  }
  .pl-open:focus-visible {
    outline: 3px solid var(--accent-text);
    outline-offset: 2px;
  }
  .pl-pic {
    flex: none;
  }
  /* People, things and voices look different at a glance. */
  .pl-tile[data-kind='character'] {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .pl-tile[data-kind='object'] {
    background: var(--warning-soft);
    color: var(--text);
  }
  .pl-tile[data-kind='voice_line'],
  .pl-tile[data-kind='sound'],
  .pl-tile[data-kind='music'] {
    background: var(--success-soft);
    color: var(--success);
  }
  .pl-body {
    flex: 1;
    min-inline-size: 0;
    display: grid;
    gap: var(--sp-1);
  }
  .pl-body b {
    font-size: var(--fs-m);
  }
  .pl-line {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    color: var(--text-dim);
    font-size: var(--fs-m);
    overflow-wrap: anywhere;
  }
  .pl-line.now {
    color: var(--text);
    font-weight: 500;
  }
  .pl-line.ok {
    color: var(--success);
  }
  .pl-line.bad {
    color: var(--danger);
  }
  /* The clock sits at the end of the line and never jiggles as it ticks. */
  .pl-clock {
    margin-inline-start: auto;
    padding-inline-start: var(--sp-2);
    font-variant-numeric: tabular-nums;
    color: var(--text-dim);
    font-weight: 400;
  }
  .pl-of {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .pl-bar {
    display: block;
    block-size: 8px;
    border-radius: 999px;
    background: var(--surface-3);
    overflow: hidden;
  }
  .pl-bar i {
    display: block;
    block-size: 100%;
    background: var(--accent-solid);
    border-radius: 999px;
  }
  .pl-from {
    color: var(--accent-text);
    font-size: var(--fs-s);
  }
  .pl-switch {
    flex: none;
    min-block-size: var(--cr-hit);
    padding-inline: var(--sp-2);
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .pl-switch:focus-visible {
    outline: 3px solid var(--accent-text);
    outline-offset: 2px;
  }
  .pl-go {
    flex: none;
    color: var(--text-dim);
  }
</style>
