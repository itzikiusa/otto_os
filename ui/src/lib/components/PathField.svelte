<script lang="ts">
  import type { Snippet } from 'svelte';
  import FolderPicker from './FolderPicker.svelte';
  interface Props {
    value?: string;
    start?: string;
    files?: boolean;
    disabled?: boolean;
    onpick?: (path: string) => void;
    children: Snippet;
  }
  let { value = $bindable(''), start = '', files = false, disabled = false, onpick, children }: Props = $props();
  let open = $state(false);
  const initial = $derived.by(() => {
    const path = value && !value.includes('{{') ? value : start;
    return files && path.includes('/') ? path.slice(0, path.lastIndexOf('/')) || '/' : path;
  });
</script>
<div class="path-field">
  {@render children()}
  <button type="button" class="btn" {disabled} title={files ? 'Browse files' : 'Browse folders'} onclick={() => (open = true)}>Browse…</button>
</div>
{#if open}
  <FolderPicker title={files ? 'Choose file' : 'Choose folder'} start={initial} {files}
    onpick={(path) => { value = path; onpick?.(path); open = false; }} onclose={() => (open = false)} />
{/if}
<style>
  .path-field { display: flex; align-items: center; gap: 6px; min-width: 0; width: 100%; }
  .path-field :global(input) { flex: 1; min-width: 0; width: 0; }
  button { flex-shrink: 0; }
</style>
