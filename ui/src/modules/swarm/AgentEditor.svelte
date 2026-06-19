<script lang="ts">
  // Create or edit a swarm agent: identity, soul, scope, provider, reports-to,
  // skills (with must-use), and an optional schedule.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { AgentSchedule, AgentSkill, CreateAgentReq, SwarmAgent } from './types';

  interface Props {
    agent?: SwarmAgent | null;
    /** Prefill (e.g. from the recruiter). Ignored when `agent` is set. */
    prefill?: Partial<CreateAgentReq> | null;
    onclose: () => void;
  }
  let { agent = null, prefill = null, onclose }: Props = $props();

  // Props are read once for this keyed one-shot modal (untrack avoids the
  // "only captures initial value" reactivity warning).
  const editing = untrack(() => !!agent);
  const src = untrack(() => agent ?? prefill ?? {}) as Partial<SwarmAgent>;
  const initSched = untrack(
    () => (agent?.schedule ?? prefill?.schedule ?? null) as AgentSchedule | null,
  );

  let name = $state(src.name ?? '');
  let title = $state(src.title ?? '');
  let provider = $state(src.provider ?? swarm.detail?.config.provider ?? 'claude');
  let reportsTo = $state<string>((src.reports_to ?? '') as string);
  let specialization = $state(src.specialization ?? '');
  let soulMd = $state((src.soul_md ?? '') as string);
  let scopeMd = $state((src.scope_md ?? '') as string);
  let avatar = $state(src.avatar ?? '🤖');
  let skills = $state<AgentSkill[]>([...((src.skills ?? []) as AgentSkill[])]);
  let newSkill = $state('');
  let scheduled = $state(!!initSched?.enabled);
  let cadence = $state<AgentSchedule['cadence']>(initSched?.cadence ?? 'daily');
  let everyMin = $state(initSched?.every_min ?? 60);
  let at = $state(initSched?.at ?? '09:00');
  let weekday = $state(initSched?.weekday ?? 1);
  let directive = $state(initSched?.directive ?? '');

  const providers = $derived(
    Array.from(new Set([provider, 'claude', 'codex', 'agy'])).filter(Boolean),
  );

  let busy = $state(false);

  function addSkill() {
    const n = newSkill.trim();
    if (!n) return;
    if (!skills.some((s) => s.name === n)) skills = [...skills, { name: n, must_use: false }];
    newSkill = '';
  }

  function buildSchedule(): AgentSchedule | null {
    if (!scheduled) return null;
    const base: AgentSchedule = { cadence, directive, enabled: true };
    if (cadence === 'interval') base.every_min = everyMin;
    if (cadence === 'daily') base.at = at;
    if (cadence === 'weekly') {
      base.at = at;
      base.weekday = weekday;
    }
    return base;
  }

  async function save() {
    if (!name.trim() || busy || !swarm.detail) return;
    busy = true;
    const body: CreateAgentReq = {
      name: name.trim(),
      provider,
      title: title.trim(),
      reports_to: reportsTo || null,
      specialization: specialization.trim(),
      soul_md: soulMd.trim() || null,
      scope_md: scopeMd.trim(),
      avatar: avatar.trim(),
      skills,
      schedule: buildSchedule(),
    };
    try {
      if (editing && agent) await swarm.updateAgent(agent.id, body);
      else await swarm.createAgent(swarm.detail.id, body);
      toasts.success(editing ? 'Agent updated' : 'Agent hired');
      onclose();
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={editing ? `Edit ${agent?.name}` : 'New agent'} width={560} {onclose}>
  <div class="grid2">
    <div class="field">
      <label for="ag-name">Name</label>
      <input id="ag-name" class="input" bind:value={name} />
    </div>
    <div class="field">
      <label for="ag-title">Title / role</label>
      <input id="ag-title" class="input" bind:value={title} placeholder="e.g. CTO" />
    </div>
    <div class="field">
      <label for="ag-provider">Provider</label>
      <select id="ag-provider" class="input" bind:value={provider}>
        {#each providers as p (p)}<option value={p}>{p}</option>{/each}
      </select>
    </div>
    <div class="field">
      <label for="ag-reports">Reports to</label>
      <select id="ag-reports" class="input" bind:value={reportsTo}>
        <option value="">— top of org —</option>
        {#each (swarm.detail?.agents ?? []).filter((a) => a.id !== agent?.id) as a (a.id)}
          <option value={a.id}>{a.name} ({a.title})</option>
        {/each}
      </select>
    </div>
    <div class="field">
      <label for="ag-avatar">Avatar (emoji)</label>
      <input id="ag-avatar" class="input" bind:value={avatar} maxlength="4" />
    </div>
    <div class="field">
      <label for="ag-spec">Specialization</label>
      <input id="ag-spec" class="input" bind:value={specialization} />
    </div>
  </div>

  <div class="field">
    <label for="ag-soul">Soul (background + traits)</label>
    <textarea id="ag-soul" class="input" rows="3" bind:value={soulMd}></textarea>
  </div>
  <div class="field">
    <label for="ag-scope">Scope (what they own)</label>
    <textarea id="ag-scope" class="input" rows="2" bind:value={scopeMd}></textarea>
  </div>

  <div class="field">
    <label for="ag-skill">Skills (must-use toggle)</label>
    <div class="skill-add">
      <input
        id="ag-skill"
        class="input grow"
        placeholder="skill name…"
        bind:value={newSkill}
        onkeydown={(e) => e.key === 'Enter' && addSkill()}
      />
      <button class="btn small" onclick={addSkill}>Add</button>
    </div>
    <div class="skills">
      {#each skills as s, i (s.name)}
        <span class="skill-chip" class:must={s.must_use}>
          <button class="link" onclick={() => (skills[i] = { ...s, must_use: !s.must_use })}>
            {s.must_use ? '★' : '☆'}
          </button>
          {s.name}
          <button class="link" onclick={() => (skills = skills.filter((_, j) => j !== i))}>×</button>
        </span>
      {/each}
    </div>
  </div>

  <div class="field">
    <label class="row">
      <input type="checkbox" bind:checked={scheduled} /> Scheduled runs
    </label>
    {#if scheduled}
      <div class="sched">
        <select class="input small" bind:value={cadence}>
          <option value="interval">every N minutes</option>
          <option value="daily">daily</option>
          <option value="weekly">weekly</option>
        </select>
        {#if cadence === 'interval'}
          <input class="input small" type="number" min="1" bind:value={everyMin} /> min
        {/if}
        {#if cadence === 'daily' || cadence === 'weekly'}
          <input class="input small" type="time" bind:value={at} />
        {/if}
        {#if cadence === 'weekly'}
          <select class="input small" bind:value={weekday}>
            <option value={0}>Mon</option><option value={1}>Tue</option>
            <option value={2}>Wed</option><option value={3}>Thu</option>
            <option value={4}>Fri</option><option value={5}>Sat</option>
            <option value={6}>Sun</option>
          </select>
        {/if}
      </div>
      <textarea class="input" rows="2" placeholder="Standing directive (what to do each run)…" bind:value={directive}></textarea>
    {/if}
  </div>

  {#snippet footer()}
    <button class="btn ghost" onclick={onclose}>Cancel</button>
    <button class="btn primary" onclick={save} disabled={!name.trim() || busy}>
      {editing ? 'Save' : 'Hire'}
    </button>
  {/snippet}
</Modal>

<style>
  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .skill-add {
    display: flex;
    gap: 6px;
  }
  .skills {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-top: 6px;
  }
  .skill-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: 1px solid var(--border);
    border-radius: 999px;
    padding: 2px 8px;
    font-size: 11.5px;
  }
  .skill-chip.must {
    border-color: var(--accent);
    color: var(--accent);
  }
  .link {
    border: none;
    background: transparent;
    color: inherit;
    cursor: pointer;
    padding: 0;
  }
  .sched {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 6px 0;
    flex-wrap: wrap;
  }
</style>
