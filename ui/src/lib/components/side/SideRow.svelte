<script lang="ts">
  // One row of a list pane (design.md §3, §7.10) — one look for every list,
  // tree and nav: 5 px radius, text 16 px in, selected = accent tint + bold
  // title, hover = --hover, keyboard focus = an inset ring.
  //
  //   line      30 px: [twisty] · 14 px icon · title · trailing · ⋯ on hover/focus
  //   two-line  44 px: `meta` adds one dim line under the title
  //   tree      `depth` indents 12 px a level; `expanded` adds the twisty
  //
  // The whole row is one button (Enter / Space opens). Its actions are `menu`:
  // the ⋯ on hover / focus, right-click, long-press and the ContextMenu key
  // all open the same ctxMenu. Lists mark the open row aria-current="true";
  // a tree passes `tree` (treeitem + aria-selected / aria-level). Trees and
  // virtual lists that draw their own rows use the same global classes
  // (app.css "Side surfaces": .side-row, .selected, --side-depth).
  import type { Snippet } from 'svelte';
  import Icon, { type IconName } from '../Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../contextmenu.svelte';
  import { rowMenu } from '../../rowMenu';

  interface Props {
    title: string;
    icon?: IconName;
    /** Second, dim line: the row becomes two-line (44 px). */
    meta?: string;
    selected?: boolean;
    /** Multi-select: checked rows show a check and the fill. */
    checked?: boolean;
    /** Tree indent level (0 = top). */
    depth?: number;
    /** A tree node: true / false draws the twisty; undefined = a leaf. */
    expanded?: boolean;
    ontoggle?: () => void;
    /** Tree semantics (treeitem, aria-selected, aria-level). */
    tree?: boolean;
    /** Count, badge, status word — never colour alone. */
    trailing?: Snippet;
    menu?: () => MenuItem[];
    tip?: string;
    testid?: string;
    onclick: (e: MouseEvent) => void;
  }
  let { title, icon, meta, selected = false, checked, depth = 0, expanded, ontoggle, tree = false, trailing, menu, tip, testid, onclick }: Props = $props();

  function openMenu(e: MouseEvent): void {
    if (!menu) return;
    e.preventDefault();
    ctxMenu.show(e, menu());
  }
</script>

<div class="side-row" class:two-line={!!meta} class:selected={selected || checked} style:--side-depth={depth} data-testid={testid}>
  {#if expanded !== undefined}
    <button type="button" class="side-twisty" tabindex="-1" aria-label="{expanded ? 'Collapse' : 'Expand'} {title}" title="{expanded ? 'Collapse' : 'Expand'} {title}" data-side-toggle onclick={() => ontoggle?.()}>
      <Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
  {/if}
  <button
    type="button"
    class="side-row-main"
    data-side-row
    title={tip}
    role={tree ? 'treeitem' : undefined}
    aria-selected={tree ? selected : undefined}
    aria-level={tree ? depth + 1 : undefined}
    aria-expanded={expanded}
    aria-current={!tree && selected ? 'true' : undefined}
    {onclick}
    oncontextmenu={menu ? openMenu : undefined}
    use:rowMenu
  >
    {#if checked !== undefined}<Icon name={checked ? 'check' : 'square'} size={14} />{:else if icon}<Icon name={icon} size={14} />{/if}
    <span class="side-row-text">
      <span class="side-row-title" dir="auto">{title}</span>
      {#if meta}<span class="side-row-meta" dir="auto">{meta}</span>{/if}
    </span>
    {#if trailing}<span class="side-row-trailing">{@render trailing()}</span>{/if}
  </button>
  {#if menu}
    <button type="button" class="icon-btn side-row-more" tabindex="-1" aria-label="{title} actions" title="More" onclick={openMenu}>
      <Icon name="more" size={14} />
    </button>
  {/if}
</div>
