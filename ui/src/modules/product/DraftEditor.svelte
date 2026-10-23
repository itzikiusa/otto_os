<script lang="ts">
  // The draft story's Overview body (two columns): the title/body editor with
  // Save and the publish bar on the left; transcripts and attachments on the
  // right. Split out of OverviewTab (architecture-5-04). The parent owns the
  // publish dialog (`onpublish`); this owns the unsaved-draft form, its
  // per-field reseeding and the leave guard.
  import { untrack } from 'svelte';
  import { toastError } from '../../lib/toastError';
  import { product } from '../../lib/stores/product.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { router } from '../../lib/router.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { reseedDraft, type DraftSeed } from './draftSeed';
  import AttachmentsPanel from './AttachmentsPanel.svelte';
  import StoryTranscripts from './StoryTranscripts.svelte';
  import type { ProductAttachment, ProductStory, ProductStoryVersion } from './types';

  interface Props {
    story: ProductStory;
    /** The editable source version (null for a draft with no body yet). */
    source: ProductStoryVersion | null;
    onpublish: (mode: 'story' | 'rfc') => void;
  }
  let { story, source, onpublish }: Props = $props();
  const isDraft = $derived(story?.source_kind === 'draft');

  // ── Draft edit state ─────────────────────────────────────────────────────
  let draftTitle = $state('');
  let draftBody = $state('');
  /** The draft form differs from the saved draft (Save enabled; leaving asks). */
  const draftDirty = $derived(
    isDraft && !!story && (draftTitle !== story.title || draftBody !== (source?.body_md ?? '')),
  );
  async function approveDraftLeave(): Promise<boolean> {
    if (!draftDirty) return true;
    const allowed = await confirmer.ask('You have unsaved changes to this draft. Leaving now discards them.', {
      title: 'Discard unsaved changes?', danger: true, confirmLabel: 'Discard', cancelLabel: 'Keep editing',
    });
    if (allowed) { draftTitle = story?.title ?? ''; draftBody = source?.body_md ?? ''; }
    return allowed;
  }
  $effect(() => product.registerDraftLeave(approveDraftLeave));
  $effect(() => router.guard(approveDraftLeave));
  let draftSaving = $state(false);

  // ── AttachmentsPanel ref + screenshot paste counter ───────────────────────
  // panelRef exposes uploadBlob(blob, opts) for the textarea paste handler.
  let panelRef = $state<{ uploadBlob: (blob: Blob, opts?: { filename?: string; kind?: string }) => Promise<ProductAttachment> } | null>(null);
  // Session-local counter for unique screenshot filenames.
  let bodyScreenshotIdx = $state(0);

  // Seed the draft form ONCE per story, then only per UNEDITED field
  // (r3-02-07). `product.detail` is replaced by updateStory / patchStory /
  // loadDetail / ActionCard; re-seeding on every replacement overwrote the
  // user's unsaved draft and collapsed the transcripts. Plain (non-reactive)
  // bookkeeping: the story id + the values last seeded into each field.
  let seeded: DraftSeed = { id: null, title: '', body: '' };
  $effect(() => {
    const s = story;
    const src = source;
    const draft = isDraft;
    untrack(() => {
      const id = s?.id ?? null;
      if (!draft || !s) {
        seeded = { ...seeded, id };
        return;
      }
      const next: DraftSeed = { id, title: s.title, body: src?.body_md ?? '' };
      const form = reseedDraft(seeded, { title: draftTitle, body: draftBody }, next);
      draftTitle = form.title;
      draftBody = form.body;
      seeded = next;
    });
  });

  async function saveDraft(): Promise<void> {
    draftSaving = true;
    try {
      await product.updateDraft({ title: draftTitle, body_md: draftBody });
      toasts.success('Draft saved');
    } catch (e) {
      toastError('Couldn’t save the draft', e);
    } finally {
      draftSaving = false;
    }
  }

  /**
   * Paste handler for the draft body `<textarea>`. When the clipboard contains
   * an image, upload it via the AttachmentsPanel's uploadBlob action, then
   * splice `![filename](attachment:<id>)` at the caret so the markdown body
   * references the attachment portably (not a blob URL).
   */
  async function handleBodyPaste(e: ClipboardEvent): Promise<void> {
    if (!e.clipboardData) return;
    for (const item of Array.from(e.clipboardData.items)) {
      if (item.kind === 'file' && item.type.startsWith('image/')) {
        e.preventDefault();
        const blob = item.getAsFile();
        if (!blob || !panelRef) return;
        const idx = ++bodyScreenshotIdx;
        const filename = `screenshot-${idx}.png`;
        try {
          const textarea = e.currentTarget as HTMLTextAreaElement;
          const caretPos = textarea.selectionStart ?? draftBody.length;
          const att = await panelRef.uploadBlob(blob, { filename, kind: 'image' });
          // Splice the markdown reference at the caret position.
          const token = `![${att.filename}](attachment:${att.id})`;
          draftBody =
            draftBody.slice(0, caretPos) + token + draftBody.slice(caretPos);
        } catch (ex) {
          toastError('Couldn’t upload the screenshot', ex);
        }
        return; // only handle the first image item
      }
    }
  }
