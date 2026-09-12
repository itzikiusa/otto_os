<script module lang="ts">
  // Side-by-side diff of two records (Vertical / JSON views' "Compare"): both
  // are flattened to dotted leaf paths (`flattenPaths` — arrays by index, BSON
  // sentinels as scalars) and lined up path by path, so a changed `items.2.qty`
  // is one row rather than two walls of JSON. "Only differences" hides the
  // unchanged rows; "Copy as JSON patch" copies the ops that turn LEFT into
  // RIGHT (`set` / `unset` per leaf path) for a hand-written update. The two
  // pure helpers live in the module script so they are importable (and
  // testable) without mounting the modal.
  import { flattenPaths } from './expansion-plan';
  import { cellStr } from './results-format';

  export type DiffKind = 'same' | 'changed' | 'only-left' | 'only-right';
  export interface DiffRow {
    path: string;
    kind: DiffKind;
    l: unknown;
    r: unknown;
  }
  export type PatchOp = { op: 'set'; path: string; value: unknown } | { op: 'unset'; path: string };

  /** Leaf equality: same rendered text AND same JS type, so `1` vs `"1"` and
   *  `null` vs `"null"` count as changes. */
  function sameLeaf(a: unknown, b: unknown): boolean {
    return typeof a === typeof b && cellStr(a) === cellStr(b);
  }

  /** One row per leaf path of either record — left's order first, then the
   *  paths only the right one has. */
  export function diffRecords(left: unknown, right: unknown): DiffRow[] {
    const L = flattenPaths(left);
    const R = flattenPaths(right);
    const out: DiffRow[] = [];
    for (const [path, l] of L) {
      if (!R.has(path)) out.push({ path, kind: 'only-left', l, r: undefined });
      else out.push({ path, kind: sameLeaf(l, R.get(path)) ? 'same' : 'changed', l, r: R.get(path) });
    }
    for (const [path, r] of R) if (!L.has(path)) out.push({ path, kind: 'only-right', l: undefined, r });
    return out;
  }

  /** `[{op:'set',path,value} | {op:'unset',path}]` turning LEFT into RIGHT. */
  export function recordPatch(rows: DiffRow[]): PatchOp[] {
    return rows
      .filter((r) => r.kind !== 'same')
      .map((r) => (r.kind === 'only-left' ? { op: 'unset', path: r.path } : { op: 'set', path: r.path, value: r.r }));
  }
</script>

<script lang="ts">
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { copyText } from './results-format';

  interface Props {
    left: Record<string, unknown>;
    right: Record<string, unknown>;
    leftLabel: string;
    rightLabel: string;
    onclose: () => void;
  }
  let { left, right, leftLabel, rightLabel, onclose }: Props = $props();

  let onlyDiff = $state(true);

  const rows = $derived(diffRecords(left, right));
  const diffCount = $derived(rows.filter((r) => r.kind !== 'same').length);
  const shown = $derived(onlyDiff ? rows.filter((r) => r.kind !== 'same') : rows);

  function show(v: unknown, present: boolean): string {
    if (!present) return '—';
    return v === null || v === undefined ? '∅' : cellStr(v);
  }

  function copyPatch(): void {
    const ops = recordPatch(rows);
    void copyText(JSON.stringify(ops, null, 2), ['Copied', `JSON patch with ${ops.length} op${ops.length === 1 ? '' : 's'}`]);
  }
</script>

<Modal title="Compare records" width={900} {onclose}>
  <div class="rd" data-testid="record-diff">
    <div class="rd-bar">
      <span class="rd-count">
        {#if diffCount === 0}No differences — the records are identical.{:else}<strong>{diffCount}</strong> difference{diffCount === 1 ? '' : 's'} across {rows.length} field{rows.length === 1 ? '' : 's'}{/if}
      </span>
      <span class="grow"></span>
      <label class="rd-only"><input type="checkbox" bind:checked={onlyDiff} /> only differences</label>
      <button class="btn small" disabled={diffCount === 0} onclick={copyPatch} title="Copy the set/unset operations that turn {leftLabel} into {rightLabel}"><Icon name="copy" size={11} />Copy as JSON patch</button>
    </div>
    <div class="rd-scroll">
      <table class="rd-table mono">
        <thead>
          <tr><th class="rd-path">field</th><th>{leftLabel}</th><th>{rightLabel}</th></tr>
        </thead>
        <tbody>
          {#each shown as row (row.path)}
            <tr class={row.kind}>
              <td class="rd-path" title={row.path}>{row.path}</td>
              <td class="rd-val">{show(row.l, row.kind !== 'only-right')}</td>
              <td class="rd-val">{show(row.r, row.kind !== 'only-left')}</td>
            </tr>
          {:else}
            <tr><td class="rd-empty" colspan="3">Nothing to show.</td></tr>
          {/each}
        </tbody>
      </table>
    </div>
  </div>
</Modal>

<style>
  .rd {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
  }
  .rd-bar {
    display: flex;
    align-items: center;
    gap: 10px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .rd-only {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 12px;
  }
  .grow {
    flex: 1;
  }
  .rd-scroll {
    max-height: 60vh;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
  }
  .rd-table {
    width: 100%;
    border-collapse: collapse;
    font-size: 12px;
  }
  .rd-table th {
    position: sticky;
    top: 0;
    text-align: left;
    padding: 5px 8px;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
  }
  .rd-table td {
    padding: 4px 8px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
    vertical-align: top;
  }
  .rd-path {
    width: 28%;
    color: var(--text-dim);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    max-width: 260px;
  }
  .rd-val {
    width: 36%;
    white-space: pre-wrap;
    word-break: break-word;
  }
  /* Row tints: amber = changed, red = only left (removed), green = only right (added). */
  tr.changed td {
    background: var(--status-warn-soft);
  }
  tr.only-left td.rd-val:nth-child(2),
  tr.only-left td.rd-path {
    background: color-mix(in srgb, var(--status-exited) 14%, transparent);
  }
  tr.only-right td.rd-val:nth-child(3),
  tr.only-right td.rd-path {
    background: color-mix(in srgb, var(--status-working) 14%, transparent);
  }
  .rd-empty {
    text-align: center;
    color: var(--text-dim);
    padding: 14px;
  }
</style>
