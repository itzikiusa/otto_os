// What a ListDetail tells the ListPane inside it (and a nested ListDetail
// inside its content): the pane's name, whether it can be hidden here, and
// whether the content side is on screen.
export const LIST_DETAIL = Symbol('otto.listDetail');

export interface ListDetailCtx {
  readonly name: string;
  /** Desktop / tablet with items: the pane can be hidden (⋯ › Hide <name> ⌘B). */
  readonly canHide: boolean;
  hide(): void;
  /** The content side is on screen (a phone showing the list: false). */
  contentVisible(): boolean;
}
