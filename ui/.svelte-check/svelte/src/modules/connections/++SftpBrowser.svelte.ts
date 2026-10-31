///<reference types="svelte" />
;
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
// SFTP file browser for an SSH connection — a "MobaXterm-lite" pane that
// browses, transfers, and edits files over the connection's existing SSH
// auth (the daemon drives the system `sftp` binary). Opened from the
// Connections page; no terminal session required.
import type { Connection, SftpEntry, SftpTransfer } from '../../lib/api/types';
import { toastError } from '../../lib/toastError';
import { resourceAccess } from '../../lib/stores/resource-access.svelte';
import { sftp } from '../../lib/stores/sftp.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { confirmProd, isProdEnv } from '../../lib/confirmProd';
import EnvBadge from '../../lib/components/EnvBadge.svelte';
import Modal from '../../lib/components/Modal.svelte';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import { TableWindow } from '../../lib/tableWindow.svelte';
import { pollWhileVisible, type Poller } from '../../lib/poll';
import { untrack } from 'svelte';
;

  interface Props {
    conn: Connection;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { conn, onclose }: Props = $props();

  const canWrite = $derived(resourceAccess.can('connection',conn.id,'sftp_write','connections','edit'));
  $effect(()=>{void resourceAccess.load('connection',conn.id);});
  const view = $derived(sftp.state(conn.id));
  // A production host: every write confirms against PRODUCTION (confirmProd).
  const isProd = $derived(isProdEnv(conn.environment));
  const hostLabel = (): string => `${conn.name}${view.cwd ? ` · ${view.cwd}` : ''}`;

  // Picker overlays: download-dir picker (per remote file) and upload-file picker.
  let downloadFor: SftpEntry | null = $state(null);
  let uploadOpen = $state(false);
  let busy: Record<string, boolean> = $state({});

  // Text-viewer modal state.
  let viewing: { name: string; text: string; truncated: boolean } | null = $state(null);

  // Search box — filters the current directory's listing by name (case-insensitive).
  let query = $state('');
  const shownEntries = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return q ? view.entries.filter((e) => e.name.toLowerCase().includes(q)) : view.entries;
  });

  // Window the listing: a 50k-entry directory (/var/log, a spool, /tmp) took
  // ~6 s of layout and 1M nodes rendered whole. Only the visible rows (plus
  // overscan) are mounted, between two spacers.
  const tw = new TableWindow(300);
  let listEl = $state<HTMLDivElement | null>(null);
  const win = $derived(tw.range(shownEntries.length));
  const rowsWindow = $derived(shownEntries.slice(win.start, win.end));
  $effect(() => {
    void rowsWindow;
    tw.measure(listEl, '.sftp-row.entry');
  });
  // ⌘F over every (filtered) entry, not just the mounted slice.
  $effect(() =>
    tw.findRows(
      () => listEl,
      () => shownEntries,
      (e) =>
        [e.name, e.symlink_target ? `→ ${e.symlink_target}` : '', e.kind === 'dir' ? '' : humanSize(e.size), e.mtime ?? '', e.perms].join('\n'),
      '.sftp-row.entry',
    ),
  );
  // A new directory (or filter) starts at the top.
  $effect(() => {
    void view.entries;
    void query;
    tw.reset(listEl);
  });

  // Initial load: resolve the remote home/working dir then list it.
  $effect(() => {
    if (!view.attempted && !view.loading) void sftp.list(conn.id);
  });

  // Restore in-flight transfers after reopening the browser. A failed poll is
  // surfaced once; authentication failures never create an automatic retry loop.
  // Chained (never overlaps), paused while the window is hidden, and fast
  // (1.5 s) only while a transfer is live — an idle browser checks every 15 s
  // (was 1.5 s forever, hidden or not).
  $effect(() => {
    const id = conn.id;
    if (!canWrite) return;
    let poller: Poller | undefined;
    const live = (): boolean => sftp.state(id).transfers.some((t) => t.status === 'running' || t.status === 'finalizing');
    untrack(() => {
      poller = pollWhileVisible(
        async () => {
          try {
            await sftp.loadTransfers(id);
          } catch (error) {
            poller?.stop();
            toastError('Couldn’t refresh transfers', error);
          }
        },
        { get ms() { return live() ? 1500 : 15_000; }, floorMs: 1000 },
      );
    });
    return () => poller?.stop();
  });

  // Reset the filter whenever we navigate to a different directory.
  $effect(() => {
    view.cwd;
    query = '';
  });

  function isTextLike(e: SftpEntry): boolean {
    // Offer the viewer for small-ish files; the backend caps at 1 MiB anyway.
    return e.kind === 'file' && e.size <= 1024 * 1024;
  }

  // Transfer status enum → words (the raw value read "timed_out").
  const TRANSFER_STATUS: Record<SftpTransfer['status'], string> = {
    running: 'Running', finalizing: 'Finalizing', completed: 'Completed', cancelled: 'Canceled',
    failed: 'Failed', timed_out: 'Timed out', outcome_unknown: 'Outcome unknown',
  };
  const WRITE_DENIED = "You don’t have write access to this connection’s files";
  // Downloads/uploads read or write a file on the Otto host computer (S6-302).
  const HOST_PATH_DENIED = "Only the root user can transfer files to or from the Otto host computer";

  function humanSize(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const units = ['KB', 'MB', 'GB', 'TB'];
    let v = bytes / 1024;
    let i = 0;
    while (v >= 1024 && i < units.length - 1) {
      v /= 1024;
      i++;
    }
    return `${v >= 100 ? v.toFixed(0) : v.toFixed(1)} ${units[i]}`;
  }

  // Breadcrumb segments of the current absolute path.
  const crumbs = $derived(buildCrumbs(view.cwd));
  function buildCrumbs(cwd: string): { label: string; path: string }[] {
    if (!cwd) return [];
    const parts = cwd.split('/').filter(Boolean);
    const out: { label: string; path: string }[] = [{ label: '/', path: '/' }];
    let acc = '';
    for (const part of parts) {
      acc += `/${part}`;
      out.push({ label: part, path: acc });
    }
    return out;
  }

  function onRowActivate(e: SftpEntry): void {
    if (e.kind === 'dir' || (e.kind === 'symlink' && !isTextLike(e))) {
      void sftp.navigate(conn.id, sftp.childPath(conn.id, e.name));
    } else if (isTextLike(e)) {
      void openViewer(e);
    }
  }

  async function openViewer(e: SftpEntry): Promise<void> {
    const path = sftp.childPath(conn.id, e.name);
    busy = { ...busy, [e.name]: true };
    try {
      const resp = await sftp.readText(conn.id, path);
      viewing = { name: e.name, text: resp.text, truncated: resp.truncated };
    } catch (err) {
      toastError('Couldn’t read the file', err);
    } finally {
      busy = { ...busy, [e.name]: false };
    }
  }

  async function doDownload(localDir: string): Promise<void> {
    const e = downloadFor;
    downloadFor = null;
    if (!e) return;
    const path = sftp.childPath(conn.id, e.name);
    busy = { ...busy, [e.name]: true };
    try {
      const r = await sftp.download(conn.id, path, localDir);
      toasts.success('Downloaded', `${e.name} → ${r.local_path} (${humanSize(r.bytes)})`);
    } catch (err) {
      toastError('Couldn’t download the file', err);
    } finally {
      busy = { ...busy, [e.name]: false };
    }
  }

  async function doUpload(localPath: string): Promise<void> {
    uploadOpen = false;
    // Overwriting an existing remote file asks first on EVERY environment (the
    // upload replaces it with no undo); prod gets its own confirm below anyway.
    const base = localPath.split(/[\\/]/).pop() ?? '';
    if (!isProd && base && view.entries.some((e) => e.name === base)) {
      const ok = await confirmer.ask(`“${base}” already exists in ${view.cwd || '/'} on ${hostLabel()}. Uploading replaces it.`, {
        title: 'Replace the existing file?',
        confirmLabel: 'Replace',
        danger: true,
      });
      if (!ok) return;
    }
    if (isProd) {
      const ok = await confirmProd({
        env: conn.environment,
        verb: 'Upload',
        where: `SFTP ${hostLabel()}`,
        what: `${localPath} → ${view.cwd || '/'} (an existing file with the same name is overwritten)`,
      });
      if (!ok) return;
    }
    try {
      await sftp.upload(conn.id, localPath);
      toasts.success('Uploaded', localPath);
      await sftp.refresh(conn.id);
    } catch (err) {
      toastError('Couldn’t upload the file', err);
    }
  }

  async function newFolder(): Promise<void> {
    const name = await confirmer.promptText('New folder name', {
      title: 'New folder',
      confirmLabel: 'Create',
    });
    if (!name) return;
    try {
      await sftp.mkdir(conn.id, name);
      await sftp.refresh(conn.id);
    } catch (err) {
      toastError('Couldn’t create the item', err);
    }
  }

  async function renameEntry(e: SftpEntry): Promise<void> {
    const next = await confirmer.promptText(`Rename “${e.name}” to`, {
      title: 'Rename',
      confirmLabel: 'Rename',
      initial: e.name,
    });
    if (!next || next === e.name) return;
    if (isProd) {
      const ok = await confirmProd({
        env: conn.environment,
        verb: 'Rename',
        where: `SFTP ${hostLabel()}`,
        what: `“${e.name}” → “${next}”`,
      });
      if (!ok) return;
    }
    const path = sftp.childPath(conn.id, e.name);
    try {
      await sftp.rename(conn.id, path, next);
      await sftp.refresh(conn.id);
    } catch (err) {
      toastError('Couldn’t rename the item', err);
    }
  }

  async function deleteEntry(e: SftpEntry): Promise<void> {
    const isDir = e.kind === 'dir';
    const ok = await confirmProd({
      env: conn.environment,
      verb: 'Delete',
      title: isDir ? 'Delete directory?' : 'Delete file?',
      where: `SFTP ${hostLabel()}`,
      what: `${isDir ? 'Directory' : 'File'} “${e.name}”${isDir ? ' (must be empty)' : ''}. This can’t be undone.`,
      typed: isProd ? e.name : undefined,
      danger: true,
    });
    if (!ok) return;
    const path = sftp.childPath(conn.id, e.name);
    try {
      await sftp.remove(conn.id, path, isDir);
      await sftp.refresh(conn.id);
    } catch (err) {
      toastError('Couldn’t delete the item', err);
    }
  }

  function iconFor(e: SftpEntry): IconName {
    if (e.kind === 'dir') return 'folder';
    if (e.kind === 'symlink') return 'link';
    return 'file';
  }

  function close(): void {
    onclose();
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Files — ${conn.name}`,"width":860,"onclose":close,}});
   { svelteHTML.createElement("div", { "class":`sftp`,});
    if(isProd){
       { svelteHTML.createElement("div", {   "class":`prod-banner`,"role":`note`,});
         { const $$_egdaBvnE3C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE3C({ target: __sveltets_2_any(), props: {  "env":conn.environment,}});}
         { svelteHTML.createElement("span", {});            }
       }
    }
    
     { svelteHTML.createElement("div", { "class":`toolbar`,});
       { svelteHTML.createElement("button", {           "class":`btn small`,"title":`Up one folder`,"aria-label":`Up one folder`,"disabled":!view.cwd || view.cwd === '/',"onclick":() => sftp.up(conn.id),});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":13,}});}
       }
       { svelteHTML.createElement("button", {         "class":`btn small`,"title":`Refresh`,"aria-label":`Refresh`,"onclick":() => sftp.refresh(conn.id),});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});}
       }
       { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":!canWrite,"title":canWrite ? undefined : WRITE_DENIED,"onclick":newFolder,});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  
       }
       { svelteHTML.createElement("button", {       "class":`btn small primary`,"disabled":!canWrite || !auth.isRoot,"title":!auth.isRoot ? HOST_PATH_DENIED : canWrite ? undefined : WRITE_DENIED,"onclick":() => (uploadOpen = true),});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":12,}});} 
       }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`sftp-search`,"type":`text`,"placeholder":`Filter this folder…`,"aria-label":`Filter files in this directory`,"bind:value":query,});/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }

    if(view.transfers.length > 0){
       { svelteHTML.createElement("div", {   "class":`transfers`,"aria-label":`File transfers`,});
           for(let transfer of __sveltets_2_ensureArray(view.transfers)){transfer.id;
           { svelteHTML.createElement("div", { "class":`transfer`,});
             { svelteHTML.createElement("span", {     "class":`ellipsis`,"dir":`ltr`,"title":transfer.remote_path,});transfer.direction === 'upload' ? 'Upload' : 'Download'; transfer.remote_path; }
             { svelteHTML.createElement("span", {});TRANSFER_STATUS[transfer.status] ?? transfer.status;  humanSize(transfer.bytes);transfer.total_bytes !== null ? ` / ${humanSize(transfer.total_bytes)}` : '';  transfer.elapsed_secs;  }
            if(transfer.status === 'running'){
               { svelteHTML.createElement("progress", {     "max":transfer.total_bytes ?? undefined,"value":transfer.total_bytes ? transfer.bytes : undefined,"aria-label":`Transferred bytes`,}); }
               { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void sftp.cancelTransfer(conn.id, transfer.id).catch(e => toastError('Couldn’t cancel', e)),});  }
            }
            if(transfer.error){ { svelteHTML.createElement("span", { "class":`err`,});transfer.error; }}
           }
        }
       }
    }

    
     { svelteHTML.createElement("div", {   "class":`crumbs mono`,"dir":`ltr`,});
          for(let c of __sveltets_2_ensureArray(crumbs)){let i = 1;c.path;
        if(i > 0){ { svelteHTML.createElement("span", { "class":`sep`,});  }}
         { svelteHTML.createElement("button", {   "class":`crumb`,"onclick":() => sftp.navigate(conn.id, c.path),});c.label; }
      }
     }

    if(view.truncated && !view.loading){
       { svelteHTML.createElement("p", { "class":`list-trunc`,});                 }
    }

    
     { const $$_div2 = svelteHTML.createElement("div", {     "class":`list`,"onscroll":tw.onscroll,});listEl = $$_div2;tw.viewH= $$_div2.clientHeight;
      if(view.loading){
         { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {      "rows":8,"height":24,"label":`files`,}});} }
      } else if (view.error){
         { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {        "what":`the folder`,"variant":`compact`,"error":view.error,"empty":true,"onretry":() => sftp.refresh(conn.id),}});} }
      } else if (view.entries.length === 0){
         { svelteHTML.createElement("div", { "class":`dim pad`,});  }
      } else if (shownEntries.length === 0){
         { svelteHTML.createElement("div", { "class":`dim pad`,});   query.trim();  }
      }else{
         { svelteHTML.createElement("div", { "class":`head sftp-row`,});
           { svelteHTML.createElement("span", { "class":`cell name`,});  }
           { svelteHTML.createElement("span", { "class":`cell size`,});  }
           { svelteHTML.createElement("span", { "class":`cell mtime`,});  }
           { svelteHTML.createElement("span", { "class":`cell perms mono`,});  }
           { svelteHTML.createElement("span", { "class":`cell actions`,}); }
         }
        if(win.top){ { svelteHTML.createElement("div", {     "class":`tw-spacer`,"aria-hidden":`true`,"style":`height:${win.top}px`,}); }}
           for(let e of __sveltets_2_ensureArray(rowsWindow)){e.name;
           { svelteHTML.createElement("div", { "class":`sftp-row entry`,});
             { svelteHTML.createElement("button", {     "class":`cell name nav`,"onclick":() => onRowActivate(e),});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":iconFor(e),"size":13,}});}
               { svelteHTML.createElement("span", {     "class":`ellipsis`,"dir":`ltr`,"title":e.name,});e.name; }
              if(e.symlink_target){ { svelteHTML.createElement("span", {     "class":`link-to dim ellipsis`,"dir":`ltr`,"title":e.symlink_target,}); e.symlink_target; }}
             }
             { svelteHTML.createElement("span", {   "class":`cell size mono`,"dir":`ltr`,});e.kind === 'dir' ? '' : humanSize(e.size); }
             { svelteHTML.createElement("span", {   "class":`cell mtime dim`,"dir":`ltr`,});e.mtime ?? ''; }
             { svelteHTML.createElement("span", {   "class":`cell perms mono dim`,"dir":`ltr`,});e.perms; }
             { svelteHTML.createElement("span", { "class":`cell actions`,});
              if(isTextLike(e)){
                 { svelteHTML.createElement("button", {           "class":`icon-btn`,"title":`View`,"aria-label":`View`,"disabled":busy[e.name],"onclick":() => openViewer(e),});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":13,}});}
                 }
              }
              if(e.kind !== 'dir'){
                 { svelteHTML.createElement("button", {           "class":`icon-btn`,"title":auth.isRoot ? 'Download' : HOST_PATH_DENIED,"aria-label":`Download`,"disabled":busy[e.name] || !auth.isRoot,"onclick":() => (downloadFor = e),});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`fetch`,"size":13,}});}
                 }
              }
               { svelteHTML.createElement("button", {           "class":`icon-btn`,"title":canWrite ? 'Rename' : WRITE_DENIED,"aria-label":`Rename`,"disabled":!canWrite,"onclick":() => renameEntry(e),});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":13,}});}
               }
               { svelteHTML.createElement("button", {           "class":`icon-btn`,"title":canWrite ? 'Delete' : WRITE_DENIED,"aria-label":`Delete`,"disabled":!canWrite,"onclick":() => deleteEntry(e),});
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":13,}});}
               }
             }
           }
        }
        if(win.bottom){ { svelteHTML.createElement("div", {     "class":`tw-spacer`,"aria-hidden":`true`,"style":`height:${win.bottom}px`,}); }}
      }
     }
   }
 Modal}


if(downloadFor){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {         "title":`Download “${downloadFor.name}” to…`,"start":sftp.localDir,"onpick":(dir) => void doDownload(dir),"onclose":() => (downloadFor = null),}});}
}


if(uploadOpen){
   { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {           "title":`Choose a local file to upload`,"start":sftp.localDir,"files":true,"onpick":(p) => void doUpload(p),"onclose":() => (uploadOpen = false),}});}
}


if(viewing){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`View — ${viewing.name}`,"width":820,"onclose":() => (viewing = null),}});
    if(viewing.truncated){
       { svelteHTML.createElement("div", { "class":`trunc`,});         }
    }
     { svelteHTML.createElement("pre", {   "class":`viewer mono`,"dir":`ltr`,});viewing.text; }
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SftpBrowser__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SftpBrowser__SvelteComponent_ = ReturnType<typeof SftpBrowser__SvelteComponent_>;
/*Ωignore_endΩ*/export default SftpBrowser__SvelteComponent_;