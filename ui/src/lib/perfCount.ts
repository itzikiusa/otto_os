// Recomputation counters for the perf specs (e2e/desktop-threads-runtime-perf):
// a heavy derivation calls `perfCount('name')`; it counts only while a spec
// has set `window.__ottoPerfCounts = {}`, so outside a measurement it is one
// property read. Import-free (pure modules use it under Node unit tests).

export function perfCount(name: string): void {
  const m = (globalThis as { __ottoPerfCounts?: Record<string, number> }).__ottoPerfCounts;
  if (m) m[name] = (m[name] ?? 0) + 1;
}
