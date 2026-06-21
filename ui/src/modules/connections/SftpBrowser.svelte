<script lang="ts">
  // SFTP file browser for an SSH connection — a "MobaXterm-lite" pane that
  // browses, transfers, and edits files over the connection's existing SSH
  // auth (the daemon drives the system `sftp` binary). Opened from the
  // Connections page; no terminal session required.
  import type { Connection, SftpEntry } from '../../lib/api/types';
  import { sftp } from '../../lib/stores/sftp.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';

  interface Props {
    conn: Connection;
    onclose: () => void;
  }
  let { conn, onclose }: Props = $props();

  const view = $derived(sftp.state(conn.id));

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

  // Initial load: resolve the remote home/working dir then list it.
  $effect(() => {
    if (!view.loaded && !view.loading) void sftp.list(conn.id);
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
      toasts.error('Read failed', err instanceof Error ? err.message : String(err));
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
      toasts.error('Download failed', err instanceof Error ? err.message : String(err));
    } finally {
      busy = { ...busy, [e.name]: false };
    }
  }

  async function doUpload(localPath: string): Promise<void> {
    uploadOpen = false;
    try {
      await sftp.upload(conn.id, localPath);
      toasts.success('Uploaded', localPath);
      await sftp.refresh(conn.id);
    } catch (err) {
      toasts.error('Upload failed', err instanceof Error ? err.message : String(err));
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
      toasts.error('Create failed', err instanceof Error ? err.message : String(err));
    }
  }

  async function renameEntry(e: SftpEntry): Promise<void> {
    const next = await confirmer.promptText(`Rename “${e.name}” to`, {
      title: 'Rename',
      confirmLabel: 'Rename',
    });
    if (!next || next === e.name) return;
    const path = sftp.childPath(conn.id, e.name);
    try {
      await sftp.rename(conn.id, path, next);
      await sftp.refresh(conn.id);
    } catch (err) {
      toasts.error('Rename failed', err instanceof Error ? err.message : String(err));
    }
  }

  async function deleteEntry(e: SftpEntry): Promise<void> {
    const isDir = e.kind === 'dir';
    const ok = await confirmer.ask(
      `Delete ${isDir ? 'directory' : 'file'} “${e.name}”${isDir ? ' (must be empty)' : ''}?`,
      { title: 'Delete', confirmLabel: 'Delete' },
    );
    if (!ok) return;
    const path = sftp.childPath(conn.id, e.name);
    try {
      await sftp.remove(conn.id, path, isDir);
      await sftp.refresh(conn.id);
    } catch (err) {
      toasts.error('Delete failed', err instanceof Error ? err.message : String(err));
    }
  }

  function iconFor(e: SftpEntry): string {
    if (e.kind === 'dir') return 'folder';
    if (e.kind === 'symlink') return 'link';
    return 'file';
  }

  function close(): void {
    onclose();
  }
</script>

