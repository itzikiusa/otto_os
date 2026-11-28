<script lang="ts">
  // A page's LEFT pane (design.md §3 "List pane", §7.3–§7.6):
  //
  //   ┌ name  count ············ [＋] [⋯] ┐  36 px header: ONE primary, the rest under ⋯
  //   │ search (optional, one row)         │   (⋯ starts with "Hide <name> ⌘B")
  //   │ filters: Tabs or chips (one row)   │
  //   │ body: SideSection / SideRow / own  │  ↑ ↓ Home End → ← Enter (use:listNav)
  //   └ footer (optional, one row)         ┘
  //
  // `views` replaces the name with a segmented control (Files | Structure);
  // `selection` replaces the whole header while multi-selecting (count,
  // Clear, one action, ⋯). Loading = skeleton rows, error = compact LoadState
  // with Retry, both inside the pane; an EMPTY list is not a ListPane state —
  // ListDetail `empty` drops the pane and the page's EmptyState owns the page.
  // As a ListDetail's `list` the name comes from it; on its own (a pane that
  // is not a list/detail page, or a second-level list inside a page's content:
  // the IDE's Git log) pass `name` — it keeps its own header, has no Hide and
  // never pushes (give it its own ListDetail for that). On a phone, where the
  // page's list is the page, its header moves into PageHeader (☰ · name count
  // · primary · ⋯, or the views in the name's place).
  import { getContext, type Snippet } from 'svelte';
  import Icon, { type IconName } from '../Icon.svelte';
  import LoadState from '../LoadState.svelte';
  import { ctxMenu, type MenuItem } from '../../contextmenu.svelte';
  import { listNav } from './listNav';
  import { LIST_HOST, type ListDetailCtx } from './context';

  interface Props {
    /** The pane's name (defaults to the ListDetail's). */
    name?: string;
    count?: number | null;
    /** The ONE header action (usually "New"); none when the page's primary does the same. */
    primary?: { icon: IconName; label: string; onclick: () => void; testid?: string };
    /** Everything else, under ⋯ (after "Hide <name> ⌘B"). */
    menu?: () => MenuItem[];
    /** A segmented control in place of the name (views AS the header). */
    views?: Snippet;
    /** Multi-select: replaces the header (count · Clear · one action · ⋯). */
    selection?: Snippet;
    /** One search row (`searchRow`, not `search`: a page's own `search` state would shadow it). */
    searchRow?: Snippet;
    /** One row of Tabs or chips — never two. */
    filterRow?: Snippet;
    footer?: Snippet;
    loading?: boolean;
    error?: string | null;
    onretry?: () => void;
    /** `nav` for a section nav (Settings), else a labelled aside. */
    as?: 'aside' | 'nav';
    /** Rows selector for the keyboard walk (default [data-side-row]). */
    rows?: string;
    testid?: string;
    children: Snippet;
  }
  let { name, count = null, primary, menu, views, selection, searchRow, filterRow, footer, loading = false, error = null, onretry, as = 'aside', rows, testid, children }: Props = $props();

  const ld = getContext<ListDetailCtx | null | undefined>(LIST_HOST);
  const label = $derived(name ?? ld?.name ?? '');
  const canHide = $derived(!!ld?.canHide);
  // A phone showing the list as the page: PageHeader draws this header.
  $effect(() => ld?.setHeader({ name: label, count, primary, menu, views }));

  function openMenu(e: MouseEvent): void {
    const items: MenuItem[] = [];
    if (canHide) items.push({ label: `Hide ${label}`, icon: 'sidebar', hint: '⌘B', action: () => ld?.hide() });
    const extra = menu?.() ?? [];
    if (items.length && extra.length) items.push({ separator: true });
    items.push(...extra);
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, { align: 'end' });
  }
</script>

<svelte:element this={as} class="side-list" aria-label={label} data-testid={testid}>
  {#if selection}
    <header class="side-pane-head selecting">{@render selection()}</header>
  {:else if ld?.phoneList}
    <!-- The page header carries name · count · primary · ⋯ (one row on a phone). -->
  {:else}
    <header class="side-pane-head" class:views={!!views}>
      {#if views}
        <div class="sph-views">{@render views()}</div>
      {:else}
        <h2 class="sph-title">{label}</h2>
        {#if count !== null}<span class="sph-count" aria-label="{count} items">{count}</span>{/if}
        <span class="sph-grow"></span>
      {/if}
      {#if primary}
        <button type="button" class="icon-btn sph-primary" aria-label={primary.label} title={primary.label} data-testid={primary.testid} onclick={primary.onclick}>
          <Icon name={primary.icon} size={14} />
        </button>
      {/if}
      {#if canHide || menu}
        <button type="button" class="icon-btn" aria-haspopup="menu" aria-label="{label} actions" title="More" data-testid={testid ? `${testid}-more` : undefined} onclick={openMenu}>
          <Icon name="more" size={14} />
        </button>
      {/if}
    </header>
  {/if}
  {#if searchRow}<div class="side-pane-search">{@render searchRow()}</div>{/if}
  {#if filterRow}<div class="side-pane-filters">{@render filterRow()}</div>{/if}
  <div class="side-pane-body" use:listNav={{ rows }}>
    {#if loading || error}
      <LoadState what={label.toLowerCase()} {loading} {error} {onretry} empty variant="compact" rows={6} />
    {:else}
      {@render children()}
    {/if}
  </div>
  {#if footer}<footer class="side-pane-foot">{@render footer()}</footer>{/if}
</svelte:element>
