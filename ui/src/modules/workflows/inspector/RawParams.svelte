<script lang="ts">
  // Fallback raw-JSON params editor for unrecognised or future node kinds.
  import type { WorkflowNode } from '../../../lib/api/types';

  let { node, setParams }: { node: WorkflowNode; setParams: (params: unknown) => void } = $props();
</script>

<!-- Fallback raw-JSON editor for unrecognised or future node kinds -->
<label for="np-raw">Params (JSON)</label>
<textarea dir="ltr"
  id="np-raw"
  rows="5"
  placeholder="&#123;&#125;"
  value={node.params != null ? JSON.stringify(node.params, null, 2) : ''}
  oninput={(e) => {
    try {
      setParams(JSON.parse(e.currentTarget.value));
    } catch { /* keep typing */ }
  }}
></textarea>
