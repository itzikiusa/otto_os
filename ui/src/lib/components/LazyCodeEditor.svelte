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
  import type { ComponentProps } from 'svelte';
  import type CodeEditor from './CodeEditor.svelte';
  import LoadState from './LoadState.svelte';

  type Props = ComponentProps<typeof CodeEditor>;
  let props: Props = $props();

  let inner = $state<ReturnType<typeof CodeEditor> | undefined>();

  export function focus(): void {
    inner?.focus();
  }
  export function openSearch(): void {
    inner?.openSearch();
  }
  export function goToLine(line: number, col?: number): void {
    inner?.goToLine(line, col);
  }
</script>

{#if CodeEditorLazy.component}
  {@const E = CodeEditorLazy.component}
  <E bind:this={inner} {...props} />
{:else}
  <LoadState
    what="the editor"
    variant="panel"
    empty
    loading={!CodeEditorLazy.error}
    error={CodeEditorLazy.error}
    onretry={() => CodeEditorLazy.retry()}
  />
{/if}
