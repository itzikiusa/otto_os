<script lang="ts">
  // Read-only "View full document" (Mongo) / "View raw JSON" (any engine): the
  // complete row pretty-printed — the same Extended JSON the document editor
  // seeds from ($oid, $date, …) — in a read-only CodeEditor, so every nested
  // value is on the page and ⌘F / Find searches all of it (the JSON tree only
  // renders its open branches). CodeMirror virtualizes, so a 100 KB+ document
  // opens instantly. Mounted by ResultsGrid while `flow.rawDoc` is set.
  import Icon from '../../lib/components/Icon.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import CodeEditor from '../../lib/components/CodeEditor.svelte';
  import type { EditFlow } from './EditFlow.svelte';
  import { copyText, fmtBytes } from './results-format';
  import { claimFind, editorEscGuard } from './doc-modal';

  interface Props {
    flow: EditFlow;
  }
  let { flow }: Props = $props();

  let editor = $state<ReturnType<typeof CodeEditor> | null>(null);
  let wrap = $state(true);

  $effect(() => claimFind(() => editor?.openSearch()));
  $effect(() => {
    if (!editor) return;
    const raf = requestAnimationFrame(() => editor?.focus());
    return () => cancelAnimationFrame(raf);
  });
</script>

{#if flow.rawDoc}
  {@const doc = flow.rawDoc}
  <Modal title={flow.engine === 'mongodb' ? 'Full document' : 'Raw JSON'} width={760} onclose={() => (flow.rawDoc = null)}>
    <div class="raw-doc">
      <div class="rd-tools">
        <span class="rd-meta">{fmtBytes(doc.text.length)} · read-only</span>
        <span class="grow"></span>
        <button class="icon-btn" onclick={() => editor?.openSearch()} aria-label="Find in document" title="Find in document (⌘F)">
          <Icon name="search" size={13} />
        </button>
        <button class="icon-btn wrap-toggle" aria-pressed={wrap} onclick={() => (wrap = !wrap)} aria-label="Wrap long lines" title="Wrap long lines">
          <Icon name="text" size={13} />
        </button>
        <button class="btn small" onclick={() => copyText(doc.text, ['Copied', 'Full document copied'])} title="Copy the full document">
          <Icon name="copy" size={12} />Copy
        </button>
      </div>
      <div class="rd-body" use:editorEscGuard>
        <CodeEditor
          bind:this={editor}
          findOwner={false}
          path="document.json"
          root=""
          content={doc.text}
          readOnly
          {wrap}
          highlightLineLimit={20000}
        />
      </div>
    </div>
  </Modal>
{/if}

<style>
  .raw-doc {
    display: flex;
    flex-direction: column;
    gap: 8px;
    min-height: 0;
  }
  .rd-tools {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .rd-meta {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .wrap-toggle[aria-pressed='true'] {
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  .rd-body {
    height: clamp(240px, 65vh, 640px);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
</style>
