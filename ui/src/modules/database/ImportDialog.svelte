<script lang="ts">
  import { plural, pluralNoun } from '../../lib/plural';
  import { toastError } from '../../lib/toastError';
  // File → table import dialog (0002): the mirror of the export-to-path
  // ("Export all rows…") dialog. Picks a local file on the daemon host, a
  // format, a target table, and a batch size, then streams batched INSERTs
  // through the same guarded write path a query uses — so a Prod/read-only
  // connection refuses it until the user types the connection name (the
  // identical typed-confirmation flow `runQuery` already uses). v1 is SQL-only.
  import { databaseAccessChild } from '../../lib/access-options';
  import { resourceAccess } from '../../lib/stores/resource-access.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { database } from '../../lib/stores/database.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { postNdjsonStream } from '../../lib/api/client';
  import type { ImportFormat, ImportReq, ImportResult } from '../../lib/api/types';
  import { lsGet, lsSet } from '../../lib/storage';

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
    const v = lsGet(LS_FORMAT) || 'csv';
    return IMPORT_FORMATS.some((f) => f.value === v) ? (v as ImportFormat) : 'csv';
  }
  function loadDir(): string {
    return lsGet(LS_DIR) || '~/Downloads';
  }

  let format = $state<ImportFormat>(loadFormat());
  let filePath = $state('');
  let table = $state(database.importTable);
  let batchSize = $state('500');
  let pickingFile = $state(false);
  let importing = $state(false);
  // Streamed result: the final {done…}/{error} line, surfaced in the live region.
  let progress = $state<ImportResult | null>(null);
  // In-flight stream controller — the footer Cancel aborts it while importing.
  let importAbort: AbortController | null = null;
  /** Inline batch-size problem (blank = the default 500). `bind:value` on a
   *  number input yields a number (or null when cleared) — normalise first. */
  function batchSizeError(raw: unknown): string | null {
    const text = String(raw ?? '').trim();
    if (!text) return null;
    const n = Number(text);
    return Number.isFinite(n) && n > 0 ? null : 'Enter a positive number (default 500).';
  }

  // Reading a daemon-host file is the host owner's power: root only (S6-302).
  const canImport = $derived(auth.isRoot && !!database.selectedConnId && resourceAccess.can('connection',database.selectedConnId,'db_data','database','edit',databaseAccessChild(database.activeDb)));
  $effect(()=>{if(database.selectedConnId)void resourceAccess.load('connection',database.selectedConnId,databaseAccessChild(database.activeDb));});
  const connName = $derived(database.selectedConn?.name ?? 'this connection');

  /** `dbImport` with a cancel signal: stream the NDJSON lines into `progress`
   *  and resolve with the final {done…}/{error} line (mirrors api/client). */
  async function importStream(id: string, body: ImportReq): Promise<ImportResult> {
    importAbort = new AbortController();
    let last: ImportResult = {};
    await postNdjsonStream(
      `/connections/${id}/db/import`,
      body,
      (msg) => {
        last = msg as ImportResult;
        progress = last;
      },
      importAbort.signal,
    );
    return last;
  }

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
    const objectPath = database.selectedObjectPath;
    const selectedTable = database.importTable;
    const origin = database.selectedConn ? { ...database.selectedConn } : null;
    const importFormat = format;
    const current = () => database.selectedConnId === id && database.importTable === selectedTable && canImport;
    if (!id || importing || !canImport) return;
    const path = filePath.trim();
    const tbl = table.trim();
    // Field problems are shown inline (and keep Import disabled) — these are
    // only guards for a call that slipped past the disabled button.
    if (!path || !tbl || batchSizeError(batchSize)) return;
    const rawSize = String(batchSize ?? '').trim();
    const size = rawSize ? Number(rawSize) : 500;

    importing = true;
    progress = null;
    try {
      // First pass: no confirm. A guarded (Prod/read-only) connection comes back
      // with a final {error} line starting `write_blocked:` — we then run the
      // SAME typed-confirmation flow the query path uses and retry with confirm.
      let res = await importStream(id, { local_path: path, format: importFormat, table: tbl, batch_size: size });

      if (typeof res.error === 'string' && res.error.startsWith('write_blocked:')) {
        if (!current() || importAbort?.signal.aborted) return;
        const answer = await confirmGuardedWrite(origin);
        if (answer !== 'confirmed' || !current() || importAbort?.signal.aborted) {
          // A mistyped name was already explained by the gate.
          if (answer !== 'mismatch') toasts.info('Import canceled');
          progress = null;
          return;
        }
        progress = null;
        res = await importStream(id, {
          local_path: path,
          format: importFormat,
          table: tbl,
          batch_size: size,
          confirm_write: true,
        });
      }

      if (typeof res.error === 'string') {
        toasts.error('Couldn’t import', res.error);
        return;
      }
      if (res.done) {
        // Guarded: a blocked storage write must not turn a finished import
        // into "Couldn’t import" (and skip ondone / onclose).
        lsSet(LS_FORMAT, importFormat);
        const dir = path.replace(/\/[^/]*$/, '');
        if (dir) lsSet(LS_DIR, dir);
        const rows = res.rows ?? 0;
        const batches = res.batches ?? 0;
        toasts.success(
          'Imported',
          `${rows.toLocaleString()} ${pluralNoun(rows, 'row')} in ${plural(batches, 'batch', 'batches')} → ${tbl}`,
        );
        if (current()) database.importDialogOpen = false;
        // Import never authorizes executing the editor's unsubmitted draft.
        // Only refresh the same metadata view, if it still owns the selection.
        if (current() && objectPath && database.selectedObjectPath === objectPath) {
          void database.refreshObject();
        }
      }
    } catch (e) {
      // A user-initiated cancel isn't a failure. The server-side stream stops
      // when the connection drops; batches already committed stay in the table.
      if (e instanceof DOMException && e.name === 'AbortError') {
        toasts.info('Import canceled', 'Batches already written stay in the table.');
        progress = null;
        return;
      }
      toastError('Couldn’t import', e);
    } finally {
      importing = false;
      importAbort = null;
    }
  }

  /** Typed confirmation for an import into a guarded connection — the store's
   *  gate (`database.confirmGuardedWrite`) worded for an import: titled by what
   *  the connection IS (a read-only dev connection is not "production"), no
   *  glyph, and a mistyped name is explained (`'mismatch'`) instead of reading
   *  as a plain cancel. */
  async function confirmGuardedWrite(
    conn: { name: string; environment?: string | null } | null,
  ): Promise<'confirmed' | 'declined' | 'mismatch'> {
    if (!conn) return 'declined';
    const prod = conn.environment === 'prod';
    const label = prod ? 'PRODUCTION' : 'read-only';
    const typed = await confirmer.promptText(
      `You are about to IMPORT rows into the ${label} connection "${conn.name}". ` +
        `This writes data. Type the connection name to confirm.`,
      {
        title: prod ? 'Confirm production import' : 'Confirm import on a read-only connection',
        confirmLabel: 'Run import',
        placeholder: conn.name,
        danger: true,
      },
    );
    if (typed == null) return 'declined';
    if (typed.trim().toLowerCase() !== conn.name.trim().toLowerCase()) {
      toasts.info('Nothing was changed', 'What you typed didn’t match, so the action was canceled.');
      return 'mismatch';
    }
    return 'confirmed';
  }
