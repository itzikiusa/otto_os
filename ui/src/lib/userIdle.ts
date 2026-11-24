// How long since the person last touched this document (pointer or key) —
// background work that only matters while someone is here (the database
// keep-alive) stops when they have been away.

let lastInputAt = Date.now();
if (typeof window !== 'undefined') {
  const mark = (): void => {
    lastInputAt = Date.now();
  };
  window.addEventListener('pointerdown', mark, { capture: true, passive: true });
  window.addEventListener('keydown', mark, { capture: true, passive: true });
}

/** Milliseconds since the last pointer/key input. */
export function idleMs(): number {
  return Date.now() - lastInputAt;
}
