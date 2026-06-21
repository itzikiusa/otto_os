<script lang="ts">
  // Settings export/import and state backup/restore (C3).
  // Routes used:
  //   GET  /api/v1/settings/export  — download scrubbed settings JSON
  //   POST /api/v1/settings/import  — upload settings JSON to merge
  //   GET  /api/v1/state/backup     — download full non-secret state snapshot
  //   POST /api/v1/state/restore    — restore non-secret settings from snapshot
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { downloadJson } from '../../lib/components/exporters';

  // ---- Module-local types (mirrors the Rust response structs) ---------------

  interface SettingsExportResp {
    settings: Record<string, unknown>;
    excluded_keys: string[];
    export_format: number;
  }

  interface StateManifest {
    workspace_names: string[];
    workspace_count: number;
    migration_level: string;
    daemon_version: string;
    snapshot_at: string;
  }

  interface StateBackupResp {
    settings: Record<string, unknown>;
    excluded_keys: string[];
    manifest: StateManifest;
    backup_format: number;
  }

  // ---- Settings export/import -----------------------------------------------

  let exporting = $state(false);
  let importing = $state(false);
  let lastExport: SettingsExportResp | null = $state(null);

  async function exportSettings(): Promise<void> {
    exporting = true;
    try {
      const resp = await api.get<SettingsExportResp>('/settings/export');
      lastExport = resp;
      downloadJson(resp, `otto-settings-${dateSlug()}.json`);
      const excCount = resp.excluded_keys.length;
      toasts.success(
        'Settings exported',
        excCount > 0
          ? `${excCount} secret key${excCount === 1 ? '' : 's'} excluded`
          : 'No secrets in export',
      );
    } catch (e) {
      toasts.error('Export failed', e instanceof Error ? e.message : String(e));
    } finally {
      exporting = false;
    }
  }

  async function importSettings(e: Event): Promise<void> {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    // Reset the file input so the same file can be picked again after an error.
    input.value = '';

    let parsed: SettingsExportResp;
    try {
      parsed = JSON.parse(await file.text()) as SettingsExportResp;
    } catch {
      toasts.error('Import failed', 'Could not parse the JSON file.');
      return;
    }

    const keyCount = Object.keys(parsed.settings ?? {}).length;
    const ok = await confirmer.ask(
      `Merge ${keyCount} setting${keyCount === 1 ? '' : 's'} from "${file.name}"?\n` +
        'Secret-keyed entries will be rejected automatically.',
      { title: 'Import Settings', confirmLabel: 'Import' },
    );
    if (!ok) return;

    importing = true;
    try {
      await api.post('/settings/import', { settings: parsed.settings ?? {} });
      toasts.success('Settings imported', `${keyCount} entr${keyCount === 1 ? 'y' : 'ies'} merged.`);
    } catch (err) {
      toasts.error('Import failed', err instanceof Error ? err.message : String(err));
    } finally {
      importing = false;
    }
  }

  // ---- State backup/restore -------------------------------------------------

  let backingUp = $state(false);
  let restoring = $state(false);
  /** Typed restore-confirm string — must match the sentinel below. */
  let restoreConfirmText = $state('');
  const RESTORE_SENTINEL = 'restore';

  async function downloadBackup(): Promise<void> {
    backingUp = true;
    try {
      const resp = await api.get<StateBackupResp>('/state/backup');
      downloadJson(resp, `otto-state-backup-${dateSlug()}.json`);
      toasts.success('State backup downloaded', `${resp.manifest.workspace_count} workspace${resp.manifest.workspace_count === 1 ? '' : 's'} in manifest.`);
    } catch (e) {
      toasts.error('Backup failed', e instanceof Error ? e.message : String(e));
    } finally {
      backingUp = false;
    }
  }

  async function restoreState(e: Event): Promise<void> {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    input.value = '';

    if (restoreConfirmText.trim().toLowerCase() !== RESTORE_SENTINEL) {
      toasts.error('Confirm required', `Type "${RESTORE_SENTINEL}" to enable restore.`);
      return;
    }

    let backup: StateBackupResp;
    try {
      backup = JSON.parse(await file.text()) as StateBackupResp;
    } catch {
      toasts.error('Restore failed', 'Could not parse the backup file.');
      return;
    }

    const keyCount = Object.keys(backup.settings ?? {}).length;
    const ok = await confirmer.ask(
      `Restore ${keyCount} setting${keyCount === 1 ? '' : 's'} from backup "${file.name}"?\n` +
        `Snapshot: ${backup.manifest?.snapshot_at ?? 'unknown'}\n` +
        `Daemon: ${backup.manifest?.daemon_version ?? 'unknown'}\n\n` +
        'This DOES NOT wipe the database or delete sessions.',
      { title: 'Restore State', confirmLabel: 'Restore', danger: true },
    );
    if (!ok) return;

    restoring = true;
    try {
      await api.post('/state/restore', { backup, confirm: true });
      toasts.success('State restored', `${keyCount} setting${keyCount === 1 ? '' : 's'} applied.`);
      restoreConfirmText = '';
    } catch (err) {
      toasts.error('Restore failed', err instanceof Error ? err.message : String(err));
    } finally {
      restoring = false;
    }
  }

  // ---- helpers ---------------------------------------------------------------

  function dateSlug(): string {
    return new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
  }
