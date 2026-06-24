<script lang="ts">
  // ActionCard — renders ONE agent-proposed DiscoveryAction as a trustworthy
  // card with an explicit Apply button. Nothing is applied until the PO clicks;
  // every apply is reversible (sticky "✓ … · Undo" row) and toasted.
  //
  //   apply_draft   → diff-preview against the current draft, "Replace draft"
  //                   (danger, confirm when non-empty); Undo re-applies the
  //                   captured prior body via product.updateDraft.
  //   add_questions → per-item checkbox (default all on), "Add N questions";
  //                   Undo deletes the created question ids.
  //   add_notes     → per-item checkbox (default all on), "Add note(s)";
  //                   Undo deletes the created note ids.
  //   create_canvas → lazy Mermaid thumbnail (if any), "Open in Canvas";
  //                   navigates on canvas_id, then toasts.
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { router } from '../../lib/router.svelte';
  import { api } from '../../lib/api/client';
  import DiffView from '../../lib/components/DiffView.svelte';
  import { renderMermaid } from '../canvas/mermaid';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import type { DiscoveryAction, UpdateDraftReq } from './types';

  let { cid, action }: { cid: string; action: DiscoveryAction } = $props();

  // ── Shared apply state ──────────────────────────────────────────────────────
  let applying = $state(false);
  // After a successful apply the card collapses to a sticky confirmation row.
  let applied = $state(false);
  let appliedLabel = $state('');
  let undoing = $state(false);
  // Captured so Undo can reverse the change.
  let priorDraftBody = $state<string | null>(null); // apply_draft snapshot
  let createdQuestionIds = $state<string[]>([]); // add_questions
  let createdNoteIds = $state<string[]>([]); // add_notes

  // ── apply_draft: diff preview + danger confirm ─────────────────────────────
  let showDiff = $state(false);
  const currentDraftBody = $derived(product.detail?.source?.body_md ?? '');
  const currentDraftTitle = $derived(product.detail?.story?.title ?? '');
  // Force line mode on phone/tablet (split/word are too dense for narrow screens).
  const diffMode = $derived<'line' | 'word'>(
    viewport.isPhone || viewport.isTablet ? 'line' : 'word',
  );

  async function applyDraft(): Promise<void> {
    if (applying || action.type !== 'apply_draft') return;
    const current = currentDraftBody;
    // Snapshot BEFORE applying so Undo can restore the exact prior text.
    priorDraftBody = current;
    if (current.trim()) {
      const ok = await confirmer.ask(
        'Replace the current draft? This overwrites your text.',
        { confirmLabel: 'Replace', danger: true },
      );
      if (!ok) return;
    }
    applying = true;
    try {
      await product.applyDiscoveryAction(cid, action);
      await product.loadDetail();
      applied = true;
      appliedLabel = 'Draft replaced';
      toasts.success('Draft replaced');
    } catch (e) {
      toasts.error('Could not replace draft', product.errMsg(e));
    } finally {
      applying = false;
    }
  }

  async function undoDraft(): Promise<void> {
    if (undoing || priorDraftBody === null) return;
    undoing = true;
    try {
      const req: UpdateDraftReq = {
        title: product.detail?.story?.title ?? currentDraftTitle,
        body_md: priorDraftBody,
      };
      await product.updateDraft(req);
      applied = false;
      toasts.success('Draft restored');
    } catch (e) {
      toasts.error('Could not undo', product.errMsg(e));
    } finally {
      undoing = false;
    }
  }

  // ── add_questions / add_notes: per-item checkboxes ─────────────────────────
  // Track *unchecked* indices instead of a per-item boolean array: that way the
  // default ("all checked") needs no read of `action` in a $state initializer
  // (which Svelte flags) — an index is checked unless it's in this set.
  const itemCount = $derived(
    action.type === 'add_questions'
      ? action.questions.length
      : action.type === 'add_notes'
        ? action.notes.length
        : 0,
  );
  let unchecked = $state<Set<number>>(new Set());
  const checkedCount = $derived(itemCount - unchecked.size);

  function isChecked(i: number): boolean {
    return !unchecked.has(i);
  }
  function toggle(i: number, on: boolean): void {
    const next = new Set(unchecked);
    if (on) next.delete(i);
    else next.add(i);
    unchecked = next;
  }

  async function applyQuestions(): Promise<void> {
    if (applying || action.type !== 'add_questions') return;
    const picked = action.questions.filter((_, i) => isChecked(i));
    if (picked.length === 0) return;
    applying = true;
    try {
      const result = await product.applyDiscoveryAction(cid, {
        type: 'add_questions',
        questions: picked,
      });
      createdQuestionIds = result.created_question_ids;
      await product.loadQuestions();
      applied = true;
      appliedLabel = `Added ${picked.length} question${picked.length === 1 ? '' : 's'}`;
      toasts.success(appliedLabel);
    } catch (e) {
      toasts.error('Could not add questions', product.errMsg(e));
    } finally {
      applying = false;
    }
  }

  async function applyNotes(): Promise<void> {
    if (applying || action.type !== 'add_notes') return;
    const picked = action.notes.filter((_, i) => isChecked(i));
    if (picked.length === 0) return;
    applying = true;
    try {
      const result = await product.applyDiscoveryAction(cid, {
        type: 'add_notes',
        notes: picked,
      });
      createdNoteIds = result.created_note_ids;
      await product.loadNotes();
      applied = true;
      appliedLabel = `Added ${picked.length} note${picked.length === 1 ? '' : 's'}`;
      toasts.success(appliedLabel);
    } catch (e) {
      toasts.error('Could not add notes', product.errMsg(e));
    } finally {
      applying = false;
    }
  }

  async function undoCreated(): Promise<void> {
    if (undoing) return;
    undoing = true;
    try {
      for (const qid of createdQuestionIds) {
        await api.del(`/product/questions/${qid}`);
      }
      for (const nid of createdNoteIds) {
        await api.del(`/product/notes/${nid}`);
      }
      if (createdQuestionIds.length) await product.loadQuestions();
      if (createdNoteIds.length) await product.loadNotes();
      createdQuestionIds = [];
      createdNoteIds = [];
      applied = false;
      toasts.success('Undone');
    } catch (e) {
      toasts.error('Could not undo', product.errMsg(e));
    } finally {
      undoing = false;
    }
  }

  // ── create_canvas: lazy Mermaid thumbnail + open ───────────────────────────
  let canvasSvg = $state<string | null>(null);
  let canvasThumbErr = $state<string | null>(null);
  let thumbId = `actcard-mmd-${Math.random().toString(36).slice(2)}`;

  $effect(() => {
    if (action.type !== 'create_canvas') return;
    const src = action.mermaid?.trim();
    if (!src) return;
    let alive = true;
    void renderMermaid(thumbId, src).then((r) => {
      if (!alive) return;
      if (r.svg) canvasSvg = r.svg;
      else canvasThumbErr = r.error ?? 'Diagram error';
    });
    return () => {
      alive = false;
    };
  });

  async function applyCanvas(): Promise<void> {
    if (applying || action.type !== 'create_canvas') return;
    applying = true;
    try {
      const result = await product.applyDiscoveryAction(cid, action);
      applied = true;
      appliedLabel = 'Canvas created';
      if (result.canvas_id) {
        // Deep-link: ask the Canvas module to auto-open this scene on mount, then
        // navigate there.
        canvas.pendingOpenId = result.canvas_id;
        router.go('canvas');
        toasts.success('Opened in Canvas');
      } else {
        toasts.success('Canvas created');
      }
    } catch (e) {
      toasts.error('Could not create canvas', product.errMsg(e));
    } finally {
      applying = false;
    }
  }

  function undoApplicable(): boolean {
    return (
      priorDraftBody !== null ||
      createdQuestionIds.length > 0 ||
      createdNoteIds.length > 0
    );
  }

  function onUndo(): void {
    if (action.type === 'apply_draft') void undoDraft();
    else void undoCreated();
  }
