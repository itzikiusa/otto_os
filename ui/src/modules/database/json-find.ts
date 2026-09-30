// ⌘F over the JSON / Vertical result views. Both trees are lazy (a CLOSED
// container is one summary line and mounts nothing under it, open arrays draw
// CHUNK slices, long strings are clipped), so FindInPage's DOM walk can't see
// what a collapsed branch holds — "does game X exist in lobby.structure[*]"
// found nothing. The views register a FindProvider (lib/findProviders.ts)
// whose rows are the LINES the tree would draw with everything open — one per
// field / element, in render order — built from the records themselves:
//
//   - a container line is its key; a leaf line is `key\nvalue` in the view's
//     own leaf form (JSON: quoted strings + `null`; Vertical: raw text + `∅`);
//   - array-index labels (`0`, `12`, …) are NOT searchable — a numeric query
//     would otherwise hit every element label. The trees mark them (and the
//     summaries / chevrons / "more" buttons the model leaves out) with
//     `data-find-skip`, so the n-th occurrence in a line maps onto the DOM;
//   - lines are built one record at a time and only when a search reads them
//     (a tiny LRU), so a 500 × 88KB result costs a record's lines, not all.
//
// Revealing a line opens its ancestors in THAT record only (RevealState — not
// the sticky cross-record overrides, which would unfold the branch in every
// drawn record), grows the "show more" slices it sits past and unclips its
// string. Nothing here is Svelte: the views own the reactive RevealState.
import { bsonScalar } from './bson';

/** Inline string cap shared by JsonTree / VerticalTree (full text one click away). */
export const STR_MAX = 200;

export type FindFlavor = 'json' | 'vertical';
/** A record's top-level rows: `[path, label, value]` (Vertical: one per column). */
export type FindTop = [path: string, label: string, value: unknown];

export interface FindLine {
  /** Index into the record's tops. */
  top: number;
  /** Dotted path the tree renders the line at (its `data-jpath`). */
  path: string;
  /** What the line shows, as the find model searches it. */
  text: string;
  /** A string leaf past STR_MAX — mounted clipped until revealed. */
  long: boolean;
}

/** Per-record disclosure a find reveal adds on top of the expansion plan. The
 *  views hand in SvelteSet / SvelteMap instances so the trees react. */
export interface RevealState {
  /** Container paths forced open. */
  opens: Set<string>;
  /** Container path → children to draw at least (grows the CHUNK slices). */
  shown: Map<string, number>;
  /** String leaf paths drawn unclipped. */
  strs: Set<string>;
}

/** Array indices (and numeric keys) — shown, but not searchable. */
export function searchableLabel(label: string): boolean {
  return !/^\d+$/.test(label);
}

function isContainer(v: unknown): v is Record<string, unknown> | unknown[] {
  return v !== null && typeof v === 'object' && bsonScalar(v) === null;
}

/** A leaf as the view draws it (an empty container is a leaf too). */
function leafText(v: unknown, flavor: FindFlavor): string {
  const b = bsonScalar(v);
  if (b !== null) return b;
  if (v === null || v === undefined) return flavor === 'json' ? 'null' : '∅';
  if (typeof v === 'string') return flavor === 'json' ? `"${v}"` : v;
  if (typeof v === 'object') return Array.isArray(v) ? '[]' : '{}';
  return String(v);
}

/** Every line of one record, in render (pre-)order. */
export function recordLines(tops: FindTop[], flavor: FindFlavor): FindLine[] {
  const out: FindLine[] = [];
  const walk = (top: number, path: string, label: string, v: unknown): void => {
    const keyed = searchableLabel(label);
    if (isContainer(v)) {
      const kids: [string, unknown][] = Array.isArray(v) ? v.map((x, i) => [String(i), x]) : Object.entries(v);
      if (kids.length > 0) {
        out.push({ top, path, text: keyed ? label : '', long: false });
        for (const [k, c] of kids) walk(top, `${path}.${k}`, k, c);
        return;
      }
    }
    const val = leafText(v, flavor);
    out.push({ top, path, text: keyed ? `${label}\n${val}` : val, long: typeof v === 'string' && v.length > STR_MAX });
  };
  tops.forEach(([path, label, v], t) => walk(t, path, label, v));
  return out;
}

