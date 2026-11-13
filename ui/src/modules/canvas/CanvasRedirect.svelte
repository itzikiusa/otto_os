<script lang="ts">
  // `#/canvas` — the Canvas page folded into Design Hall: every scene is
  // mirrored there live and opens as a Whiteboard design. This page forwards:
  // a scene someone asked to open (`canvas.pendingOpenId`, set by callers that
  // still use the old hand-off, or `#/canvas/<scene id>`) goes to its Hall
  // copy; anything else lands on the Whiteboard studio. A board with NO Hall
  // copy (too large to copy with its images, an image missing, no Design Hall
  // access) or whose copy is a mirror behind it (`meta.sync_error`) is never a
  // dead end: the kept editor (CanvasPage.svelte, loaded lazily) opens it
  // here, under one line that says why. `#/canvas/<id>/editor` (nav.ts
  // `canvasEditorRoute`, used once a caller already looked) skips the wait.
  import { onMount, type Component } from 'svelte';
  import { router } from '../../lib/router.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { findSceneHallCopy } from '../design-hall/nav'; // ui-guards: allow — the Canvas route forwards into Design Hall
  import { legacyEditorNote, opensInHall, type SceneHallCopy } from '../design-hall/mirrorSync'; // ui-guards: allow — why a board opens in the Canvas editor
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { announceShown } from '../../lib/announce.svelte';

  let Editor: Component | null = $state(null);
  let editorFailed = $state(false);
  let note = $state('');
  /** The board's Hall copy, when it has one (the note links to it). */
  let hallId: string | null = $state(null);
  let sceneId: string | null = null;

  function explain(copy: SceneHallCopy | null | 'unavailable'): void {
    note = legacyEditorNote(copy);
    hallId = copy && copy !== 'unavailable' ? copy.id : null;
  }

  async function openEditor(): Promise<void> {
    editorFailed = false;
    canvas.pendingOpenId = sceneId; // CanvasPage opens it on mount
    try {
      Editor = (await import('./CanvasPage.svelte')).default as Component;
    } catch {
      editorFailed = true;
    }
  }

  onMount(() => {
    sceneId = canvas.pendingOpenId ?? router.parts[1] ?? null;
    canvas.pendingOpenId = null;
    if (!sceneId) {
      router.replace('design/studio/whiteboard');
      return;
    }
    const id = sceneId;
    if (router.parts[2] === 'editor') {
      // The caller already looked for a Hall copy: open now, explain once known.
      explain(null);
      void openEditor();
      void findSceneHallCopy(id, 1).then(explain);
      return;
    }
    void findSceneHallCopy(id).then((copy) => {
      if (opensInHall(copy)) {
        router.replace(`design/a/${encodeURIComponent(copy.id)}`);
        return;
      }
      explain(copy);
      void openEditor();
    });
  });
</script>

{#if Editor}
  <div class="legacy">
    <p class="why" data-testid="canvas-legacy-note" use:announceShown={note}>
      <Icon name="info" size={12} />
      <span>{note}</span>
      {#if hallId}<button class="btn small ghost" onclick={() => hallId && router.go(`design/a/${encodeURIComponent(hallId)}`)}>Open Design Hall’s copy</button>{/if}
    </p>
    <div class="page"><Editor /></div>
  </div>
{:else if editorFailed}
  <LoadState what="the Canvas editor" variant="page" error="Otto may have updated while this window was open." onretry={() => void openEditor()} />
{:else}
  <div class="fwd" aria-busy="true" aria-label="Opening the board">
    <Skeleton rows={2} height={160} />
  </div>
{/if}

<style>
  .fwd {
    padding: 16px;
  }
  .legacy {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    height: 100%;
    min-height: 0;
  }
  .why {
    margin: 0;
    padding: 6px 12px;
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    border-block-end: 1px solid var(--border);
    background: var(--info-soft);
    color: var(--text);
    font-size: var(--fs-xs);
  }
  .why > :global(svg) {
    color: var(--info);
    flex: none;
  }
  .why > span {
    flex: 1 1 220px;
    min-width: 0;
  }
  .page {
    min-height: 0;
    overflow: hidden;
  }
</style>
