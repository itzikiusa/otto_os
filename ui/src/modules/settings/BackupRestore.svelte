<script lang="ts">
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import { sectionLabel } from './sections';
  import PageBody from '../../lib/components/PageBody.svelte';
  // Settings export/import and state backup/restore (C3).
  // Routes used:
  //   GET  /api/v1/settings/export  — download scrubbed settings JSON
  //   POST /api/v1/settings/import  — upload settings JSON to merge
  //   GET  /api/v1/state/backup     — download settings plus workspace manifest
  //   POST /api/v1/state/restore    — restore non-secret settings from snapshot
  import ConnectionsExport from './ConnectionsExport.svelte';
  import GitBackup from './GitBackup.svelte';
  import FullBackup from './FullBackup.svelte';
  import { api } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { downloadJson } from '../../lib/components/exporters';
  import SectionIntro from './SectionIntro.svelte';
  import Icon from '../../lib/components/Icon.svelte';

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
  // Real buttons drive hidden file inputs (a <label> around a display:none
  // input can't be reached with Tab).
  let importEl: HTMLInputElement | null = $state(null);
  let restoreEl: HTMLInputElement | null = $state(null);

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
      toasts.error('Couldn’t export settings', e instanceof Error ? e.message : String(e));
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
      toasts.error('Couldn’t import settings', `“${file.name}” isn't valid JSON.`);
      return;
    }

    const keyCount = Object.keys(parsed.settings ?? {}).length;
    const ok = await confirmer.ask(
      `Merge ${keyCount} setting${keyCount === 1 ? '' : 's'} from “${file.name}” into this Otto? Matching settings are overwritten; secret-keyed entries are rejected automatically.`,
      { title: 'Import settings', confirmLabel: 'Import', danger: false },
    );
    if (!ok) return;

    importing = true;
    try {
      await api.post('/settings/import', { settings: parsed.settings ?? {} });
      toasts.success('Settings imported', `${keyCount} entr${keyCount === 1 ? 'y' : 'ies'} merged.`);
    } catch (err) {
      toasts.error('Couldn’t import settings', err instanceof Error ? err.message : String(err));
    } finally {
      importing = false;
    }
  }

  // ---- State backup/restore -------------------------------------------------

  let backingUp = $state(false);
  let restoring = $state(false);

  async function downloadBackup(): Promise<void> {
    backingUp = true;
    try {
      const resp = await api.get<StateBackupResp>('/state/backup');
      downloadJson(resp, `otto-state-backup-${dateSlug()}.json`);
      toasts.success('Settings backup downloaded', `${resp.manifest.workspace_count} workspace${resp.manifest.workspace_count === 1 ? '' : 's'} in manifest.`);
    } catch (e) {
      toasts.error('Couldn’t download the settings backup', e instanceof Error ? e.message : String(e));
    } finally {
      backingUp = false;
    }
  }

  async function restoreState(e: Event): Promise<void> {
    const input = e.target as HTMLInputElement;
    const file = input.files?.[0];
    if (!file) return;
    input.value = '';

    let backup: StateBackupResp;
    try {
      backup = JSON.parse(await file.text()) as StateBackupResp;
    } catch {
      toasts.error('Couldn’t restore settings', `“${file.name}” isn't a valid settings backup.`);
      return;
    }

    const keyCount = Object.keys(backup.settings ?? {}).length;
    const snap = backup.manifest?.snapshot_at ? new Date(backup.manifest.snapshot_at).toLocaleString() : 'an unknown date';
    const ok = await confirmer.ask(
      `Overwrite ${keyCount} setting${keyCount === 1 ? '' : 's'} with the values in “${file.name}” (taken ${snap}, daemon ${backup.manifest?.daemon_version ?? 'unknown'})? The database, workspaces, sessions and credentials are not touched.`,
      { title: 'Restore settings', confirmLabel: 'Restore', danger: true },
    );
    if (!ok) return;

    restoring = true;
    try {
      await api.post('/state/restore', { backup, confirm: true });
      toasts.success('Settings restored', `${keyCount} setting${keyCount === 1 ? '' : 's'} applied.`);
    } catch (err) {
      toasts.error('Couldn’t restore settings', err instanceof Error ? err.message : String(err));
    } finally {
      restoring = false;
    }
  }

  // ---- helpers ---------------------------------------------------------------

  function dateSlug(): string {
    return new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
  }
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('backup')} subtitle="Back up Otto data, restore an archive or transfer settings" />
  <PageBody width="readable">
  <SectionIntro>
    Everything here stays on this Mac unless you push a Git backup to a remote. Credentials are never included
    unless you explicitly export connection passwords.
  </SectionIntro>

  <FullBackup />
  <GitBackup />
  <ConnectionsExport />

  <!-- Settings only: export/import + backup/restore with a manifest -->
  <section class="card pad" aria-label="Settings file">
    <h2 class="card-title">Settings only</h2>
    <p class="hint">
      Daemon settings as a JSON file — for moving your configuration to another Mac. Secrets (tokens,
      passwords, Keychain refs) are filtered out automatically; review the file before sharing it.
    </p>
    <div class="row-actions">
      <button class="btn" disabled={exporting} onclick={exportSettings}>
        <Icon name="download" size={13} /> {exporting ? 'Exporting…' : 'Export settings'}
      </button>
      <button class="btn" disabled={importing} title="Merge settings from a previously exported JSON file" onclick={() => importEl?.click()}>
        {importing ? 'Importing…' : 'Import settings…'}
      </button>
      <input
        type="file"
        accept=".json,application/json"
        class="file-input"
        disabled={importing}
        onchange={importSettings}
        bind:this={importEl}
        tabindex="-1"
        aria-hidden="true"
      />
    </div>
    {#if lastExport}
      <div class="hint-line">
        Last export: {lastExport.excluded_keys.length > 0
          ? `${lastExport.excluded_keys.length} secret key${lastExport.excluded_keys.length === 1 ? '' : 's'} excluded`
          : 'no secrets present'}
      </div>
    {/if}

    <h3 class="sub-title">Settings backup with manifest</h3>
    <p class="hint">
      The scrubbed settings plus a manifest (workspace names, migration level, daemon version). It does
      <em>not</em> include session data, terminal output, secrets or database rows. Restoring overwrites
      matching settings only — the database, workspaces, sessions and credentials are untouched, and the
      restore is audited.
    </p>
    <div class="row-actions">
      <button class="btn" disabled={backingUp} onclick={downloadBackup}>
        <Icon name="download" size={13} /> {backingUp ? 'Downloading…' : 'Download settings backup'}
      </button>
      <button class="btn danger" disabled={restoring} title="Choose a settings backup file to restore from" onclick={() => restoreEl?.click()}>
        {restoring ? 'Restoring…' : 'Restore from backup…'}
      </button>
      <input
        type="file"
        accept=".json,application/json"
        class="file-input"
        disabled={restoring}
        onchange={restoreState}
        bind:this={restoreEl}
        tabindex="-1"
        aria-hidden="true"
      />
    </div>
  </section>
  </PageBody>
</div>

<style>
  /* Section chrome: shared PageHeader bar + scrolling PageBody. */
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .card.pad {
    padding: 16px 18px;
    max-width: 880px;
    margin-bottom: 16px;
  }
  .card-title {
    margin: 0 0 6px;
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .sub-title {
    margin: 20px 0 6px;
    padding-top: 16px;
    border-top: 1px solid var(--border);
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .hint {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin: 0 0 12px;
    line-height: 1.5;
  }
  .hint-line {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-top: 8px;
  }
  .row-actions {
    display: flex;
    gap: 8px;
    flex-wrap: wrap;
    align-items: center;
  }
  .file-input {
    display: none;
  }
</style>
