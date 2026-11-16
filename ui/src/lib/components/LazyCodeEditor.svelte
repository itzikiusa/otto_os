<script module lang="ts">
  import { lazyComponent } from '../lazy-component.svelte';

  // ONE shared loader for every LazyCodeEditor: the CodeMirror 6 chunk is
  // fetched the first time any secondary editor embeds, then every later mount
  // renders synchronously in the same frame (lazy-component.svelte.ts).
  const CodeEditorLazy = lazyComponent(() => import('./CodeEditor.svelte'));
</script>

<script lang="ts">
  // CodeEditor.svelte, code-split. Surfaces that only mount an editor when the
  // user opens a file preview, raw request body/script tab, or raw response
  // body (FileTree, RequestBuilder, ResponseViewer) use this so CodeMirror 6
  // stays out of the page/panel's static import set. Editor-first hosts (IDE,
  // Workbench, Database QueryEditor, Vault NoteView) keep the static import.
  import { onMount, untrack, type ComponentProps } from 'svelte';
  import { loadErrorText } from '../loadError';
  import type CodeEditor from './CodeEditor.svelte';
  import LoadState from './LoadState.svelte';

  type Props = ComponentProps<typeof CodeEditor>;
  let props: Props = $props();

  let inner = $state<ReturnType<typeof CodeEditor> | undefined>();
  let pendingFocus = false;
  let pendingOpenSearch = false;
  let pendingGoToLine: { line: number; col?: number } | null = null;

  onMount(() => {
    untrack(() => {
      if (CodeEditorLazy.error) CodeEditorLazy.retry();
    });
  });

  $effect(() => {
    const inst = inner;
    if (!inst) return;
    untrack(() => {
      if (pendingGoToLine) {
        const { line, col } = pendingGoToLine;
        pendingGoToLine = null;
        inst.goToLine(line, col);
      }
      if (pendingOpenSearch) {
        pendingOpenSearch = false;
        inst.openSearch();
      }
      if (pendingFocus) {
        pendingFocus = false;
        inst.focus();
      }
    });
  });

  export function focus(): void {
    if (inner) inner.focus();
    else pendingFocus = true;
  }
  export function openSearch(): void {
    if (inner) inner.openSearch();
    else pendingOpenSearch = true;
  }
  export function goToLine(line: number, col?: number): void {
    if (inner) inner.goToLine(line, col);
    else pendingGoToLine = { line, col };
  }
</script>

{#if CodeEditorLazy.component}
  {@const E = CodeEditorLazy.component}
  <E bind:this={inner} {...props} />
{:else}
  <div class="lazy-editor-fallback">
    <LoadState
      what="the editor"
      variant="compact"
      empty
      loading={!CodeEditorLazy.error}
      error={CodeEditorLazy.error ? loadErrorText(CodeEditorLazy.error) : null}
      onretry={() => CodeEditorLazy.retry()}
    />
  </div>
{/if}

<style>
  .lazy-editor-fallback {
    display: flex;
    align-items: center;
    justify-content: center;
    inline-size: 100%;
    block-size: 100%;
    min-block-size: 0;
    overflow-y: auto;
    box-sizing: border-box;
  }
</style>
