// What a ListDetail tells the ListPane inside it (and a nested ListDetail
// inside its content): the pane's name, whether it can be hidden here,
// whether the content side is on screen, and — on a phone, where the list IS
// the page — where the pane's header goes (PageHeader draws it, pageNav).
import type { Snippet } from 'svelte';
import type { IconName } from '../Icon.svelte';
import type { MenuItem } from '../../contextmenu.svelte';

export const LIST_DETAIL = Symbol('otto.listDetail');

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
