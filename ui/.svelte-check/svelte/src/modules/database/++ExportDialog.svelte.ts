///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { pluralNoun } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
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
import { auth } from '../../lib/stores/auth.svelte';
import type { DbExportFormat, ExportToPathResp } from '../../lib/api/types';
import { postNdjsonStream } from '../../lib/api/client';
import { postForBlob } from '../../lib/api/files';
import Modal from '../../lib/components/Modal.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import { fmtBytes } from './results-format';
import { obviousWriteVerb } from '../../lib/database/sql-dialect';
import { lsGet, lsSet } from '../../lib/storage';
;

  interface Props {
    /** The statement to run uncapped (the tab's ran statement). */
    statement: string;
    connectionId: string;
    /**
     * The scope the on-screen result RAN with (the tab's `ran_node`; `null` = no
     * database). The export re-runs the statement, so it must hit the same
     * database the rows came from — not whatever the selector shows now.
     * Omitted → the explorer's active database.
     */
    node?: string | null;
    /** Export permission on the connection — re-checked before each run. */
    canExport: boolean;
    /** Prefill (an agent's `otto.ui_db_export`): the format / row limit to
     *  start from. The person still picks the folder + name and confirms. */
    initialFormat?: DbExportFormat;
    initialMaxRows?: number;
    /** Who asked for this export — shown in the dialog so it's attributed. */
    requestedBy?: string;
    /** Called once when the export finished writing (before `onclose`). */
    ondone?: (r: ExportToPathResp) => void;
    onclose: () => void;
  };;

  type ExportFmtOpt = { value: DbExportFormat; label: string };;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let {
    statement,
    connectionId,
    node,
    canExport,
    initialFormat,
    initialMaxRows,
    requestedBy,
    ondone,
    onclose,
  }: Props = $props();

  const TEXT_FORMATS: ExportFmtOpt[] = [
    { value: 'csv', label: 'CSV' },
    { value: 'csv_with_names', label: 'CSV (with header)' },
    { value: 'tsv', label: 'TSV' },
    { value: 'tsv_with_names', label: 'TSV (with header)' },
    { value: 'json', label: 'JSON (array)' },
    { value: 'ndjson', label: 'NDJSON / JSONEachRow' },
  ];
  // Rendered by the database server itself (ClickHouse `FORMAT …`).
  const BINARY_FORMATS: ExportFmtOpt[] = [
    { value: 'parquet', label: 'Parquet' },
    { value: 'arrow', label: 'Arrow' },
    { value: 'orc', label: 'ORC' },
  ];
  const clickhouse = $derived(database.capabilities?.engine === 'clickhouse');
  const EXPORT_FORMATS = $derived(clickhouse ? [...TEXT_FORMATS, ...BINARY_FORMATS] : TEXT_FORMATS);
  const EXT_BY_FORMAT: Record<DbExportFormat, string> = {
    csv: 'csv',
    csv_with_names: 'csv',
    tsv: 'tsv',
    tsv_with_names: 'tsv',
    json: 'json',
    ndjson: 'ndjson',
    parquet: 'parquet',
    arrow: 'arrow',
    orc: 'orc',
  };
  /** Where the file goes: a browser download (any editor), or a path on the
   *  Otto host (root only — it writes the daemon's disk). */
  let destination = $state<'download' | 'host'>('download');
  const LS_FORMAT = 'otto_db_export_format';
  const LS_DIR = 'otto_db_export_dir';

  // Guarded storage: a blocked accessor must not break the dialog, and a
  // throwing post-success write must not report a finished export as failed.
  function loadFormat(): DbExportFormat {
    const v = lsGet(LS_FORMAT) || 'csv';
    return [...TEXT_FORMATS, ...BINARY_FORMATS].some((f) => f.value === v) ? (v as DbExportFormat) : 'csv';
  }
  function loadDir(): string {
    return lsGet(LS_DIR) || '~/Downloads';
  }

  // Export RE-RUNS the statement uncapped. For an obvious write
  // (`UPDATE … RETURNING *`, `INSERT … RETURNING`) that would apply it a second
  // time — refuse here (the daemon also runs exports read-only; this is the
  // early, explained refusal, incl. for an agent's prefilled dialog).
  const writeVerb = $derived(obviousWriteVerb(statement ?? ''));

  // Default a filename from the statement (a leading table-ish token) or 'result'.
  function defaultExportName(): string {
    const fromStmt = statement?.match(/\bfrom\s+["'`]?([\w.]+)/i)?.[1];
    const base = (fromStmt || 'result').replace(/[^\w.-]+/g, '_').slice(0, 60) || 'result';
    return `${base}.${EXT_BY_FORMAT[exportFormat]}`;
  }

  // The dialog mounts fresh on every open, so the remembered format/directory
  // and the default file name are seeded right here.
  let pickingDir = $state(false);
  // A prefill (initialFormat / initialMaxRows) is read once, at mount.
  // svelte-ignore state_referenced_locally
  let exportFormat = $state<DbExportFormat>(initialFormat ?? loadFormat());
  let exportDir = $state<string>(loadDir());
  let exportName = $state(defaultExportName());
  // svelte-ignore state_referenced_locally
  let exportLimit = $state(initialMaxRows ? String(initialMaxRows) : '');
  /** Inline row-limit problem. `bind:value` on a number input yields a number
   *  (or null when cleared), so normalise before reading it. */
  const limitError = $derived.by(() => {
    const raw = String(exportLimit ?? '').trim();
    if (!raw) return null;
    const n = Number(raw);
    return Number.isFinite(n) && n > 0 ? null : 'Leave blank for all rows, or enter a positive number.';
  });
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
    exportName = exportName.replace(/\.(csv|tsv|json|ndjson|parquet|arrow|orc)$/i, '') + `.${ext}`;
  }

  function joinPath(dir: string, name: string): string {
    const d = dir.replace(/\/+$/, '');
    return `${d}/${name}`;
  }

  /** Stream the uncapped result through the daemon and save it as a browser
   *  download (`POST …/db/export`) — no daemon-disk access, so not root-only. */
  async function runDownload(): Promise<void> {
    if (!canExport || writeVerb || !connectionId || !statement || exportingPath || limitError) return;
    const name = exportName.trim() || defaultExportName();
    const rawLimit = String(exportLimit ?? '').trim();
    exportingPath = true;
    exportProgress = null;
    exportAbort = new AbortController();
    try {
      const blob = await postForBlob(
        `/connections/${connectionId}/db/export`,
        {
          statement,
          node: (node === undefined ? database.activeDb : node) ?? undefined,
          format: exportFormat,
          max_rows: rawLimit ? Number(rawLimit) : undefined,
        },
        exportAbort.signal,
      );
      const url = URL.createObjectURL(blob);
      const a = document.createElement('a');
      a.href = url;
      a.download = name;
      document.body.appendChild(a);
      a.click();
      a.remove();
      setTimeout(() => URL.revokeObjectURL(url), 1500);
      lsSet(LS_FORMAT, exportFormat);
      onclose();
      toasts.success('Exported', `${fmtBytes(blob.size)} → ${name}`);
    } catch (e) {
      if (e instanceof DOMException && e.name === 'AbortError') toasts.info('Export canceled');
      else toastError('Couldn’t export', e);
    } finally {
      exportingPath = false;
      exportAbort = null;
    }
  }

  async function runPathExport(): Promise<void> {
    if (!canExport || writeVerb || !auth.isRoot) return;
    if (!connectionId || !statement || exportingPath) return;
    const name = exportName.trim() || defaultExportName();
    const dir = exportDir.trim() || '~/Downloads';
    const localPath = joinPath(dir, name);
    // An invalid limit is shown inline and keeps Export disabled — this only
    // guards a call that slipped past the disabled button.
    if (limitError) return;
    const rawLimit = String(exportLimit ?? '').trim();
    const maxRows = rawLimit ? Number(rawLimit) : undefined;
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
          node: (node === undefined ? database.activeDb : node) ?? undefined,
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
        lsSet(LS_FORMAT, exportFormat);
        lsSet(LS_DIR, dir);
        ondone?.(r);
        onclose();
        toasts.success(
          'Exported',
          `${r.rows.toLocaleString()} ${pluralNoun(r.rows, 'row')} · ${fmtBytes(r.bytes)} → ${r.local_path}`,
        );
      }
    } catch (e) {
      // A user-initiated cancel isn't a failure — the partial file stays where
      // the export was writing it.
      if (e instanceof DOMException && e.name === 'AbortError') {
        toasts.info('Export canceled', 'The partially written file was left in place.');
      } else {
        toastError('Couldn’t export', e);
      }
    } finally {
      exportingPath = false;
      exportProgress = null;
      exportAbort = null;
    }
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  $effect(() => {
    if (writeVerb) announce(`This statement is a write (${writeVerb}). Export would apply it again.`, { assertive: true });
  });
  $effect(() => { if (exportingPath) announce('Exporting all rows…'); });
