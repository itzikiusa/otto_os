// Which thread's "Related threads" popover is open. The header chip owns the
// popover; a ⋯ menu ("Related threads…", page or pane) asks it to open here,
// so a thread with nothing related yet can still message one or start a room.
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
