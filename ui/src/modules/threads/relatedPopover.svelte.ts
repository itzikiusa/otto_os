// Which thread's "Send to another thread" popover is open. ⋯ › Send to
// another thread… (page or pane) asks ThreadsPage to open it under the
// toolbar's ⋯: message a thread, open one beside, start a room.
class RelatedPopover {
  openFor = $state<string | null>(null);

  open(id: string): void {
    this.openFor = id;
  }

  close(): void {
    this.openFor = null;
  }
}

export const relatedPopover = new RelatedPopover();
