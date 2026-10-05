// Keyboard path into node editing. Every editable canvas node used to open its
// editor on double-click only; CanvasFlow now turns Enter / F2 on the one
// selected node into a request here, and the node with that id opens its own
// editor (each node owns its draft state, so the request is just "id, again").
import { untrack } from 'svelte';

let req = $state({ id: '', n: 0 });

/** Ask the node `id` to start editing (Enter / F2 in CanvasFlow). */
export function requestEdit(id: string): void {
  req = { id, n: req.n + 1 };
}

/** Call from a node's component init: runs `start` whenever an edit is
 *  requested for this node's id. Requests made before mount are ignored. */
export function onEditRequest(id: () => string, start: () => void): void {
  let seen = untrack(() => req.n);
  $effect(() => {
    const { id: want, n } = req;
    if (n === seen) return;
    seen = n;
    if (want === untrack(id)) untrack(start);
  });
}

/** `use:dblclickEdit={start}` — the pointer gesture for the same edit. An
 *  action rather than `ondblclick` on a static wrapper: the keyboard path is
 *  Enter / F2 above, and giving the wrapper `role="button"` would nest
 *  interactive content (a node's own buttons, its editor). */
export function dblclickEdit(node: HTMLElement, start: (e: MouseEvent) => void) {
  let fn = start;
  const on = (e: MouseEvent): void => fn(e);
  node.addEventListener('dblclick', on);
  return {
    update(next: (e: MouseEvent) => void) {
      fn = next;
    },
    destroy() {
      node.removeEventListener('dblclick', on);
    },
  };
}
