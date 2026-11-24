<script lang="ts">
  // ⋯ › Additional directories… — the folders an agent may access beyond its
  // working folder (meta.extra_dirs → `--add-dir`). Its own chunk: a rarely
  // used dialog, loaded when opened (SessionView).
  import { untrack } from 'svelte';
  import DirList from './DirList.svelte';
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
      {#snippet pathField(inner: import('svelte').Snippet)}<PathField bind:value={dirDraft}>{@render inner()}</PathField>{/snippet}
      <DirList bind:dirs={extraDirs} bind:draft={dirDraft} inputId="sv-extra-dir" field={pathField} />
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
