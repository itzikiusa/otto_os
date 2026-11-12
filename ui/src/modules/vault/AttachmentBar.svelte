<script lang="ts">
  // Inline status for paste/drop attachments above the note editor: one row
  // per file while it saves, and a failed save stays as an error row with
  // Retry (same bytes) and Dismiss. A saved file's embed is inserted at the
  // cursor of the note it was dropped into — never into another note.
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { uploadVaultAttachment } from '../../lib/api/vault';
  import { loadErrorText } from '../../lib/loadError';
  import { announce } from '../../lib/announce.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { vault } from './vault.svelte';
  import { MAX_ATTACHMENT_BYTES, insertionFor, preflight, uploadName } from './attachmentUpload';

  interface Props {
    /** The open note (anchors the vault's `./` attachment settings). */
    note: string | null;
    /** Insert at the cursor if `note` is still the edited note; false otherwise. */
    insert: (text: string, note: string | null) => boolean;
  }
  let { note, insert }: Props = $props();

  interface Item {
    id: number;
    file: File;
    name: string;
    label: string;
    saving: boolean;
    error: string;
    retry: boolean;
    index: number;
    vaultId: number;
    note: string | null;
  }
  let items = $state<Item[]>([]);
  let seq = 0;

  /** Queue `files` from one paste/drop; each saves independently. */
  export function add(files: File[], source: 'paste' | 'drop'): void {
    const v = vault.current;
    if (!v) return;
    files.forEach((file, index) => {
      const refused = preflight(file);
      const item: Item = {
        id: ++seq,
        file,
        name: uploadName(file, source),
        label: file.name || 'Pasted image',
        saving: !refused,
        error: refused ?? '',
        retry: !refused,
        index,
        vaultId: v.id,
        note,
      };
      items = [...items, item];
      if (!refused) void send(item.id);
    });
  }

  function patch(id: number, p: Partial<Item>): void {
    items = items.map((i) => (i.id === id ? { ...i, ...p } : i));
  }

  function errorText(e: unknown): string {
    if ((e as { status?: number } | null)?.status === 413) {
      return `It is larger than ${MAX_ATTACHMENT_BYTES / (1024 * 1024)} MB.`;
    }
    return loadErrorText(e);
  }

  async function send(id: number): Promise<void> {
    const it = items.find((i) => i.id === id);
    if (!it) return;
    patch(id, { saving: true, error: '' });
    try {
      const saved = await uploadVaultAttachment(vault.wsId, it.vaultId, it.note, it.name, it.file);
      items = items.filter((i) => i.id !== id);
      const placed = vault.current?.id === it.vaultId && insert(insertionFor(saved.embed, it.index), it.note);
      if (placed) announce(`Attached ${saved.path}`);
      else toasts.info(`Saved ${saved.path}`, `The note changed before the link was inserted. Add it with ${saved.embed}`);
    } catch (e) {
      patch(id, { saving: false, error: errorText(e), retry: true });
    }
  }

  function dismiss(id: number): void {
    items = items.filter((i) => i.id !== id);
  }
</script>

{#if items.length}
  <div class="att">
    {#each items as it (it.id)}
      {#if it.saving}
        <div class="att-saving"><Icon name="image" size={13} /> Saving {it.label}…</div>
      {:else}
        <LoadState
          variant="compact"
          what="attachment"
          errorTitle={`Couldn’t attach ${it.label}`}
          error={it.error}
          empty
          onretry={it.retry ? () => void send(it.id) : undefined}
        >
          {#snippet errorActions()}
            <button class="icon-btn" aria-label="Dismiss" title="Dismiss" onclick={() => dismiss(it.id)}>
              <Icon name="x" size={12} />
            </button>
          {/snippet}
        </LoadState>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .att {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-block: 6px;
    padding-inline: 12px;
  }
  .att-saving {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    background: var(--info-soft);
    color: var(--text);
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .att-saving :global(svg) {
    flex: none;
    color: var(--info);
  }
</style>
