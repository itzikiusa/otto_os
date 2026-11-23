<script lang="ts">
  // Workspace-confined asset export sheet shared by 3D Studio (GLB/glTF/OBJ)
  // and Voice Studio (TTS WAV/MP3/M4A/AIFF). Backend enforces workspace root
  // confinement, symlink/traversal rejection and explicit overwrite opt-in.
  import { untrack } from 'svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { ApiError } from '../../../lib/api/client';
  import { loadErrorText } from '../../../lib/loadError';
  import { toasts } from '../../../lib/toast.svelte';
  import type { ExportWorkspaceResp } from '../../../lib/api/types';

  interface Props {
    title: string;
    defaultPath: string;
    hint?: string;
    onexport: (relativePath: string, overwrite: boolean) => Promise<ExportWorkspaceResp>;
    onclose: () => void;
    onexported?: (resp: ExportWorkspaceResp) => void;
  }
  let { title, defaultPath, hint, onexport, onclose, onexported }: Props = $props();

  let relativePath = $state(untrack(() => defaultPath));
  let overwrite = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let conflictExists = $state(false);

  async function submit(): Promise<void> {
    const clean = relativePath.trim();
    if (!clean || busy) return;
    busy = true;
    error = null;
    try {
      const resp = await onexport(clean, overwrite);
      toasts.success('Exported to workspace', resp.path);
      onexported?.(resp);
      onclose();
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) {
        conflictExists = true;
        error = 'A file already exists at that path. Enable Overwrite to replace it.';
      } else {
        error = loadErrorText(e);
      }
    } finally {
      busy = false;
    }
  }
</script>

<Modal {title} width={480} {onclose}>
  <form
    class="we"
    onsubmit={(e) => {
      e.preventDefault();
      void submit();
    }}
    data-testid="workspace-export-modal"
  >
    <div class="we-field">
      <label for="we-path">Workspace-relative path</label>
      <input
        id="we-path"
        dir="ltr"
        type="text"
        class="input"
        bind:value={relativePath}
        placeholder="assets/models/hero.glb"
        required
        data-testid="workspace-export-path"
      />
      {#if hint}<span class="we-hint">{hint}</span>{/if}
    </div>

    <label class="we-check">
      <input type="checkbox" bind:checked={overwrite} />
      <span>Overwrite existing file if present</span>
    </label>

    {#if error}
      <p class="we-err" use:announceShown={{ text: error, assertive: true }}>
        <Icon name="warning" size={12} /> {error}
      </p>
    {/if}
    {#if conflictExists && !overwrite}
      <p class="we-hint">Check “Overwrite existing file” above to replace the file safely.</p>
    {/if}
  </form>

  {#snippet footer()}
    <button type="button" class="btn" onclick={onclose}>Cancel</button>
    <button
      type="button"
      class="btn primary"
      disabled={!relativePath.trim() || busy}
      onclick={() => void submit()}
      data-testid="workspace-export-submit"
    >{busy ? 'Exporting…' : 'Export to workspace'}</button>
  {/snippet}
</Modal>

<style>
  .we {
    display: flex;
    flex-direction: column;
    gap: 12px;
    max-block-size: 72dvh;
    overflow-y: auto;
    min-block-size: 0;
  }
  .we-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .we-field label {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .we-hint {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin: 0;
  }
  .we-check {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    color: var(--text);
    cursor: pointer;
  }
  .we-err {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-s);
  }
  @media (pointer: coarse) {
    .we :global(.input) {
      min-block-size: 40px;
      font-size: 16px; /* ui-guards: allow */
    }
  }
</style>