</script>

<Modal title="Import file into a table" width={520} onclose={close}>
  <div class="imp-form">
    <p class="imp-hint">
      Reads a local file on the daemon host and inserts it into an existing table as
      <strong>batched INSERTs</strong>, through the same write guard as a query — so a Prod/read-only
      connection asks you to type its name first. v1 supports SQL engines (MySQL/ClickHouse).
    </p>
    {#if !auth.isRoot}
      <p class="imp-hint" role="note">
        Only the root user can read files on the Otto host computer. Ask the root user to run this
        import.
      </p>
    {/if}

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
        <input dir="ltr" aria-label="Import file path"
          class="imp-input mono"
          bind:value={filePath}
          spellcheck="false"
          placeholder="~/Downloads/data.csv"
        />
        <button class="btn small" onclick={() => (pickingFile = true)} title="Browse the daemon host">
          <Icon name="folder" size={12} />Browse…
        </button>
      </div>
    </div>

    <label class="imp-row">
      <span class="imp-label">{database.capabilities?.engine === 'mongodb' ? 'Collection' : 'Table'}</span>
      <input dir="ltr"
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
        aria-invalid={batchSizeError(batchSize) ? 'true' : undefined}
        aria-describedby={batchSizeError(batchSize) ? 'imp-batch-err' : undefined}
      />
    </label>
    {#if batchSizeError(batchSize)}<p class="imp-err" id="imp-batch-err">{batchSizeError(batchSize)}</p>{/if}

    <div class="imp-dest" title="Where the rows are written">
      → <span class="mono">{table.trim() || 'target_table'}</span> on
      <span class="mono">{connName}</span>
    </div>

    {#if importing || progress}
      <div class="imp-progress" role="status" aria-live="polite">
        {#if importing && !progress}
          <div class="indeterminate" role="progressbar" aria-label="Importing"></div>
          <div class="imp-prog-text mono">Importing…</div>
        {:else if progress?.error}
          <div class="imp-prog-text err mono">{progress.error}</div>
        {:else if progress?.done}
          <div class="imp-prog-text ok mono">
            Imported {(progress.rows ?? 0).toLocaleString()} {pluralNoun(progress.rows ?? 0, 'row')} in {plural(progress.batches ?? 0, 'batch', 'batches')}
          </div>
        {/if}
      </div>
    {/if}
  </div>

  {#snippet footer()}
    <button class="btn" onclick={() => (importing ? importAbort?.abort() : close())}>
      {importing ? 'Cancel import' : 'Cancel'}
    </button>
    <button
      class="btn primary"
      onclick={() => void runImport()}
      disabled={!canImport || importing || !filePath.trim() || !table.trim() || batchSizeError(batchSize) !== null}
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
  .imp-err {
    margin: -4px 0 0;
    color: var(--danger);
    font-size: var(--fs-s);
  }
  /* Mirrors the export dialog's form styling (ResultsGrid `.exp-*`). */
  .imp-form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .imp-hint {
    margin: 0;
    font-size: var(--fs-s);
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
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .imp-select,
  .imp-input {
    flex: 1;
    height: 30px;
    padding: 0 8px;
    font-size: var(--fs-m);
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
  .imp-dest {
    font-size: var(--fs-s);
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
  .imp-prog-text {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .imp-prog-text.ok {
    color: var(--accent-text);
  }
  .imp-prog-text.err {
    color: var(--danger);
    white-space: pre-wrap;
    word-break: break-word;
  }
</style>
