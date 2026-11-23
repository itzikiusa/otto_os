<script lang="ts">
  // 1-click "Create Product story from note…" — creates a Product draft story
  // seeded with the note's title, body, and `otto://vault/<id>/<path>` back-link,
  // then navigates straight to `#/product/<id>`.
  import { onDestroy } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { api } from '../../lib/api/client';
  import type { ProductStoryDetail } from '../../lib/api/types/product';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { announceShown } from '../../lib/announce.svelte';
  import { buildStoryBodyFromNote, extractNoteTitle, vaultRefUri } from './noteToStory';

  interface Props {
    vaultId: number;
    notePath: string;
    raw: string;
    metaTitle?: string | null;
  }
  let { vaultId, notePath, raw, metaTitle = null }: Props = $props();

  const canCreate = $derived(auth.can('product', 'edit'));
  const backLink = $derived(vaultRefUri(vaultId, notePath));

  let open = $state(false);
  let title = $state('');
  let includeBody = $state(true);
  let creating = $state(false);
  let error = $state<string | null>(null);
  let alive = true;

  onDestroy(() => {
    alive = false;
  });

  function openModal(): void {
    title = extractNoteTitle(notePath, raw, metaTitle);
    includeBody = true;
    error = null;
    open = true;
  }

  async function submit(): Promise<void> {
    if (creating) return;
    const wsId = ws.currentId;
    const trimmedTitle = title.trim();
    if (!wsId) {
      error = 'Select a workspace first.';
      return;
    }
    if (!trimmedTitle) {
      error = 'Enter a story title.';
      return;
    }
    creating = true;
    error = null;
    try {
      const detail = await api.post<ProductStoryDetail>(
        `/workspaces/${encodeURIComponent(wsId)}/product/drafts`,
        { title: trimmedTitle },
      );
      const bodyMd = buildStoryBodyFromNote({ vaultId, notePath, raw, includeBody });
      await api.patch<ProductStoryDetail>(
        `/product/stories/${encodeURIComponent(detail.story.id)}/draft`,
        { title: trimmedTitle, body_md: bodyMd },
      );
      if (!alive) return;
      open = false;
      toasts.success('Product story created', `Seeded from ${notePath}.`);
      router.go(`product/${encodeURIComponent(detail.story.id)}`);
    } catch (e) {
      if (!alive) return;
      error = loadErrorText(e);
    } finally {
      if (alive) creating = false;
    }
  }
</script>

{#if canCreate}
  <button
    type="button"
    class="mode-btn"
    title="Create Product story from this note…"
    aria-label="Create Product story from this note"
    onclick={openModal}
    data-testid="vault-create-story-btn"
  >
    <Icon name="compass" size={14} />
  </button>
{/if}

{#if open}
  <Modal title="Create Product story from note" width={480} dismissable={!creating} onclose={() => (open = false)}>
    <div class="cs-form">
      <p class="cs-lead">
        Creates a new Product draft story back-linked to <code>{backLink}</code>.
      </p>
      <div class="cs-field">
        <label for="cs-title">Story title</label>
        <input
          id="cs-title"
          class="input"
          dir="auto"
          bind:value={title}
          disabled={creating}
          placeholder="Story title"
          onkeydown={(e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void submit(); } }}
        />
      </div>
      <label class="cs-check">
        <input type="checkbox" bind:checked={includeBody} disabled={creating} />
        <span>Copy note body into the story specification</span>
      </label>
      {#if error}
        <p class="cs-err" use:announceShown={{ text: error, assertive: true }}>
          <Icon name="warning" size={13} /> {error}
        </p>
      {/if}
    </div>
    {#snippet footer()}
      <button type="button" class="btn" onclick={() => (open = false)} disabled={creating}>Cancel</button>
      <button
        type="button"
        class="btn primary"
        onclick={() => void submit()}
        disabled={creating || !title.trim()}
        data-testid="vault-create-story-confirm"
      >
        {creating ? 'Creating…' : 'Create story'}
      </button>
    {/snippet}
  </Modal>
{/if}

<style>
  .cs-form {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .cs-lead {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.45;
  }
  .cs-lead code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  .cs-field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .cs-field label {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text);
  }
  .cs-check {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .cs-err {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--danger);
  }
</style>
