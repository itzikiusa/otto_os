<script lang="ts">
  // One review thread in the Comments tab: the pin number, author, time and
  // the version it was made on ("made on v3" jumps there), the body and its
  // replies; Reply / Resolve · Reopen for editors, Edit / Delete (⋯) for the
  // author (a workspace admin may delete any — the daemon decides). Selecting
  // the thread highlights its pin on the stage.
  import Icon from '../../../lib/components/Icon.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { toastError } from '../../../lib/toastError';
  import type { DesignComment } from '../../../lib/api/types';
  import type { CommentsStore } from './commentsStore.svelte';
  import { pinWords, versionChip, type Thread } from './pins';

  interface Props {
    thread: Thread;
    store: CommentsStore;
    shownVersionId: string | null;
    meId: string | null;
    canComment: boolean;
    /** Root may delete anyone's comment (workspace admins get the daemon's answer). */
    canDeleteAny: boolean;
    onjump: (versionId: string, seq: number) => void;
  }
  let { thread, store, shownVersionId, meId, canComment, canDeleteAny, onjump }: Props = $props();

  const root = $derived(thread.root);
  const selected = $derived(store.selectedId === root.id);
  const chip = $derived(versionChip(root, shownVersionId));
  let el = $state<HTMLElement | null>(null);
  let replying = $state(false);
  let replyText = $state('');
  let editingId = $state<string | null>(null);
  let editText = $state('');
  let busy = $state(false);

  $effect(() => {
    if (selected) el?.scrollIntoView({ block: 'nearest' });
  });

  const own = (c: DesignComment) => c.author_kind === 'user' && c.author_id === meId;
  const when = (iso: string) => new Date(iso).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });

  async function run(what: string, f: () => Promise<unknown>): Promise<boolean> {
    if (busy) return false;
    busy = true;
    try {
      await f();
      return true;
    } catch (e) {
      toastError(what, e);
      return false;
    } finally {
      busy = false;
    }
  }

  async function sendReply(): Promise<void> {
    const body = replyText.trim();
    if (!body) return;
    if (await run('Couldn’t post the reply', () => store.create(body, { parentId: root.id }))) {
      replyText = '';
      replying = false;
    }
  }

  async function saveEdit(c: DesignComment): Promise<void> {
    const body = editText.trim();
    if (!body || body === c.body) return void (editingId = null);
    if (await run('Couldn’t save the comment', () => store.update(c.id, { body }))) editingId = null;
  }

  async function remove(c: DesignComment): Promise<void> {
    const isRoot = !c.parent_id;
    const ok = await confirmer.ask(
      isRoot && thread.replies.length
        ? `Delete this comment and its ${thread.replies.length === 1 ? 'reply' : `${thread.replies.length} replies`}? This can’t be undone.`
        : 'Delete this comment? This can’t be undone.',
      { title: 'Delete comment', confirmLabel: 'Delete', danger: true },
    );
    if (ok) await run('Couldn’t delete the comment', () => store.remove(c.id));
  }

  function more(e: MouseEvent, c: DesignComment): void {
    const items: MenuItem[] = [
      { label: 'Edit', icon: 'edit', disabled: !own(c) || !canComment, action: () => { editingId = c.id; editText = c.body; } },
      { label: 'Delete…', icon: 'trash', danger: true, disabled: !(own(c) || canDeleteAny) || !canComment, action: () => void remove(c) },
    ];
    ctxMenu.show(e, items);
  }

  function onKey(e: KeyboardEvent, submit: () => void, cancel: () => void): void {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      submit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancel();
    }
  }
</script>

