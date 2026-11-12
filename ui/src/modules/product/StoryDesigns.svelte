<script lang="ts">
  // Product → story → Design tab: this story's designs, from Design Hall — the
  // one home for designs. No second editor lives here: the cards are the
  // Hall's paged DesignGrid scoped to the story (an epic: its children's too),
  // "New design" is the Hall's sheet with the story preset, and "Ask Otto"
  // creates a draft implementing the story and starts one generate turn (the
  // lobby's Generate, with the story set). Every card opens the Hall's artifact
  // view with `?from=story/<id>`, which shows a breadcrumb back here.
  //
  // Mirrored designs (a story attachment — an agent's `otto-mockup` included —
  // or a Canvas board) appear within seconds of each save, badged where they
  // come from and read-only until someone presses "Edit here" in the Hall.
  //
  // ⋯ → "Open the classic arena" mounts the old DesignArena for this story: it
  // still owns pinned review notes (MockupAnnotations) and the Blender bridge,
  // which the Hall doesn't have yet. Telling whether a story HAS pinned notes
  // costs one request per attachment, so the entry is always offered.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { router } from '../../lib/router.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import type { DesignStudio } from '../../lib/api/types';
  import type { ProductStory } from './types';
  import DesignGrid from '../design-hall/DesignGrid.svelte'; // ui-guards: allow — Product embeds Design Hall (one sidebar module)
  import GridFilterBar from '../design-hall/GridFilterBar.svelte'; // ui-guards: allow — Product embeds Design Hall (one sidebar module)
  import NewDesignModal from '../design-hall/NewDesignModal.svelte'; // ui-guards: allow — Product embeds Design Hall (one sidebar module)
  import StudioBadge from '../design-hall/StudioBadge.svelte'; // ui-guards: allow — Product embeds Design Hall (one sidebar module)
  import { library } from '../design-hall/library.svelte'; // ui-guards: allow — Product embeds Design Hall (one sidebar module)
  import { NO_FILTERS, type GridFilters, type GroupBy } from '../design-hall/designGrid'; // ui-guards: allow — Product embeds Design Hall (one sidebar module)
  import { STUDIOS, studioInfo, titleFromPrompt } from '../design-hall/model'; // ui-guards: allow — Product embeds Design Hall (one sidebar module)
  import { generateFromBrief } from '../design-hall/assist/handoff'; // ui-guards: allow — Product embeds Design Hall (one sidebar module)
  import DesignArena from './design/DesignArena.svelte';
  import { storyArtifactRoute, storyHallRoute, storyLinkRoute } from './storyDesigns';

  interface Props {
    story: ProductStory;
    isEpic: boolean;
  }
  let { story, isEpic }: Props = $props();

  const canView = $derived(auth.can('design', 'view'));
  /** How copy names the story: its key, else (a local draft has none) its title. */
  const storyName = $derived(story.source_key || `“${story.title}”`);
  const canEdit = $derived(auth.can('design', 'edit'));

  /** The old arena, on demand (⋯ → Open the classic arena). */
  let classic = $state(false);

  // ── Grid ────────────────────────────────────────────────────────────────
  const scope = $derived({ story_id: story.id, include_children: isEpic });
  let filters = $state<GridFilters>({ ...NO_FILTERS });
  // An epic reads best grouped by its child stories.
  let groupBy = $state<GroupBy>(untrack(() => (isEpic ? 'story' : 'recent')));
  let count = $state(0);
  let gridLoaded = $state(false);

  // The Hall's library names stories on cards / group headings and lists the
  // projects the New design sheet offers. Seed THIS story (and an epic's
  // children) so the sheet's "Implements story" names it even before any
  // design links it; the library merges, never drops, what it resolves.
  $effect(() => {
    const refs = [story, ...(isEpic ? product.childrenOf(story.id) : [])];
    untrack(() => {
      const add: typeof library.stories = {};
      for (const s of refs) {
        if (!library.stories[s.id]) add[s.id] = { id: s.id, source_key: s.source_key, title: s.title, parent_id: s.parent_id, tree_kind: s.tree_kind };
      }
      if (Object.keys(add).length) library.stories = { ...library.stories, ...add };
      if (canView && !library.loaded && !library.loading) void library.load();
    });
  });

  // Cards link to `#/design/a/<id>`; opened from here they carry the story
  // (`?from=story/<id>`) so the artifact view leads back. A capture listener
  // on the grid rewrites a plain click (a modified click keeps the browser's
  // own behaviour); other links in the grid (group headings) are untouched.
  let gridEl = $state<HTMLElement | null>(null);
  $effect(() => {
    const el = gridEl;
    if (!el) return;
    const onClick = (e: MouseEvent): void => {
      if (e.defaultPrevented || e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return;
      const a = (e.target as Element | null)?.closest?.('a[href]');
      const route = storyLinkRoute(a?.getAttribute('href'), story.id);
      if (!route) return;
      e.preventDefault();
      router.go(route);
    };
    el.addEventListener('click', onClick, true);
    return () => el.removeEventListener('click', onClick, true);
  });

  // ── New design (the Hall's sheet, story preset) ─────────────────────────
  let newOpen = $state(false);
  function openNew(): void {
    if (!canEdit) {
      toasts.warn('You can view designs but not create them', 'Ask a workspace admin for Design Hall edit access.');
      return;
    }
    newOpen = true;
  }
  function created(id: string): void {
    newOpen = false;
    void library.load();
    router.go(storyArtifactRoute(id, story.id));
  }

  // ── Ask Otto (the lobby's Generate, with the story set) ─────────────────
  const studios = STUDIOS.filter((s) => s.formats.length > 0);
  let brief = $state('');
  let studio = $state<DesignStudio>('frames');
  let asking = $state(false);
  const ready = $derived(!!brief.trim() && !asking && canEdit);

  function studioMenu(e: MouseEvent): void {
    ctxMenu.showAt(
      e.currentTarget as HTMLElement,
      studios.map((s) => ({ label: s.name, icon: s.icon, checked: s.id === studio, hint: s.blurb, action: () => (studio = s.id) })),
    );
  }

  /** Create a draft that implements this story (so Otto's brief carries the
   *  story and its acceptance criteria), start one generate turn, and open it
   *  on its Otto tab. The draft exists even when Otto can't start. */
  async function ask(): Promise<void> {
    const text = brief.trim();
    if (!ready || !text) return;
    const wsId = ws.currentId;
    if (!wsId) {
      toasts.warn('Pick a workspace first', 'New designs are filed under a workspace.');
      return;
    }
    asking = true;
    try {
      const { artifactId, assistError } = await generateFromBrief({
        workspaceId: wsId,
        studio,
        format: studioInfo(studio).formats[0],
        title: titleFromPrompt(text),
        storyId: story.id,
        brief: text,
        variants: 1,
        references: [],
      });
      brief = '';
      if (assistError) toasts.warn('Draft created, but Otto couldn’t start', assistError);
      router.go(storyArtifactRoute(artifactId, story.id, 'otto'));
    } catch (e) {
      toastError('Couldn’t create the draft', e);
    } finally {
      asking = false;
    }
  }

  function moreMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'Open in Design Hall', icon: 'external', disabled: !canView, action: () => router.go(storyHallRoute(story.id)) },
      {
        label: 'Open the classic arena',
        icon: 'layers',
        hint: 'Pinned review notes and Blender rendering, until Design Hall has them',
        action: () => (classic = true),
      },
    ];
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, { align: 'end' });
  }
