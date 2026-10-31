///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { onDestroy, untrack } from 'svelte';
import { plural } from '../../lib/plural';
import { api, baseUrl } from '../../lib/api/client';
import { auth } from '../../lib/stores/auth.svelte';
import type { GitBackupStatus, GitBackupPreview, RestorePreview, RestoreResult, RestoreConflictPolicy } from '../../lib/api/types';
import FolderPicker from '../../lib/components/FolderPicker.svelte';
import RestorePreviewPanel from './RestorePreviewPanel.svelte';
import { confirmer } from '../../lib/confirm.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  

  let repoPath = $state('');
  let browsing = $state(false);
  let busy = $state(false);
  let error = $state('');
  let notice = $state('');
  let status: GitBackupStatus | null = $state(null);
  let preview: GitBackupPreview | null = $state(null);
  let exported = $state(false);
  let message = $state('');
  let remote = $state('');
  let restorePreview: RestorePreview | null = $state(null);
  let conflicts: RestoreConflictPolicy = $state('skip_existing');
  let reviewed = $state(false);
  let generation = 0;
  let preferenceKey = '';
  const changes = $derived.by(() => preview?.changes.filter(change => change.action !== 'unchanged') ?? []);
  $effect(() => {
    const key = `otto_backup_git_repo:${baseUrl()}:${auth.me?.id ?? 'anonymous'}`;
    untrack(() => {
      generation++; preferenceKey = key; reset();
      try { repoPath = localStorage.getItem(key) ?? ''; } catch { repoPath = ''; }
    });
  });
  onDestroy(() => { generation++; });

  function reset() { busy = false; status = null; preview = null; exported = false; restorePreview = null; reviewed = false; error = ''; notice = ''; }
  function chooseRepo(path: string) { repoPath = path; browsing = false; generation++; reset(); }
  function useStatus(next: GitBackupStatus) {
    status = next; repoPath = next.repo_path;
    if (!next.remotes.includes(remote)) remote = next.remotes[0] ?? '';
    try { localStorage.setItem(preferenceKey, next.repo_path); } catch { /* Browsing still works without storage. */ }
  }
  // `what` names the step for the inline error ("Couldn’t check the
  // repository. <daemon reason>") — a bare daemon message didn't say which
  // of the six buttons failed.
  async function perform(what: string, action: () => Promise<void>) {
    const seq = generation;
    busy = true; error = ''; notice = '';
    try { await action(); } catch (e) { if (seq === generation) error = `Couldn’t ${what}. ${e instanceof Error ? e.message : String(e)}`; }
    finally { if (seq === generation) busy = false; }
  }
  async function loadStatus() {
    const seq = generation;
    await perform('check the repository', async () => {
      const next = await api.post<GitBackupStatus>('/state/git/status', { repo_path: repoPath });
      if (seq !== generation) return;
      useStatus(next); preview = null; exported = false; restorePreview = null;
    });
  }
  async function previewExport() {
    const seq = generation;
    await perform('preview the snapshot', async () => {
      const next = await api.post<GitBackupPreview>('/state/git/preview', { repo_path: repoPath });
      if (seq !== generation) return;
      preview = next; exported = false; restorePreview = null; useStatus(next.status);
    });
  }
  async function writeSnapshot() {
    if (!preview) return;
    const seq = generation;
    await perform('write the snapshot', async () => {
      const next = await api.post<GitBackupPreview>('/state/git/export', { repo_path: repoPath, preview_token: preview!.token });
      if (seq !== generation) return;
      preview = next; exported = true; restorePreview = null; reviewed = false; useStatus(next.status); notice = 'Snapshot written to .otto-sync in this repository.';
    });
  }
  async function commitSnapshot() {
    if (!status || !preview || !message.trim()) return;
    const seq = generation;
    await perform('commit the snapshot', async () => {
      const next = await api.post<{ status: GitBackupStatus }>('/state/git/commit', {
        repo_path: repoPath, expected_head: status!.head, snapshot_digest: preview!.snapshot_digest, message: message.trim(),
      });
      if (seq !== generation) return;
      useStatus(next.status); exported = false; preview = null; restorePreview = null; reviewed = false; notice = 'Snapshot committed. Other staged changes were preserved.';
    });
  }
  async function sync(action: 'fetch' | 'pull' | 'push') {
    if (!status || !remote) return;
    // Pushing leaves this Mac: say where it goes and what is sent first.
    if (
      action === 'push' &&
      !(await confirmer.ask(
        `Push “${status.branch}” to the remote “${remote}”? This sends the branch’s full history — the Otto snapshot and any other commits in this repository — to everyone with access to that remote.`,
        { title: 'Push backup branch', confirmLabel: 'Push', danger: false },
      ))
    )
      return;
    const seq = generation;
    await perform(action === 'fetch' ? 'fetch from the remote' : action === 'pull' ? 'pull from the remote' : 'push to the remote', async () => {
      const next = await api.post<{ status: GitBackupStatus }>('/state/git/sync', { repo_path: repoPath, expected_head: status!.head, action, remote });
      if (seq !== generation) return;
      useStatus(next.status); preview = null; exported = false; restorePreview = null;
      notice = action === 'fetch' ? 'Remote references refreshed.' : action === 'pull' ? 'Repository updated. Preview a restore to import new Otto items.' : 'Current branch pushed to the selected remote.';
    });
  }
  async function previewRestore() {
    const seq = generation;
    await perform('preview the Git restore', async () => {
      const next = await api.post<RestorePreview>('/state/git/restore/preview', { repo_path: repoPath, conflicts });
      if (seq !== generation) return;
      restorePreview = next; reviewed = false;
    });
  }
  async function restoreSnapshot() {
    if (!restorePreview?.can_restore || !reviewed) return;
    const seq = generation;
    await perform('restore from Git', async () => {
      try {
        const next = await api.post<RestoreResult>('/state/git/restore', { repo_path: repoPath, conflicts, preview_token: restorePreview!.preview_token, confirm: true });
        if (seq === generation) notice = `Restored ${next.records_inserted} records and ${next.files_restored} files; kept ${next.records_skipped} existing records and ${next.files_skipped} existing files.`;
      } finally { if (seq === generation) { restorePreview = null; reviewed = false; } }
    });
  }

  // a11y-6-01: announce at the state edge — a status element inserted
  // together with its text is not read by VoiceOver/WebKit.
  $effect(() => { if (busy) announce('Working…'); });
  $effect(() => { if (notice) announce(notice); });
