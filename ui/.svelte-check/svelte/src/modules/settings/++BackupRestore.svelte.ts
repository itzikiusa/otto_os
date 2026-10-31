///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import PageHeader from '../../lib/components/PageHeader.svelte';
import { toastError } from '../../lib/toastError';
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
import DatabaseCompact from './DatabaseCompact.svelte';
import DesignStorage from './DesignStorage.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { api } from '../../lib/api/client';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { downloadJson } from '../../lib/components/exporters';
import SectionIntro from './SectionIntro.svelte';
import Icon from '../../lib/components/Icon.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
          ? `${plural(excCount, 'secret key')} excluded`
          : 'No secrets in export',
      );
    } catch (e) {
      toastError('Couldn’t export settings', e);
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
      toasts.error('Couldn’t import settings', `“${file.name}” isn’t valid JSON.`);
      return;
    }

    const keyCount = Object.keys(parsed.settings ?? {}).length;
    const ok = await confirmer.ask(
      `Merge ${plural(keyCount, 'setting')} from “${file.name}” into this Otto? Matching settings are overwritten; secret-keyed entries are rejected automatically.`,
      { title: 'Import settings', confirmLabel: 'Import', danger: false },
    );
    if (!ok) return;

    importing = true;
    try {
      await api.post('/settings/import', { settings: parsed.settings ?? {} });
      toasts.success('Settings imported', `${keyCount} entr${keyCount === 1 ? 'y' : 'ies'} merged.`);
    } catch (err) {
      toastError('Couldn’t import settings', err);
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
      toasts.success('Settings backup downloaded', `${plural(resp.manifest.workspace_count, 'workspace')} in manifest.`);
    } catch (e) {
      toastError('Couldn’t download the settings backup', e);
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
      toasts.error('Couldn’t restore settings', `“${file.name}” isn’t a valid settings backup.`);
      return;
    }

    const keyCount = Object.keys(backup.settings ?? {}).length;
    const snap = backup.manifest?.snapshot_at ? new Date(backup.manifest.snapshot_at).toLocaleString() : 'an unknown date';
    const ok = await confirmer.ask(
      `Overwrite ${plural(keyCount, 'setting')} with the values in “${file.name}” (taken ${snap}, daemon ${backup.manifest?.daemon_version ?? 'unknown'})? The database, workspaces, sessions and credentials are not touched.`,
      { title: 'Restore settings', confirmLabel: 'Restore', danger: true },
    );
    if (!ok) return;

    restoring = true;
    try {
      await api.post('/state/restore', { backup, confirm: true });
      toasts.success('Settings restored', `${plural(keyCount, 'setting')} applied.`);
    } catch (err) {
      toastError('Couldn’t restore settings', err);
    } finally {
      restoring = false;
    }
  }

  // ---- helpers ---------------------------------------------------------------

  function dateSlug(): string {
    return new Date().toISOString().replace(/[:.]/g, '-').slice(0, 19);
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`settings-section`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":sectionLabel('backup'),"subtitle":`Archives, Git sync and exports`,}});}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"width":`readable`,}});
   { const $$_ortnInoitceS2C = __sveltets_2_ensureComponent(SectionIntro); new $$_ortnInoitceS2C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
                      
         
   SectionIntro}

   { const $$_pukcaBlluF2C = __sveltets_2_ensureComponent(FullBackup); new $$_pukcaBlluF2C({ target: __sveltets_2_any(), props: {}});}
   { const $$_pukcaBtiG2C = __sveltets_2_ensureComponent(GitBackup); new $$_pukcaBtiG2C({ target: __sveltets_2_any(), props: {}});}
   { const $$_tropxEsnoitcennoC2C = __sveltets_2_ensureComponent(ConnectionsExport); new $$_tropxEsnoitcennoC2C({ target: __sveltets_2_any(), props: {}});}
  if(auth.isRoot){ { const $$_tcapmoCesabataD2C = __sveltets_2_ensureComponent(DatabaseCompact); new $$_tcapmoCesabataD2C({ target: __sveltets_2_any(), props: {}});} { const $$_egarotSngiseD2C = __sveltets_2_ensureComponent(DesignStorage); new $$_egarotSngiseD2C({ target: __sveltets_2_any(), props: {}});}}

  
   { svelteHTML.createElement("section", {   "class":`card pad`,"aria-label":`Settings file`,});
     { svelteHTML.createElement("h2", { "class":`card-title`,});  }
     { svelteHTML.createElement("p", { "class":`hint`,});
                     
                  
     }
     { svelteHTML.createElement("div", { "class":`row-actions`,});
       { svelteHTML.createElement("button", {     "class":`btn`,"disabled":exporting,"onclick":exportSettings,});
         { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":13,}});} exporting ? 'Exporting…' : 'Export settings';
       }
       { svelteHTML.createElement("button", {       "class":`btn`,"disabled":importing,"title":`Merge settings from a previously exported JSON file`,"onclick":() => importEl?.click(),});
        importing ? 'Importing…' : 'Import settings…';
       }
       { const $$_input4 = svelteHTML.createElement("input", {                "type":`file`,"accept":`.json,application/json`,"class":`file-input`,"disabled":importing,"onchange":importSettings,"tabindex":-1,"aria-hidden":`true`,});importEl = $$_input4;}
     }
    if(lastExport){
       { svelteHTML.createElement("div", { "class":`hint-line`,});
          lastExport.excluded_keys.length > 0
          ? `${plural(lastExport.excluded_keys.length, 'secret key')} excluded`
          : 'no secrets present';
       }
    }

     { svelteHTML.createElement("h3", { "class":`sub-title`,});    }
     { svelteHTML.createElement("p", { "class":`hint`,});
                   
       { svelteHTML.createElement("em", {});  }           
                   
        
     }
     { svelteHTML.createElement("div", { "class":`row-actions`,});
       { svelteHTML.createElement("button", {     "class":`btn`,"disabled":backingUp,"onclick":downloadBackup,});
         { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":13,}});} backingUp ? 'Downloading…' : 'Download settings backup';
       }
       { svelteHTML.createElement("button", {       "class":`btn danger`,"disabled":restoring,"title":`Choose a settings backup file to restore from`,"onclick":() => restoreEl?.click(),});
        restoring ? 'Restoring…' : 'Restore from backup…';
       }
       { const $$_input4 = svelteHTML.createElement("input", {                "type":`file`,"accept":`.json,application/json`,"class":`file-input`,"disabled":restoring,"onchange":restoreState,"tabindex":-1,"aria-hidden":`true`,});restoreEl = $$_input4;}
     }
   }
   PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BackupRestore__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BackupRestore__SvelteComponent_ = ReturnType<typeof BackupRestore__SvelteComponent_>;
/*Ωignore_endΩ*/export default BackupRestore__SvelteComponent_;