</script>

<div class="two-col draft-layout">
  <!-- Left: title + body editor + save + publish bar -->
  <div class="col-left">
    <div class="draft-section">
      <div class="draft-hint">
        Refine this draft with the Analysis / Questions / Rewrite tabs, then publish when ready.
      </div>

      <div class="field">
        <label class="label" for="draft-title">Title</label>
        <input dir="auto"
          id="draft-title"
          class="input wide-input"
          bind:value={draftTitle}
          placeholder="e.g. Players can set a weekly deposit limit"
          spellcheck="false"
        />
      </div>

      <div class="field">
        <label class="label" for="draft-body">Body (Markdown)</label>
        <textarea dir="auto"
          id="draft-body"
          class="textarea"
          bind:value={draftBody}
          rows={14}
          placeholder="e.g. As a player, I want to cap my weekly deposits… (or paste meeting notes)"
          spellcheck="false"
          onpaste={handleBodyPaste}
        ></textarea>
      </div>

      <div class="draft-save-row">
        <button
          class="btn"
          onclick={saveDraft}
          disabled={draftSaving || !draftDirty}
          title={draftDirty ? undefined : 'No unsaved changes'}
        >
          {draftSaving ? 'Saving…' : draftDirty ? 'Save draft' : 'Saved'}
        </button>
      </div>

      <!-- ── Publish bar ─────────────────────────────────────── -->
      <div class="publish-bar">
        <button class="btn" onclick={() => onpublish('rfc')}>
          Publish as Confluence RFC…
        </button>
        <button class="btn primary" onclick={() => onpublish('story')}>
          Publish as Jira story…
        </button>
      </div>
    </div>
  </div>

  <!-- Right: Transcripts + Attachments -->
  <div class="col-right">
    <StoryTranscripts />

    <!-- Attachments panel: paste/drag/file-picker with previews. -->
    <!-- bind:this captures the exported uploadBlob action. -->
    <AttachmentsPanel bind:this={panelRef} />
  </div>
</div>

<style>
  /* ── Two-column grid ───────────────────────────────────────── */
  .two-col {
    display: grid;
    grid-template-columns: minmax(0, 1.6fr) minmax(0, 1fr);
    gap: 20px;
    align-items: start;
  }
  .col-left {
    min-width: 0;
  }
  .col-right {
    min-width: 0;
  }

  /* Responsive: collapse to a single column at tablet width and below */
  @media (max-width: 1024px) {
    .two-col {
      grid-template-columns: 1fr;
    }
  }

  /* ── Draft editing ─────────────────────────────────────────── */
  .draft-section {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding-top: 4px;
  }
  .draft-hint {
    font-size: var(--fs-s);
    color: var(--text-dim);
    background: var(--accent-faint);
    border: 1px solid var(--accent-soft-strong);
    border-radius: var(--radius-s);
    padding: 8px 12px;
    line-height: 1.5;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .label {
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: .06em;
  }
  /* Shared .input (app.css); these two span their form's width. */
  .wide-input {
    width: 100%;
    box-sizing: border-box;
  }
  .textarea {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-s);
    font-family: var(--font-mono);
    padding: 8px 10px;
    outline: none;
    width: 100%;
    box-sizing: border-box;
    resize: vertical;
    line-height: 1.55;
  }
  .textarea:focus {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .draft-save-row {
    display: flex;
    justify-content: flex-end;
  }
  /* Save is a quiet, neutral action; the publish bar below owns the one
     filled button in this pane. */

  /* ── Publish bar ───────────────────────────────────────────── */
  .publish-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 14px;
    border-top: 1px solid var(--border);
    flex-wrap: wrap;
  }
</style>