</script>

<div class="page">
  <div class="page-header">
    <div>
      <h1>Backup &amp; Restore</h1>
      <div class="sub">Export settings, download a full non-secret state snapshot, or restore from a backup.</div>
    </div>
  </div>

  <!-- Settings export / import -->
  <div class="section-title">Settings</div>
  <div class="card pad">
    <p class="hint">
      Export downloads a JSON file with all daemon settings. Secrets (tokens, passwords, Keychain
      refs) are excluded automatically — the file is safe to store outside this machine.
    </p>
    <div class="row-actions">
      <button class="btn primary" disabled={exporting} onclick={exportSettings}>
        {exporting ? 'Exporting…' : 'Export settings'}
      </button>

      <label class="btn" class:disabled={importing} title="Import settings from a previously exported JSON file">
        {importing ? 'Importing…' : 'Import settings'}
        <input
          type="file"
          accept=".json,application/json"
          class="file-input"
          disabled={importing}
          onchange={importSettings}
        />
      </label>
    </div>

    {#if lastExport}
      <div class="hint-line dim">
        Last export: {lastExport.excluded_keys.length > 0
          ? `${lastExport.excluded_keys.length} secret key${lastExport.excluded_keys.length === 1 ? '' : 's'} excluded`
          : 'no secrets present'}
      </div>
    {/if}
  </div>

  <!-- State backup / restore -->
  <div class="section-title">State Backup</div>
  <div class="card pad">
    <p class="hint">
      A state backup bundles the scrubbed settings with a manifest (workspace names, migration
      level, daemon version). It does <em>not</em> include session data, PTY output, secrets, or
      raw database rows.
    </p>
    <button class="btn primary" disabled={backingUp} onclick={downloadBackup}>
      {backingUp ? 'Downloading…' : 'Download state backup'}
    </button>
  </div>

  <div class="section-title">Restore</div>
  <div class="card pad">
    <p class="hint">
      Restoring applies the non-secret settings from a backup file. It does <em>not</em> wipe the
      database, delete workspaces, or touch credentials. The daemon restarts any reloaded providers
      immediately.
    </p>
    <div class="confirm-row">
      <label for="restore-confirm" class="confirm-label">
        Type <span class="mono">{RESTORE_SENTINEL}</span> to unlock restore:
      </label>
      <input
        id="restore-confirm"
        class="input mono"
        type="text"
        placeholder={RESTORE_SENTINEL}
        bind:value={restoreConfirmText}
        autocomplete="off"
        spellcheck={false}
      />
    </div>

    <label
      class="btn danger"
      class:disabled={restoring || restoreConfirmText.trim().toLowerCase() !== RESTORE_SENTINEL}
      title="Select a backup JSON file to restore from"
    >
      {restoring ? 'Restoring…' : 'Restore from backup'}
      <input
        type="file"
        accept=".json,application/json"
        class="file-input"
        disabled={restoring || restoreConfirmText.trim().toLowerCase() !== RESTORE_SENTINEL}
        onchange={restoreState}
      />
    </label>

    <p class="warn-note">
      This action is audited. The restore merges settings — existing workspaces, sessions, and
      credentials are unaffected.
    </p>
  </div>
</div>

<style>
  .card.pad {
    padding: 14px 16px;
    max-width: 540px;
    margin-bottom: 8px;
  }
  .hint {
    font-size: 12px;
    color: var(--text-dim);
    margin: 0 0 12px;
    line-height: 1.5;
  }
  .hint-line {
    font-size: 11.5px;
    margin-top: 8px;
  }
  .row-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    align-items: center;
  }
  .confirm-row {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-bottom: 10px;
  }
  .confirm-label {
    font-size: 12px;
    color: var(--text-dim);
  }
  .input {
    max-width: 200px;
  }
  .warn-note {
    font-size: 11.5px;
    color: var(--text-dim);
    margin: 10px 0 0;
  }
  /* Hide the real file input but keep it accessible */
  .file-input {
    display: none;
  }
  label.btn {
    display: inline-flex;
    align-items: center;
    cursor: pointer;
    user-select: none;
  }
  label.btn.disabled,
  label.btn:has(input:disabled) {
    opacity: 0.5;
    pointer-events: none;
    cursor: not-allowed;
  }
</style>