/** Line count of one record without building it (the provider's `count`). */
export function countLines(tops: FindTop[]): number {
  const count = (v: unknown): number => {
    if (!isContainer(v)) return 1;
    let n = 1;
    if (Array.isArray(v)) for (const x of v) n += count(x);
    else for (const k in v) n += count(v[k]);
    return n;
  };
  let n = 0;
  for (const [, , v] of tops) n += count(v);
  return n;
}

/** Line model over a record list: prefix sums for `count`, per-record lines
 *  built on demand (FindInPage reads `text(i)` in order, so a scan builds each
 *  record once and keeps only the last few). Build a new one when the records
 *  change — it caches by record index. */
export class LineIndex {
  private starts: number[] | null = null;
  private total = 0;
  private cache = new Map<number, FindLine[]>();
  private static readonly KEEP = 4;

  constructor(
    readonly records: number,
    private readonly topsOf: (rec: number) => FindTop[],
    private readonly flavor: FindFlavor,
  ) {}

  count(): number {
    this.build();
    return this.total;
  }

  /** The record + line behind flat row `i`, or null when out of range. */
  locate(i: number): { rec: number; line: FindLine } | null {
    const starts = this.build();
    if (i < 0 || i >= this.total) return null;
    // Last record whose start is ≤ i (an empty record shares its successor's
    // start, so the upper bound lands on the one that owns the line).
    let lo = 0;
    let hi = starts.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (starts[mid] <= i) lo = mid + 1;
      else hi = mid;
    }
    const rec = lo - 1;
    const line = this.lines(rec)[i - starts[rec]];
    return line ? { rec, line } : null;
  }

  lines(rec: number): FindLine[] {
    let ls = this.cache.get(rec);
    if (ls) {
      this.cache.delete(rec); // refresh its LRU position
    } else {
      ls = recordLines(this.topsOf(rec), this.flavor);
      if (this.cache.size >= LineIndex.KEEP) this.cache.delete(this.cache.keys().next().value!);
    }
    this.cache.set(rec, ls);
    return ls;
  }

  private build(): number[] {
    if (this.starts) return this.starts;
    const starts = new Array<number>(this.records);
    let n = 0;
    for (let r = 0; r < this.records; r++) {
      starts[r] = n;
      n += countLines(this.topsOf(r));
    }
    this.total = n;
    this.starts = starts;
    return starts;
  }
}

/** What revealing `line` needs: the container paths to open (outermost
 *  first) and the "show more" slice each must reach so the next hop is drawn.
 *  `rootChunk`: the tops are the children of a chunked root at path `''`
 *  (JsonView's record tree); Vertical draws every column, unchunked. */
export function revealSteps(
  tops: FindTop[],
  line: FindLine,
  chunk: number,
  rootChunk: boolean,
): { opens: string[]; shown: [string, number][] } {
  const opens: string[] = [];
  const shown: [string, number][] = [];
  const need = (pos: number): number => Math.ceil((pos + 1) / chunk) * chunk;
  if (rootChunk) {
    opens.push('');
    if (line.top >= chunk) shown.push(['', need(line.top)]);
  }
  const [topPath, , topVal] = tops[line.top] ?? ['', '', undefined];
  const rest = line.path === topPath ? [] : line.path.slice(topPath.length + 1).split('.');
  let path = topPath;
  let v: unknown = topVal;
  for (const seg of rest) {
    if (!isContainer(v)) break;
    opens.push(path);
    const pos = Array.isArray(v) ? Number(seg) : Object.keys(v).indexOf(seg);
    if (pos >= chunk) shown.push([path, need(pos)]);
    v = (v as Record<string, unknown>)[seg];
    path = `${path}.${seg}`;
  }
  return { opens, shown };
}
