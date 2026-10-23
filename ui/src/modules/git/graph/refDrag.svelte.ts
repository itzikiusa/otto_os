// Drag-to-merge in the refs sidebar (arch-1-09: split out of GraphView.svelte).
// The branch being dragged ({ name, remote }) and the local branch the pointer
// hovers as a drop target. A valid drop is a local branch whose name differs
// from the source's. Nothing merges here: the drop asks the parent (through
// `onmergerequest`) to open its merge approval modal.

export interface RefDragHost {
  readonly onmergerequest: ((source: string, target: string) => void) | undefined;
}

export type RefDrag = ReturnType<typeof createRefDrag>;

export function createRefDrag(h: RefDragHost) {
  let source = $state<{ name: string; remote: boolean } | null>(null);
  let overTarget = $state<string | null>(null);

  /** The source as a merge source: a remote ref like "origin/feature" merges
   *  as its local short name. */
  function sourceName(): string | null {
    if (!source) return null;
    return source.remote ? source.name.replace(/^[^/]+\//, '') : source.name;
  }

  function isValidTarget(localName: string): boolean {
    const src = sourceName();
    return src !== null && src !== localName;
  }

  function start(e: DragEvent, name: string, remote: boolean): void {
    source = { name, remote };
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = 'move';
      // Stash a payload too so the drag is "real" in all browsers/webviews.
      e.dataTransfer.setData('text/plain', name);
    }
  }

  function end(): void {
    source = null;
    overTarget = null;
  }

  function over(e: DragEvent, localName: string): void {
    if (!isValidTarget(localName)) return;
    e.preventDefault(); // allow drop
    if (e.dataTransfer) e.dataTransfer.dropEffect = 'move';
    overTarget = localName;
  }

  function leave(localName: string): void {
    if (overTarget === localName) overTarget = null;
  }

  function drop(e: DragEvent, localName: string): void {
    if (!isValidTarget(localName)) return;
    e.preventDefault();
    const name = sourceName();
    overTarget = null;
    source = null;
    if (name && h.onmergerequest) h.onmergerequest(name, localName);
  }

  return {
    get source() { return source; },
    get overTarget() { return overTarget; },
    sourceName,
    isValidTarget,
    start,
    end,
    over,
    leave,
    drop,
  };
}