;
async () => {

  { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {      children:() => { return __sveltets_2_any(0); },"title":`Export all rows`,"width":520,"onclose":() => {
    exportAbort?.abort();
    onclose();
  },footer:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {       "class":`btn`,"onclick":() => (exportingPath ? exportAbort?.abort() : onclose()),"title":exportingPath ? 'Stop the running export (the partial file is left in place)' : undefined,});
      exportingPath ? 'Cancel export' : 'Cancel';
     }
     { svelteHTML.createElement("button", {         "class":`btn primary`,"onclick":() => void (destination === 'download' ? runDownload() : runPathExport()),"disabled":exportingPath || !!writeVerb || (destination === 'host' && !auth.isRoot) || limitError !== null,"title":writeVerb ? 'Export re-runs the statement — not available for a write' : undefined,});
      exportingPath ? 'Exporting…' : 'Export all';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { svelteHTML.createElement("div", { "class":`exp-form`,});
    if(requestedBy){
       { svelteHTML.createElement("p", {  "class":`exp-agent`,...__sveltets_2_empty({"data-agent-target":true}),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});}requestedBy;          
       }
    }
    if(writeVerb){
       { svelteHTML.createElement("p", { "class":`exp-write`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}     writeVerb;   
                       
       }
    }
     { svelteHTML.createElement("p", { "class":`exp-hint`,});
           { svelteHTML.createElement("strong", {});  }        
                           
     }

     { svelteHTML.createElement("div", {     "class":`exp-row`,"role":`radiogroup`,"aria-label":`Save to`,});
       { svelteHTML.createElement("span", { "class":`exp-label`,});  }
       { svelteHTML.createElement("div", { "class":`exp-dest-choice`,});
         { svelteHTML.createElement("label", {}); { svelteHTML.createElement("input", {     "type":`radio`,"value":`download`,});destination = __sveltets_2_any(null);}    }
         { svelteHTML.createElement("label", { "title":auth.isRoot ? undefined : 'Only the root user can write files on the Otto host computer',});
           { svelteHTML.createElement("input", {       "type":`radio`,"value":`host`,"disabled":!auth.isRoot,});destination = __sveltets_2_any(null);}   
         }
       }
     }

     { svelteHTML.createElement("label", { "class":`exp-row`,});
       { svelteHTML.createElement("span", { "class":`exp-label`,});  }
       { svelteHTML.createElement("select", {     "class":`exp-select`,"bind:value":exportFormat,"onchange":onFormatChange,});/*Ωignore_startΩ*/() => exportFormat = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let f of __sveltets_2_ensureArray(EXPORT_FORMATS)){f.value;
           { svelteHTML.createElement("option", { "value":f.value,});f.label; }
        }
       }
     }

    if(destination === 'host'){
     { svelteHTML.createElement("div", { "class":`exp-row`,});
       { svelteHTML.createElement("span", { "class":`exp-label`,});  }
       { svelteHTML.createElement("div", { "class":`exp-dir`,});
         { svelteHTML.createElement("input", {            "dir":`ltr`,"aria-label":`Export folder`,"class":`exp-input mono`,"bind:value":exportDir,"spellcheck":`false`,"placeholder":`~/Downloads`,});/*Ωignore_startΩ*/() => exportDir = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":() => (pickingDir = true),"title":`Browse the daemon host`,});
           { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`folder`,"size":12,}});}
         }
       }
     }

    }

     { svelteHTML.createElement("label", { "class":`exp-row`,});
       { svelteHTML.createElement("span", { "class":`exp-label`,});  }
       { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`exp-input mono`,"bind:value":exportName,"spellcheck":`false`,"placeholder":`result.csv`,});/*Ωignore_startΩ*/() => exportName = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }

     { svelteHTML.createElement("label", { "class":`exp-row`,});
       { svelteHTML.createElement("span", { "class":`exp-label`,});  }
       { svelteHTML.createElement("input", {                 "class":`exp-input mono`,"bind:value":exportLimit,"type":`number`,"min":`1`,"spellcheck":`false`,"placeholder":`all rows`,"aria-invalid":limitError ? 'true' : undefined,"aria-describedby":limitError ? 'exp-limit-err' : undefined,});/*Ωignore_startΩ*/() => exportLimit = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
    if(limitError){ { svelteHTML.createElement("p", {   "class":`exp-err`,"id":`exp-limit-err`,});limitError; }}

    if(destination === 'host'){
     { svelteHTML.createElement("div", {   "class":`exp-dest mono`,"title":`Resolved destination on the daemon host`,});
       joinPath(exportDir.trim() || '~/Downloads', exportName.trim() || defaultExportName());
     }
    }

    if(exportingPath){
       { svelteHTML.createElement("div", { "class":`exp-progress`,});
         { svelteHTML.createElement("div", {     "class":`indeterminate`,"role":`progressbar`,"aria-label":`Exporting`,}); }
         { svelteHTML.createElement("div", { "class":`exp-prog-text mono`,});
          exportProgress ? fmtBytes(exportProgress.bytes) : '0 B'; 
         }
       }
    }
   }

  
 Modal}

if(pickingDir){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {         "title":`Choose export folder (daemon host)`,"start":exportDir,"onpick":(p) => {
      exportDir = p;
      pickingDir = false;
    },"onclose":() => (pickingDir = false),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ExportDialog__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ExportDialog__SvelteComponent_ = ReturnType<typeof ExportDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default ExportDialog__SvelteComponent_;