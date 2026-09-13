<script lang="ts">
  import { onDestroy } from 'svelte';
  import RestorePreviewPanel from './RestorePreviewPanel.svelte';
  import { api } from '../../lib/api/client';
  import type { StateArchive, RestoreConflictPolicy, RestorePreview, RestoreResult } from '../../lib/api/types';
  import { downloadText } from '../../lib/components/exporters';

  let exporting = $state(false);
  let busy = $state(false);
  let error = $state('');
  let archive: StateArchive | null = $state(null);
  let preview: RestorePreview | null = $state(null);
  let result: RestoreResult | null = $state(null);
  let filename = $state('');
  let conflicts: RestoreConflictPolicy = $state('skip_existing');
  let reviewed = $state(false);
  let downloaded: { records: number; files: number; excluded: string[]; reconnect: string[] } | null = $state(null);
  let generation = 0;
  onDestroy(() => { generation++; });

  async function exportArchive() {
    const seq = generation;
    exporting = true;
    error = '';
    try {
      const data = await api.get<StateArchive>('/state/archive');
      if (seq !== generation) return;
      downloadText(JSON.stringify(data), `otto-data-${new Date().toISOString().slice(0, 10)}.json`, 'application/json');
      downloaded = {
        records: Object.values(data.records).reduce((sum, rows) => sum + rows.length, 0),
        files: data.files.length, excluded: data.excluded, reconnect: data.reconnect,
      };
    } catch (e) { if (seq === generation) error = e instanceof Error ? e.message : String(e); }
    finally { if (seq === generation) exporting = false; }
  }

  async function readArchive(event: Event) {
    const input = event.target as HTMLInputElement;
    const file = input.files?.[0];
    input.value = '';
    if (!file) return;
    const seq = ++generation;
    archive = null; preview = null; result = null; reviewed = false; error = ''; filename = file.name;
    busy = true;
    try {
      if (file.size > 256 * 1024 * 1024) throw new Error('This archive exceeds the 256 MiB import limit.');
      const data: unknown = JSON.parse(await file.text());
      if (!data || typeof data !== 'object' || !('archive_format' in data) || data.archive_format !== 2) {
        throw new Error('Choose an Otto data archive (format 2). Older settings backups use the settings restore section below.');
      }
      if (seq === generation) archive = data as StateArchive;
    } catch (e) { if (seq === generation) error = e instanceof Error ? e.message : String(e); }
    finally { if (seq === generation) busy = false; }
  }

  function changePolicy() { preview = null; result = null; reviewed = false; }

  async function inspectArchive() {
    if (!archive) return;
    const seq = generation;
    busy = true; error = ''; preview = null; result = null; reviewed = false;
    try {
      const data = await api.post<RestorePreview>('/state/archive/preview', { archive, conflicts });
      if (seq === generation) preview = data;
    } catch (e) { if (seq === generation) error = e instanceof Error ? e.message : String(e); }
    finally { if (seq === generation) busy = false; }
  }

  async function restoreArchive() {
    if (!archive || !preview?.can_restore || !reviewed) return;
    const seq = generation;
    busy = true; error = '';
    try {
      const data = await api.post<RestoreResult>('/state/archive/restore', {
        archive, conflicts, preview_token: preview.preview_token, confirm: true,
      });
      if (seq !== generation) return;
      result = data; preview = null; reviewed = false;
    } catch (e) {
      if (seq === generation) {
        error = e instanceof Error ? e.message : String(e);
        preview = null; reviewed = false; // A failed/stale preview must be checked again.
      }
    } finally { if (seq === generation) busy = false; }
  }
</script>

<section class="full-backup" aria-label="Otto data backup">
  <h3>Otto data backup</h3>
  <p>Export saved data across Otto: workflows, scheduled tasks, connections, API collections,
    session records, and owned documents and attachments, including Vault files.</p>
  <p class="dim">Stored credentials and live processes are excluded. Documents and history can contain private information; keep the archive private. Repository working trees and external database
    contents remain in their original locations. The archive lists exclusions and anything that needs reconnecting.</p>
  <button class="btn primary" disabled={exporting || busy} onclick={exportArchive}>{exporting ? 'Preparing archive…' : 'Download data archive'}</button>
  {#if downloaded}
    <div class="notice" role="status">Downloaded {downloaded.records} records and {downloaded.files} files.</div>
    {#if downloaded.excluded.length}<details><summary>Excluded from this archive ({downloaded.excluded.length})</summary><ul>{#each downloaded.excluded as item}<li>{item}</li>{/each}</ul></details>{/if}
    {#if downloaded.reconnect.length}<details><summary>Reconnect after restore ({downloaded.reconnect.length})</summary><ul>{#each downloaded.reconnect as item}<li>{item}</li>{/each}</ul></details>{/if}
  {/if}

  <h4>Restore saved data</h4>
  <p>Choose an archive and review its contents before restoring. Existing records and files are preserved.
    Restored schedules and other automatic activity stay inactive until you enable them.</p>
  <div class="controls">
    <label class="btn">Choose data archive<input type="file" accept=".json,application/json" disabled={busy || exporting} onchange={readArchive} /></label>
    {#if filename}<span class="filename" title={filename}>{filename}</span>{/if}
  </div>
  {#if archive}
    <div class="controls">
      <label>When an item already exists
        <select class="input" bind:value={conflicts} disabled={busy} onchange={changePolicy}>
          <option value="skip_existing">Keep existing; restore only new items</option>
          <option value="abort">Stop if there are any conflicts</option>
        </select>
      </label>
      <button class="btn" disabled={busy} onclick={inspectArchive}>{busy ? 'Working…' : 'Preview restore'}</button>
    </div>
  {/if}
  {#if preview}
    <RestorePreviewPanel {preview} {busy} bind:reviewed onrestore={restoreArchive} />
  {/if}
  {#if result}
    <div class="notice" role="status">Restored {result.records_inserted} records and {result.files_restored} files.
      Kept {result.records_skipped} existing records and {result.files_skipped} existing files.</div>
    {#if result.reconnect.length}<ul>{#each result.reconnect as item}<li>{item}</li>{/each}</ul>{/if}
  {/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</section>

<style>
  .full-backup { border: 1px solid var(--border); border-radius: var(--radius-m); padding: 18px; margin: 18px 0; }
  h3 { margin: 0 0 8px; font-size: 16px; }
  h4 { margin: 24px 0 8px; }
  p { font-size: 13px; line-height: 1.5; }
  .dim { color: var(--text-dim); }
  .controls { display: flex; flex-wrap: wrap; align-items: end; gap: 10px; margin: 12px 0; }
  .controls label:not(.btn) { display: flex; flex-direction: column; gap: 5px; font-size: 12px; max-width: 100%; }
  select { max-width: 100%; }
  label.btn { cursor: pointer; }
  label.btn:has(input:disabled) { opacity: .5; pointer-events: none; }
  input[type=file] { display: none; }
  .filename { max-width: 100%; overflow-wrap: anywhere; font-size: 12px; }
  details { margin: 10px 0; font-size: 12px; }
  summary { cursor: pointer; }
  ul { max-height: 240px; overflow: auto; overflow-wrap: anywhere; }
  li { margin: 5px 0; }
  .notice { margin: 10px 0; font-size: 12px; color: var(--text-dim); }
  .error { color: var(--danger); overflow-wrap: anywhere; }
</style>
