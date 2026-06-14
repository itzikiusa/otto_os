<script lang="ts">
  // Info tab: shows active session metadata + attached Jira issue.
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import AttachIssue from '../agents/AttachIssue.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import type { AttachedIssue } from '../../lib/api/types';

  const session = $derived(ws.activeSession);
  const workspace = $derived(ws.current);
  const attachedIssue = $derived(
    (session?.meta?.issue as AttachedIssue | undefined) ?? null,
  );

  // Per-workspace default agent (overrides the global default). '' = inherit.
  const providers = $derived(auth.meta?.providers ?? ['claude', 'codex', 'shell']);
  const wsDefaultAgent = $derived(
    typeof workspace?.settings?.default_provider === 'string'
      ? (workspace.settings.default_provider as string)
      : '',
  );

  async function onDefaultAgentChange(e: Event): Promise<void> {
    const value = (e.currentTarget as HTMLSelectElement).value;
    try {
      await ws.saveDefaultAgent(value);
      toasts.info(
        value === '' ? 'Workspace uses the global default agent' : `Workspace default agent: ${value}`,
      );
    } catch (err) {
      toasts.error('Could not save default agent', err instanceof Error ? err.message : String(err));
    }
  }

  // extra_dirs from meta — always a string array
  const extraDirs = $derived(
    Array.isArray(session?.meta?.extra_dirs)
      ? (session!.meta!.extra_dirs as string[])
      : [],
  );

  let attachOpen = $state(false);
  let folderPickerOpen = $state(false);

  function fmt(iso: string): string {
    try {
      return new Date(iso).toLocaleString(undefined, {
        dateStyle: 'medium',
        timeStyle: 'short',
      });
    } catch {
      return iso;
    }
  }

  async function detach(): Promise<void> {
    if (!session) return;
    try {
      await ws.detachIssue(session.id);
      toasts.info('Issue detached');
    } catch (e) {
      toasts.error('Detach failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function addDir(path: string): Promise<void> {
    if (!session) return;
    folderPickerOpen = false;
    if (extraDirs.includes(path)) return;
    try {
      await ws.setSessionDirs(session.id, [...extraDirs, path]);
      toasts.info('Folder added — session restarted');
    } catch (e) {
      toasts.error('Failed to add folder', e instanceof Error ? e.message : String(e));
    }
  }

  async function removeDir(dir: string): Promise<void> {
    if (!session) return;
    try {
      await ws.setSessionDirs(session.id, extraDirs.filter((d) => d !== dir));
      toasts.info('Folder removed — session restarted');
    } catch (e) {
      toasts.error('Failed to remove folder', e instanceof Error ? e.message : String(e));
    }
  }
</script>

{#if !session && !workspace}
  <EmptyState icon="info" title="No workspace" body="Select a workspace to see its details here." />
{:else}
  <div class="info">
    {#if workspace}
      <section class="section">
        <div class="section-title">Workspace</div>
        <div class="row">
          <span class="key">Default agent</span>
          <span class="val">
            <select class="ws-select" value={wsDefaultAgent} onchange={onDefaultAgentChange}>
              <option value="">Global default</option>
              {#each providers as p (p)}
                <option value={p}>{p}</option>
              {/each}
            </select>
          </span>
        </div>
        <p class="hint">Agent used for new sessions &amp; channel replies in this workspace. Overrides the global default.</p>
      </section>
    {/if}

    {#if session}
    <section class="section">
      <div class="section-title">Session</div>
      <div class="row">
        <span class="key">Title</span>
        <span class="val">{session.title}</span>
      </div>
      <div class="row">
        <span class="key">Provider</span>
        <span class="val chip">{session.provider}</span>
      </div>
      <div class="row">
        <span class="key">Status</span>
        <span class="val chip">{session.status}</span>
      </div>
      {#if session.cwd}
        <div class="row">
          <span class="key">CWD</span>
          <span class="val mono cwd" title={session.cwd}>{session.cwd}</span>
        </div>
      {/if}
      <div class="row">
        <span class="key">Created</span>
        <span class="val">{fmt(session.created_at)}</span>
      </div>
    </section>

    {#if session.kind === 'agent'}
      <section class="section">
        <div class="section-title">Additional folders</div>
        {#if extraDirs.length > 0}
          <ul class="dir-list">
            {#each extraDirs as dir (dir)}
              <li class="dir-row">
                <span class="dir-path mono" title={dir}>{dir}</span>
                <button
                  class="dir-remove"
                  title="Remove folder"
                  onclick={() => removeDir(dir)}
                >✕</button>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="dim no-dirs">No extra folders.</p>
        {/if}
        <button class="btn btn-sm add-dir-btn" onclick={() => (folderPickerOpen = true)}>
          + Add folder…
        </button>
      </section>
    {/if}

    <section class="section">
      <div class="section-title">Jira Issue</div>
      {#if attachedIssue}
        <div class="issue-card">
          <div class="issue-head">
            <a
              class="issue-key"
              href={attachedIssue.url}
              target="_blank"
              rel="noopener noreferrer"
              title="Open in browser"
            >
              {attachedIssue.key}
              <Icon name="external" size={10} />
            </a>
            <span class="chip">{attachedIssue.status}</span>
          </div>
          <div class="issue-summary">{attachedIssue.summary}</div>
          <div class="issue-actions">
            <button class="btn btn-sm" onclick={() => (attachOpen = true)}>Change…</button>
            <button class="btn btn-sm" onclick={detach}>Detach</button>
          </div>
        </div>
      {:else}
        <div class="no-issue dim">
          <p>No issue attached.</p>
          <button class="btn primary btn-sm" onclick={() => (attachOpen = true)}>
            Attach Jira issue…
          </button>
        </div>
      {/if}
    </section>
    {/if}
  </div>
{/if}

{#if attachOpen && session}
  <AttachIssue sessionId={session.id} onclose={() => (attachOpen = false)} />
{/if}

{#if folderPickerOpen && session}
  <FolderPicker
    title="Add folder"
    start={session.cwd}
    onpick={(path) => addDir(path)}
    onclose={() => (folderPickerOpen = false)}
  />
{/if}

<style>
  .info {
    padding: 12px 10px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .section-title {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-dim);
    padding-bottom: 4px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 4px;
  }
  .row {
    display: flex;
    align-items: baseline;
    gap: 6px;
    font-size: 12px;
    min-height: 20px;
  }
  .key {
    width: 58px;
    flex-shrink: 0;
    color: var(--text-dim);
    font-size: 11.5px;
  }
  .val {
    flex: 1;
    min-width: 0;
    word-break: break-all;
  }
  .ws-select {
    width: 100%;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: 11.5px;
    padding: 3px 6px;
  }
  .ws-select:focus {
    outline: none;
    border-color: var(--accent);
  }
  .hint {
    margin: 4px 0 0;
    font-size: 10.5px;
    line-height: 1.4;
    color: var(--text-dim);
  }
  .cwd {
    font-size: 10.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: block;
  }
  .issue-card {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .issue-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .issue-key {
    font-family: var(--font-mono);
    font-size: 12px;
    font-weight: 700;
    color: var(--accent);
    text-decoration: none;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .issue-key:hover {
    text-decoration: underline;
  }
  .issue-summary {
    font-size: 12px;
    line-height: 1.45;
    color: var(--text);
  }
  .issue-actions {
    display: flex;
    gap: 6px;
    margin-top: 2px;
  }
  .btn-sm {
    height: 24px;
    padding: 0 10px;
    font-size: 11.5px;
  }
  .no-issue {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    padding: 6px 0;
  }
  .no-issue p {
    margin: 0;
    font-size: 12.5px;
  }
  .dir-list {
    list-style: none;
    margin: 0 0 6px;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .dir-row {
    display: flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
  }
  .dir-path {
    flex: 1;
    min-width: 0;
    font-size: 10.5px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .dir-remove {
    flex-shrink: 0;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: 10px;
    padding: 2px 4px;
    border-radius: 3px;
    line-height: 1;
  }
  .dir-remove:hover {
    color: var(--danger, #e5534b);
    background: color-mix(in srgb, var(--danger, #e5534b) 12%, transparent);
  }
  .no-dirs {
    font-size: 12px;
    margin: 0 0 6px;
  }
  .add-dir-btn {
    align-self: flex-start;
  }
</style>
