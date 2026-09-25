<script lang="ts">
  // Info tab: shows active session metadata + attached Jira issue.
  import { ws } from '../../lib/stores/workspace.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { events } from '../../lib/events.svelte';
  import { sessionState } from '../../lib/status';
  import AttachIssue from '../agents/AttachIssue.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import type { AttachedIssue } from '../../lib/api/types';
  import { allProviders } from '../../lib/providers';
  import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
  import { confirmer } from '../../lib/confirm.svelte';

  const session = $derived(ws.activeSession);
  const workspace = $derived(ws.current);
  const attachedIssue = $derived(
    (session?.meta?.issue as AttachedIssue | undefined) ?? null,
  );
  // The same live state the tab strip and navigator show (events-fed status,
  // needs-you flag, reconnecting) — not the row's raw `status` enum.
  const liveState = $derived(
    session
      ? sessionState(session, ws.statusMap[session.id], ws.needsYou[session.id] === true, {
          stale: events.state !== 'connected',
        })
      : null,
  );

  // Per-workspace default agent (overrides the global default). '' = inherit.
  const providers = $derived(allProviders());
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

  /** Save the folder list and restart so `--add-dir` takes effect. A WORKING
   *  agent would lose its in-flight turn, so that one case asks first — the
   *  same rule as every other restart path (`ws.requestRestart`); this one
   *  used to restart it silently. Restarting through `ws.restartSession` also
   *  bumps the restart nonce, so the terminal re-attaches to the new PTY. */
  async function applyDirs(dirs: string[], verb: 'added' | 'removed'): Promise<void> {
    if (!session) return;
    const id = session.id;
    if (ws.statusMap[id] === 'working') {
      const ok = await confirmer.ask(
        `“${session.title}” is working right now. Changing its folders restarts it, which stops its current turn (it resumes its saved conversation where it can).`,
        { title: 'Restart working session?', confirmLabel: `Restart and ${verb === 'added' ? 'add' : 'remove'} folder`, danger: true },
      );
      if (!ok) return;
    }
    try {
      await ws.updateSessionMeta(id, { extra_dirs: dirs });
      await ws.restartSession(id, { quiet: true });
      toasts.success(verb === 'added' ? 'Folder added' : 'Folder removed', 'The session restarted with the new folders.');
    } catch (e) {
      toasts.error(verb === 'added' ? 'Could not add folder' : 'Could not remove folder', e instanceof Error ? e.message : String(e));
    }
  }

  async function addDir(path: string): Promise<void> {
    folderPickerOpen = false;
    if (extraDirs.includes(path)) {
      toasts.info('Already added', path);
      return;
    }
    await applyDirs([...extraDirs, path], 'added');
  }

  async function removeDir(dir: string): Promise<void> {
    await applyDirs(extraDirs.filter((d) => d !== dir), 'removed');
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
        <span class="val val-provider"><ProviderIcon provider={session.provider} size={13} />{session.provider}</span>
      </div>
      <div class="row">
        <span class="key">Status</span>
        <span class="val">{#if liveState}<StatusBadge status={liveState} variant="text" />{/if}</span>
      </div>
      {#if session.cwd}
        <div class="row">
          <span class="key">Folder</span>
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
                  class="icon-btn dir-remove"
                  title="Remove {dir} (restarts the session)"
                  aria-label="Remove {dir} (restarts the session)"
                  onclick={() => removeDir(dir)}
                >
                  <Icon name="x" size={12} />
                </button>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="dim no-dirs">No extra folders.</p>
        {/if}
        <button class="btn small add-dir-btn" onclick={() => (folderPickerOpen = true)}>
          <Icon name="plus" size={12} /> Add folder…
        </button>
        <p class="hint">Adding or removing a folder restarts the session.</p>
      </section>
    {/if}

    <section class="section">
      <div class="section-title">Jira issue</div>
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
              <Icon name="external" size={12} />
            </a>
            <span class="chip">{attachedIssue.status}</span>
          </div>
          <div class="issue-summary">{attachedIssue.summary}</div>
          <div class="issue-actions">
            <button class="btn small" onclick={() => (attachOpen = true)}>Change…</button>
            <button class="btn small" onclick={detach}>Detach</button>
          </div>
        </div>
      {:else}
        <div class="no-issue dim">
          <p>No issue attached.</p>
          <!-- Secondary: a side panel never carries the view's primary action. -->
          <button class="btn small" onclick={() => (attachOpen = true)}>
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
    font-size: var(--fs-xs);
    font-weight: 600;
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
    font-size: var(--fs-s);
    min-height: 20px;
  }
  .key {
    /* Wide enough for "Default agent" on one line. */
    width: 76px;
    flex-shrink: 0;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .val {
    flex: 1;
    min-width: 0;
    /* Wrap long tokens (paths, ids) without splitting ordinary words mid-word. */
    overflow-wrap: anywhere;
  }
  .val-provider {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .ws-select {
    width: 100%;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-xs);
    padding: 3px 6px;
  }
  .ws-select:focus {
    outline: none;
    border-color: var(--accent);
  }
  .hint {
    margin: 4px 0 0;
    font-size: var(--fs-xs);
    line-height: 1.4;
    color: var(--text-dim);
  }
  .cwd {
    font-size: var(--fs-xs);
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
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--accent-text);
    text-decoration: none;
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .issue-key:hover {
    text-decoration: underline;
  }
  .issue-summary {
    font-size: var(--fs-s);
    line-height: 1.45;
    color: var(--text);
  }
  .issue-actions {
    display: flex;
    gap: 6px;
    margin-top: 2px;
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
    font-size: var(--fs-s);
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
    font-size: var(--fs-xs);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .dir-remove {
    flex-shrink: 0;
    width: 20px;
    height: 20px;
  }
  .dir-remove:hover {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }
  .no-dirs {
    font-size: var(--fs-s);
    margin: 0 0 6px;
  }
  .add-dir-btn {
    align-self: flex-start;
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
</style>