</script>

{#if classic}
  <div class="classic-bar" role="note" data-testid="story-designs-classic">
    <Icon name="info" size={14} />
    <span class="txt">Classic arena: pinned review notes and the Blender bridge stay here until Design Hall has them.</span>
    <button class="btn small" onclick={() => (classic = false)} data-testid="story-designs-back">
      <Icon name="chevronLeft" size={12} /> Back to designs
    </button>
  </div>
  <DesignArena keepStory />
{:else if !canView}
  <EmptyState
    icon="lock"
    title="Designs live in Design Hall"
    body="You don’t have Design Hall access in this workspace — ask a workspace admin for it. The classic arena still opens this story’s design attachments."
    actionLabel="Open the classic arena"
    actionKind="secondary"
    onaction={() => (classic = true)}
  />
{:else}
  <section class="story-designs" aria-label="Designs for this story" data-testid="story-designs">
    <div class="sd-head">
      <h2 class="sd-title">
        Designs
        {#if gridLoaded}<span class="sd-count" data-testid="story-designs-count">{count}</span>{/if}
      </h2>
      <div class="sd-actions">
        <button class="btn primary" onclick={openNew} disabled={!canEdit}
          title={canEdit ? `A new design that implements ${storyName}` : 'You can view designs but not create them'}
          data-testid="story-designs-new">
          <Icon name="plus" size={12} /> New design
        </button>
        <button class="icon-btn" onclick={moreMenu} aria-label="More design actions" title="More design actions"
          aria-haspopup="menu" data-testid="story-designs-more">
          <Icon name="more" size={14} />
        </button>
      </div>
    </div>

    <form class="sd-ask" onsubmit={(e) => { e.preventDefault(); void ask(); }} data-testid="story-designs-ask">
      <button type="button" class="btn small sd-studio" onclick={studioMenu} disabled={!canEdit || asking}
        aria-label={`Studio: ${studioInfo(studio).name}`} title="The studio Otto designs in" aria-haspopup="menu">
        <StudioBadge {studio} showName />
        <Icon name="chevronDown" size={12} />
      </button>
      <input dir="auto" class="input sd-brief" bind:value={brief} disabled={!canEdit || asking}
        placeholder={canEdit ? `Ask Otto to design something for ${storyName}…` : 'You can view designs but not create them'}
        aria-label="Ask Otto for a design for this story" />
      <button class="btn" type="submit" disabled={!ready}>
        <Icon name="sparkle" size={12} /> {asking ? 'Starting…' : 'Ask Otto'}
      </button>
    </form>

    <GridFilterBar bind:filters bind:groupBy />

    <div class="sd-grid" bind:this={gridEl}>
      <DesignGrid {scope} {filters} {groupBy} bind:count bind:loaded={gridLoaded} onclearfilters={() => (filters = { ...NO_FILTERS })}>
        {#snippet empty()}
          <EmptyState
            icon="palette"
            title="No designs for this story yet"
            body={`Start one with New design, or describe it to Otto above. Designs made in Design Hall that implement ${storyName}${isEpic ? ' or its stories' : ''} show here too.`}
          />
        {/snippet}
      </DesignGrid>
    </div>
  </section>
{/if}

{#if newOpen}
  <NewDesignModal storyId={story.id} onclose={() => (newOpen = false)} oncreated={created} />
{/if}

<style>
  .story-designs {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-width: 0;
  }
  .sd-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px 12px;
    flex-wrap: wrap;
  }
  .sd-title {
    display: flex;
    align-items: baseline;
    gap: 8px;
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .sd-count {
    color: var(--text-dim);
    font-size: var(--fs-s);
    font-weight: 500;
  }
  .sd-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .sd-ask {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    min-width: 0;
  }
  .sd-studio {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .sd-brief {
    flex: 1 1 200px;
    min-width: 0;
  }
  .sd-grid {
    min-width: 0;
  }
  .classic-bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px 10px;
    margin-block-end: 10px;
    padding: 8px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--info-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .classic-bar .txt {
    flex: 1 1 220px;
    min-width: 0;
  }
</style>
