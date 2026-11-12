// Exact ordering for 64-bit integer cells in the grid's in-memory sort. The
// daemon sends integers beyond ±(2^53 − 1) as exact digit STRINGS; the sort's
// `Number(v)` key rounds them, so neighbouring ids (9007199254740993 and
// …992) tied and kept load order. This breaks such ties on the digits.

const INT = /^-?\d+$/;

/** Numeric order of two integer digit strings of any length: sign, then
 *  magnitude by length (leading zeros ignored), then lexically. */
export function compareIntStrings(a: string, b: string): number {
  const negA = a.startsWith('-');
  const negB = b.startsWith('-');
  if (negA !== negB) return negA ? -1 : 1;
  const da = (negA ? a.slice(1) : a).replace(/^0+(?=\d)/, '');
  const db = (negB ? b.slice(1) : b).replace(/^0+(?=\d)/, '');
  const mag = da.length !== db.length ? da.length - db.length : da < db ? -1 : da > db ? 1 : 0;
  return Math.sign(negA ? -mag : mag);
}

/** Tie-break for two cells whose numeric sort keys are equal: their exact
 *  integer order when both are exact integers (digit string or bigint), else
 *  0 (a real tie). */
export function exactIntTie(a: unknown, b: unknown): number {
  const sa = typeof a === 'bigint' ? a.toString() : typeof a === 'string' ? a.trim() : null;
  const sb = typeof b === 'bigint' ? b.toString() : typeof b === 'string' ? b.trim() : null;
  if (sa === null || sb === null || !INT.test(sa) || !INT.test(sb)) return 0;
  return compareIntStrings(sa, sb);
}
