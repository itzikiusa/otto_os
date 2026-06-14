// Find-in-page store — opened by Cmd+F when no terminal is focused.

class FindInPageStore {
  open = $state(false);

  /** Open the find bar and optionally pre-fill a query. */
  show(): void {
    this.open = true;
  }

  hide(): void {
    this.open = false;
  }
}

export const findInPage = new FindInPageStore();