<Modal title={`Files — ${conn.name}`} width={860} onclose={close}>
  <div class="sftp">
    <!-- Toolbar: up / refresh / new folder / upload -->
    <div class="toolbar">
      <button class="btn small" title="Up" aria-label="Up" onclick={() => sftp.up(conn.id)}>
        <Icon name="arrowUp" size={13} />
      </button>
      <button
        class="btn small"
        title="Refresh"
        aria-label="Refresh"
        onclick={() => sftp.refresh(conn.id)}
      >
        <Icon name="refresh" size={13} />
      </button>
      <button class="btn small" onclick={newFolder}>
        <Icon name="plus" size={12} /> New folder
      </button>
      <button class="btn small primary" onclick={() => (uploadOpen = true)}>
        <Icon name="arrowUp" size={12} /> Upload
      </button>
      <span class="grow"></span>
      <input
        class="sftp-search"
        type="text"
        placeholder="Filter this folder…"
        aria-label="Filter files in this directory"
        bind:value={query}
      />
    </div>

    <!-- Breadcrumb -->
    <div class="crumbs mono">
      {#each crumbs as c, i (c.path)}
        {#if i > 0}<span class="sep">/</span>{/if}
        <button class="crumb" onclick={() => sftp.navigate(conn.id, c.path)}>{c.label}</button>
      {/each}
    </div>

    <!-- Listing -->
    <div class="list">
      {#if view.loading}
        <div class="dim pad">Loading…</div>
      {:else if view.error}
        <div class="err pad">{view.error}</div>
      {:else if view.entries.length === 0}
        <div class="dim pad">Empty directory.</div>
      {:else if shownEntries.length === 0}
        <div class="dim pad">No files match “{query.trim()}”.</div>
      {:else}
        <div class="head row">
          <span class="cell name">Name</span>
          <span class="cell size">Size</span>
          <span class="cell mtime">Modified</span>
          <span class="cell perms mono">Perms</span>
          <span class="cell actions"></span>
        </div>
        {#each shownEntries as e (e.name)}
          <div class="row">
            <button
              class="cell name nav"
              ondblclick={() => onRowActivate(e)}
              onkeydown={(ev) => {
                if (ev.key === 'Enter') onRowActivate(e);
              }}
            >
              <Icon name={iconFor(e)} size={13} />
              <span class="ellipsis">{e.name}</span>
              {#if e.symlink_target}<span class="link-to dim">→ {e.symlink_target}</span>{/if}
            </button>
            <span class="cell size mono">{e.kind === 'dir' ? '' : humanSize(e.size)}</span>
            <span class="cell mtime dim">{e.mtime ?? ''}</span>
            <span class="cell perms mono dim">{e.perms}</span>
            <span class="cell actions">
              {#if isTextLike(e)}
                <button
                  class="icon-btn"
                  title="View"
                  aria-label="View"
                  disabled={busy[e.name]}
                  onclick={() => openViewer(e)}
                >
                  <Icon name="eye" size={13} />
                </button>
              {/if}
              {#if e.kind !== 'dir'}
                <button
                  class="icon-btn"
                  title="Download"
                  aria-label="Download"
                  disabled={busy[e.name]}
                  onclick={() => (downloadFor = e)}
                >
                  <Icon name="fetch" size={13} />
                </button>
              {/if}
              <button
                class="icon-btn"
                title="Rename"
                aria-label="Rename"
                onclick={() => renameEntry(e)}
              >
                <Icon name="edit" size={13} />
              </button>
              <button
                class="icon-btn"
                title="Delete"
                aria-label="Delete"
                onclick={() => deleteEntry(e)}
              >
                <Icon name="trash" size={13} />
              </button>
            </span>
          </div>
        {/each}
      {/if}
    </div>
  </div>
</Modal>

<!-- Download destination picker (a local dir on the daemon host). -->
{#if downloadFor}
  <FolderPicker
    title={`Download “${downloadFor.name}” to…`}
    start={sftp.localDir}
    onpick={(dir) => void doDownload(dir)}
    onclose={() => (downloadFor = null)}
  />
{/if}

<!-- Upload source picker (a local file on the daemon host). -->
{#if uploadOpen}
  <FolderPicker
    title="Choose a local file to upload"
    start={sftp.localDir}
    files={true}
    onpick={(p) => void doUpload(p)}
    onclose={() => (uploadOpen = false)}
  />
{/if}

<!-- Read-only text viewer. -->
{#if viewing}
  <Modal title={`View — ${viewing.name}`} width={820} onclose={() => (viewing = null)}>
    {#if viewing.truncated}
      <div class="trunc">Showing the first 1 MiB — file is larger.</div>
    {/if}
    <pre class="viewer mono">{viewing.text}</pre>
  </Modal>
{/if}

<style>
  .sftp {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 420px;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .grow {
    flex: 1;
  }
  .sftp-search {
    width: 180px;
    background: var(--surface-2, var(--surface));
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: 12px;
    padding: 4px 8px;
  }
  .sftp-search:focus {
    outline: none;
    border-color: var(--accent);
  }
  .sftp-search::placeholder {
    color: var(--text-dim);
  }
  .crumbs {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 2px;
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 2px 0;
  }
  .crumb {
    border: none;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    padding: 1px 3px;
    border-radius: 3px;
    font: inherit;
  }
  .crumb:hover {
    color: var(--accent);
    background: var(--surface);
  }
  .sep {
    color: var(--text-dim);
    opacity: 0.5;
  }
  .list {
    flex: 1;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    max-height: 52vh;
  }
  .row {
    display: grid;
    grid-template-columns: 1fr 84px 130px 110px auto;
    align-items: center;
    gap: 8px;
    padding: 4px 8px;
    border-bottom: 1px solid color-mix(in srgb, var(--border) 50%, transparent);
  }
  .row:last-child {
    border-bottom: none;
  }
  .row.head {
    position: sticky;
    top: 0;
    background: var(--surface);
    font-size: 10.5px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
    z-index: 1;
  }
  .cell {
    font-size: 12.5px;
    min-width: 0;
    overflow: hidden;
  }
  .cell.size,
  .cell.mtime,
  .cell.perms {
    white-space: nowrap;
  }
  .cell.name.nav {
    display: flex;
    align-items: center;
    gap: 7px;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: start;
    padding: 1px 0;
    font: inherit;
  }
  .cell.name.nav:hover {
    color: var(--accent);
  }
  .link-to {
    font-size: 11px;
    flex-shrink: 0;
  }
  .cell.actions {
    display: flex;
    align-items: center;
    gap: 2px;
    justify-content: flex-end;
  }
  .pad {
    padding: 16px;
    font-size: 12.5px;
  }
  .err {
    color: var(--danger, #e5534b);
  }
  .viewer {
    max-height: 60vh;
    overflow: auto;
    margin: 0;
    padding: 10px 12px;
    font-size: 12px;
    line-height: 1.5;
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    white-space: pre;
  }
  .trunc {
    font-size: 11.5px;
    color: var(--text-dim);
    padding: 0 2px 8px;
  }
</style>
