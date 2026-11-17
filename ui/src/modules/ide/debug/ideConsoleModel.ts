// The debug console's line model (I2-04, I2-12): a debug adapter writes output
// in chunks (`print("tick", i)` arrives as `tick`, ` `, `0`, `\n`), so rows are
// built by stitching consecutive chunks of one category and splitting on `\n`;
// a trailing partial line stays open for the next chunk. Every chunk carries a
// monotonic sequence number, so Clear and row keys survive the daemon trimming
// the front of its buffer. Evaluations sit at the point in time they were made.
//
// Import-free (type-only imports), so Node runs it directly in unit tests.

/** One adapter output chunk (`IdeDebugOutputEntry`). */
export interface OutputChunk {
  category: string;
  text: string;
  seq?: number;
}

/** One console row. `seq` = the chunk it started in; `endSeq` = the last chunk
 *  that wrote to it. `key` is unique and stable while the start chunk is kept. */
export interface ConsoleLine {
  key: string;
  seq: number;
  endSeq: number;
  text: string;
  isError: boolean;
}

/** Stitch chunks (with their sequence numbers) into lines. A category change
 *  ends the open line: stderr never glues onto a stdout fragment. */
export function stitchOutput(chunks: readonly OutputChunk[], seqs: readonly number[]): ConsoleLine[] {
  const out: ConsoleLine[] = [];
  let open: ConsoleLine | null = null;
  let openCat = '';
  for (let i = 0; i < chunks.length; i++) {
    const c = chunks[i];
    const seq = seqs[i] ?? i + 1;
    if (c.text === '') continue;
    if (open && openCat !== c.category) open = null;
    const parts = c.text.replace(/\r\n/g, '\n').split('\n');
    for (let p = 0; p < parts.length; p++) {
      const piece = parts[p];
      const last = p === parts.length - 1;
      // An empty piece after the final `\n` only means "this line is done".
      if (last && piece === '' && p > 0) { open = null; break; }
      if (open) {
        open.text += piece;
        open.endSeq = seq;
      } else {
        open = { key: `${seq}.${p}`, seq, endSeq: seq, text: piece, isError: c.category === 'stderr' };
        openCat = c.category;
        out.push(open);
      }
      if (!last) open = null;
    }
  }
  return out;
}

/** The sequence number of each chunk. A daemon that sends `seq` is the source
 *  of truth; an older one doesn't, so the client keeps its own counter by
 *  aligning the new window with the previous one (the daemon only appends and
 *  trims from the front). Limitation of the fallback: a window whose content is
 *  periodic AND trimmed by a whole period between two snapshots looks unchanged. */
export class OutputSeqTracker {
  #session = '';
  #prev: readonly OutputChunk[] = [];
  #prevSeqs: number[] = [];

  next(session: string, chunks: readonly OutputChunk[]): number[] {
    let seqs: number[];
    if (chunks.length > 0 && chunks.every((c) => typeof c.seq === 'number')) {
      seqs = chunks.map((c) => c.seq as number);
    } else if (session !== this.#session || this.#prev.length === 0) {
      seqs = chunks.map((_, i) => i + 1);
    } else {
      const d = alignOffset(this.#prev, chunks);
      const base = d < this.#prev.length ? this.#prevSeqs[d] : (this.#prevSeqs[this.#prevSeqs.length - 1] ?? 0) + 1;
      seqs = chunks.map((_, i) => base + i);
    }
    this.#session = session;
    this.#prev = chunks;
    this.#prevSeqs = seqs;
    return seqs;
  }
}

const ALIGN_PROBE = 64;
const same = (a: OutputChunk, b: OutputChunk): boolean => a.text === b.text && a.category === b.category;

/** Smallest `d` such that `next` starts with `prev[d..]` (probing at most
 *  ALIGN_PROBE chunks); `prev.length` when nothing overlaps. */
export function alignOffset(prev: readonly OutputChunk[], next: readonly OutputChunk[]): number {
  for (let d = 0; d < prev.length; d++) {
    const n = Math.min(prev.length - d, next.length, ALIGN_PROBE);
    if (n === 0) break;
    let ok = true;
    for (let i = 0; i < n; i++) {
      if (!same(prev[d + i], next[i])) { ok = false; break; }
    }
    if (ok) return d;
  }
  return prev.length;
}

/** The last output sequence number of a snapshot, when the daemon sends one. */
export function lastOutputSeq(output: readonly OutputChunk[] | null | undefined): number | undefined {
  const last = output && output.length > 0 ? output[output.length - 1] : undefined;
  return typeof last?.seq === 'number' ? last.seq : undefined;
}

/** A console row stream item: an output line or an evaluation. */
export type ConsoleItem<L, R> = { kind: 'line'; line: L } | { kind: 'repl'; entry: R };

/** Put each evaluation after the output that existed when it was made
 *  (`afterSeq`), VS Code's debug console order. An evaluation with no known
 *  position goes last, as before. Both inputs keep their own order. */
export function interleaveRepl<L extends { seq: number }, R extends { afterSeq?: number }>(
  lines: readonly L[],
  repl: readonly R[],
): ConsoleItem<L, R>[] {
  const out: ConsoleItem<L, R>[] = [];
  let r = 0;
  for (const line of lines) {
    while (r < repl.length && typeof repl[r].afterSeq === 'number' && (repl[r].afterSeq as number) < line.seq) {
      out.push({ kind: 'repl', entry: repl[r++] });
    }
    out.push({ kind: 'line', line });
  }
  while (r < repl.length) out.push({ kind: 'repl', entry: repl[r++] });
  return out;
}
