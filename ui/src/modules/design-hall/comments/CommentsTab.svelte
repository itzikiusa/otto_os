<script lang="ts">
  // The details panel's Comments tab: a composer for a note about the whole
  // design (or "Pin on the design" → Comment mode on the stage), then the
  // threads — open first, resolved collapsed below. Each shows the version it
  // was made on; its chip opens that version. On a phone the details panel is
  // the full-screen sheet this tab lives in (the view's Details button).
  import { untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { designBus } from '../../../lib/events.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import * as api from '../../../lib/api/design';
  import type { DesignVersion } from '../../../lib/api/types';
  import CommentThread from './CommentThread.svelte';
  import type { CommentsStore } from './commentsStore.svelte';
  import { splitThreads } from './pins';

  interface Props {
    store: CommentsStore;
    /** The loaded version rows (a jump looks here first). */
    versions: DesignVersion[];
    /** The version the stage shows (new comments are made on it). */
    shownVersionId: string | null;
    canComment: boolean;
    meId: string | null;
    canDeleteAny?: boolean;
    /** Show one version on the stage. */
    onview: (v: DesignVersion) => void;
    /** Go to the stage in Comment mode (absent where the stage has no pins). */
    onpin?: () => void;
  }
  let { store, versions, shownVersionId, canComment, meId, canDeleteAny = false, onview, onpin }: Props = $props();

  $effect(() => {
    const s = store;
    untrack(() => s.open(designBus));
  });
  $effect(() => {
    void designBus.seq;
    void designBus.resyncTick;
    untrack(() => store.follow(designBus));
  });

  const groups = $derived(splitThreads(store.threads));
  let showResolved = $state(false);
  let text = $state('');
  let posting = $state(false);

  async function post(): Promise<void> {
    const body = text.trim();
    if (!body || posting) return;
    posting = true;
    try {
      const c = await store.create(body, { versionId: shownVersionId });
      text = '';
      store.selectedId = c.id;
    } catch (e) {
      toastError('Couldn’t post the comment', e);
    } finally {
      posting = false;
    }
  }

  function pin(): void {
    store.mode = true;
    onpin?.();
  }

  async function jump(versionId: string, seq: number): Promise<void> {
    let v = versions.find((x) => x.id === versionId);
    if (!v) {
      try {
        v = (await api.listVersions(store.artifactId, { limit: 200 })).find((x) => x.id === versionId);
      } catch (e) {
        return void toastError(`Couldn’t open v${seq}`, e);
      }
    }
    if (v) onview(v);
    else toasts.info(`v${seq} is no longer in history`, 'It was tidied away; the comment stays.');
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void post();
    }
  }
</script>

<div class="ct" data-testid="design-comments">
  {#if canComment}
    <div class="compose">
      <textarea dir="auto" bind:value={text} rows="2" maxlength="4000" placeholder="Comment on the whole design…" aria-label="New comment" onkeydown={onKey}></textarea>
      <div class="ct-row">
        {#if onpin}
          <button class="btn small ghost" onclick={pin} title="Click a spot on the design to pin a comment (C)" data-testid="design-comment-pin">
            <Icon name="pin" size={12} /> Pin on the design
          </button>
        {/if}
        <span class="grow"></span>
        <button class="btn small primary" onclick={() => void post()} disabled={!text.trim() || posting}>{posting ? 'Posting…' : 'Comment'}</button>
      </div>
    </div>
  {/if}
  <LoadState what="comments" loading={store.loading || (!store.loaded && !store.error)} error={store.error} empty={store.items.length === 0} onretry={() => void store.load()}>
    {#snippet emptyView()}
      <EmptyState variant="panel" icon="comment" title="No comments yet"
        body={onpin ? 'Press C or use Comment to pin one on the design.' : 'Write one above about the whole design.'} />
    {/snippet}
    {#if groups.open.length}
      <ul class="list" aria-label="Open comments">
        {#each groups.open as t (t.root.id)}
          <CommentThread thread={t} {store} {shownVersionId} {meId} {canComment} {canDeleteAny} onjump={(id, seq) => void jump(id, seq)} />
        {/each}
      </ul>
    {:else}
      <p class="none">Every comment is resolved.</p>
    {/if}
    {#if groups.resolved.length}
      <button class="btn small ghost fold" aria-expanded={showResolved} onclick={() => (showResolved = !showResolved)} data-testid="design-comments-resolved">
        <Icon name={showResolved ? 'chevronDown' : 'chevronRight'} size={12} /> Resolved ({groups.resolved.length})
      </button>
      {#if showResolved}
        <ul class="list" aria-label="Resolved comments">
          {#each groups.resolved as t (t.root.id)}
            <CommentThread thread={t} {store} {shownVersionId} {meId} {canComment} {canDeleteAny} onjump={(id, seq) => void jump(id, seq)} />
          {/each}
        </ul>
      {/if}
    {/if}
  </LoadState>
</div>

<style>
  .ct {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 12px 14px;
  }
  .compose {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .compose textarea {
    inline-size: 100%;
    font: inherit;
    font-size: var(--fs-s);
    resize: vertical;
  }
  .ct-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .grow {
    flex: 1;
  }
  .list {
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .none {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .fold {
    align-self: flex-start;
  }
</style>
