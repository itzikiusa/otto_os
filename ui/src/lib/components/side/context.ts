// What a ListDetail tells the ListPane that is its list (and a nested
// ListDetail inside its content): the pane's name, whether it can be hidden
// here, whether the content side is on screen, and — on a phone, where the
// list IS the page — where the pane's header goes (PageHeader draws it, pageNav).
//
// A ListPane belongs to the nearest ListDetail that declares it as its `list`
// (LIST_HOST, set by ListHost around that snippet only). Anything else under a
// ListDetail (its content, its Detail pane) sees LIST_DETAIL — a nested
// ListDetail reads its parent's depth and visibility from it — but a ListPane
// there is on its own: its own header, no Hide, no phone push.
import type { Snippet } from 'svelte';
import type { IconName } from '../Icon.svelte';
import type { MenuItem } from '../../contextmenu.svelte';

export const LIST_DETAIL = Symbol('otto.listDetail');
/** The ListDetail whose `list` snippet this is (null elsewhere under one). */
export const LIST_HOST = Symbol('otto.listHost');

/** Phone, the list is the page: its ⋯ holds the pane's menu, then the page's
 *  own `more` rows (page-level actions such as Switch context stay reachable). */
export function listPageMenu(pane?: () => MenuItem[], page?: () => MenuItem[]): (() => MenuItem[]) | undefined {
  if (!pane || !page) return pane ?? page;
  return () => {
    const a = pane();
    const b = page();
    return a.length && b.length ? [...a, { separator: true }, ...b] : [...a, ...b];
  };
}

/** A list pane's header, as PageHeader draws it on a phone. */
export interface PaneHeader {
  name: string;
  count: number | null;
  primary?: { icon: IconName; label: string; onclick: () => void; testid?: string };
  menu?: () => MenuItem[];
  /** A pane with views: its segmented control takes the title's place. */
  views?: Snippet;
}

export interface ListDetailCtx {
  readonly name: string;
  /** 0 = the page's list, 1 = a ListDetail nested in its content, … */
  readonly depth: number;
  /** Desktop / tablet with items: the pane can be hidden (⋯ › Hide <name> ⌘B). */
  readonly canHide: boolean;
  /** Phone, the list is the page: its header moves into the page header. */
  readonly phoneList: boolean;
  hide(): void;
  /** The content side is on screen (a phone showing the list: false). */
  contentVisible(): boolean;
  /** The ListPane's header (from an $effect; returns the release). */
  setHeader(h: PaneHeader): () => void;
}
