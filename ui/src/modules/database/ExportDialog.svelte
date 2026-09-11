<script lang="ts">
  // ── Large-batch streaming export to a local file ─────────────────────────────
  // Runs the statement uncapped on the daemon and STREAMS the result straight to
  // a file the user chooses on the daemon host — for result sets too big to pull
  // into the browser. Format is selectable; the destination directory is picked
  // via the shared FolderPicker (the same /fs/browse picker used elsewhere). Last
  // format + directory are remembered in localStorage. Mounted by ResultsGrid
  // while its "Export all rows…" dialog is open; `onclose` dismisses it.
  import Icon from '../../lib/components/Icon.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import type { DbExportFormat, ExportToPathResp } from '../../lib/api/types';
  import { postNdjsonStream } from '../../lib/api/client';
  import Modal from '../../lib/components/Modal.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import { fmtBytes } from './results-format';

  interface Props {
    /** The statement to run uncapped (the tab's ran statement). */
    statement: string;
    connectionId: string;
    /** Export permission on the connection — re-checked before each run. */
    canExport: boolean;
    onclose: () => void;
  }
  let { statement, connectionId, canExport, onclose }: Props = $props();

  type ExportFmtOpt = { value: DbExportFormat; label: string };
  const EXPORT_FORMATS: ExportFmtOpt[] = [
    { value: 'csv', label: 'CSV' },
    { value: 'csv_with_names', label: 'CSV (with header)' },
    { value: 'tsv', label: 'TSV' },
    { value: 'tsv_with_names', label: 'TSV (with header)' },
    { value: 'json', label: 'JSON (array)' },
    { value: 'ndjson', label: 'NDJSON' },
  ];
  const EXT_BY_FORMAT: Record<DbExportFormat, string> = {
    csv: 'csv',
    csv_with_names: 'csv',
    tsv: 'tsv',
    tsv_with_names: 'tsv',
    json: 'json',
    ndjson: 'ndjson',
  };
  const LS_FORMAT = 'otto_db_export_format';
  const LS_DIR = 'otto_db_export_dir';

  function loadFormat(): DbExportFormat {
    const v = (typeof localStorage !== 'undefined' && localStorage.getItem(LS_FORMAT)) || 'csv';
    return EXPORT_FORMATS.some((f) => f.value === v) ? (v as DbExportFormat) : 'csv';
  }
  function loadDir(): string {
    return (typeof localStorage !== 'undefined' && localStorage.getItem(LS_DIR)) || '~/Downloads';
  }

  // Default a filename from the statement (a leading table-ish token) or 'result'.
  function defaultExportName(): string {
    const fromStmt = statement?.match(/\bfrom\s+["'`]?([\w.]+)/i)?.[1];
    const base = (fromStmt || 'result').replace(/[^\w.-]+/g, '_').slice(0, 60) || 'result';
    return `${base}.${EXT_BY_FORMAT[exportFormat]}`;
  }

  // The dialog mounts fresh on every open, so the remembered format/directory
  // and the default file name are seeded right here.
  let pickingDir = $state(false);
  let exportFormat = $state<DbExportFormat>(loadFormat());
  let exportDir = $state<string>(loadDir());
  let exportName = $state(defaultExportName());
  let exportLimit = $state('');
  let exportingPath = $state(false);
  // Live progress for the streaming export (bytes written so far). Null when no
  // export is running; drives the dialog's progress bar.
  let exportProgress = $state<{ bytes: number } | null>(null);
  // In-flight stream controller — the dialog's Cancel aborts it while exporting.
  let exportAbort: AbortController | null = null;

  // Keep the filename extension in sync when the format changes (only if the user
  // hasn't typed a custom, non-default-stem name).
  function onFormatChange(): void {
    const ext = EXT_BY_FORMAT[exportFormat];
    if (!exportName) {
      exportName = defaultExportName();
      return;
    }
    exportName = exportName.replace(/\.(csv|tsv|json|ndjson)$/i, '') + `.${ext}`;
  }

  function joinPath(dir: string, name: string): string {
    const d = dir.replace(/\/+$/, '');
    return `${d}/${name}`;
  }

  async function runPathExport(): Promise<void> {
    if (!canExport) return;
    if (!connectionId || !statement || exportingPath) return;
    const name = exportName.trim() || defaultExportName();
    const dir = exportDir.trim() || '~/Downloads';
    const localPath = joinPath(dir, name);
    const maxRows = exportLimit.trim() ? Number(exportLimit.trim()) : undefined;
    if (maxRows !== undefined && (!Number.isFinite(maxRows) || maxRows <= 0)) {
      toasts.error('Invalid row limit', 'Leave blank for all rows, or enter a positive number.');
      return;
    }
    exportingPath = true;
    exportProgress = { bytes: 0 };
    exportAbort = new AbortController();
    let done: ExportToPathResp | null = null;
    let failed: string | null = null;
    try {
      // The endpoint streams NDJSON progress lines ({bytes:N}) and a final line
      // ({done,local_path,rows,bytes,duration_ms} or {error}); read them live so
      // the bar moves and a long export never idles out the browser fetch.
      await postNdjsonStream(
        `/connections/${connectionId}/db/export-to-path`,
        {
          statement,
          node: database.activeDb ?? undefined,
          format: exportFormat,
          local_path: localPath,
          max_rows: maxRows,
        },
        (msg) => {
          const m = msg as Record<string, unknown>;
          if (typeof m.error === 'string') failed = m.error;
          else if (m.done) done = m as unknown as ExportToPathResp;
          else if (typeof m.bytes === 'number') exportProgress = { bytes: m.bytes };
        },
        exportAbort.signal,
      );
      if (failed) throw new Error(failed);
      if (done) {
        const r: ExportToPathResp = done;
        if (typeof localStorage !== 'undefined') {
          localStorage.setItem(LS_FORMAT, exportFormat);
          localStorage.setItem(LS_DIR, dir);
        }
        onclose();
        toasts.success(
          'Exported',
          `${r.rows.toLocaleString()} row${r.rows === 1 ? '' : 's'} · ${fmtBytes(r.bytes)} → ${r.local_path}`,
        );
      }
    } catch (e) {
      // A user-initiated cancel isn't a failure — the partial file stays where
      // the export was writing it.
      if (e instanceof DOMException && e.name === 'AbortError') {
        toasts.info('Export cancelled', 'The partially written file was left in place.');
      } else {
        toasts.error('Export failed', e instanceof Error ? e.message : String(e));
      }
    } finally {
      exportingPath = false;
      exportProgress = null;
      exportAbort = null;
    }
  }
</script>

<Modal
  title="Export all rows"
  width={520}
  onclose={() => {
    exportAbort?.abort();
    onclose();
  }}
>
  <div class="exp-form">
    <p class="exp-hint">
      Runs the statement on the daemon host and <strong>streams</strong> the full result to a local
      file — for sets too large to pull into the browser. Choose the format, destination directory,
      and an optional row limit.
    </p>

    <label class="exp-row">
      <span class="exp-label">Format</span>
      <select class="exp-select" bind:value={exportFormat} onchange={onFormatChange}>
        {#each EXPORT_FORMATS as f (f.value)}
          <option value={f.value}>{f.label}</option>
        {/each}
      </select>
    </label>

    <div class="exp-row">
      <span class="exp-label">Folder</span>
      <div class="exp-dir">
        <input class="exp-input mono" bind:value={exportDir} spellcheck="false" placeholder="~/Downloads" />
        <button class="tb-btn" onclick={() => (pickingDir = true)} title="Browse the daemon host">
          <Icon name="folder" size={11} />Browse…
        </button>
      </div>
    </div>

    <label class="exp-row">
      <span class="exp-label">File name</span>
      <input class="exp-input mono" bind:value={exportName} spellcheck="false" placeholder="result.csv" />
    </label>

    <label class="exp-row">
      <span class="exp-label">Row limit</span>
      <input
        class="exp-input mono"
        bind:value={exportLimit}
        type="number"
        min="1"
        spellcheck="false"
        placeholder="all rows"
      />
    </label>

    <div class="exp-dest mono" title="Resolved destination on the daemon host">
      → {joinPath(exportDir.trim() || '~/Downloads', exportName.trim() || defaultExportName())}
    </div>

    {#if exportingPath}
      <div class="exp-progress" role="status" aria-live="polite">
        <div class="exp-bar"><div class="exp-bar-fill"></div></div>
        <div class="exp-prog-text mono">
          {exportProgress ? fmtBytes(exportProgress.bytes) : '0 B'} written…
        </div>
      </div>
    {/if}
  </div>

  {#snippet footer()}
    <button
      class="btn"
      onclick={() => (exportingPath ? exportAbort?.abort() : onclose())}
      title={exportingPath ? 'Stop the running export (the partial file is left in place)' : undefined}
    >
      {exportingPath ? 'Cancel export' : 'Cancel'}
    </button>
    <button class="btn primary" onclick={() => void runPathExport()} disabled={exportingPath}>
      {exportingPath ? 'Exporting…' : 'Export all'}
    </button>
  {/snippet}
</Modal>

{#if pickingDir}
  <FolderPicker
    title="Choose export folder (daemon host)"
    start={exportDir}
    onpick={(p) => {
      exportDir = p;
      pickingDir = false;
    }}
    onclose={() => (pickingDir = false)}
  />
{/if}

<style>
  /* Scoped copy of ResultsGrid's toolbar button (the Browse… button). */
  .tb-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 9px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: 11.5px;
    cursor: pointer;
  }
  .tb-btn:hover {
    border-color: color-mix(in srgb, var(--accent) 45%, transparent);
    color: var(--accent);
  }

  /* ── Local-file export dialog ─────────────────────────────────────────────── */
  .exp-form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .exp-hint {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text-dim);
  }
  .exp-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .exp-label {
    flex: 0 0 76px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .exp-select,
  .exp-input {
    flex: 1;
    min-width: 0;
    padding: 6px 9px;
    font-size: 12.5px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
  }
  .exp-select:focus,
  .exp-input:focus {
    outline: none;
    border-color: var(--accent);
  }
  .exp-dir {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .exp-dir .exp-input {
    flex: 1;
  }
  .exp-dest {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 6px 9px;
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* Streaming-export progress: total size is unknown up front, so the bar is an
     indeterminate sweep + a live bytes-written readout. */
  .exp-progress {
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .exp-bar {
    position: relative;
    height: 6px;
    border-radius: 999px;
    background: var(--surface-3, var(--surface-2));
    overflow: hidden;
  }
  .exp-bar-fill {
    position: absolute;
    top: 0;
    left: 0;
    height: 100%;
    width: 35%;
    border-radius: 999px;
    background: var(--accent);
    animation: exp-sweep 1.1s ease-in-out infinite;
  }
  @keyframes exp-sweep {
    0% { left: -35%; }
    100% { left: 100%; }
  }
  .exp-prog-text {
    font-size: 11px;
    color: var(--text-dim);
  }
  @media (prefers-reduced-motion: reduce) {
    .exp-bar-fill {
      animation: none;
      left: 0;
      width: 100%;
      opacity: 0.5;
    }
  }
</style>
