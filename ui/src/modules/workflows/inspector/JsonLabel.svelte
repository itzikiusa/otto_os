<script lang="ts">
  // Label + zoom button for a cramped node-form JSON field (R10); the big
  // editor is the page's modal.
  import Icon from '../../../lib/components/Icon.svelte';

  let { forId, label, onzoom }: { forId: string; label: string; onzoom: () => void } = $props();
</script>

<div class="np-jsonlabel">
  <label for={forId}>{label}</label>
  <button type="button" class="np-zoom" title="Zoom — edit in a big editor" aria-label="Zoom JSON editor" onclick={onzoom}>
    <Icon name="maximize" size={12} />
  </button>
</div>
