<script lang="ts">
  // condition — on run data (ux.md §9.3): a field, an operator and a value,
  // with the expression it writes shown under it; "Write an expression
  // instead" keeps the free expression (`passed == true` stays valid).
  import type { WorkflowNode } from '../../../lib/api/types';
  import { CONDITION_OPS, parseRule, ruleExpr, ruleFields, type ConditionOp, type ConditionRule } from '../editor/conditionRule';
  import { paramStr, type SetParam } from './util';

  let { node, set, inputKeys }: { node: WorkflowNode; set: SetParam; inputKeys: readonly string[] } = $props();
  const ids = $props.id();
  const expr = $derived(paramStr(node, 'expr'));
  const parsed = $derived(parseRule(expr));
  let free = $state(false);
  const freeMode = $derived(free || (!!expr && !parsed));
  const rule = $derived<ConditionRule>(parsed ?? { field: 'len(run.repos)', op: 'gt', value: '1' });
  const fields = $derived.by(() => {
    const known = ruleFields(inputKeys);
    return known.some((f) => f.field === rule.field) ? known : [...known, { field: rule.field, label: rule.field }];
  });
  const write = (patch: Partial<ConditionRule>): void => set('expr', ruleExpr({ ...rule, ...patch }));
</script>

{#if freeMode}
  <label for="{ids}-expr">Expression</label>
  <input dir="ltr" id="{ids}-expr" type="text" class="dc-mono" placeholder="passed == true" value={expr} oninput={(e) => set('expr', e.currentTarget.value)} />
  <p class="node-hint">Sees the previous step’s output and <code>run.input</code>, <code>run.repos</code>, <code>run.prs</code>.</p>
  {#if !expr || parsed}<button type="button" class="btn ghost small dc-switch" onclick={() => (free = false)}>Use a rule instead</button>{/if}
{:else}
  <label for="{ids}-field">Field</label>
  <select id="{ids}-field" value={rule.field} onchange={(e) => write({ field: e.currentTarget.value })}>
    {#each fields as f (f.field)}<option value={f.field}>{f.label}</option>{/each}
  </select>
  <div class="dc-row">
    <select aria-label="Operator" value={rule.op} onchange={(e) => write({ op: e.currentTarget.value as ConditionOp })}>
      {#each CONDITION_OPS as o (o.op)}<option value={o.op}>{o.label}</option>{/each}
    </select>
    {#if rule.op !== 'empty'}
      <input dir="auto" type="text" aria-label="Value" value={rule.value} oninput={(e) => write({ value: e.currentTarget.value })} />
    {/if}
  </div>
  <p class="node-hint">Writes <code class="dc-mono">{ruleExpr(rule)}</code></p>
  <button type="button" class="btn ghost small dc-switch" onclick={() => (free = true)}>Write an expression instead</button>
{/if}
<p class="node-hint">Outgoing connections marked yes / no follow the result.</p>

<style>
  .dc-row {
    display: grid;
    grid-template-columns: minmax(0, 8em) minmax(0, 1fr);
    gap: 6px;
  }
  .dc-switch {
    align-self: flex-start;
  }
  .dc-mono {
    font-family: var(--font-mono);
  }
</style>
