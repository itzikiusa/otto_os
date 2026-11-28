<script lang="ts">
  // The .NET page's Unity debugger row (rc.6 A1): DotRush's Mono debug adapter,
  // downloaded on request (it carries Microsoft components under their own
  // terms, so Otto never bundles it). Installing also adds .NET 10 when missing.
  import Icon from '../../lib/components/Icon.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import type { IdeToolStatus } from '../../lib/api/types';

  let { status, busy, oninstall }: { status: IdeToolStatus; busy: boolean; oninstall: () => void } = $props();
</script>

<section class="tc-tool-box" aria-labelledby="tc-unity-heading" data-testid="ide-tc-unity-debugger">
  <div class="tc-tool-head">
    <div>
      <h3 id="tc-unity-heading" class="tc-sec-title">Unity debugger</h3>
      <span class="tc-tool-name" dir="ltr">{status.name} {status.version ?? ''}</span>
    </div>
    <StatusBadge tone={status.available ? 'success' : 'warning'} label={status.available ? 'Installed' : 'Missing'} />
  </div>
  <p class="tc-tool-hint">Attach to the Unity Editor on this Mac. DotRush Mono debugger (MIT; includes Microsoft components under their own licence terms), downloaded from its GitHub release and checked. Adds .NET 10 when it is missing.</p>
  <div class="tc-tool-actions">
    <button type="button" class="btn small" class:ghost={status.available} disabled={busy} onclick={oninstall}>
      <Icon name="bug" size={12} />
      {busy ? 'Installing…' : status.available ? 'Reinstall Unity debugger' : 'Install Unity debugger'}
    </button>
  </div>
</section>

<style>
  .tc-tool-box {
    display: flex; flex-direction: column; gap: 8px; padding: 12px;
    border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border); min-inline-size: 0;
  }
  .tc-tool-head { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; }
  .tc-sec-title { margin: 0; font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .tc-tool-name { display: block; font-family: var(--font-mono); font-size: var(--fs-xs); color: var(--text-dim); margin-block-start: 2px; }
  .tc-tool-hint { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); }
  .tc-tool-actions { display: flex; align-items: center; gap: 6px; margin-block-start: auto; padding-block-start: 4px; }
</style>
