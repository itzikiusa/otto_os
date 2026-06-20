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
    const n = a.length;
    const m = b.length;
    const out: Array<{ tag: Tag; i: number; j: number }> = [];
    // Size cap: beyond this the O(n·m) table is too big — degrade to del-all/add-all.
    if (n * m > 4_000_000) {
      for (let i = 0; i < n; i++) out.push({ tag: 'del', i, j: -1 });
      for (let j = 0; j < m; j++) out.push({ tag: 'add', i: -1, j });
      return out;
    }
    const w = m + 1;
    const dp = new Int32Array((n + 1) * (m + 1));
    const ka = a.map(key);
    const kb = b.map(key);
    for (let i = n - 1; i >= 0; i--) {
      for (let j = m - 1; j >= 0; j--) {
        dp[i * w + j] =
          ka[i] === kb[j]
            ? dp[(i + 1) * w + (j + 1)] + 1
            : Math.max(dp[(i + 1) * w + j], dp[i * w + (j + 1)]);
      }
    }
    let i = 0;
    let j = 0;
    while (i < n && j < m) {
      if (ka[i] === kb[j]) {
        out.push({ tag: 'eq', i, j });
        i++;
        j++;
      } else if (dp[(i + 1) * w + j] >= dp[i * w + (j + 1)]) {
        out.push({ tag: 'del', i, j: -1 });
        i++;
      } else {
        out.push({ tag: 'add', i: -1, j });
        j++;
      }
    }
    while (i < n) {
      out.push({ tag: 'del', i, j: -1 });
      i++;
    }
    while (j < m) {
      out.push({ tag: 'add', i: -1, j });
      j++;
    }
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
      for (let p = 0; p < dels.length; p++) {
        if (p < pairCount) {
          const { left } = diffWords(dels[p], adds[p]);
          out.push({ kind: 'del', segs: left, aNo: aNo++ });
        } else {
          out.push({ kind: 'del', segs: [{ t: 'del', s: dels[p] }], aNo: aNo++ });
        }
      }
      for (let p = 0; p < adds.length; p++) {
        if (p < pairCount) {
          const { right } = diffWords(dels[p], adds[p]);
          out.push({ kind: 'add', segs: right, bNo: bNo++ });
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
</script>

<div class="dv" class:split={mode === 'split'}>
  {#if mode === 'split'}
    {#each splitRows as row, ri (ri)}
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
    {#each rows as row, ri (ri)}
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
    font-family: var(--mono, ui-monospace, SFMono-Regular, Menlo, monospace);
    font-size: 12px;
    line-height: 1.45;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    background: var(--surface-1, var(--surface-2));
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
    text-align: right;
    padding-right: 8px;
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
    color: var(--status-exited);
  }
  .dv-row.add .dv-sign {
    color: var(--status-working);
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
    border-right: 1px solid var(--border);
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
