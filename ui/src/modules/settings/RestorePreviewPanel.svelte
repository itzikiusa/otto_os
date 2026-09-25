<script lang="ts">
  import type { RestorePreview } from '../../lib/api/types';
  let { preview, busy, reviewed = $bindable(false), onrestore }: {
    preview: RestorePreview; busy: boolean; reviewed?: boolean; onrestore: () => void;
  } = $props();
</script>

    <div class="preview" aria-label="Restore preview">
      <strong>{preview.record_count} records · {preview.file_count} files · {preview.conflicts.length} conflicts</strong>
      <details open><summary>Included records</summary><div class="scroll"><table><thead><tr><th scope="col">Area</th><th scope="col" class="num">Records</th></tr></thead><tbody>{#each Object.entries(preview.table_counts).sort(([a], [b]) => a.localeCompare(b)) as [table, count]}<tr><td>{table.replaceAll('_', ' ')}</td><td class="num">{count}</td></tr>{/each}</tbody></table></div></details>
      {#if preview.conflicts.length}<details><summary>Existing items ({preview.conflicts.length})</summary><ul>{#each preview.conflicts.slice(0, 100) as conflict}<li>{conflict.location}: {conflict.reason}</li>{/each}{#if preview.conflicts.length > 100}<li>Showing the first 100 conflicts.</li>{/if}</ul></details>{/if}
      {#if preview.excluded.length}<details><summary>Excluded items</summary><ul>{#each preview.excluded as item}<li>{item}</li>{/each}</ul></details>{/if}
      {#if preview.reconnect.length}<details open><summary>Reconnect after restore</summary><ul>{#each preview.reconnect as item}<li>{item}</li>{/each}</ul></details>{/if}
      {#if preview.warnings.length}<ul class="warnings">{#each preview.warnings as warning}<li>{warning}</li>{/each}</ul>{/if}
      {#if !preview.can_restore}<p class="error">This archive cannot be restored with the selected policy. Review the issues above.</p>{/if}
      <label class="reviewed"><input type="checkbox" bind:checked={reviewed} disabled={busy || !preview.can_restore} /> I reviewed the contents, conflicts, and reconnect requirements.</label>
      <button class="btn primary" disabled={busy || !preview.can_restore || !reviewed} onclick={onrestore}>{busy ? 'Restoring…' : 'Restore new items'}</button>
    </div>

<style>
  .preview { border-top: 1px solid var(--border); padding-top: 14px; margin-top: 14px; font-size: var(--fs-s); }
  details { margin: 10px 0; font-size: var(--fs-s); }
  summary { cursor: pointer; }
  ul, .scroll { max-height: 240px; overflow: auto; overflow-wrap: anywhere; }
  li { margin: 4px 0; }
  table { width: 100%; max-width: 480px; border-collapse: collapse; }
  th, td { padding: 4px 8px; border-bottom: 1px solid var(--border); text-align: start; }
  th { font-size: var(--fs-xs); font-weight: 600; color: var(--text-dim); }
  .num { text-align: end; font-variant-numeric: tabular-nums; }
  .warnings { color: var(--warning); }
  .reviewed { display: flex; align-items: flex-start; gap: 7px; margin: 14px 0; font-size: var(--fs-m); }
  .error { color: var(--danger); overflow-wrap: anywhere; }
</style>
