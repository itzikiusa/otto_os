<script lang="ts">
  // The details panel's Comments tab: a composer for a note about the whole
  // design (or "Pin on the design" → Comment mode on the stage), then the
  // threads — open first, resolved collapsed below. Each shows the version it
  // was made on; its chip opens that version. On a phone the details panel is
  // the full-screen sheet this tab lives in (the view's Details button).
  import { onDestroy, untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { api as http } from '../../../lib/api/client';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { copyWithToast } from '../../../lib/clipboard';
  import { plural } from '../../../lib/plural';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { designBus } from '../../../lib/events.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import * as api from '../../../lib/api/design';
  import type { CommentPin3d, DesignVersion } from '../../../lib/api/types';
  import CommentThread from './CommentThread.svelte';
  import type { CommentsStore } from './commentsStore.svelte';
  import { splitThreads } from './pins';
  import { formatPinsMarkdown, formatRegenerateFeedbackPrompt, pickTargetAgentSession } from './pinsDispatch';

  interface Props {
    store: CommentsStore;
    /** The loaded version rows (a jump looks here first). */
    versions: DesignVersion[];
    /** The version the stage shows (new comments are made on it). */
    shownVersionId: string | null;
    shownVersionSeq?: number | null;
    artifact?: { id: string; title: string } | null;
    storyId?: string | null;
    canComment: boolean;
    meId: string | null;
    canDeleteAny?: boolean;
    /** Show one version on the stage. */
    onview: (v: DesignVersion) => void;
    /** Go to the stage in Comment mode (absent where the stage has no pins). */
    onpin?: () => void;
    /** 3D Studio: fly camera to a polygon/mesh comment and highlight it. */
    onfocus3d?: (pin: CommentPin3d) => void;
    /** 3D Studio: open 3D generator pre-filled with open polygon feedback. */
    onregenerate3d?: (prompt: string) => void;
  }
  let {
    store,
    versions,
    shownVersionId,
    shownVersionSeq = null,
    artifact = null,
    storyId = null,
    canComment,
    meId,
    canDeleteAny = false,
    onview,
    onpin,
    onfocus3d,
    onregenerate3d,
  }: Props = $props();

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
  let sending = $state(false);
  let alive = true;
  onDestroy(() => {
    alive = false;
  });

  function buildChecklist(): string {
    return formatPinsMarkdown({
      artifactId: artifact?.id ?? store.artifactId,
      artifactTitle: artifact?.title ?? 'Design',
      versionSeq: shownVersionSeq,
      threads: store.threads,
    });
  }

  async function sendToAgent(sessionId: string, sessionName: string): Promise<void> {
    if (sending) return;
    sending = true;
    try {
      await http.post(`/sessions/${encodeURIComponent(sessionId)}/input`, {
        text: `${buildChecklist()}\n`,
        submit: false,
      });
      if (!alive) return;
      toasts.success(
        'Open pins drafted in agent session',
        `Loaded ${plural(groups.open.length, 'item')} into “${sessionName}”.`,
      );
      ws.navigateToSession(sessionId);
    } catch (e) {
      if (!alive) return;
      toastError('Couldn’t send pins to agent session', e);
    } finally {
      if (alive) sending = false;
    }
  }

  async function sendToStory(targetStoryId: string): Promise<void> {
    if (sending) return;
    sending = true;
    try {
      await http.post(`/product/stories/${encodeURIComponent(targetStoryId)}/notes`, {
        body: buildChecklist(),
        section: 'Design review',
      });
      if (!alive) return;
      toasts.success(
        'Open pins added to story notes',
        `Saved ${plural(groups.open.length, 'item')} to the linked story.`,
      );
    } catch (e) {
      if (!alive) return;
      toastError('Couldn’t add pins to the story', e);
    } finally {
      if (alive) sending = false;
    }
  }

  function openSendMenu(e: MouseEvent): void {
    const targetSession = pickTargetAgentSession(ws.plainAgentSessions, ws.activeSessionId);
    const items: MenuItem[] = [
      targetSession
        ? {
            label: `Send to agent (${targetSession.title || 'Active session'})`,
            icon: 'terminal',
            action: () => void sendToAgent(targetSession.id, targetSession.title || 'Active session'),
          }
        : {
            label: 'No open agent session',
            icon: 'terminal',
            disabled: true,
            action: () => {},
          },
      ...(storyId
        ? [
            {
              label: 'Append checklist to linked story notes',
              icon: 'note' as const,
              action: () => void sendToStory(storyId),
            },
          ]
        : []),
      ...(onregenerate3d
        ? [
            {
              label: 'Regenerate 3D with polygon feedback…',
              icon: 'sparkle' as const,
              action: () => onregenerate3d(formatRegenerateFeedbackPrompt(artifact?.title ?? '3D model', store.threads)),
            },
          ]
        : []),
      { separator: true },
      {
        label: 'Copy checklist as Markdown',
        icon: 'copy',
        action: () => void copyWithToast(buildChecklist(), 'Copied open pins checklist'),
      },
    ];
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, { align: 'end' });
  }

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
      <div class="ct-dispatch">
        <span class="ct-open-count">{groups.open.length} open</span>
        <span class="grow"></span>
        <button
          type="button"
          class="btn small ghost"
          onclick={openSendMenu}
          disabled={sending}
          aria-haspopup="menu"
          title="Send unresolved review pins to an active agent session or linked story"
          data-testid="design-send-pins"
        >
          <Icon name="send" size={12} /> {sending ? 'Sending…' : 'Send to Agent / Plan…'}
        </button>
      </div>
      <ul class="list" aria-label="Open comments">
        {#each groups.open as t (t.root.id)}
          <CommentThread thread={t} {store} {shownVersionId} {meId} {canComment} {canDeleteAny} onjump={(id, seq) => void jump(id, seq)} {onfocus3d} />
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
            <CommentThread thread={t} {store} {shownVersionId} {meId} {canComment} {canDeleteAny} onjump={(id, seq) => void jump(id, seq)} {onfocus3d} />
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
  .ct-dispatch {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-block-end: 4px;
    border-block-end: 1px solid var(--border);
  }
  .ct-open-count {
    font-size: var(--fs-xs);
    color: var(--text-dim);
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
