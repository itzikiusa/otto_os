<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { api, baseUrl } from '../../lib/api/client';
  import { auth } from '../../lib/stores/auth.svelte';
  import type { GitBackupStatus, GitBackupPreview, RestorePreview, RestoreResult, RestoreConflictPolicy } from '../../lib/api/types';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import RestorePreviewPanel from './RestorePreviewPanel.svelte';
  import { confirmer } from '../../lib/confirm.svelte';

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
  async function perform(action: () => Promise<void>) {
    const seq = generation;
    busy = true; error = ''; notice = '';
    try { await action(); } catch (e) { if (seq === generation) error = e instanceof Error ? e.message : String(e); }
    finally { if (seq === generation) busy = false; }
  }
  async function loadStatus() {
    const seq = generation;
    await perform(async () => {
      const next = await api.post<GitBackupStatus>('/state/git/status', { repo_path: repoPath });
      if (seq !== generation) return;
      useStatus(next); preview = null; exported = false; restorePreview = null;
    });
  }
  async function previewExport() {
    const seq = generation;
    await perform(async () => {
      const next = await api.post<GitBackupPreview>('/state/git/preview', { repo_path: repoPath });
      if (seq !== generation) return;
      preview = next; exported = false; restorePreview = null; useStatus(next.status);
    });
  }
  async function writeSnapshot() {
    if (!preview) return;
    const seq = generation;
    await perform(async () => {
      const next = await api.post<GitBackupPreview>('/state/git/export', { repo_path: repoPath, preview_token: preview!.token });
      if (seq !== generation) return;
      preview = next; exported = true; restorePreview = null; reviewed = false; useStatus(next.status); notice = 'Snapshot written to .otto-sync in this repository.';
    });
  }
  async function commitSnapshot() {
    if (!status || !preview || !message.trim()) return;
    const seq = generation;
    await perform(async () => {
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
        `Push “${status.branch}” to the remote “${remote}”? This sends the branch's full history — the Otto snapshot and any other commits in this repository — to everyone with access to that remote.`,
        { title: 'Push backup branch', confirmLabel: 'Push', danger: false },
      ))
    )
      return;
    const seq = generation;
    await perform(async () => {
      const next = await api.post<{ status: GitBackupStatus }>('/state/git/sync', { repo_path: repoPath, expected_head: status!.head, action, remote });
      if (seq !== generation) return;
      useStatus(next.status); preview = null; exported = false; restorePreview = null;
      notice = action === 'fetch' ? 'Remote references refreshed.' : action === 'pull' ? 'Repository updated. Preview a restore to import new Otto items.' : 'Current branch pushed to the selected remote.';
    });
  }
  async function previewRestore() {
    const seq = generation;
    await perform(async () => {
      const next = await api.post<RestorePreview>('/state/git/restore/preview', { repo_path: repoPath, conflicts });
      if (seq !== generation) return;
      restorePreview = next; reviewed = false;
    });
  }
  async function restoreSnapshot() {
    if (!restorePreview?.can_restore || !reviewed) return;
    const seq = generation;
    await perform(async () => {
      try {
        const next = await api.post<RestoreResult>('/state/git/restore', { repo_path: repoPath, conflicts, preview_token: restorePreview!.preview_token, confirm: true });
        if (seq === generation) notice = `Restored ${next.records_inserted} records and ${next.files_restored} files; kept ${next.records_skipped} existing records and ${next.files_skipped} existing files.`;
      } finally { if (seq === generation) { restorePreview = null; reviewed = false; } }
    });
  }
</script>