;
async () => {

 { svelteHTML.createElement("section", {   "class":`git-backup`,"aria-label":`Git backup sync`,});
   { svelteHTML.createElement("h2", { "class":`card-title`,});   }
   { svelteHTML.createElement("p", {});             
                           }
   { svelteHTML.createElement("label", { "for":`backup-git-repo`,});   }
   { svelteHTML.createElement("div", { "class":`controls`,});
     { svelteHTML.createElement("input", {              "dir":`ltr`,"id":`backup-git-repo`,"class":`input mono`,"bind:value":repoPath,"disabled":busy,"oninput":() => { generation++; reset(); },"placeholder":`/path/to/backup-repository`,});/*Ωignore_startΩ*/() => repoPath = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":() => browsing = true,});  }
     { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy || !repoPath.trim(),"onclick":loadStatus,});  }
   }
  if(status){
     { svelteHTML.createElement("p", { "class":`dim status-line`,});status.branch ?? 'Detached HEAD';   status.upstream ?? 'none';  status.ahead;   status.behind;   status.dirty ? 'Local changes present' : 'Working tree clean'; }
     { svelteHTML.createElement("h3", { "class":`sub-title`,});  }
     { svelteHTML.createElement("div", { "class":`controls`,}); { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":previewExport,});  } }
    if(preview){
       { svelteHTML.createElement("div", {   "class":`snapshot-preview`,"aria-label":`Git snapshot preview`,});
         { svelteHTML.createElement("strong", {});plural(changes.length, 'changed file'); }
         { svelteHTML.createElement("p", { "class":`dim`,});                 }
         { svelteHTML.createElement("div", { "class":`scroll`,}); { svelteHTML.createElement("ul", {});  for(let change of __sveltets_2_ensureArray(changes)){ { svelteHTML.createElement("li", {}); { svelteHTML.createElement("span", { "class":`act`,});change.action; }  { svelteHTML.createElement("code", {});change.path; } }} { svelteHTML.createElement("li", {});     } } }
        if(preview.excluded.length){ { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", {});   } { svelteHTML.createElement("ul", {});  for(let item of __sveltets_2_ensureArray(preview.excluded)){ { svelteHTML.createElement("li", {});item; }} } }}
        if(preview.reconnect.length){ { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", {});   } { svelteHTML.createElement("ul", {});  for(let item of __sveltets_2_ensureArray(preview.reconnect)){ { svelteHTML.createElement("li", {});item; }} } }}
         { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy || exported,"onclick":writeSnapshot,});exported ? 'Snapshot written' : 'Write snapshot'; }
        if(exported){
           { svelteHTML.createElement("div", { "class":`controls`,}); { svelteHTML.createElement("input", {            "dir":`auto`,"class":`input`,"aria-label":`Snapshot commit message`,"bind:value":message,"disabled":busy,"placeholder":`Describe the backup changes`,});/*Ωignore_startΩ*/() => message = __sveltets_2_any(null);/*Ωignore_endΩ*/} { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy || !message.trim(),"onclick":commitSnapshot,});  } }
        }
       }
    }
     { svelteHTML.createElement("h3", { "class":`sub-title`,});  }
    if(status.remotes.length){
       { svelteHTML.createElement("div", { "class":`controls`,});
         { svelteHTML.createElement("select", {       "class":`input`,"aria-label":`Backup remote`,"bind:value":remote,"disabled":busy,});/*Ωignore_startΩ*/() => remote = __sveltets_2_any(null);/*Ωignore_endΩ*/  for(let name of __sveltets_2_ensureArray(status.remotes)){ { svelteHTML.createElement("option", { "value":name,});name; }} }
         { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":() => sync('fetch'),});  }
         { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy || status.dirty || !status.branch,"onclick":() => sync('pull'),});  }
         { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy || !status.branch,"onclick":() => sync('push'),});   }
       }
       { svelteHTML.createElement("p", { "class":`dim`,});                                    }
    }else{ { svelteHTML.createElement("p", { "class":`dim`,});           }}
     { svelteHTML.createElement("h3", { "class":`sub-title`,});    }
     { svelteHTML.createElement("p", {});                 }
     { svelteHTML.createElement("div", { "class":`controls`,});
       { svelteHTML.createElement("select", {         "class":`input`,"aria-label":`Git restore conflict policy`,"bind:value":conflicts,"disabled":busy,"onchange":() => { restorePreview = null; reviewed = false; },});/*Ωignore_startΩ*/() => conflicts = __sveltets_2_any(null);/*Ωignore_endΩ*/ { svelteHTML.createElement("option", { "value":`skip_existing`,});      } { svelteHTML.createElement("option", { "value":`abort`,});      } }
       { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":previewRestore,});   }
     }
    if(restorePreview){ { const $$_lenaPweiverPerotseR1C = __sveltets_2_ensureComponent(RestorePreviewPanel); const $$_lenaPweiverPerotseR1 = new $$_lenaPweiverPerotseR1C({ target: __sveltets_2_any(), props: {     "preview":restorePreview,busy,reviewed,"onrestore":restoreSnapshot,}});/*Ωignore_startΩ*/() => reviewed = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_lenaPweiverPerotseR1.$$bindings = 'reviewed';}}
  }
  if(busy){ { svelteHTML.createElement("p", { "class":`dim`,});  }}
  if(notice){ { svelteHTML.createElement("p", {});notice; }}
  if(error){ { svelteHTML.createElement("p", {   "class":`error`,"role":`alert`,});error; }}
 }
if(browsing){ { const $$_rekciPredloF0C = __sveltets_2_ensureComponent(FolderPicker); new $$_rekciPredloF0C({ target: __sveltets_2_any(), props: {        "title":`Choose backup repository`,"start":repoPath || '~',"gitOnly":true,"onpick":chooseRepo,"onclose":() => browsing = false,}});}}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const GitBackup__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type GitBackup__SvelteComponent_ = ReturnType<typeof GitBackup__SvelteComponent_>;
/*Ωignore_endΩ*/export default GitBackup__SvelteComponent_;