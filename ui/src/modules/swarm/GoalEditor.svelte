<script lang="ts">
  // Create or edit a single swarm goal. Used for per-task / per-project explicit
  // goals AND (in builder mode) the swarm's standing-goal templates.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { CreateGoalReq, GoalComparator, SwarmGoal } from './types';

  interface Props {
    /** When set, edit this goal. Otherwise create a new one in `scope`. */
    goal?: SwarmGoal | null;
    /** Where a NEW goal is created — exactly one of task/project. Ignored when
     *  `goal` is set or when `onsubmit` is provided (builder mode). */
    scope?: { task?: string; project?: string } | null;
    /** Builder mode: return the request to the caller instead of POSTing it
     *  (the standing-goals editor collects a set, then PUTs them at once). */
    onsubmit?: (req: CreateGoalReq) => void;
    onclose: () => void;
  }
  let { goal = null, scope = null, onsubmit, onclose }: Props = $props();

  const editing = untrack(() => !!goal);
  const src = untrack(() => goal ?? {}) as Partial<SwarmGoal>;

  let title = $state(src.title ?? '');
  let description = $state(src.description ?? '');
  let metric = $state((src.metric ?? '') as string);
  let comparator = $state<GoalComparator>((src.comparator as GoalComparator) ?? 'lte');
  let targetValue = $state(src.target_value == null ? '' : String(src.target_value));
  let blockValue = $state(src.block_value == null ? '' : String(src.block_value));
  let verifyCmd = $state((src.verify_cmd ?? '') as string);
  let maxRetries = $state(src.max_retries ?? 2);
  let blocking = $state(src.blocking ?? false);
  let busy = $state(false);

  const COMPARATORS: { value: GoalComparator; label: string }[] = [
    { value: 'lte', label: '≤ (at most)' },
    { value: 'gte', label: '≥ (at least)' },
    { value: 'eq', label: '= (exactly)' },
    { value: 'contains', label: 'contains' },
    { value: 'absent', label: 'absent' },
  ];

  function buildReq(): CreateGoalReq {
    const req: CreateGoalReq = { title: title.trim() };
    if (description.trim()) req.description = description.trim();
    if (metric.trim()) req.metric = metric.trim();
    if (comparator) req.comparator = comparator;
    const tv = targetValue.trim();
    if (tv !== '' && Number.isFinite(Number(tv))) req.target_value = Number(tv);
    const bv = blockValue.trim();
    if (bv !== '' && Number.isFinite(Number(bv))) req.block_value = Number(bv);
    if (verifyCmd.trim()) req.verify_cmd = verifyCmd.trim();
    req.max_retries = Math.max(0, Math.floor(Number(maxRetries) || 0));
    req.blocking = blocking;
    return req;
  }

  async function save() {
    if (!title.trim() || busy) return;
    const req = buildReq();
    // Builder mode — hand the request back, no network call.
    if (onsubmit) {
      onsubmit(req);
      onclose();
      return;
    }
    busy = true;
    try {
      if (editing && goal) {
        await swarm.updateGoal(goal.id, req);
        toasts.success('Goal updated');
      } else if (scope) {
        await swarm.createGoal(scope, req);
        toasts.success('Goal added');
      }
      onclose();
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={editing ? 'Edit goal' : 'Add goal'} width={520} {onclose}>
  <div class="field">
    <label for="g-title">Title</label>
    <input id="g-title" class="input" bind:value={title} placeholder="e.g. Tests pass" />
  </div>
  <div class="field">
    <label for="g-desc">Description <span class="dim">(what the verifier checks)</span></label>
    <textarea id="g-desc" class="input" rows="2" bind:value={description}></textarea>
  </div>

  <div class="grid">
    <div class="field">
      <label for="g-metric">Metric <span class="dim">(optional)</span></label>
      <input id="g-metric" class="input" bind:value={metric} placeholder="e.g. failing_tests" />
    </div>
    <div class="field">
      <label for="g-cmp">Comparator</label>
      <select id="g-cmp" class="input" bind:value={comparator}>
        {#each COMPARATORS as c (c.value)}<option value={c.value}>{c.label}</option>{/each}
      </select>
    </div>
    <div class="field">
      <label for="g-target">Target value</label>
      <input id="g-target" class="input" type="number" step="any" bind:value={targetValue} placeholder="pass when met" />
    </div>
    <div class="field">
      <label for="g-block">Block value <span class="dim">(blocks if breached)</span></label>
      <input id="g-block" class="input" type="number" step="any" bind:value={blockValue} placeholder="optional hard stop" />
    </div>
  </div>

  <div class="field">
    <label for="g-cmd">Verify command <span class="dim">(optional — run to measure)</span></label>
    <input id="g-cmd" class="input mono" bind:value={verifyCmd} placeholder="e.g. cargo test --workspace" />
  </div>

  <div class="grid">
    <div class="field">
      <label for="g-retries">Max retries</label>
      <input id="g-retries" class="input" type="number" min="0" bind:value={maxRetries} />
    </div>
    <div class="field check">
      <label class="row">
        <input type="checkbox" bind:checked={blocking} />
        Blocking <span class="dim">(must pass before the task is done)</span>
      </label>
    </div>
  </div>

  {#snippet footer()}
    <button class="btn ghost" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={save} disabled={!title.trim() || busy}>
      {editing ? 'Save' : 'Add'}
    </button>
  {/snippet}
</Modal>

<style>
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-bottom: 10px;
  }
  .field label {
    font-size: 12px;
    color: var(--text-dim);
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .check {
    justify-content: flex-end;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
    color: var(--text);
  }
  .mono {
    font-family: var(--mono, monospace);
    font-size: 12px;
  }
</style>
