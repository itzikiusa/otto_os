<script lang="ts">
  // ⋯ › Additional directories… — the folders an agent may access beyond its
  // working folder (meta.extra_dirs → `--add-dir`). Its own chunk: a rarely
  // used dialog, loaded when opened (SessionView).
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import PathField from '../../lib/components/PathField.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { saveAndRestartChoice } from '../../lib/freshRestart';
  import { loadErrorText } from '../../lib/loadError';
  import { toastError } from '../../lib/toastError';
  import { toasts } from '../../lib/toast.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import type { Session } from '../../lib/api/types';

  let { sessionId, session, onclose }: { sessionId: string; session: Session | null; onclose: () => void } = $props();

  let dirsBusy = $state(false);
  // The dialog edits a copy taken when it opens.
  const seed = untrack(() => session?.meta?.extra_dirs);
  let extraDirs = $state<string[]>(Array.isArray(seed) ? seed.filter((d): d is string => typeof d === 'string') : []);
  let dirDraft = $state('');

  function addDir(): void {
    const path = dirDraft.trim();
    if (path === '' || extraDirs.includes(path)) {
      dirDraft = '';
      return;
    }
    extraDirs = [...extraDirs, path];
    dirDraft = '';
  }

  function removeDir(dir: string): void {
    extraDirs = extraDirs.filter((d) => d !== dir);
  }

  function onDirKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      addDir();
    }
  }

  /** Collect the list, folding any typed-but-unadded draft, trimming + de-duping. */
  function collectDirs(): string[] {
    const dirs = [...extraDirs];
    const pending = dirDraft.trim();
    if (pending !== '' && !dirs.includes(pending)) dirs.push(pending);
    return dirs;
  }

  async function saveDirs(alsoRestart: boolean): Promise<void> {
    if (dirsBusy) return;
    // Working-guard (ws.requestRestart's) + the can't-resume ask; null = cancelled.
    const restart = alsoRestart ? await saveAndRestartChoice(session, () => ws.confirmRestart(sessionId), confirmer) : false; if (restart === null) return;
    dirsBusy = true;
    try {
      try {
        await ws.updateSessionMeta(sessionId, { extra_dirs: collectDirs() });
      } catch (e) {
        // Nothing was saved: keep the dialog open so the edit isn't lost.
        toastError('Couldn’t save the session', e);
        return;
      }
      // The directories ARE saved from here on: close the dialog (a second
      // Save would only repeat it), and report a failed restart as exactly that.
      onclose();
      if (!restart) {
        toasts.success('Directories saved', 'Applies the next time this session restarts.');
        return;
      }
      try {
        await ws.restartSession(sessionId, { quiet: true });
        toasts.success('Directories saved', 'Session restarted with the new directories.');
      } catch (e) {
        toasts.push('error', 'Directories saved, but the session didn’t restart', loadErrorText(e), 12000, {
          action: { label: 'Restart', run: () => ws.requestRestart(sessionId) },
        });
      }
    } finally {
      dirsBusy = false;
    }
  }
</script>

  <Modal title="Additional directories" {onclose}>
    <div class="field">
      <label for="sv-extra-dir">Folders the agent may access <span class="dim">(beyond its working folder)</span></label>
      {#if extraDirs.length > 0}
        <ul class="dir-list">
          {#each extraDirs as dir (dir)}
            <li class="dir-row">
              <span class="dir-path mono" dir="ltr" title={dir}>{dir}</span>
              <button
                type="button"
                class="dir-remove"
                title="Remove directory"
                aria-label="Remove {dir}"
                onclick={() => removeDir(dir)}
              ><Icon name="x" size={12} /></button>
            </li>
          {/each}
        </ul>
      {/if}
      <div class="dir-add">
        <PathField bind:value={dirDraft}><input dir="ltr"
          id="sv-extra-dir"
          class="input mono"
          bind:value={dirDraft}
          spellcheck="false"
          placeholder="/absolute/path/to/repo"
          onkeydown={onDirKeydown}
        /></PathField>
        <button type="button" class="btn" disabled={dirDraft.trim() === ''} onclick={addDir}>Add</button>
      </div>
      <span class="hint">Passed as <code>--add-dir</code>. Takes effect on the next session restart.</span>
    </div>

    {#snippet footer()}
      <button class="btn" onclick={onclose}>Cancel</button>
      <button class="btn" disabled={dirsBusy} onclick={() => saveDirs(false)}>
        {dirsBusy ? 'Saving…' : 'Save'}
      </button>
      <button class="btn primary" disabled={dirsBusy} onclick={() => saveDirs(true)}>
        {dirsBusy ? 'Saving…' : 'Save & restart'}
      </button>
    {/snippet}
  </Modal>

<style>
  .dir-list {
    list-style: none;
    margin: 0 0 6px;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .dir-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 4px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
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
    display: inline-flex;
    align-items: center;
    background: none;
    border: none;
    cursor: pointer;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    padding: 2px 4px;
    border-radius: var(--radius-s);
    line-height: 1;
  }
  .dir-remove:hover {
    color: var(--danger);
    background: color-mix(in srgb, var(--danger) 12%, transparent);
  }
  .dir-add {
    display: flex;
    gap: 6px;
  }
  .dir-add :global(.input) {
    flex: 1;
    min-width: 0;
  }
</style>
