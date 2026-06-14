<script lang="ts">
  // Add-workspace sheet: name + project directory. The backend expands `~`
  // and creates the directory if missing, then makes the creator its admin.
  import Modal from '../../lib/components/Modal.svelte';
  import FolderPicker from '../../lib/components/FolderPicker.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  let name = $state('');
  let rootPath = $state('~/');
  let busy = $state(false);
  let pickerOpen = $state(false);

  // Suggest a workspace name from the last path segment as the user types,
  // unless they've already typed a name themselves.
  let nameTouched = $state(false);
  $effect(() => {
    if (nameTouched) return;
    const seg = rootPath.replace(/\/+$/, '').split('/').pop() ?? '';
    if (seg !== '' && seg !== '~') name = seg;
  });

  const canCreate = $derived(name.trim() !== '' && rootPath.trim() !== '' && !busy);

  async function create(): Promise<void> {
    if (!canCreate) return;
    busy = true;
    try {
      const w = await ws.createWorkspace(name, rootPath);
      onclose();
      router.go('agents');
      toasts.success('Workspace created', w.name);
    } catch (e) {
      toasts.error('Could not create workspace', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) void create();
  }
</script>

<Modal title="Add Workspace" {onclose}>
  <div class="field">
    <label for="nw-name">Name</label>
    <input
      id="nw-name"
      class="input"
      bind:value={name}
      oninput={() => (nameTouched = true)}
      onkeydown={onKeydown}
      placeholder="My Project"
    />
  </div>

  <div class="field">
    <label for="nw-path">Project directory</label>
    <div class="path-row">
      <input id="nw-path" class="input mono" bind:value={rootPath} spellcheck="false" onkeydown={onKeydown} placeholder="~/code/my-project" />
      <button class="btn" type="button" onclick={() => (pickerOpen = true)}>Browse…</button>
    </div>
    <span class="hint">Created if it doesn't exist. Sessions and repos run inside it. <code>~</code> expands to your home.</span>
  </div>

  {#snippet footer()}
    <button class="btn" onclick={onclose}>Cancel</button>
    <button class="btn primary" disabled={!canCreate} onclick={create}>
      {busy ? 'Creating…' : 'Create Workspace'}
    </button>
  {/snippet}
</Modal>

{#if pickerOpen}
  <FolderPicker
    title="Choose project directory"
    start={rootPath}
    onpick={(p) => {
      rootPath = p;
      nameTouched = false;
      pickerOpen = false;
    }}
    onclose={() => (pickerOpen = false)}
  />
{/if}

<style>
  .path-row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .path-row .input {
    flex: 1;
  }
  .hint code {
    font-family: var(--font-mono);
    font-size: 11px;
    padding: 0 3px;
    border-radius: 3px;
    background: var(--surface-2);
  }
</style>
