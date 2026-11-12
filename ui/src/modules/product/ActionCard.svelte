<script lang="ts">
  import { plural } from '../../lib/plural';
  import { toastError } from '../../lib/toastError';
  // ActionCard — renders ONE agent-proposed card from the story Chat with an
  // explicit Apply button. Nothing is applied until the PO clicks; every apply
  // is toasted and, where it created rows, reversible (sticky "✓ … · Undo").
  //
  //   add_questions → per-item checkbox (default all on), "Add N questions";
  //                   Undo deletes the created question ids.
  //   add_notes     → per-item checkbox (default all on), "Add note(s)";
  //                   Undo deletes the created note ids.
  //   create_canvas → lazy Mermaid thumbnail (if any), "Open in Design Hall";
  //                   opens the new board's Hall mirror, then toasts.
  //
  // A proposed story body is never a card: the Chat saves it as a suggested
  // version and the message links to Rewrite (see RefineChat).
  import Icon from '../../lib/components/Icon.svelte';
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { api } from '../../lib/api/client';
  import { renderMermaid } from '../../lib/render/mermaid';
  import { ui } from '../../lib/stores/ui.svelte';
  import { openSceneInHall } from '../design-hall/nav'; // ui-guards: allow — Canvas boards open in Design Hall
  import { applyChatAction } from './chatActions';
  import type { DiscoveryAction } from './types';

  /** `tid`: the Chat conversation (refinement thread) the card came from. */
  let { tid, action }: { tid: string; action: DiscoveryAction } = $props();

  // ── Shared apply state ──────────────────────────────────────────────────────
  let applying = $state(false);
  // After a successful apply the card collapses to a sticky confirmation row.
  let applied = $state(false);
  let appliedLabel = $state('');
  let undoing = $state(false);
  // Captured so Undo can reverse the change.
  let createdQuestionIds = $state<string[]>([]); // add_questions
  let createdNoteIds = $state<string[]>([]); // add_notes

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
      const result = await applyChatAction(tid, {
        type: 'add_questions',
        questions: picked,
      });
      createdQuestionIds = result.created_question_ids;
      await product.loadQuestions();
      applied = true;
      appliedLabel = `Added ${plural(picked.length, 'question')}`;
      toasts.success(appliedLabel);
    } catch (e) {
      toastError('Couldn’t add questions', e);
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
      const result = await applyChatAction(tid, {
        type: 'add_notes',
        notes: picked,
      });
      createdNoteIds = result.created_note_ids;
      await product.loadNotes();
      applied = true;
      appliedLabel = `Added ${plural(picked.length, 'note')}`;
      toasts.success(appliedLabel);
    } catch (e) {
      toastError('Couldn’t add notes', e);
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
      toastError('Couldn’t undo', e);
    } finally {
      undoing = false;
    }
  }

  // ── create_canvas: lazy Mermaid thumbnail + open ───────────────────────────
  let canvasSvg = $state<string | null>(null);
  let canvasThumbErr = $state<string | null>(null);
  let thumbId = `actcard-mmd-${Math.random().toString(36).slice(2)}`;

  // The SVG is agent-written: it renders in a sandbox="" iframe (no scripts,
  // no same-origin), like MockupViewer — never inlined into the app origin,
  // so a Mermaid/DOMPurify bypass can't reach the daemon (S18-08).
  function svgDoc(svg: string): string {
    return `<!doctype html><html><head><meta charset="utf-8">` +
      `<style>html,body{margin:0;padding:0;background:transparent;overflow:hidden;}` +
      `svg{max-width:100%;max-height:146px;height:auto;display:block;margin:0 auto;}</style></head>` +
      `<body>${svg}</body></html>`;
  }

  $effect(() => {
    if (action.type !== 'create_canvas') return;
    const src = action.mermaid?.trim();
    if (!src) return;
    // Re-render on a theme flip — the light palette is unreadable on dark.
    const dark = ui.resolvedScheme === 'dark';
    let alive = true;
    void renderMermaid(thumbId, src, { dark }).then((r) => {
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
      const result = await applyChatAction(tid, action);
      applied = true;
      appliedLabel = 'Canvas created';
      if (result.canvas_id) {
        // A board lives in Design Hall now: open its mirror there.
        void openSceneInHall(result.canvas_id);
        toasts.success('Opened in Design Hall');
      } else {
        toasts.success('Canvas created');
      }
    } catch (e) {
      toastError('Couldn’t create canvas', e);
    } finally {
      applying = false;
    }
  }

  function undoApplicable(): boolean {
    return createdQuestionIds.length > 0 || createdNoteIds.length > 0;
  }
</script>

{#if applied}
  <!-- Sticky post-apply confirmation row -->
  <div class="action-card applied-row">
    <span class="applied-text"><Icon name="check" size={12} /> {appliedLabel}</span>
    {#if undoApplicable()}
      <button class="btn small" onclick={() => void undoCreated()} disabled={undoing}>
        {undoing ? 'Undoing…' : 'Undo'}
      </button>
    {/if}
  </div>
{:else}
  <div class="action-card">
    {#if action.type === 'add_questions'}
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
          class="btn small primary"
          onclick={applyQuestions}
          disabled={applying || checkedCount === 0}
        >
          {applying
            ? 'Adding…'
            : `Add ${plural(checkedCount, 'question')}`}
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
          class="btn small primary"
          onclick={applyNotes}
          disabled={applying || checkedCount === 0}
        >
          {applying ? 'Adding…' : checkedCount === 1 ? 'Add note' : 'Add notes'}
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
            <div class="mmd-thumb">
              <iframe
                class="mmd-frame"
                title="Diagram preview"
                sandbox=""
                referrerpolicy="no-referrer"
                srcdoc={svgDoc(canvasSvg)}
              ></iframe>
            </div>
          {:else if canvasThumbErr}
            <div class="mmd-err">Diagram preview unavailable</div>
          {:else}
            <div class="mmd-loading">Rendering preview…</div>
          {/if}
        </div>
      {/if}
      <div class="card-actions">
        <button class="btn small primary" onclick={applyCanvas} disabled={applying}>
          {applying ? 'Opening…' : 'Open in Design Hall'}
        </button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .action-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    margin-top: 8px;
    overflow: hidden;
  }

  .card-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
  }
  /* The proposal's kind: a neutral micro-label at the 11px floor. */
  .card-kind {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    padding: 1px 6px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    white-space: nowrap;
    flex-shrink: 0;
  }
  .card-title {
    font-size: var(--fs-s);
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
    gap: 6px;
    padding: 4px 6px;
    cursor: pointer;
  }
  .item-label:hover {
    background: var(--hover);
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
    font-size: var(--fs-s);
    line-height: 1.45;
    color: var(--text);
    overflow-wrap: break-word;
  }
  .item-sub {
    font-size: var(--fs-xs);
    line-height: 1.4;
    color: var(--text-dim);
  }
  .item-cat {
    align-self: flex-start;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    padding: 1px 4px;
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
  .mmd-frame {
    inline-size: 100%;
    block-size: 146px;
    border: 0;
    display: block;
    background: transparent;
  }
  .mmd-err,
  .mmd-loading {
    font-size: var(--fs-xs);
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
  /* ── Applied (sticky confirmation) row ──────────────────────────────────── */
  .applied-row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
    background: var(--success-soft);
    border-color: color-mix(in srgb, var(--success) 35%, transparent);
  }
  .applied-text {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--success);
  }
</style>
