<script lang="ts">
  // Product Story Analysis page — left sidebar (stories list + import), a
  // Stories | Learnings toggle, a per-story tab strip, and tab content.
  // Tabs 6.3-6.6 are placeholders; only Overview (6.2) is implemented.
  import Icon from '../../lib/components/Icon.svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import ImportDialog from './ImportDialog.svelte';
  import OverviewTab from './OverviewTab.svelte';
  import AnalysisTab from './AnalysisTab.svelte';
  import QuestionsTab from './QuestionsTab.svelte';
  import NotesTab from './NotesTab.svelte';
  import RewriteTab from './RewriteTab.svelte';
  import TestCasesTab from './TestCasesTab.svelte';
  import PlanTab from './PlanTab.svelte';
  import HistoryTab from './HistoryTab.svelte';
  import InjectTab from './InjectTab.svelte';
  import DiscoveryTab from './DiscoveryTab.svelte';
  import MockupsTab from './MockupsTab.svelte';
  import RefineTab from './RefineTab.svelte';
  import LearningsView from './LearningsView.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { ProductStory } from './types';

  let importOpen = $state(false);
  let draftCreating = $state(false);

  // ── Mobile (≤640px) accordion state ───────────────────────────────────────
  // On a phone the two panels (story list + the per-story content) stack and
  // each becomes an independently-scrollable, collapsible section. Exactly one
  // is expanded at a time so the open panel gets the full remaining height to
  // scroll in; the other shows just its tappable header. This is a no-op on
  // desktop/tablet where the CSS for these classes is never applied.
  let mobileSection = $state<'list' | 'content'>('list');

  async function createDraft(): Promise<void> {
    draftCreating = true;
    try {
      await product.createDraft();
      product.tab = 'overview';
      // On mobile, reveal the new draft's content panel right away.
      mobileSection = 'content';
    } catch (e) {
      console.error('createDraft failed', e);
    } finally {
      draftCreating = false;
    }
  }

  let learningsFilter = $state<'all' | 'pattern' | 'avoid'>('all');

  // Tag filter state
  let activeTagFilter = $state<string | null>(null);

  /** Parse csv tags → deduplicated, trimmed, non-empty array.
   *  Null-safe: a missing/garbage `tags` value must never throw, or it would
   *  break the `allTags`/`filteredStories` deriveds and freeze the whole page. */
  function parseTags(csv: string | null | undefined): string[] {
    if (!csv) return [];
    return [...new Set(csv.split(',').map((t) => t.trim()).filter(Boolean))];
  }

  /** All distinct tags across all stories, sorted. */
  const allTags = $derived(
    [...new Set(product.stories.flatMap((s) => parseTags(s.tags)))].sort(),
  );

  /** Stories shown after applying the tag filter. */
  const filteredStories = $derived(
    activeTagFilter === null
      ? product.stories
      : product.stories.filter((s) => parseTags(s.tags).includes(activeTagFilter!)),
  );

  // Reload stories whenever the workspace changes (mirrors DatabasePage pattern).
  $effect(() => {
    if (ws.currentId) {
      void product.loadStories();
    }
  });

  const TABS: { id: string; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    { id: 'analysis', label: 'Analysis' },
    { id: 'questions', label: 'Questions' },
    { id: 'notes', label: 'Notes' },
    { id: 'rewrite', label: 'Rewrite' },
    { id: 'testcases', label: 'Test Cases' },
    { id: 'plan', label: 'Plan' },
    { id: 'history', label: 'History' },
    { id: 'inject', label: 'Inject' },
    { id: 'discovery', label: 'Discovery' },
    { id: 'refine', label: 'Refine' },
    { id: 'mockups', label: 'Mockups' },
  ];

  function stageColor(stage: string): string {
    switch (stage) {
      case 'draft': return 'stage-draft';
      case 'review': return 'stage-review';
      case 'approved': return 'stage-approved';
      case 'done': return 'stage-done';
      default: return 'stage-other';
    }
  }

  function sourceIcon(kind: string): string {
    switch (kind) {
      case 'jira': return 'ticket';
      case 'confluence': return 'globe';
      default: return 'file';
    }
  }

  function selectStory(s: ProductStory): void {
    void product.select(s.id);
    // Reset to overview whenever a new story is selected.
    product.tab = 'overview';
    // On mobile, switch to the content panel so the picked story is visible.
    mobileSection = 'content';
  }

  async function deleteStory(s: ProductStory): Promise<void> {
    const ok = await confirmer.ask(
      `Delete "${s.title}"? This removes it from Otto (the Jira/Confluence item is untouched).`,
      { title: 'Delete story', confirmLabel: 'Delete', danger: true },
    );
    if (!ok) return;
    void product.deleteStory(s.id);
  }
