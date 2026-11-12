<script lang="ts">
  // A TLS certificate / key field that takes a file PATH or a pasted PEM.
  // A single-line <input> strips the line breaks out of pasted text, so a
  // pasted certificate used to arrive as one line; this is a textarea that
  // is one row for a path and grows (up to 8 rows, then scrolls) for PEM.
  // The file picker stays.
  import PathField from '../../lib/components/PathField.svelte';

  interface Props {
    id: string;
    label: string;
    value: string;
    placeholder?: string;
  }
  let { id, label, value = $bindable(''), placeholder = '' }: Props = $props();

  const rows = $derived(Math.min(8, Math.max(1, value.split('\n').length)));
</script>

<label for={id}>{label} <span class="dim">(optional)</span></label>
<PathField bind:value files>
  <textarea dir="ltr" {id} class="input mono pem" bind:value {rows} {placeholder} spellcheck="false" autocomplete="off"></textarea>
</PathField>

<style>
  .pem {
    flex: 1;
    min-width: 0;
    width: 0;
    height: auto;
    min-height: 27px;
    padding: 4px 8px;
    line-height: 1.35;
    resize: vertical;
    white-space: pre;
    overflow-x: auto;
  }
</style>
