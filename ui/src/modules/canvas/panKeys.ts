// Keyboard panning for the zoomable diagram surfaces (Mermaid / D2). After a
// zoom the diagram is bigger than its viewport and a pointer drag was the only
// way to move it; with the surface focused, the arrow keys pan it instead
// (⇧ = a big step). Only keys pressed ON the surface count — a key from the
// code editor beside it (or any control inside) is left alone.

export const PAN_STEP = 40;
export const PAN_STEP_BIG = 160;

/** The (dx, dy) the VIEW should move for a pan key, or null when it isn't one.
 *  → shows more to the right, so callers subtract it from their translation. */
export function panDelta(e: KeyboardEvent): { dx: number; dy: number } | null {
  if (e.target !== e.currentTarget) return null;
  if (e.metaKey || e.ctrlKey || e.altKey) return null;
  const step = e.shiftKey ? PAN_STEP_BIG : PAN_STEP;
  switch (e.key) {
    case 'ArrowLeft':
      return { dx: -step, dy: 0 };
    case 'ArrowRight':
      return { dx: step, dy: 0 };
    case 'ArrowUp':
      return { dx: 0, dy: -step };
    case 'ArrowDown':
      return { dx: 0, dy: step };
    default:
      return null;
  }
}

export const PAN_LABEL = 'Diagram — drag to pan, scroll to zoom. With this focused, arrow keys pan (Shift for larger steps).';
