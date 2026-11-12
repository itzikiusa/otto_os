<script lang="ts">
  import { rowMenu } from '../../lib/rowMenu';
  // One design as a card (the lobby's All designs, project / studio / story
  // pages): preview, title + head version, studio + status, when it last
  // changed, and chips — where a mirrored design comes from, Archived, the
  // stories it implements and how often it is referenced. The card is a link
  // to the artifact view; an archived card with `onunarchive` also carries a
  // real Unarchive button (outside the link: a button can't nest in an <a>).
  import Icon from '../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { router } from '../../lib/router.svelte';
  import { canPopout, popoutItems } from '../../lib/popoutMenu';
  import { rel } from '../../lib/stores/now.svelte';
  import type { DesignArtifact } from '../../lib/api/types';
  import ArtifactThumb from './ArtifactThumb.svelte';
  import StudioBadge from './StudioBadge.svelte';
  import StatusPill from './StatusPill.svelte';
  import { mirrorLabel } from './designGrid';

  interface Props {
    artifact: DesignArtifact;
    /** Story keys this artifact implements ("LOY-142"). */
    stories?: string[];
    /** Incoming references (`DesignSearchHit.reference_count`). */
    referenceCount?: number;
    live?: boolean;
    /** Restore an archived design (status → draft). Omitted: no action. */
    onunarchive?: (a: DesignArtifact) => void;
  }
  let { artifact, stories = [], referenceCount = 0, live = false, onunarchive }: Props = $props();
  const by = $derived(artifact.created_by_kind === 'agent' ? ' · drafted by Otto' : '');
  const archived = $derived(artifact.status === 'archived');
  const mirror = $derived(mirrorLabel(artifact.source_kind));
  const href = $derived(`#/design/a/${encodeURIComponent(artifact.id)}`);

  function menu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'Open', icon: 'chevronRight', action: () => router.go(`design/a/${encodeURIComponent(artifact.id)}`) },
    ];
    if (archived && onunarchive) items.push({ label: 'Unarchive', icon: 'undo', action: () => onunarchive?.(artifact) });
    if (canPopout) items.push(...popoutItems(`design/a/${encodeURIComponent(artifact.id)}`, artifact.title));
    ctxMenu.show(e, items);
  }
</script>

{#snippet inner()}
  <div class="pic"><ArtifactThumb {artifact} {live} /></div>
  <div class="body">
    <div class="line1">
      <span class="title" title={artifact.title}>{artifact.title}</span>
      {#if artifact.head_seq != null}<span class="ver">v{artifact.head_seq}</span>{/if}
    </div>
    <div class="line2">
      <StudioBadge studio={artifact.studio} showName />
      {#if !archived}<StatusPill status={artifact.status} />{/if}
    </div>
    <div class="meta" title={new Date(artifact.updated_at).toLocaleString()}>
      edited {rel(artifact.updated_at)}{by}
    </div>
    {#if archived || mirror || stories.length || referenceCount}
      <div class="chips">
        {#if archived}
          <span class="chip tag" data-testid="design-card-archived"><Icon name="archive" size={12} /> Archived</span>
        {/if}
        {#if mirror}
          <span class="chip tag" data-testid="design-card-mirror" title="Kept in sync with where it was made; edit it there">
            <Icon name={mirror.icon} size={12} /> {mirror.label}
          </span>
        {/if}
        {#if referenceCount}
          <span class="chip"><Icon name="link" size={12} /> used in {referenceCount}</span>
        {/if}
        {#each stories.slice(0, 2) as key, i (i)}
          <span class="chip mono-chip"><Icon name="ticket" size={12} /> {key}</span>
        {/each}
      </div>
    {/if}
  </div>
{/snippet}

{#if archived && onunarchive}
  <div class="acard has-foot" class:archived data-testid="design-artifact-card">
    <a use:rowMenu class="link" {href} oncontextmenu={menu}>{@render inner()}</a>
    <div class="foot">
      <button class="btn small" onclick={() => onunarchive?.(artifact)} data-testid="design-card-unarchive"
        title="Move this design back to Draft">
        <Icon name="undo" size={12} /> Unarchive
      </button>
    </div>
  </div>
{:else}
  <a use:rowMenu class="acard link" class:archived {href} data-testid="design-artifact-card"
    oncontextmenu={canPopout || (archived && onunarchive) ? menu : undefined}>
    {@render inner()}
  </a>
{/if}

<style>
  .acard {
    /* A 500-artifact library: off-screen cards skip layout/paint (SD-16). */
    content-visibility: auto;
    contain-intrinsic-size: auto 260px;
    display: flex;
    flex-direction: column;
    min-width: 0;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    overflow: hidden;
    color: var(--text);
    text-decoration: none;
    transition: border-color var(--dur-fast) ease-out;
  }
  .acard:hover {
    border-color: var(--border-strong);
  }
  .link {
    display: flex;
    flex-direction: column;
    min-width: 0;
    color: var(--text);
    text-decoration: none;
  }
  .has-foot .link:focus-visible,
  .acard:focus-visible {
    outline: 2px solid color-mix(in srgb, var(--accent-text) 70%, transparent);
    outline-offset: 1px;
  }
  .pic {
    height: 148px;
    border-block-end: 1px solid var(--border);
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px 12px;
    min-width: 0;
  }
  .line1 {
    display: flex;
    align-items: baseline;
    gap: 8px;
    min-width: 0;
  }
  .title {
    flex: 1;
    min-width: 0;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ver {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .line2 {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-width: 0;
  }
  .meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .mono-chip {
    font-family: var(--font-mono);
  }
  /* Archived reads from the chip text + a dimmed preview, never colour alone. */
  .archived .pic {
    opacity: 0.6;
  }
  .tag {
    font-weight: 500;
  }
  .foot {
    display: flex;
    justify-content: flex-end;
    padding: 0 12px 12px;
  }
</style>
