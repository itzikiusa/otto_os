// perf K8s R5: the log viewer's capped line buffer, appended IN PLACE.
// `lines.concat(add)` per animation frame copied the whole 20k-line buffer
// (plus a second copy to trim it) ~60×/s while a busy workload streamed; an
// in-place push + front splice touches only the new lines and the trimmed
// head. The caller bumps a version counter so the view re-renders.

/** Append `add` to `buf` in place and trim the head so at most `cap` lines
 *  remain. Returns the lines trimmed off the head (oldest first) — the
 *  caller needs them to drop the same lines from a filtered view. */
export function appendCapped(buf: string[], add: readonly string[], cap: number): string[] {
  // A loop, not push(...add): spreading a huge chunk overflows the call stack.
  for (const l of add) buf.push(l);
  const overflow = buf.length - Math.max(0, cap);
  return overflow > 0 ? buf.splice(0, overflow) : [];
}

/** Keep a filtered view in step with {@link appendCapped}: append the new
 *  matches, then drop as many leading matches as the ring trimmed. */
export function appendFiltered(
  view: string[],
  add: readonly string[],
  trimmed: readonly string[],
  keep: (l: string) => boolean,
): void {
  for (const l of add) if (keep(l)) view.push(l);
  let drop = 0;
  for (const l of trimmed) if (keep(l)) drop++;
  if (drop) view.splice(0, drop);
}
