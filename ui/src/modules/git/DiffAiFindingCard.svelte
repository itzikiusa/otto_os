<script lang="ts">
  // Inline AI review finding / draft comment card rendered inside DiffViewer
  // on matching diff lines (E-08). Follows patterns.md §2 attribution:
  // neutral 2px left rule (--border-strong), <AgentChip />, severity <Badge>,
  // and explicit human actions (Edit, Post to PR…, Decline / Dismiss, Ask Agent to Fix).
  import AgentChip from '../../lib/components/AgentChip.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { isComposerSubmitKey } from './gitUiHelpers';
  import { renderPrMarkdown } from './prMarkdown';
  import {
    aiOverlaySeverityLabel,
    aiOverlaySeverityTone,
    type AiOverlayItem,
    type DiffAiOverlayActions,
  } from './diffAiOverlay';

  interface Props extends DiffAiOverlayActions {
    item: AiOverlayItem;
  }
  let { item, onEditComment, onPostComment, onDismissItem, onFixItem }: Props = $props();

  let editing = $state(false);
  let draftBody = $state('');
  let busy = $state<'edit' | 'post' | 'dismiss' | 'fix' | null>(null);

  const tone = $derived(aiOverlaySeverityTone(item.severity));
  const sevLabel = $derived(aiOverlaySeverityLabel(item.severity));
  const canEdit = $derived(item.kind === 'comment' && !!onEditComment);
  const canPost = $derived(item.kind === 'comment' && !!onPostComment);

  function startEdit(): void {
    if (item.kind !== 'comment') return;
    draftBody = item.comment.body;
    editing = true;
  }

  function cancelEdit(): void {
    editing = false;
    draftBody = '';
  }

  async function saveEdit(): Promise<void> {
    if (item.kind !== 'comment' || !onEditComment || busy) return;
    const trimmed = draftBody.trim();
    if (!trimmed) return;
    if (trimmed === item.comment.body.trim()) {
      cancelEdit();
      return;
    }
    busy = 'edit';
    try {
      const ok = await onEditComment(item.comment, trimmed);
      if (ok) cancelEdit();
    } finally {
      busy = null;
    }
  }

  async function runPost(): Promise<void> {
    if (item.kind !== 'comment' || !onPostComment || busy) return;
    busy = 'post';
    try {
      await onPostComment(item.comment);
    } finally {
      busy = null;
    }
  }

  async function runDismiss(): Promise<void> {
    if (!onDismissItem || busy) return;
    busy = 'dismiss';
    try {
      await onDismissItem(item);
    } finally {
      busy = null;
    }
  }

  async function runFix(): Promise<void> {
    if (!onFixItem || busy) return;
    busy = 'fix';
    try {
      await onFixItem(item);
    } finally {
      busy = null;
    }
  }

  function onEditKey(e: KeyboardEvent): void {
    if (isComposerSubmitKey(e)) {
      e.preventDefault();
      void saveEdit();
    } else if (e.key === 'Escape') {
      e.preventDefault();
      cancelEdit();
    }
  }
</script>

<article class="ai-finding-card" data-testid="diff-ai-finding-card" data-kind={item.kind}>
  <header class="afc-head">
    <AgentChip />
    <Badge {tone} label={sevLabel} />
    {#if item.kind === 'comment'}
      <span class="afc-meta">Draft review comment</span>
    {:else if item.finding.category}
      <span class="afc-meta">{item.finding.category}</span>
    {/if}
    {#if item.line !== null}
      <span class="afc-line mono">
        Line {item.line}{#if item.kind === 'finding' && item.lineEnd && item.lineEnd !== item.line}–{item.lineEnd}{/if}
      </span>
    {/if}
  </header>

  {#if item.kind === 'finding' && item.title}
    <div class="afc-title" dir="auto">{item.title}</div>
  {/if}

  {#if editing}
    <div class="afc-edit">
      <textarea
        dir="auto"
        class="input afc-textarea"
        rows="3"
        aria-label="Edit draft review comment"
        bind:value={draftBody}
        disabled={busy === 'edit'}
        onkeydown={onEditKey}
      ></textarea>
      <div class="afc-actions">
        <button type="button" class="btn small" disabled={busy !== null} onclick={cancelEdit}>Cancel</button>
        <button
          type="button"
          class="btn small primary"
          disabled={busy !== null || draftBody.trim() === ''}
          title="Save edit (⌘↵)"
          onclick={() => void saveEdit()}
        >
          {busy === 'edit' ? 'Saving…' : 'Save'}
        </button>
      </div>
    </div>
  {:else}
    <div class="afc-body md-body" dir="auto">
      <!-- renderPrMarkdown sanitizes output via allowlist -->
      {@html renderPrMarkdown(item.body)}
    </div>

    <div class="afc-actions">
      {#if canEdit}
        <button type="button" class="btn small ghost" disabled={busy !== null} onclick={startEdit}>
          <Icon name="edit" size={12} /> Edit
        </button>
      {/if}
      {#if canPost}
        <button
          type="button"
          class="btn small"
          disabled={busy !== null}
          title="Review and post this draft comment to the pull request"
          onclick={() => void runPost()}
        >
          <Icon name="send" size={12} /> {busy === 'post' ? 'Posting…' : 'Post to PR…'}
        </button>
      {/if}
      {#if onDismissItem}
        <button
          type="button"
          class="btn small ghost"
          disabled={busy !== null}
          onclick={() => void runDismiss()}
        >
          {busy === 'dismiss' ? 'Dismissing…' : item.kind === 'comment' ? 'Decline' : 'Dismiss'}
        </button>
      {/if}
      {#if onFixItem}
        <button
          type="button"
          class="btn small ghost"
          disabled={busy !== null}
          onclick={() => void runFix()}
        >
          <Icon name="zap" size={12} /> {busy === 'fix' ? 'Starting…' : 'Ask Agent to Fix'}
        </button>
      {/if}
    </div>
  {/if}
</article>

<style>
  .ai-finding-card {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 8px 12px;
    margin-block: 4px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-inline-start: 2px solid var(--border-strong);
    border-radius: var(--radius-m);
    font-family: var(--font-ui);
    font-size: var(--fs-s);
    color: var(--text);
    white-space: normal;
    word-break: normal;
  }
  .afc-head {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
  }
  .afc-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .afc-line {
    margin-inline-start: auto;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .afc-title {
    font-weight: 600;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .afc-body {
    font-size: var(--fs-s);
    line-height: 1.45;
    color: var(--text);
  }
  .afc-edit {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .afc-textarea {
    width: 100%;
    font-size: var(--fs-s);
    resize: vertical;
  }
  .afc-actions {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    padding-block-start: 2px;
  }
</style>