</script>

{#if applied}
  <!-- Sticky post-apply confirmation row -->
  <div class="action-card applied-row">
    <span class="applied-text">✓ {appliedLabel}</span>
    {#if undoApplicable()}
      <button class="undo-btn" onclick={onUndo} disabled={undoing}>
        {undoing ? 'Undoing…' : 'Undo'}
      </button>
    {/if}
  </div>
{:else}
  <div class="action-card">
    {#if action.type === 'apply_draft'}
      <div class="card-head">
        <span class="card-kind">Draft</span>
        <span class="card-title">{action.title}</span>
      </div>
      <div class="card-body">
        <button
          class="toggle-link"
          onclick={() => (showDiff = !showDiff)}
          aria-expanded={showDiff}
        >
          {showDiff ? 'Hide changes' : 'Preview changes'}
        </button>
        <div class="diff-wrap" class:peek={!showDiff}>
          <DiffView
            before={currentDraftBody}
            after={action.body_md}
            mode={diffMode}
            contextLines={showDiff ? 3 : undefined}
          />
        </div>
      </div>
      <div class="card-actions">
        <button class="apply-btn danger" onclick={applyDraft} disabled={applying}>
          {applying ? 'Replacing…' : 'Replace draft'}
        </button>
      </div>
    {:else if action.type === 'add_questions'}
      <div class="card-head">
        <span class="card-kind">Questions</span>
        <span class="card-title">{action.questions.length} proposed</span>
      </div>
      <div class="card-body">
        <ul class="item-list">
          {#each action.questions as q, i (i)}
            <li class="item-row">
              <label class="item-label">
                <input
                  type="checkbox"
                  checked={isChecked(i)}
                  onchange={(e) => toggle(i, e.currentTarget.checked)}
                />
                <span class="item-main">
                  <span class="item-text">{q.text}</span>
                  {#if q.rationale}<span class="item-sub">{q.rationale}</span>{/if}
                  {#if q.category}<span class="item-cat">{q.category}</span>{/if}
                </span>
              </label>
            </li>
          {/each}
        </ul>
      </div>
      <div class="card-actions">
        <button
          class="apply-btn"
          onclick={applyQuestions}
          disabled={applying || checkedCount === 0}
        >
          {applying
            ? 'Adding…'
            : `Add ${checkedCount} question${checkedCount === 1 ? '' : 's'}`}
        </button>
      </div>
    {:else if action.type === 'add_notes'}
      <div class="card-head">
        <span class="card-kind">Notes</span>
        <span class="card-title">{action.notes.length} proposed</span>
      </div>
      <div class="card-body">
        <ul class="item-list">
          {#each action.notes as n, i (i)}
            <li class="item-row">
              <label class="item-label">
                <input
                  type="checkbox"
                  checked={isChecked(i)}
                  onchange={(e) => toggle(i, e.currentTarget.checked)}
                />
                <span class="item-main">
                  <span class="item-text">{n.body}</span>
                </span>
              </label>
            </li>
          {/each}
        </ul>
      </div>
      <div class="card-actions">
        <button
          class="apply-btn"
          onclick={applyNotes}
          disabled={applying || checkedCount === 0}
        >
          {applying ? 'Adding…' : `Add note${checkedCount === 1 ? '' : 's'}`}
        </button>
      </div>
    {:else if action.type === 'create_canvas'}
      <div class="card-head">
        <span class="card-kind">Canvas</span>
        <span class="card-title">{action.title}</span>
      </div>
      {#if action.mermaid}
        <div class="card-body">
          {#if canvasSvg}
            <div class="mmd-thumb">{@html canvasSvg}</div>
          {:else if canvasThumbErr}
            <div class="mmd-err">Diagram preview unavailable</div>
          {:else}
            <div class="mmd-loading">Rendering preview…</div>
          {/if}
        </div>
      {/if}
      <div class="card-actions">
        <button class="apply-btn" onclick={applyCanvas} disabled={applying}>
          {applying ? 'Opening…' : 'Open in Canvas'}
        </button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .action-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2, var(--surface));
    margin-top: 8px;
    overflow: hidden;
  }

  .card-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 7px 10px;
    border-bottom: 1px solid var(--border);
  }
  .card-kind {
    font-size: 9.5px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 1px 6px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .card-title {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .card-body {
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  /* ── Diff preview (apply_draft) ─────────────────────────────────────────── */
  .toggle-link {
    align-self: flex-start;
    padding: 0;
    border: none;
    background: transparent;
    color: var(--accent);
    font-size: 11.5px;
    font-weight: 600;
    cursor: pointer;
  }
  .toggle-link:hover {
    text-decoration: underline;
  }
  .diff-wrap {
    min-width: 0;
  }
  /* Collapsed 3-line peek of the diff. */
  .diff-wrap.peek {
    max-height: 4.4em;
    overflow: hidden;
    position: relative;
    -webkit-mask-image: linear-gradient(to bottom, #000 55%, transparent);
    mask-image: linear-gradient(to bottom, #000 55%, transparent);
  }

  /* ── Item lists (questions / notes) ─────────────────────────────────────── */
  .item-list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .item-row {
    border-radius: var(--radius-s);
  }
  .item-label {
    display: flex;
    align-items: flex-start;
    gap: 7px;
    padding: 4px 6px;
    cursor: pointer;
  }
  .item-label:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
    border-radius: var(--radius-s);
  }
  .item-label input {
    margin-top: 2px;
    flex-shrink: 0;
    accent-color: var(--accent);
  }
  .item-main {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .item-text {
    font-size: 12.5px;
    line-height: 1.45;
    color: var(--text);
    overflow-wrap: break-word;
  }
  .item-sub {
    font-size: 11px;
    line-height: 1.4;
    color: var(--text-dim);
  }
  .item-cat {
    align-self: flex-start;
    font-size: 9.5px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 1px 5px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--text-dim) 14%, transparent);
    color: var(--text-dim);
  }

  /* ── Mermaid thumbnail (create_canvas) ──────────────────────────────────── */
  .mmd-thumb {
    max-height: 160px;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    padding: 6px;
    display: flex;
    justify-content: center;
  }
  .mmd-thumb :global(svg) {
    max-width: 100%;
    height: auto;
  }
  .mmd-err,
  .mmd-loading {
    font-size: 11.5px;
    color: var(--text-dim);
    font-style: italic;
    padding: 8px;
  }

  /* ── Actions ────────────────────────────────────────────────────────────── */
  .card-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
    padding: 8px 10px;
    border-top: 1px solid var(--border);
  }
  .apply-btn {
    padding: 5px 12px;
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    background: var(--accent);
    color: #fff;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    transition: opacity 110ms;
  }
  .apply-btn:hover:not(:disabled) {
    opacity: 0.88;
  }
  .apply-btn:disabled {
    opacity: 0.45;
    cursor: not-allowed;
  }
  .apply-btn.danger {
    background: var(--danger, #ef4444);
    border-color: var(--danger, #ef4444);
  }

  /* ── Applied (sticky confirmation) row ──────────────────────────────────── */
  .applied-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 7px 11px;
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
  }
  .applied-text {
    font-size: 12px;
    font-weight: 600;
    color: var(--accent);
  }
  .undo-btn {
    padding: 3px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text);
    font-size: 11.5px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
  }
  .undo-btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  .undo-btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
</style>
