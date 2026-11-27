<script lang="ts">
  // Project environment: OTTO_IDE_TRUST_PROJECT_EXECUTION shown as what it is, a switch
  // (on = "1", off = the variable removed; see withProjectTrust in ideLspTrust.ts).
  import Switch from '../../../lib/components/Switch.svelte';
  import { ide } from '../ideStore.svelte';

  let { checked, onchange }: { checked: boolean; onchange: (next: boolean) => void } = $props();
  const where = $derived(ide.target.kind === 'ssh' ? `on ${ide.target.connection_name || 'the SSH host'}` : 'on this Mac');
</script>

<section class="env-trust" aria-labelledby="env-trust-heading" data-testid="ide-env-trust">
  <Switch {checked} label="Trust this project — language servers may run its build code" {onchange} />
  <div class="copy">
    <h3 id="env-trust-heading" class="title">Trust this project — language servers may run its build code</h3>
    <p class="sub">Lets the Zig, Kotlin, Vue and .NET language servers run build.zig, the Gradle or Maven import, the plugins tsconfig.json names or MSBuild targets {where}. Turn it on only for code you trust.</p>
  </div>
</section>

<style>
  .env-trust {
    display: flex; align-items: flex-start; gap: 10px; padding: 12px;
    border-radius: var(--radius-m); background: var(--surface-2); border: 1px solid var(--border);
  }
  .copy { display: flex; flex-direction: column; gap: 4px; min-inline-size: 0; }
  .title { margin: 0; font-size: var(--fs-s); font-weight: 600; color: var(--text); }
  .sub { margin: 0; font-size: var(--fs-xs); color: var(--text-dim); line-height: 1.4; }
</style>