{#snippet entry(c: DesignComment)}
  <div class="entry" class:reply={!!c.parent_id}>
    <div class="meta">
      {#if !c.parent_id}<span class="num" class:pinned={!!c.pin} title={c.pin ? `Pinned at ${pinWords(c.pin)}` : 'About the whole design'}>{#if c.pin}{thread.n}{:else}<Icon name="comment" size={12} />{/if}</span>{/if}
      <strong class="who">{c.author_kind === 'agent' ? `Otto · ${c.author_name}` : c.author_name}</strong>
      <time class="dim" datetime={c.created_at}>{when(c.created_at)}</time>
      {#if c.edited_at}<span class="dim">· edited</span>{/if}
      <span class="grow"></span>
      {#if canComment && (own(c) || canDeleteAny)}
        <button class="icon-btn more-btn" onclick={(e) => more(e, c)} aria-label="More actions for this comment" title="More actions" aria-haspopup="menu"><Icon name="more" size={12} /></button>
      {/if}
    </div>
    {#if editingId === c.id}
      <textarea class="field" dir="auto" bind:value={editText} rows="3" maxlength="4000" aria-label="Edit comment"
        onkeydown={(e) => onKey(e, () => void saveEdit(c), () => (editingId = null))}></textarea>
      <div class="acts">
        <button class="btn small ghost" onclick={() => (editingId = null)}>Cancel</button>
        <button class="btn small primary" onclick={() => void saveEdit(c)} disabled={busy || !editText.trim()}>Save</button>
      </div>
    {:else}
      <p class="body">{c.body}</p>
    {/if}
  </div>
{/snippet}

<li class="thread" class:sel={selected} class:resolved={root.resolved} bind:this={el} data-testid="design-comment" data-comment-id={root.id}>
  <button class="select" onclick={() => (store.selectedId = selected ? null : root.id)} aria-pressed={selected}
    aria-label={`${selected ? 'Unselect' : 'Select'} comment ${thread.n}${root.pin ? ' and highlight its pin' : ''}`} title={root.pin ? 'Highlight its pin' : 'Select'}></button>
  {@render entry(root)}
  {#if chip || root.resolved}
    <div class="tags">
      {#if chip && root.version_id && root.version_seq != null}
        {@const vid = root.version_id}
        {@const seq = root.version_seq}
        <button class="ver-jump" onclick={() => onjump(vid, seq)} title={`Show v${seq}, the version this was said about`} data-testid="design-comment-version">{chip}</button>
      {/if}
      {#if root.resolved}<span class="dim"><Icon name="check" size={12} /> Resolved{root.resolved_by_name ? ` by ${root.resolved_by_name}` : ''}</span>{/if}
    </div>
  {/if}
  {#each thread.replies as r (r.id)}
    {@render entry(r)}
  {/each}
  {#if canComment}
    {#if replying}
      <textarea class="field" dir="auto" bind:value={replyText} rows="2" maxlength="4000" aria-label="Reply" placeholder="Reply…"
        onkeydown={(e) => onKey(e, () => void sendReply(), () => (replying = false))}></textarea>
      <div class="acts">
        <button class="btn small ghost" onclick={() => (replying = false)}>Cancel</button>
        <button class="btn small primary" onclick={() => void sendReply()} disabled={busy || !replyText.trim()}>Reply</button>
      </div>
    {:else}
      <div class="acts">
        <button class="btn small ghost" onclick={() => (replying = true)}>Reply</button>
        <button class="btn small ghost" disabled={busy} data-testid="design-comment-resolve"
          onclick={() => void run(root.resolved ? 'Couldn’t reopen the comment' : 'Couldn’t resolve the comment', () => store.update(root.id, { resolved: !root.resolved }))}>
          <Icon name={root.resolved ? 'undo' : 'check'} size={12} /> {root.resolved ? 'Reopen' : 'Resolve'}
        </button>
      </div>
    {/if}
  {/if}
</li>

<style>
  .thread {
    position: relative;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--bg);
  }
  .thread.sel {
    border-color: var(--accent-solid);
    box-shadow: 0 0 0 1px var(--accent-solid);
  }
  .thread.resolved .body {
    color: var(--text-dim);
  }
  /* The whole card selects; the real controls sit above this layer. */
  .select {
    position: absolute;
    inset: 0;
    padding: 0;
    border: 0;
    background: none;
    border-radius: inherit;
    cursor: pointer;
  }
  .select:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .entry.reply {
    margin-inline-start: 14px;
    padding-inline-start: 10px;
    border-inline-start: 2px solid var(--border);
  }
  .meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .num {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-inline-size: 20px;
    block-size: 20px;
    padding: 0 6px;
    border-radius: 999px;
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
  .num.pinned {
    background: var(--accent-solid);
    border-color: var(--accent-solid);
    color: var(--accent-contrast);
  }
  .who {
    font-weight: 600;
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .grow {
    flex: 1;
  }
  /* Above the card-wide select layer. */
  .more-btn,
  .acts,
  .ver-jump,
  .field {
    position: relative;
    z-index: 1;
  }
  .body {
    margin: 0;
    font-size: var(--fs-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .tags {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }
  .ver-jump {
    padding: 0;
    border: 0;
    background: none;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .ver-jump:hover {
    text-decoration: underline;
  }
  .acts {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }
  .field {
    inline-size: 100%;
    font: inherit;
    font-size: var(--fs-s);
    resize: vertical;
  }
</style>
