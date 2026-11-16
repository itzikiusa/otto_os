<script lang="ts">
  // Editor | Editor and preview | Preview — three icon buttons at the end of the
  // tab strip (the JetBrains trio), shown only for a previewable file. The
  // stacked phone layout gets the two that apply (Editor, Preview). The pressed
  // one is the mode the pane shows; the choice is remembered per file kind.
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import type { FilePreviewMode } from '../../../lib/components/filePreviewKind';
  import { ide } from '../ideStore.svelte';
  import { idePreviewMode } from './idePreviewMode.svelte';

  const BUTTONS: Record<FilePreviewMode, { icon: IconName; label: string }> = {
    source: { icon: 'editorLines', label: 'Editor' },
    split: { icon: 'editorSplit', label: 'Editor and preview' },
    preview: { icon: 'image', label: 'Preview' },
  };

  const path = $derived(ide.activePath);
  const modes = $derived(idePreviewMode.available(path));
  const current = $derived(idePreviewMode.modeFor(path));
</script>

{#if modes.length > 0}
  <div class="segmented ide-pv-toggle" role="group" aria-label="Editor and preview mode" data-testid="ide-preview-toggle">
    {#each modes as m (m)}
      <button
        type="button"
        aria-pressed={current === m}
        aria-label={BUTTONS[m].label}
        title={BUTTONS[m].label}
        onclick={() => idePreviewMode.set(path, m)}
        data-testid={`ide-preview-mode-${m}`}
      >
        <Icon name={BUTTONS[m].icon} size={13} />
      </button>
    {/each}
  </div>
{/if}

<style>
  .ide-pv-toggle {
    flex-shrink: 0;
    margin-inline-end: var(--sp-1);
  }
  .ide-pv-toggle > button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    padding-inline: 6px;
  }
</style>
