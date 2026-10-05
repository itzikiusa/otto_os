<script lang="ts">
  // Shared client-side diff renderer (T3). Unlike the git DiffViewer — which
  // renders server-computed hunks — this computes a line/word LCS diff between
  // two raw strings. Consumers: Jira RewriteTab + version compare, self-improve
  // approval cards, skill-eval promote previews. Pure presentational, no fetch.
  //
  //   <DiffView before={a} after={b} mode="word" />
  //
  // mode: 'line' (unified, line granularity) | 'word' (unified, intra-line word
  // highlights on replaced lines) | 'split' (two columns). ignoreWhitespace
  // affects equality only (rendering always shows the original text).

  interface Props {
    before: string;
    after: string;
    mode?: 'line' | 'word' | 'split';
    language?: string;
    ignoreWhitespace?: boolean;
    /** Collapse equal runs longer than 2×contextLines into a gap marker. */
    contextLines?: number;
  }
  // `language` is accepted for forward-compat (syntax tinting) but not yet used;
  // intentionally left out of the destructure so it isn't flagged as unused.
  let { before, after, mode = 'line', ignoreWhitespace = false, contextLines }: Props =
    $props();
  const PAGE_ROWS = 500;
  let pageIndex = $state(0);
  $effect(() => { before; after; mode; ignoreWhitespace; contextLines; pageIndex = 0; });

  function changePage(delta: number): void {
    pageIndex = Math.max(0, Math.min(pageCount - 1, pageIndex + delta));
  }

  function downloadSource(side: 'before' | 'after'): void {
    const blob = new Blob([side === 'before' ? before : after], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = `${side}.txt`;
    anchor.click();
    setTimeout(() => URL.revokeObjectURL(url), 0);
  }

  type Tag = 'eq' | 'del' | 'add';
  interface Op {
    tag: Tag;
    line: string;
  }
  interface Seg {
    t: Tag;
    s: string;
  }

  // Normalize a line for *comparison* only (rendering uses the original text).
  function norm(s: string): string {
    return ignoreWhitespace ? s.replace(/\s+/g, ' ').trim() : s;
  }

  // Generic LCS over an array of comparable keys → backtracked op stream.
  function lcsOps<T>(a: T[], b: T[], key: (x: T) => string): Array<{ tag: Tag; i: number; j: number }> {
    const out: Array<{ tag: Tag; i: number; j: number }> = [];
    const ka = a.map(key);
    const kb = b.map(key);
    let cellsLeft = 4_000_000;
    let comparisonsLeft = 4_000_000;
    // Small edit distance, including repeated lines with no unique anchors.
    // Frontier snapshots have a fixed distance ceiling and share the total
    // allocation budget; snakes also spend an explicit comparison budget.
    function repeatedRun(a0: number, a1: number, b0: number, b1: number): typeof out | null {
      const n = a1 - a0, m = b1 - b0;
      const distance = Math.min(n + m, 256), offset = distance + 1, width = 2 * distance + 3;
      if (Math.abs(n - m) > distance || cellsLeft < width) return null;
      cellsLeft -= width;
      const frontier = new Int32Array(width).fill(-1);
      frontier[offset + 1] = 0;
      const trace: Int32Array[] = [];
      for (let d = 0; d <= distance; d++) {
        if (cellsLeft < width) return null;
        cellsLeft -= width;
        trace.push(frontier.slice());
        for (let k = -d; k <= d; k += 2) {
          if (--comparisonsLeft < 0) return null;
          const at = offset + k;
          let x = k === -d || (k !== d && frontier[at - 1] < frontier[at + 1]) ? frontier[at + 1] : frontier[at - 1] + 1;
          let y = x - k;
          while (x < n && y < m) {
            if (--comparisonsLeft < 0) return null;
            if (ka[a0 + x] !== kb[b0 + y]) break;
            x++; y++;
          }
          frontier[at] = x;
          if (x < n || y < m) continue;
          const result: typeof out = [];
          for (let depth = d; depth >= 0; depth--) {
            const previous = trace[depth], diagonal = x - y;
            const previousDiagonal = diagonal === -depth || (diagonal !== depth && previous[offset + diagonal - 1] < previous[offset + diagonal + 1]) ? diagonal + 1 : diagonal - 1;
            const previousX = previous[offset + previousDiagonal], previousY = previousX - previousDiagonal;
            while (x > previousX && y > previousY) result.push({ tag: 'eq', i: a0 + --x, j: b0 + --y });
            if (depth === 0) break;
            if (x === previousX) result.push({ tag: 'add', i: -1, j: b0 + --y });
            else result.push({ tag: 'del', i: a0 + --x, j: -1 });
          }
          return result.reverse();
        }
      }
      return null;
    }
    // Prefix/suffix cost is linear. For a large middle, unique matching lines
    // supply ordered anchors; no recursive quadratic tables or deep recursion.
    function region(a0: number, a1: number, b0: number, b1: number, anchorsAllowed: boolean): void {
      while (a0 < a1 && b0 < b1 && ka[a0] === kb[b0]) out.push({ tag: 'eq', i: a0++, j: b0++ });
      let suffix = 0;
      while (a0 < a1 && b0 < b1 && ka[a1 - 1] === kb[b1 - 1]) { a1--; b1--; suffix++; }
      const n = a1 - a0, m = b1 - b0;
      const cells = (n + 1) * (m + 1);
      if (n && m && cells <= cellsLeft) {
        cellsLeft -= cells;
        const w = m + 1, dp = new Int32Array(cells);
        for (let i = n - 1; i >= 0; i--) for (let j = m - 1; j >= 0; j--) {
          dp[i * w + j] = ka[a0 + i] === kb[b0 + j]
            ? dp[(i + 1) * w + j + 1] + 1
            : Math.max(dp[(i + 1) * w + j], dp[i * w + j + 1]);
        }
        let i = 0, j = 0;
        while (i < n && j < m) {
          if (ka[a0 + i] === kb[b0 + j]) out.push({ tag: 'eq', i: a0 + i++, j: b0 + j++ });
          else if (dp[(i + 1) * w + j] >= dp[i * w + j + 1]) out.push({ tag: 'del', i: a0 + i++, j: -1 });
          else out.push({ tag: 'add', i: -1, j: b0 + j++ });
        }
        while (i < n) out.push({ tag: 'del', i: a0 + i++, j: -1 });
        while (j < m) out.push({ tag: 'add', i: -1, j: b0 + j++ });
      } else if (n && m && anchorsAllowed) {
        const left = new Map<string, number>(), right = new Map<string, number>();
        for (let i = a0; i < a1; i++) left.set(ka[i], left.has(ka[i]) ? -1 : i);
        for (let j = b0; j < b1; j++) right.set(kb[j], right.has(kb[j]) ? -1 : j);
        const candidates: { i: number; j: number; prev: number }[] = [];
        const tails: number[] = [];
        for (const [text, i] of left) {
          const j = right.get(text);
          if (i < 0 || j === undefined || j < 0) continue;
          let lo = 0, hi = tails.length;
          while (lo < hi) { const mid = (lo + hi) >>> 1; if (candidates[tails[mid]].j < j) lo = mid + 1; else hi = mid; }
          candidates.push({ i, j, prev: lo ? tails[lo - 1] : -1 });
          tails[lo] = candidates.length - 1;
        }
        const anchors: { i: number; j: number }[] = [];
        for (let at = tails.at(-1) ?? -1; at >= 0; at = candidates[at].prev) anchors.push(candidates[at]);
        anchors.reverse();
        let i = a0, j = b0;
        for (const anchor of anchors) {
          region(i, anchor.i, j, anchor.j, false);
          out.push({ tag: 'eq', i: anchor.i, j: anchor.j });
          i = anchor.i + 1; j = anchor.j + 1;
        }
        region(i, a1, j, b1, false);
      } else {
        const common = n && m ? repeatedRun(a0, a1, b0, b1) : null;
        if (common) for (const op of common) out.push(op);
        else {
          for (let i = a0; i < a1; i++) out.push({ tag: 'del', i, j: -1 });
          for (let j = b0; j < b1; j++) out.push({ tag: 'add', i: -1, j });
        }
      }
      for (let k = 0; k < suffix; k++) out.push({ tag: 'eq', i: a1 + k, j: b1 + k });
    }
    region(0, a.length, 0, b.length, true);
    return out;
  }

  function diffLines(aText: string, bText: string): Op[] {
    const a = aText.length ? aText.split('\n') : [];
    const b = bText.length ? bText.split('\n') : [];
    return lcsOps(a, b, norm).map((o) => ({
      tag: o.tag,
      line: o.tag === 'add' ? b[o.j] : a[o.i],
    }));
  }

  // Intra-line word diff: tokenize into whitespace/word runs, LCS on tokens.
  function diffWords(a: string, b: string): { left: Seg[]; right: Seg[] } {
    if (a.length + b.length > 16_000) return { left: [{ t: 'del', s: a }], right: [{ t: 'add', s: b }] };
    const at = a.match(/\s+|\S+/g) ?? [];
    const bt = b.match(/\s+|\S+/g) ?? [];
    const ops = lcsOps(at, bt, (x) => x);
    const left: Seg[] = [];
    const right: Seg[] = [];
    for (const o of ops) {
      if (o.tag === 'eq') {
        left.push({ t: 'eq', s: at[o.i] });
        right.push({ t: 'eq', s: bt[o.j] });
      } else if (o.tag === 'del') {
        left.push({ t: 'del', s: at[o.i] });
      } else {
        right.push({ t: 'add', s: bt[o.j] });
      }
    }
    return { left, right };
  }

  type RenderRow =
    | { kind: 'eq'; text: string; aNo: number; bNo: number }
    | { kind: 'del'; segs: Seg[]; aNo: number }
    | { kind: 'add'; segs: Seg[]; bNo: number }
    | { kind: 'gap'; count: number };

  // Walk ops → render rows. Pairs consecutive del/add runs so word mode can
  // highlight replaced lines; collapses long equal runs when contextLines set.
  const rows = $derived.by((): RenderRow[] => {
    const ops = diffLines(before, after);
    const out: RenderRow[] = [];
    let aNo = 1;
    let bNo = 1;
    let k = 0;
    let wordCellsLeft = 4_000_000;
    while (k < ops.length) {
      const op = ops[k];
      if (op.tag === 'eq') {
        // Gather the equal run for optional collapsing.
        const run: Op[] = [];
        const startA = aNo;
        const startB = bNo;
        while (k < ops.length && ops[k].tag === 'eq') {
          run.push(ops[k]);
          k++;
        }
        const ctx = contextLines ?? -1;
        if (ctx >= 0 && run.length > ctx * 2 + 1) {
          // Keep ctx lines of leading/trailing context, collapse the middle —
          // but never collapse the very top/bottom of the file edges away.
          const head = out.length === 0 ? 0 : ctx;
          const tail = k >= ops.length ? 0 : ctx;
          for (let r = 0; r < head; r++) {
            out.push({ kind: 'eq', text: run[r].line, aNo: startA + r, bNo: startB + r });
          }
          out.push({ kind: 'gap', count: run.length - head - tail });
          for (let r = run.length - tail; r < run.length; r++) {
            out.push({ kind: 'eq', text: run[r].line, aNo: startA + r, bNo: startB + r });
          }
        } else {
          for (let r = 0; r < run.length; r++) {
            out.push({ kind: 'eq', text: run[r].line, aNo: startA + r, bNo: startB + r });
          }
        }
        aNo = startA + run.length;
        bNo = startB + run.length;
        continue;
      }
      // Collect a del-run then an add-run (a replacement block).
      const dels: string[] = [];
      const adds: string[] = [];
      while (k < ops.length && ops[k].tag === 'del') {
        dels.push(ops[k].line);
        k++;
      }
      while (k < ops.length && ops[k].tag === 'add') {
        adds.push(ops[k].line);
        k++;
      }
      const pairCount = mode === 'word' ? Math.min(dels.length, adds.length) : 0;
      // One word diff per replaced pair, shared by both sides (it used to run
      // twice — once for the left segments, once for the right).
      const pairs = Array.from({ length: pairCount }, (_, p) => {
        const cells = ((dels[p].match(/\s+|\S+/g)?.length ?? 0) + 1) * ((adds[p].match(/\s+|\S+/g)?.length ?? 0) + 1);
        if (cells > wordCellsLeft) return { left: [{ t: 'del' as const, s: dels[p] }], right: [{ t: 'add' as const, s: adds[p] }] };
        wordCellsLeft -= cells;
        return diffWords(dels[p], adds[p]);
      });
      for (let p = 0; p < dels.length; p++) {
        if (p < pairCount) {
          out.push({ kind: 'del', segs: pairs[p].left, aNo: aNo++ });
        } else {
          out.push({ kind: 'del', segs: [{ t: 'del', s: dels[p] }], aNo: aNo++ });
        }
      }
      for (let p = 0; p < adds.length; p++) {
        if (p < pairCount) {
          out.push({ kind: 'add', segs: pairs[p].right, bNo: bNo++ });
        } else {
          out.push({ kind: 'add', segs: [{ t: 'add', s: adds[p] }], bNo: bNo++ });
        }
      }
    }
    return out;
  });

  // Split view pairs del/add rows side-by-side; eq rows mirror on both sides.
  interface SplitRow {
    left: { text: string; segs?: Seg[]; no: number } | null;
    right: { text: string; segs?: Seg[]; no: number } | null;
    gap?: number;
  }
  const splitRows = $derived.by((): SplitRow[] => {
    const out: SplitRow[] = [];
    const rs = rows;
    let n = 0;
    while (n < rs.length) {
      const r = rs[n];
      if (r.kind === 'gap') {
        out.push({ left: null, right: null, gap: r.count });
        n++;
      } else if (r.kind === 'eq') {
        out.push({
          left: { text: r.text, no: r.aNo },
          right: { text: r.text, no: r.bNo },
        });
        n++;
      } else {
        // Pair a run of dels with the following run of adds positionally.
        const dels: RenderRow[] = [];
        const adds: RenderRow[] = [];
        while (n < rs.length && rs[n].kind === 'del') dels.push(rs[n++]);
        while (n < rs.length && rs[n].kind === 'add') adds.push(rs[n++]);
        const len = Math.max(dels.length, adds.length);
        for (let p = 0; p < len; p++) {
          const d = dels[p] as Extract<RenderRow, { kind: 'del' }> | undefined;
          const ad = adds[p] as Extract<RenderRow, { kind: 'add' }> | undefined;
          out.push({
            left: d ? { text: '', segs: d.segs, no: d.aNo } : null,
            right: ad ? { text: '', segs: ad.segs, no: ad.bNo } : null,
          });
        }
      }
    }
    return out;
  });
  const pageCount = $derived(Math.max(1, Math.ceil((mode === 'split' ? splitRows.length : rows.length) / PAGE_ROWS)));
  const visibleRows = $derived(rows.slice(pageIndex * PAGE_ROWS, (pageIndex + 1) * PAGE_ROWS));
  const visibleSplitRows = $derived(splitRows.slice(pageIndex * PAGE_ROWS, (pageIndex + 1) * PAGE_ROWS));
</script>

<div class="dv" class:split={mode === 'split'}>
  {#if pageCount > 1}
    <div class="dv-gap" data-find-skip>
      <button class="btn small" onclick={() => changePage(-1)} disabled={pageIndex === 0}>Previous changes</button>
      <span>Page {pageIndex + 1} of {pageCount}</span>
      <button class="btn small" onclick={() => changePage(1)} disabled={pageIndex + 1 >= pageCount}>Next changes</button>
    </div>
  {/if}
  <div class="dv-gap" data-find-skip>
    <button class="btn small" onclick={() => downloadSource('before')}>Download before</button>
    <button class="btn small" onclick={() => downloadSource('after')}>Download after</button>
  </div>
  {#if mode === 'split'}
    {#each visibleSplitRows as row, ri (ri)}
      {#if row.gap !== undefined}
        <div class="dv-gap">⋯ {row.gap} unchanged line{row.gap === 1 ? '' : 's'}</div>
      {:else}
        <div class="dv-srow">
          <div class="dv-col" class:del={!!row.left?.segs} class:empty={!row.left}>
            <span class="dv-no">{row.left ? row.left.no : ''}</span>
            <span class="dv-txt">
              {#if row.left?.segs}{#each row.left.segs as s, si (si)}<span class:wdel={s.t === 'del'}>{s.s}</span>{/each}{:else}{row.left?.text ?? ''}{/if}
            </span>
          </div>
          <div class="dv-col" class:add={!!row.right?.segs} class:empty={!row.right}>
            <span class="dv-no">{row.right ? row.right.no : ''}</span>
            <span class="dv-txt">
              {#if row.right?.segs}{#each row.right.segs as s, si (si)}<span class:wadd={s.t === 'add'}>{s.s}</span>{/each}{:else}{row.right?.text ?? ''}{/if}
            </span>
          </div>
        </div>
      {/if}
    {/each}
  {:else}
    {#each visibleRows as row, ri (ri)}
      {#if row.kind === 'gap'}
        <div class="dv-gap">⋯ {row.count} unchanged line{row.count === 1 ? '' : 's'}</div>
      {:else if row.kind === 'eq'}
        <div class="dv-row eq"><span class="dv-no">{row.aNo}</span><span class="dv-sign"> </span><span class="dv-txt">{row.text}</span></div>
      {:else if row.kind === 'del'}
        <div class="dv-row del"><span class="dv-no">{row.aNo}</span><span class="dv-sign">-</span><span class="dv-txt">{#each row.segs as s, si (si)}<span class:wdel={s.t === 'del'}>{s.s}</span>{/each}</span></div>
      {:else}
        <div class="dv-row add"><span class="dv-no">{row.bNo}</span><span class="dv-sign">+</span><span class="dv-txt">{#each row.segs as s, si (si)}<span class:wadd={s.t === 'add'}>{s.s}</span>{/each}</span></div>
      {/if}
    {/each}
  {/if}
  {#if rows.length === 0}
    <div class="dv-empty">No differences</div>
  {/if}
</div>

<style>
  .dv {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    line-height: 1.45;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
  }
  .dv-row {
    display: flex;
    white-space: pre-wrap;
    word-break: break-word;
    padding: 0 6px;
  }
  .dv-row.del {
    background: color-mix(in srgb, var(--status-exited) 12%, transparent);
  }
  .dv-row.add {
    background: color-mix(in srgb, var(--status-working) 12%, transparent);
  }
  .dv-no {
    flex: 0 0 auto;
    width: 3.2em;
    text-align: end;
    padding-inline-end: 8px;
    color: var(--text-dim);
    user-select: none;
  }
  .dv-sign {
    flex: 0 0 auto;
    width: 1em;
    user-select: none;
    color: var(--text-dim);
  }
  .dv-row.del .dv-sign {
    color: var(--danger);
  }
  .dv-row.add .dv-sign {
    color: var(--success);
  }
  .dv-txt {
    flex: 1 1 auto;
  }
  .wdel {
    background: color-mix(in srgb, var(--status-exited) 38%, transparent);
    border-radius: 2px;
  }
  .wadd {
    background: color-mix(in srgb, var(--status-working) 38%, transparent);
    border-radius: 2px;
  }
  .dv-gap {
    padding: 2px 10px;
    color: var(--text-dim);
    background: var(--surface-2);
    border-top: 1px solid var(--border);
    border-bottom: 1px solid var(--border);
    user-select: none;
  }
  .dv-empty {
    padding: 10px;
    color: var(--text-dim);
  }
  /* Split mode */
  .dv.split .dv-srow {
    display: grid;
    grid-template-columns: 1fr 1fr;
  }
  .dv-col {
    display: flex;
    white-space: pre-wrap;
    word-break: break-word;
    padding: 0 6px;
    border-inline-end: 1px solid var(--border);
  }
  .dv-col.del {
    background: color-mix(in srgb, var(--status-exited) 12%, transparent);
  }
  .dv-col.add {
    background: color-mix(in srgb, var(--status-working) 12%, transparent);
  }
  .dv-col.empty {
    background: var(--surface-2);
  }
</style>
