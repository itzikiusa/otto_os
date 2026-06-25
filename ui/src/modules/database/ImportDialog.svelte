<script lang="ts">
  // File → table import dialog (0002): the mirror of the export-to-path
  // ("Export all rows…") dialog. Picks a local file on the daemon host, a
  // format, a target table, and a batch size, then streams batched INSERTs
  // through the same guarded write path a query uses — so a Prod/read-only
  // connection refuses it until the user types the connection name (the
  // identical typed-confirmation flow `runQuery` already uses). v1 is SQL-only.
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { dbImport } from '../../lib/api/client';
  import type { ImportFormat, ImportResult } from '../../lib/api/types';

  // Format select — the four import formats (the mirror of the export formats).
  const IMPORT_FORMATS: { value: ImportFormat; label: string }[] = [
    { value: 'csv', label: 'CSV (first row = header)' },
    { value: 'tsv', label: 'TSV (first row = header)' },
    { value: 'ndjson', label: 'NDJSON (one object per line)' },
    { value: 'json', label: 'JSON (array of objects)' },
  ];

  // Persist the last format / starting folder, like the export dialog does.
  const LS_FORMAT = 'otto_db_import_format';
  const LS_DIR = 'otto_db_import_dir';
  function loadFormat(): ImportFormat {
    const v = (typeof localStorage !== 'undefined' && localStorage.getItem(LS_FORMAT)) || 'csv';
    return IMPORT_FORMATS.some((f) => f.value === v) ? (v as ImportFormat) : 'csv';
  }
  function loadDir(): string {
    return (typeof localStorage !== 'undefined' && localStorage.getItem(LS_DIR)) || '~/Downloads';
  }

  let format = $state<ImportFormat>(loadFormat());
  let filePath = $state('');
  let table = $state(database.importTable);
  let batchSize = $state('500');
  let pickingFile = $state(false);
  let importing = $state(false);
  // Streamed result: the final {done…}/{error} line, surfaced in the live region.
  let progress = $state<ImportResult | null>(null);

  const connName = $derived(database.selectedConn?.name ?? 'this connection');

  function close(): void {
    if (importing) return;
    database.importDialogOpen = false;
  }

  /** Infer the format from a picked file's extension (best-effort). */
  function inferFormat(path: string): void {
    const ext = path.split('.').pop()?.toLowerCase();
    if (ext === 'tsv') format = 'tsv';
    else if (ext === 'ndjson' || ext === 'jsonl') format = 'ndjson';
    else if (ext === 'json') format = 'json';
    else if (ext === 'csv') format = 'csv';
  }

  async function runImport(): Promise<void> {
    const id = database.selectedConnId;
    if (!id || importing) return;
    const path = filePath.trim();
    const tbl = table.trim();
    if (!path || !tbl) {
      toasts.error('Missing field', 'A file path and a target table are both required.');
      return;
    }
    const size = batchSize.trim() ? Number(batchSize.trim()) : 500;
    if (!Number.isFinite(size) || size <= 0) {
      toasts.error('Invalid batch size', 'Enter a positive number (default 500).');
      return;
    }

    importing = true;
    progress = null;
    try {
      // First pass: no confirm. A guarded (Prod/read-only) connection comes back
      // with a final {error} line starting `write_blocked:` — we then run the
      // SAME typed-confirmation flow the query path uses and retry with confirm.
      let res = await dbImport(
        id,
        { local_path: path, format, table: tbl, batch_size: size },
        (line) => (progress = line),
      );

      if (typeof res.error === 'string' && res.error.startsWith('write_blocked:')) {
        const ok = await confirmGuardedWrite();
        if (!ok) {
          toasts.info('Import cancelled');
          progress = null;
          return;
        }
        progress = null;
        res = await dbImport(
          id,
          { local_path: path, format, table: tbl, batch_size: size, confirm_write: true },
          (line) => (progress = line),
        );
      }

      if (typeof res.error === 'string') {
        toasts.error('Import failed', res.error);
        return;
      }
      if (res.done) {
        if (typeof localStorage !== 'undefined') {
          localStorage.setItem(LS_FORMAT, format);
          const dir = path.replace(/\/[^/]*$/, '');
          if (dir) localStorage.setItem(LS_DIR, dir);
        }
        const rows = res.rows ?? 0;
        const batches = res.batches ?? 0;
        toasts.success(
          'Imported',
          `${rows.toLocaleString()} row${rows === 1 ? '' : 's'} in ${batches} batch${batches === 1 ? '' : 'es'} → ${tbl}`,
        );
        database.importDialogOpen = false;
        // Reflect the new rows: re-run the active tab's query (if any) and
        // refresh the structure of the targeted object when it's open.
        if (database.tab?.statement.trim()) void database.runQuery();
        if (database.selectedObjectPath) void database.refreshObject();
      }
    } catch (e) {
      toasts.error('Import failed', e instanceof Error ? e.message : String(e));
    } finally {
      importing = false;
    }
  }

  /** Typed confirmation for a write on a guarded connection — mirrors the query
   *  path's gate (type the connection name to proceed). */
  async function confirmGuardedWrite(): Promise<boolean> {
    const conn = database.selectedConn;
    if (!conn) return false;
    const label = conn.environment === 'prod' ? 'PRODUCTION' : 'read-only';
    const typed = await confirmer.promptText(
      `You are about to IMPORT rows into the ${label} connection "${conn.name}". ` +
        `This writes data. Type the connection name to confirm.`,
      { title: '⚠ Confirm production write', confirmLabel: 'Run import', placeholder: conn.name },
    );
    return typed != null && typed.trim().toLowerCase() === conn.name.trim().toLowerCase();
  }
