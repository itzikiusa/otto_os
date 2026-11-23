<script lang="ts">
  // The IDE editor's preview (Split: beside the editor behind a PaneDivider;
  // Preview: in its place — the editor stays mounted, hidden, so undo and the
  // cursor survive). It renders the LIVE buffer, unsaved edits included:
  //   • debounced (PREVIEW_DEBOUNCE_MS after the buffer settles), and no work
  //     while the window is hidden — it catches up when shown again;
  //   • a very large Markdown/HTML/Mermaid/SVG file renders on demand ("Render
  //     preview") instead of on every pause in typing;
  //   • Markdown in Split follows the cursor's section (heading ordinal);
  //   • relative links open the file in the IDE; images and other bytes come
  //     through the daemon's authenticated file route (local projects only —
  //     a remote SSH project says so instead of a broken image).
  // The shared renderer (lib/components/FilePreview.svelte) is lazy-loaded;
  // D2 (a module's renderer) draws in IdeD2Preview.
  import { authedBlobUrl } from '../../../lib/api/client';
  import {
    headingOrdinalAtLine,
    PREVIEW_DEBOUNCE_MS,
    previewNeedsManualRender,
    previewTextUpdate,
  } from '../../../lib/components/filePreviewKind';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import PaneDivider from '../../../lib/components/PaneDivider.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  import { formatBytes } from '../../../lib/metric-format';
  import { loadPaneWidth } from '../../../lib/paneResizer';
  import { ide, type IdeOpenTab } from '../ideStore.svelte';
  import { findContainingRoot } from '../shell/ideProjectHelpers';
  import { idePreviewMode } from './idePreviewMode.svelte';

  const FilePreviewLazy = lazyComponent(() => import('../../../lib/components/FilePreview.svelte'));
  const IdeD2PreviewLazy = lazyComponent(() => import('./IdeD2Preview.svelte'));

  interface Props {
    tab: IdeOpenTab;
  }
  let { tab }: Props = $props();

  const WIDTH_KEY = 'otto.ide.previewW';
  const MIN_W = 220;
  const MAX_W = 2400;

  const layout = $derived(idePreviewMode.layoutFor(tab.path));
  const kind = $derived(idePreviewMode.kindOf(tab.path));
  const shown = $derived(layout === 'split' || layout === 'only');

  // ── Debounced live text ────────────────────────────────────────────────
  let text = $state('');
  let textPath = '';
  let visible = $state(typeof document === 'undefined' || document.visibilityState !== 'hidden');
  $effect(() => {
    const onVis = (): void => {
      visible = document.visibilityState !== 'hidden';
    };
    document.addEventListener('visibilitychange', onVis);
    return () => document.removeEventListener('visibilitychange', onVis);
  });
  $effect(() => {
    const next = tab.content;
    const p = tab.path;
    if (!shown) textPath = '';
    const step = previewTextUpdate(textPath, text, p, next, { shown, visible });
    if (step === 'now') {
      textPath = p;
      text = next;
    }
    if (step !== 'debounce') return;
    const t = setTimeout(() => (text = next), PREVIEW_DEBOUNCE_MS);
    return () => clearTimeout(t);
  });

  // ── Large files: render on demand ──────────────────────────────────────
  const large = $derived(previewNeedsManualRender(kind, text.length));
  /** The text last rendered on request (large files only), per path. */
  let rendered = $state<{ path: string; text: string } | null>(null);
  const previewText = $derived(large ? (rendered?.path === tab.path ? rendered.text : null) : text);
  const stale = $derived(large && previewText !== null && previewText !== text);

  // ── Split width ────────────────────────────────────────────────────────
  let previewW = $state(loadPaneWidth(WIDTH_KEY, 0, MIN_W, MAX_W));
  let paneEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    // First Split ever: half the editor pane.
    if (layout !== 'split' || previewW > 0 || !paneEl?.parentElement) return;
    previewW = Math.max(MIN_W, Math.round(paneEl.parentElement.clientWidth / 2));
  });

  const headingOrdinal = $derived(
    layout === 'split' && kind === 'markdown' && ide.activePath === tab.path && previewText !== null
      ? headingOrdinalAtLine(previewText, ide.cursorInfo.line)
      : null,
  );

  const mediaUrl = $derived(
    ide.target.kind === 'local' ? (p: string) => authedBlobUrl(`/fs/raw?path=${encodeURIComponent(p)}`) : null,
  );
  const previewRoot = $derived(findContainingRoot(tab.path, ide.projectRoots)?.path ?? ide.target.root);
</script>

{#if shown}
  {#if layout === 'split'}
    <PaneDivider bind:width={previewW} storageKey={WIDTH_KEY} label="Resize the preview" min={MIN_W} max={MAX_W} defaultWidth={480} invert />
  {/if}
  <div
    class="ide-preview-pane"
    class:split={layout === 'split'}
    style:inline-size={layout === 'split' && previewW > 0 ? `${previewW}px` : null}
    bind:this={paneEl}
    data-testid="ide-preview-pane"
    data-layout={layout}
  >
    {#if large && (previewText === null || stale)}
      <div class="ide-pv-large" data-testid="ide-preview-large">
        <span>
          {previewText === null
            ? `This file is large (${formatBytes(text.length)}): the preview renders when you ask.`
            : 'The preview is out of date with your edits.'}
        </span>
        <button type="button" class="btn small" onclick={() => (rendered = { path: tab.path, text })} data-testid="ide-preview-render">
          {previewText === null ? 'Render preview' : 'Render again'}
        </button>
      </div>
    {/if}
    {#if previewText !== null && kind === 'd2'}
      <LazyMount lazy={IdeD2PreviewLazy} what="the preview" variant="panel" props={{ text: previewText }} />
    {:else if previewText !== null}
      <LazyMount
        lazy={FilePreviewLazy}
        what="the preview"
        variant="panel"
        props={{
          path: tab.path,
          kind,
          text: previewText,
          root: previewRoot,
          mediaUrl,
          mediaUnavailable: 'Images and PDFs preview in local projects only; this project is on a remote SSH host.',
          onOpenLink: (p: string) => void ide.openFile(p, { preview: false }),
          headingOrdinal,
        }}
      />
    {/if}
  </div>
{/if}

<style>
  .ide-preview-pane {
    flex: 1;
    min-inline-size: 0;
    min-block-size: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  .ide-preview-pane.split {
    flex: 0 0 auto;
    max-inline-size: calc(100% - 200px);
    border-inline-start: 1px solid var(--border);
  }
  .ide-pv-large {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    padding: 8px 12px;
    border-block-end: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-s);
    flex-shrink: 0;
  }
  /* The editor pane (IdeEditorArea .ide-pane) in Split / Preview: a row with
     the editor beside this pane, or this pane alone with the editor hidden. */
  /* `.ide-panes` first: one class more than the pane's own scoped column rule. */
  :global(.ide-panes .ide-pane.pv-split) {
    flex-direction: row;
  }
  :global(.ide-panes .ide-pane.pv-split > .code-editor-outer) {
    flex: 1 1 0;
    min-inline-size: 0;
  }
  :global(.ide-panes .ide-pane.pv-only > :not(.ide-preview-pane)) {
    display: none;
  }
</style>