</script>

<div class="product-page" class:m-list-open={mobileSection === 'list'} class:m-content-open={mobileSection === 'content'}>
  <!-- ── Mobile accordion header for the list panel (phone only) ───────── -->
  <button
    class="m-acc-head"
    aria-expanded={mobileSection === 'list'}
    onclick={() => (mobileSection = 'list')}
  >
    <Icon name={mobileSection === 'list' ? 'chevronDown' : 'chevronRight'} size={14} />
    <span class="m-acc-title">{product.view === 'learnings' ? 'Learnings' : 'Stories'}</span>
    {#if product.view === 'stories'}
      <span class="m-acc-count">{product.stories.length}</span>
    {/if}
  </button>

  <!-- ── Left sidebar — always rendered to avoid layout jump ───────────── -->
  <aside class="product-side">
    <!-- Mobile-only Stories|Learnings toggle (the desktop one lives in the
         content header, which is collapsed when the list panel is open). -->
    <div class="m-view-toggle" role="tablist" aria-label="View (mobile)">
      <button
        class="vt"
        class:active={product.view === 'stories'}
        role="tab"
        aria-selected={product.view === 'stories'}
        onclick={() => (product.view = 'stories')}
      >Stories</button>
      <button
        class="vt"
        class:active={product.view === 'learnings'}
        role="tab"
        aria-selected={product.view === 'learnings'}
        onclick={() => (product.view = 'learnings')}
      >Learnings</button>
    </div>
    {#if product.view === 'stories'}
      <!-- Stories sidebar -->
      <div class="side-head">
        <span class="side-title">Stories</span>
        <div class="side-head-actions">
          <button
            class="head-btn"
            onclick={createDraft}
            title="Start a blank draft (Discovery): jot ideas, refine with agents, then publish as a Story or RFC"
            disabled={draftCreating}
          >
            <Icon name="file" size={12} /> {draftCreating ? 'Creating…' : 'New draft'}
          </button>
          <button
            class="head-btn primary"
            onclick={() => (importOpen = true)}
            title="Import an existing Jira issue / Confluence page"
          >
            <Icon name="plus" size={12} /> Import
          </button>
        </div>
      </div>

      <!-- Tag filter row (only when tags exist) -->
      {#if allTags.length > 0}
        <div class="tag-filter-row">
          <button
            class="tag-filter-btn"
            class:active={activeTagFilter === null}
            onclick={() => (activeTagFilter = null)}
          >All</button>
          {#each allTags as tag (tag)}
            <button
              class="tag-filter-btn"
              class:active={activeTagFilter === tag}
              onclick={() => (activeTagFilter = activeTagFilter === tag ? null : tag)}
            >{tag}</button>
          {/each}
        </div>
      {/if}

      <div class="story-list">
        {#if product.loadingStories}
          <div class="list-empty">Loading…</div>
        {:else if product.stories.length === 0}
          <div class="list-empty">
            No stories yet.
            <button class="link" onclick={createDraft} disabled={draftCreating}>Start a draft →</button>
            <button class="link" onclick={() => (importOpen = true)}>Import one →</button>
          </div>
        {:else if filteredStories.length === 0}
          <div class="list-empty">No stories match the selected tag.</div>
        {:else}
          {#each filteredStories as s (s.id)}
            <div
              class="story-row-wrap"
              class:active={product.selectedId === s.id}
            >
              <button
                class="story-row"
                class:active={product.selectedId === s.id}
                onclick={() => selectStory(s)}
                title={s.source_key}
              >
                <span class="story-icon"><Icon name={sourceIcon(s.source_kind)} size={13} /></span>
                <span class="story-info">
                  <span class="story-title">{s.title}</span>
                  <span class="story-meta">
                    <span class="stage-badge {stageColor(s.stage)}">{s.stage}</span>
                    {#if s.source_kind === 'draft'}
                      <span class="draft-badge">DRAFT</span>
                    {:else}
                      <span class="story-key mono">{s.source_key}</span>
                    {/if}
                  </span>
                  {#if parseTags(s.tags).length > 0}
                    <span class="story-tags">
                      {#each parseTags(s.tags) as tag (tag)}
                        <span class="story-tag-chip">{tag}</span>
                      {/each}
                    </span>
                  {/if}
                </span>
              </button>
              <button
                class="delete-btn"
                onclick={() => deleteStory(s)}
                aria-label="Delete story"
                title="Delete story"
              >
                <Icon name="trash" size={12} />
              </button>
            </div>
          {/each}
        {/if}
      </div>

      <div class="side-footer">
        <button class="import-btn" onclick={() => (importOpen = true)}>
          <Icon name="plus" size={13} />
          Import story
        </button>
      </div>
    {:else}
      <!-- Learnings sidebar — filter nav -->
      <div class="side-head">
        <span class="side-title">Learnings</span>
      </div>

      <div class="learn-nav">
        {#each ([
          { value: 'all', label: 'All' },
          { value: 'pattern', label: 'Patterns to follow' },
          { value: 'avoid', label: 'Cases to avoid' },
        ] as const) as opt (opt.value)}
          <button
            class="learn-filter-btn"
            class:active={learningsFilter === opt.value}
            onclick={() => (learningsFilter = opt.value)}
          >{opt.label}</button>
        {/each}
      </div>
    {/if}
  </aside>

  <!-- ── Mobile accordion header for the content panel (phone only) ────── -->
  <button
    class="m-acc-head"
    aria-expanded={mobileSection === 'content'}
    onclick={() => (mobileSection = 'content')}
  >
    <Icon name={mobileSection === 'content' ? 'chevronDown' : 'chevronRight'} size={14} />
    <span class="m-acc-title">
      {#if product.view === 'learnings'}
        Knowledge base
      {:else if product.selectedId && product.detail?.story}
        {product.detail.story.title}
      {:else if product.selectedId}
        Story
      {:else}
        Get started
      {/if}
    </span>
  </button>

  <!-- ── Main area ──────────────────────────────────────────────────────── -->
  <div class="product-main">
    <!-- Header row 1: Stories | Learnings toggle (always visible) -->
    <div class="product-header-row1">
      <div class="view-toggle" role="tablist" aria-label="View">
        <button
          class="vt"
          class:active={product.view === 'stories'}
          role="tab"
          aria-selected={product.view === 'stories'}
          onclick={() => (product.view = 'stories')}
        >Stories</button>
        <button
          class="vt"
          class:active={product.view === 'learnings'}
          role="tab"
          aria-selected={product.view === 'learnings'}
          onclick={() => (product.view = 'learnings')}
        >Learnings</button>
      </div>
    </div>

    <!-- Header row 2: per-story tab strip (only when a story is selected in Stories view) -->
    {#if product.view === 'stories' && product.selectedId}
      <div class="product-header-row2">
        <div class="tab-strip" role="tablist" aria-label="Story tabs">
          {#each TABS as t (t.id)}
            <button
              class="st"
              class:active={product.tab === t.id}
              role="tab"
              aria-selected={product.tab === t.id}
              onclick={() => (product.tab = t.id)}
            >{t.label}</button>
          {/each}
        </div>
      </div>
    {/if}

    <!-- Content -->
    <div class="product-body">
      {#if product.view === 'learnings'}
        <LearningsView filter={learningsFilter} />
      {:else if !product.selectedId}
        <div class="empty-state">
          <Icon name="file" size={28} />
          <p>
            Analyse a Jira/Confluence story — ask questions, draft a plan and test cases, then
            publish back. Import an existing issue or start a blank draft.
          </p>
          <div class="empty-actions">
            <button class="btn primary" onclick={createDraft} disabled={draftCreating}>
              <Icon name="plus" size={13} />
              {draftCreating ? 'Creating…' : 'Start a draft'}
            </button>
            <button class="btn ghost" onclick={() => (importOpen = true)}>
              <Icon name="plus" size={13} />
              Import story
            </button>
          </div>
        </div>
      {:else if product.tab === 'overview'}
        <OverviewTab />
      {:else if product.tab === 'analysis'}
        <AnalysisTab />
      {:else if product.tab === 'questions'}
        <QuestionsTab />
      {:else if product.tab === 'notes'}
        <NotesTab />
      {:else if product.tab === 'rewrite'}
        <RewriteTab />
      {:else if product.tab === 'testcases'}
        <TestCasesTab />
      {:else if product.tab === 'plan'}
        <PlanTab />
      {:else if product.tab === 'history'}
        <HistoryTab />
      {:else if product.tab === 'inject'}
        <InjectTab />
      {:else if product.tab === 'discovery'}
        <DiscoveryTab />
      {:else if product.tab === 'refine'}
        <RefineTab />
      {:else if product.tab === 'mockups'}
        <MockupsTab />
      {:else}
        <div class="muted">{product.tab} — coming soon</div>
      {/if}
    </div>
  </div>
</div>

{#if importOpen}
  <ImportDialog onclose={() => (importOpen = false)} />
{/if}

<style>
  .product-page {
    height: 100%;
    display: flex;
    min-height: 0;
  }

  /* ── Sidebar ─────────────────────────────────────────────────── */
  .product-side {
    width: 260px;
    flex-shrink: 0;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .side-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 10px 4px;
    flex-shrink: 0;
  }
  .side-head-actions {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .head-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 9px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: 11.5px;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    transition: background 110ms, border-color 110ms, opacity 110ms;
  }
  .head-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  .head-btn.primary {
    background: var(--accent);
    border-color: var(--accent);
    color: #fff;
  }
  .head-btn.primary:hover:not(:disabled) {
    opacity: 0.88;
  }
  .head-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .draft-badge {
    font-size: 9px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    padding: 1px 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .side-title {
    font-size: 10.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .story-list {
    flex: 1;
    overflow-y: auto;
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 0;
  }
  .list-empty {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 8px 4px;
    line-height: 1.5;
  }
  .link {
    border: none;
    background: none;
    color: var(--accent);
    cursor: pointer;
    font-size: 11.5px;
    padding: 0;
  }
  /* Wrapper handles hover background + shows delete btn */
  .story-row-wrap {
    display: flex;
    align-items: center;
    border-radius: var(--radius-s);
    transition: background 100ms ease-out;
    position: relative;
  }
  .story-row-wrap:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
  }
  .story-row-wrap.active {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
  }
  .story-row {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    flex: 1;
    min-width: 0;
    padding: 7px 8px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .story-row.active {
    color: var(--accent);
  }
  /* Delete button — hidden until row is hovered or active */
  .delete-btn {
    display: grid;
    place-items: center;
    flex-shrink: 0;
    width: 22px;
    height: 22px;
    margin-inline-end: 6px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: transparent;
    cursor: pointer;
    transition: color 100ms, background 100ms;
    padding: 0;
  }
  .story-row-wrap:hover .delete-btn,
  .story-row-wrap.active .delete-btn {
    color: var(--text-dim);
  }
  .delete-btn:hover {
    background: color-mix(in srgb, #ef4444 15%, transparent) !important;
    color: #ef4444 !important;
  }
  .story-icon {
    flex-shrink: 0;
    color: var(--text-dim);
    margin-top: 2px;
  }
  .story-row-wrap.active .story-icon {
    color: var(--accent);
  }
  .story-info {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .story-title {
    font-size: 12.5px;
    font-weight: 500;
    line-height: 1.3;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .story-meta {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .story-key {
    font-size: 10.5px;
    color: var(--text-dim);
  }
  /* Stage badges */
  .stage-badge {
    font-size: 9.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 6px;
    border-radius: 999px;
  }
  .stage-draft {
    background: color-mix(in srgb, var(--text-dim) 18%, transparent);
    color: var(--text-dim);
  }
  .stage-review {
    background: color-mix(in srgb, #f59e0b 18%, transparent);
    color: #b45309;
  }
  .stage-approved {
    background: color-mix(in srgb, var(--status-working) 18%, transparent);
    color: var(--status-working);
  }
  .stage-done {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .stage-other {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    color: var(--text-dim);
  }
  /* ── Tag filter row ─────────────────────────────────────────── */
  .tag-filter-row {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    padding: 4px 8px 4px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .tag-filter-btn {
    padding: 1px 7px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font-size: 10px;
    cursor: pointer;
    transition: background 100ms, color 100ms, border-color 100ms;
    white-space: nowrap;
  }
  .tag-filter-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }
  .tag-filter-btn.active {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    border-color: var(--accent);
    color: var(--accent);
    font-weight: 600;
  }

  /* ── Story tag chips (inline in list row) ───────────────────── */
  .story-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 3px;
    margin-top: 1px;
  }
  .story-tag-chip {
    font-size: 9px;
    padding: 1px 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    color: var(--accent);
    opacity: 0.85;
  }

  .side-footer {
    padding: 8px;
    border-top: 1px solid var(--border);
    flex-shrink: 0;
  }
  .import-btn {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    padding: 6px 10px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    cursor: pointer;
    transition: border-color 120ms, color 120ms;
  }
  .import-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  /* ── Main area ───────────────────────────────────────────────── */
  .product-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  /* Row 1: view toggle only */
  .product-header-row1 {
    display: flex;
    align-items: center;
    padding: 8px 14px 0;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  /* Row 2: per-story tab strip */
  .product-header-row2 {
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    padding: 0 14px;
    /* The inner .tab-strip scrolls horizontally; overflow:hidden here clipped it
       so tabs past the first few were unreachable on a narrow screen. */
    overflow-x: auto;
    scrollbar-width: none;
  }
  .view-toggle {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }
  .vt {
    height: 30px;
    padding: 0 12px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
    white-space: nowrap;
  }
  .vt:hover {
    color: var(--text);
  }
  .vt.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  .tab-strip {
    display: flex;
    align-items: center;
    gap: 1px;
    overflow-x: auto;
    white-space: nowrap;
    scrollbar-width: none;
  }
  .tab-strip::-webkit-scrollbar {
    display: none;
  }
  .st {
    height: 30px;
    padding: 0 11px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    border-bottom: 2px solid transparent;
    margin-bottom: -1px;
    white-space: nowrap;
    flex-shrink: 0;
  }
  .st:hover {
    color: var(--text);
  }
  .st.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  .product-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 16px;
    display: flex;
    flex-direction: column;
  }
  .empty-state {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--text-dim);
    text-align: center;
  }
  .empty-state p {
    margin: 0;
    font-size: 13px;
    max-width: 380px;
    line-height: 1.5;
  }
  .empty-actions {
    display: flex;
    gap: 8px;
  }
  .muted {
    padding: 32px 16px;
    color: var(--text-dim);
    font-size: 13px;
    font-style: italic;
  }
  .btn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 14px;
    border-radius: var(--radius-s);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    border: 1px solid var(--border);
    background: transparent;
    color: var(--text);
  }
  .btn.ghost {
    border-color: var(--border);
  }
  .btn.ghost:hover {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border-color: var(--accent);
    color: var(--accent);
  }
  .icon-btn {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .icon-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text);
  }
  .mono {
    font-family: var(--font-mono, monospace);
  }

  /* ── Learnings sidebar nav ───────────────────────────────────────── */
  .learn-nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 8px;
    flex: 1;
    min-height: 0;
  }
  .learn-filter-btn {
    display: flex;
    align-items: center;
    padding: 7px 10px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 12.5px;
    font-weight: 500;
    cursor: pointer;
    text-align: start;
    transition: background 100ms, color 100ms;
  }
  .learn-filter-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 10%, transparent);
    color: var(--text);
  }
  .learn-filter-btn.active {
    background: color-mix(in srgb, var(--accent) 15%, transparent);
    color: var(--accent);
    font-weight: 600;
  }

  /* ── Mobile-only chrome — hidden on desktop/tablet ───────────────────── */
  .m-acc-head {
    display: none;
  }
  .m-view-toggle {
    display: none;
  }

  @media (max-width: 640px) {
    /* The page becomes a vertical accordion: two tappable section headers,
       each followed by its panel. Exactly one panel is expanded at a time and
       takes all the remaining height to scroll inside; the other collapses to
       just its header. */
    .product-page {
      flex-direction: column;
    }

    /* The desktop view toggle lives in the content header; on mobile that header
       collapses with the content panel, so show the list-panel copy instead. */
    .product-header-row1 {
      display: none;
    }
    .m-view-toggle {
      display: flex;
      align-items: center;
      gap: 2px;
      flex-shrink: 0;
      padding: 4px 8px 0;
      border-bottom: 1px solid var(--border);
    }

    .m-acc-head {
      display: flex;
      align-items: center;
      gap: 8px;
      width: 100%;
      flex-shrink: 0;
      padding: 12px 14px;
      border: none;
      border-bottom: 1px solid var(--border);
      background: var(--bg-sidebar, var(--surface));
      color: var(--text);
      font-size: 15px;
      font-weight: 600;
      cursor: pointer;
      text-align: start;
      -webkit-tap-highlight-color: transparent;
    }
    .m-acc-head:active {
      background: color-mix(in srgb, var(--text-dim) 12%, transparent);
    }
    .m-acc-title {
      flex: 1;
      min-width: 0;
      overflow: hidden;
      text-overflow: ellipsis;
      white-space: nowrap;
    }
    .m-acc-count {
      flex-shrink: 0;
      font-size: 12px;
      font-weight: 600;
      color: var(--text-dim);
      background: color-mix(in srgb, var(--text-dim) 14%, transparent);
      border-radius: 999px;
      padding: 1px 9px;
    }

    /* Panels: collapsed by default; the open one gets the remaining height and
       scrolls on its own. */
    .product-side {
      width: 100%;
      border-inline-end: none;
      max-height: none;
      min-height: 0;
      overflow: hidden;
      flex: 0 0 0;
      height: 0;
    }
    .product-main {
      min-height: 0;
      overflow: hidden;
      flex: 0 0 0;
      height: 0;
    }
    .m-list-open .product-side {
      flex: 1 1 auto;
      height: auto;
      overflow-y: auto;
    }
    .m-content-open .product-main {
      flex: 1 1 auto;
      height: auto;
      overflow: hidden; /* inner .product-body owns the scroll */
    }

    /* The list's own internal scroller fills the expanded panel. */
    .m-list-open .story-list,
    .m-list-open .learn-nav {
      flex: 1 1 auto;
    }

    /* ── Bigger, more legible text on phones ───────────────────────────── */
    .side-title {
      font-size: 12px;
    }
    .head-btn {
      font-size: 13px;
      padding: 6px 11px;
    }
    .list-empty,
    .link {
      font-size: 14px;
    }
    .story-title {
      font-size: 15px;
    }
    .story-key,
    .story-meta {
      font-size: 12.5px;
    }
    .stage-badge {
      font-size: 11px;
    }
    .tag-filter-btn,
    .story-tag-chip {
      font-size: 12px;
    }
    .import-btn {
      font-size: 14px;
      padding: 9px 12px;
    }
    .vt {
      height: 38px;
      font-size: 14.5px;
      padding: 0 14px;
    }
    .st {
      height: 38px;
      font-size: 14px;
      padding: 0 13px;
    }
    .learn-filter-btn {
      font-size: 14.5px;
      padding: 10px 12px;
    }
    .empty-state p {
      font-size: 14.5px;
    }
    .btn {
      font-size: 14px;
      padding: 9px 16px;
    }
    .product-body {
      padding: 14px;
    }
    /* Comfortable touch target for the per-row delete button. */
    .delete-btn {
      width: 30px;
      height: 30px;
    }
    .story-row {
      padding: 10px 8px;
    }
  }
</style>