<section class="git-backup" aria-label="Git backup sync">
  <h2 class="card-title">Git backup sync</h2>
  <p>Keep portable configuration and documents in a Git repository. Review changes, write a snapshot,
    then commit or sync when you choose. Stored credentials and runtime history are excluded. Review document content before pushing it to a remote.</p>
  <label for="backup-git-repo">Existing local repository</label>
  <div class="controls">
    <input id="backup-git-repo" class="input mono" bind:value={repoPath} disabled={busy} oninput={() => { generation++; reset(); }} placeholder="/path/to/backup-repository" />
    <button class="btn" disabled={busy} onclick={() => browsing = true}>Browse…</button>
    <button class="btn" disabled={busy || !repoPath.trim()} onclick={loadStatus}>Check repository</button>
  </div>
  {#if status}
    <p class="dim status-line">{status.branch ?? 'Detached HEAD'} · tracking {status.upstream ?? 'none'} · {status.ahead} ahead · {status.behind} behind · {status.dirty ? 'Local changes present' : 'Working tree clean'}</p>
    <h3 class="sub-title">Snapshot</h3>
    <div class="controls"><button class="btn" disabled={busy} onclick={previewExport}>Preview snapshot</button></div>
    {#if preview}
      <div class="snapshot-preview" aria-label="Git snapshot preview">
        <strong>{changes.length} changed file{changes.length === 1 ? '' : 's'}</strong>
        <p class="dim">Only Otto's managed .otto-sync files are written. Local edits to those files require review before exporting again.</p>
        <div class="scroll"><ul>{#each changes as change}<li><span class="act">{change.action}</span> <code>{change.path}</code></li>{:else}<li>The snapshot matches this repository.</li>{/each}</ul></div>
        {#if preview.excluded.length}<details><summary>Excluded from Git</summary><ul>{#each preview.excluded as item}<li>{item}</li>{/each}</ul></details>{/if}
        {#if preview.reconnect.length}<details><summary>Reconnect after importing</summary><ul>{#each preview.reconnect as item}<li>{item}</li>{/each}</ul></details>{/if}
        <button class="btn" disabled={busy || exported} onclick={writeSnapshot}>{exported ? 'Snapshot written' : 'Write snapshot'}</button>
        {#if exported}
          <div class="controls"><input class="input" aria-label="Snapshot commit message" bind:value={message} disabled={busy} placeholder="Describe the backup changes" /><button class="btn" disabled={busy || !message.trim()} onclick={commitSnapshot}>Commit snapshot</button></div>
        {/if}
      </div>
    {/if}
    <h3 class="sub-title">Remote sync</h3>
    {#if status.remotes.length}
      <div class="controls">
        <select class="input" aria-label="Backup remote" bind:value={remote} disabled={busy}>{#each status.remotes as name}<option value={name}>{name}</option>{/each}</select>
        <button class="btn" disabled={busy} onclick={() => sync('fetch')}>Fetch</button>
        <button class="btn" disabled={busy || status.dirty || !status.branch} onclick={() => sync('pull')}>Pull fast-forward</button>
        <button class="btn" disabled={busy || !status.branch} onclick={() => sync('push')}>Push current branch…</button>
      </div>
      <p class="dim">Push sends the current branch’s full history, including other commits in this repository. Pull requires a clean working tree and never rewrites diverged history. Remote files are imported into Otto only through the restore preview below.</p>
    {:else}<p class="dim">No remote configured. Add one in Git to enable remote sync.</p>{/if}
    <h3 class="sub-title">Import from this snapshot</h3>
    <p>Restore adds missing items. Existing Otto records and files are kept, including records that differ from Git.</p>
    <div class="controls">
      <select class="input" aria-label="Git restore conflict policy" bind:value={conflicts} disabled={busy} onchange={() => { restorePreview = null; reviewed = false; }}><option value="skip_existing">Keep existing; restore only new items</option><option value="abort">Stop if there are any conflicts</option></select>
      <button class="btn" disabled={busy} onclick={previewRestore}>Preview Git restore</button>
    </div>
    {#if restorePreview}<RestorePreviewPanel preview={restorePreview} {busy} bind:reviewed onrestore={restoreSnapshot} />{/if}
  {/if}
  {#if busy}<p class="dim" role="status">Working…</p>{/if}
  {#if notice}<p role="status">{notice}</p>{/if}
  {#if error}<p class="error" role="alert">{error}</p>{/if}
</section>
{#if browsing}<FolderPicker title="Choose backup repository" start={repoPath || '~'} gitOnly onpick={chooseRepo} onclose={() => browsing = false} />{/if}

<style>
  .git-backup { background: var(--surface); border: 1px solid var(--border); border-radius: var(--radius-m); box-shadow: var(--shadow-card); padding: 16px 18px; margin: 0 0 16px; max-width: 880px; }
  .card-title { margin: 0 0 6px; font-size: var(--fs-m); font-weight: 600; }
  .sub-title { margin: 20px 0 6px; padding-top: 16px; border-top: 1px solid var(--border); font-size: var(--fs-m); font-weight: 600; }
  p { margin: 0 0 8px; font-size: var(--fs-s); line-height: 1.5; }
  label { font-size: var(--fs-s); font-weight: 500; color: var(--text-dim); }
  details { font-size: var(--fs-s); }
  .dim { color: var(--text-dim); }
  .status-line { margin-top: 4px; }
  .controls { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; margin: 8px 0; }
  .controls input { flex: 1; min-width: 150px; }
  .controls select { max-width: 100%; }
  .snapshot-preview { margin: 12px 0; padding: 12px; background: var(--surface-2); border: 1px solid var(--border); border-radius: var(--radius-m); font-size: var(--fs-s); }
  .scroll, details ul { max-height: 230px; overflow: auto; }
  ul { padding-inline-start: 20px; }
  li { margin: 4px 0; overflow-wrap: anywhere; }
  .act { color: var(--text-dim); }
  code { font-family: var(--font-mono); font-size: var(--fs-xs); }
  summary { cursor: pointer; margin: 10px 0; }
  .error { color: var(--danger); overflow-wrap: anywhere; }
</style>
