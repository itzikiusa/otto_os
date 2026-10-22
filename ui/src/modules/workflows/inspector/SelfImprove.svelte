<script lang="ts">
  // self_improve: which agents reflect (empty = the Self-Improvement settings).
  import type { WorkflowNode } from '../../../lib/api/types';
  import { agentProviders } from '../../../lib/providers';
  import { selfImproveProviders as providersOf, toggled, type SetParam } from './util';

  let { node, set }: { node: WorkflowNode; set: SetParam } = $props();
  const selfImproveProviders = (): string[] => providersOf(node);
  function toggleSelfImproveProvider(prov: string): void {
    const next = toggled(selfImproveProviders(), prov);
    set('providers', next.length ? next : undefined);
  }
</script>

<p class="insp-note">
  Reflects on the workspace’s recent agent sessions and <strong>offers</strong>
  skill/memory improvements. They are <strong>queued for approval</strong> in
  Self-Improvement — never auto-applied — and the offered list is posted to the
  trigger’s chat thread.
</p>
<span class="np-label">Providers — the agent(s) that reflect (override Self-Improvement settings)</span>
<div class="rv-provs">
  {#each agentProviders() as prov (prov)}
    <label class="rv-chip" class:on={selfImproveProviders().includes(prov)}>
      <input
        type="checkbox"
        checked={selfImproveProviders().includes(prov)}
        onchange={() => toggleSelfImproveProvider(prov)}
      />
      {prov}
    </label>
  {/each}
</div>
<p class="node-hint">
  Leave all unchecked to use the providers configured in
  <strong>Settings → Self-Improvement</strong>.
</p>
