<script lang="ts">
  // Overview → one line: "N designs → Design". A story's designs live in
  // Design Hall and are listed on its Design tab (StoryDesigns); this line
  // only counts them (an epic: its children's too) and takes you there. It
  // replaced the "Linked canvases" panel: Canvas boards linked to a story are
  // mirrored into the Hall and counted here like any other design.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { auth } from '../../lib/stores/auth.svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { designBus } from '../../lib/events.svelte';
  import { listArtifacts } from '../../lib/api/design';
  import { designCountLabel } from './storyDesigns';

  interface Props {
    storyId: string;
    isEpic: boolean;
  }
  let { storyId, isEpic }: Props = $props();

  /** One page is enough for a count; a full page reads "100+". */
  const CAP = 100;
  const canView = $derived(auth.can('design', 'view'));
  let n = $state<number | null>(null);
  let more = $state(false);
  let error = $state<string | null>(null);
  let seq = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  async function load(): Promise<void> {
    const my = ++seq;
    try {
      const rows = await listArtifacts({ story_id: storyId, include_children: isEpic, limit: CAP });
      if (my !== seq) return;
      n = rows.length;
      more = rows.length >= CAP;
      error = null;
    } catch (e) {
      if (my === seq) error = loadErrorText(e);
    }
  }

  $effect(() => {
    void storyId;
    void isEpic;
    void designBus.resyncTick;
    untrack(() => {
      if (canView) void load();
    });
  });
  // A design event can add or archive one of this story's designs: recount,
  // coalesced (the live sync and an agent turn send bursts).
  $effect(() => {
    void designBus.seq;
    untrack(() => {
      if (!canView || n === null) return;
      clearTimeout(timer);
      timer = setTimeout(() => void load(), 800);
    });
    return () => clearTimeout(timer);
  });

  const label = $derived(!canView || n === null ? 'Designs' : designCountLabel(n, more));
</script>

<div class="sd-line" data-testid="story-designs-line">
  <Icon name="palette" size={14} />
  {#if error && n === null}
    <LoadState variant="compact" what="this story’s designs" {error} empty onretry={() => void load()} />
  {:else}
    <button class="sd-link" onclick={() => void product.changeTab('mockups')} title="Open this story’s Design tab">
      {label} <span aria-hidden="true">→</span> Design
    </button>
  {/if}
</div>

<style>
  .sd-line {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-block: 12px;
    color: var(--text-dim);
    font-size: var(--fs-s);
    min-width: 0;
  }
  .sd-link {
    background: none;
    border: 0;
    padding: 4px 0;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
    text-align: start;
  }
  .sd-link:hover {
    text-decoration: underline;
  }
  .sd-link:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
    border-radius: var(--radius-s);
  }
</style>