</script>

<Modal title="Import file into a table" width={520} onclose={close}>
  <div class="imp-form">
    <p class="imp-hint">
      Reads a local file on the daemon host and inserts it into an existing table as
      <strong>batched INSERTs</strong>, through the same write guard as a query — so a Prod/read-only
      connection asks you to type its name first. v1 supports SQL engines (MySQL/ClickHouse).
    </p>

    <label class="imp-row">
      <span class="imp-label">Format</span>
      <select class="imp-select" bind:value={format}>
        {#each IMPORT_FORMATS as f (f.value)}
          <option value={f.value}>{f.label}</option>
        {/each}
      </select>
    </label>

    <div class="imp-row">
      <span class="imp-label">File</span>
      <div class="imp-dir">
        <input
          class="imp-input mono"
          bind:value={filePath}
          spellcheck="false"
          placeholder="~/Downloads/data.csv"
        />
        <button class="tb-btn" onclick={() => (pickingFile = true)} title="Browse the daemon host">
          <Icon name="folder" size={11} />Browse…
        </button>
      </div>
    </div>

    <label class="imp-row">
      <span class="imp-label">Table</span>
      <input
        class="imp-input mono"
        bind:value={table}
        spellcheck="false"
        placeholder="target_table"
      />
    </label>

    <label class="imp-row">
      <span class="imp-label">Batch size</span>
      <input
        class="imp-input mono"
        bind:value={batchSize}
        type="number"
        min="1"
        max="5000"
        spellcheck="false"
        placeholder="500"
      />
    </label>

    <div class="imp-dest" title="Where the rows are written">
      → <span class="mono">{table.trim() || 'target_table'}</span> on
      <span class="mono">{connName}</span>
    </div>

    {#if importing || progress}
      <div class="imp-progress" role="status" aria-live="polite">
        {#if importing && !progress}
          <div class="imp-bar"><div class="imp-bar-fill"></div></div>
          <div class="imp-prog-text mono">Importing…</div>
        {:else if progress?.error}
          <div class="imp-prog-text err mono">{progress.error}</div>
        {:else if progress?.done}
          <div class="imp-prog-text ok mono">
            Imported {(progress.rows ?? 0).toLocaleString()} rows in {progress.batches ?? 0} batches
          </div>
        {/if}
      </div>
    {/if}
  </div>

  {#snippet footer()}
    <button class="btn" onclick={close} disabled={importing}>Cancel</button>
    <button
      class="btn primary"
      onclick={() => void runImport()}
      disabled={importing || !filePath.trim() || !table.trim()}
    >
      {importing ? 'Importing…' : 'Import'}
    </button>
  {/snippet}
</Modal>

{#if pickingFile}
  <FolderPicker
    title="Choose a file to import (daemon host)"
    start={loadDir()}
    files={true}
    onpick={(p) => {
      filePath = p;
      inferFormat(p);
      pickingFile = false;
    }}
    onclose={() => (pickingFile = false)}
  />
{/if}

<style>
  /* Mirrors the export dialog's form styling (ResultsGrid `.exp-*`). */
  .imp-form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .imp-hint {
    margin: 0;
    font-size: 12px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .imp-row {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .imp-label {
    flex: 0 0 84px;
    font-size: 12px;
    color: var(--text-dim);
  }
  .imp-select,
  .imp-input {
    flex: 1;
    height: 30px;
    padding: 0 8px;
    font-size: 12.5px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
  }
  .imp-dir {
    flex: 1;
    display: flex;
    gap: 6px;
  }
  .imp-dir .imp-input {
    flex: 1;
  }
  .tb-btn {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 30px;
    padding: 0 9px;
    font-size: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    cursor: pointer;
    white-space: nowrap;
  }
  .tb-btn:hover {
    color: var(--text);
    border-color: var(--accent);
  }
  .imp-dest {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 6px 8px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
    word-break: break-all;
  }
  .imp-progress {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .imp-bar {
    height: 6px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .imp-bar-fill {
    height: 100%;
    width: 40%;
    border-radius: 999px;
    background: var(--accent);
    animation: imp-indet 1.1s ease-in-out infinite;
  }
  @keyframes imp-indet {
    0% {
      margin-left: -40%;
    }
    100% {
      margin-left: 100%;
    }
  }
  .imp-prog-text {
    font-size: 11.5px;
    color: var(--text-dim);
  }
  .imp-prog-text.ok {
    color: var(--accent);
  }
  .imp-prog-text.err {
    color: var(--status-exited);
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
