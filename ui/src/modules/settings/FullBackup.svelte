<script lang="ts">
  import { onDestroy } from 'svelte';
  import RestorePreviewPanel from './RestorePreviewPanel.svelte';
  import { api } from '../../lib/api/client';
  import type { StateArchive, RestoreConflictPolicy, RestorePreview, RestoreResult } from '../../lib/api/types';
  import { downloadText } from '../../lib/components/exporters';
  import Icon from '../../lib/components/Icon.svelte';

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
  let fileEl: HTMLInputElement | null = $state(null);
  onDestroy(() => { generation++; });
  // Inline errors say which step failed, then the daemon's reason.
  const msg = (e: unknown): string => (e instanceof Error ? e.message : String(e));

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
    } catch (e) { if (seq === generation) error = `Couldn’t prepare the data archive. ${msg(e)}`; }
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
    } catch (e) { if (seq === generation) error = `Couldn’t read “${file.name}”. ${msg(e)}`; }
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
    } catch (e) { if (seq === generation) error = `Couldn’t preview the restore. ${msg(e)}`; }
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
        error = `Couldn’t restore the archive. ${msg(e)} Preview it again to retry.`;
        preview = null; reviewed = false; // A failed/stale preview must be checked again.
      }
    } finally { if (seq === generation) busy = false; }
  }
</script>

<section class="full-backup" aria-label="Otto data backup">
  <h2 class="card-title">Otto data backup</h2>
  <p>Export saved data across Otto: workflows, scheduled tasks, connections, API collections,
    session records, and owned documents and attachments, including Vault files.</p>
  <p class="dim">Stored credentials and live processes are excluded. Documents and history can contain private information; keep the archive private. Repository working trees and external database
    contents remain in their original locations. The archive lists exclusions and anything that needs reconnecting.</p>
  <div class="controls"><button class="btn primary" disabled={exporting || busy} onclick={exportArchive}><Icon name="download" size={13} /> {exporting ? 'Preparing archive…' : 'Download data archive'}</button></div>
  {#if downloaded}
    <div class="notice" role="status">Downloaded {downloaded.records} records and {downloaded.files} files.</div>
    {#if downloaded.excluded.length}<details><summary>Excluded from this archive ({downloaded.excluded.length})</summary><ul>{#each downloaded.excluded as item}<li>{item}</li>{/each}</ul></details>{/if}
    {#if downloaded.reconnect.length}<details><summary>Reconnect after restore ({downloaded.reconnect.length})</summary><ul>{#each downloaded.reconnect as item}<li>{item}</li>{/each}</ul></details>{/if}
  {/if}

  <h3 class="sub-title">Restore saved data</h3>
  <p>Choose an archive and review its contents before restoring. Existing records and files are preserved.
    Restored schedules and other automatic activity stay inactive until you enable them.</p>
  <div class="controls">
    <!-- A real button (keyboard-reachable) driving a hidden file input. -->
    <button class="btn" disabled={busy || exporting} onclick={() => fileEl?.click()}><Icon name="folder" size={13} /> Choose data archive…</button>
    <input type="file" class="file-input" accept=".json,application/json" disabled={busy || exporting} onchange={readArchive} bind:this={fileEl} tabindex="-1" aria-hidden="true" />
    {#if filename}<span class="filename" title={filename}>{filename}</span>{/if}
  </div>
  {#if archive}
    <div class="controls">
      <label class="policy">When an item already exists
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
  /* One settings card per backup tool — same shape in FullBackup, GitBackup
     and ConnectionsExport so the Backup page reads as one form. */
  .full-backup { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-m); box-shadow: var(--shadow-card); padding: 16px 18px; margin: 0 0 16px; max-width: var(--settings-col); }
  .card-title { margin: 0 0 6px; font-size: var(--fs-m); font-weight: 600; }
  .sub-title { margin: 20px 0 6px; padding-top: 16px; border-top: 1px solid var(--border); font-size: var(--fs-m); font-weight: 600; }
  p { margin: 0 0 8px; font-size: var(--fs-s); line-height: 1.5; }
  .dim { color: var(--text-dim); }
  .controls { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 8px; margin: 12px 0 0; }
  .controls label.policy { display: flex; flex-direction: column; gap: 4px; font-size: var(--fs-s); font-weight: 500; color: var(--text-dim); max-width: 100%; }
  select { max-width: 100%; }
  .file-input { display: none; }
  .filename { max-width: 100%; overflow-wrap: anywhere; font-size: var(--fs-s); align-self: center; }
  details { margin: 10px 0 0; font-size: var(--fs-s); }
  summary { cursor: pointer; }
  ul { max-height: 240px; overflow: auto; overflow-wrap: anywhere; margin: 6px 0 0; }
  li { margin: 4px 0; }
  .notice { margin: 10px 0 0; font-size: var(--fs-s); color: var(--success); }
  .error { margin: 10px 0 0; color: var(--danger); overflow-wrap: anywhere; }
</style>
