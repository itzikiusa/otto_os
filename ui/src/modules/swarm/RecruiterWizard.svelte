<script lang="ts">
  // Recruiter: name a role → an AI helper proposes a full agent definition
  // (title, reports-to, specialization, soul, skills, provider, schedule) → edit
  // → Hire.
  import Modal from '../../lib/components/Modal.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type { AgentSkill, CreateAgentReq, RecruitedAgent } from './types';

  interface Props {
    onclose: () => void;
  }
  let { onclose }: Props = $props();

  let step = $state(0);
  let role = $state('');
  let context = $state('');
  let busy = $state(false);

  // Editable proposal fields.
  let name = $state('');
  let title = $state('');
  let provider = $state('claude');
  let reportsTo = $state('');
  let specialization = $state('');
  let soulMd = $state('');
  let scopeMd = $state('');
  let avatar = $state('🤖');
  let skills = $state<AgentSkill[]>([]);
  let scheduleRaw = $state<RecruitedAgent['suggested_schedule']>(null);

  async function recruit() {
    if (!role.trim() || busy) return;
    busy = true;
    try {
      const r = await swarm.recruit(role.trim(), context.trim() || undefined);
      name = r.name;
      title = r.title || role.trim();
      provider = r.suggested_provider || 'claude';
      specialization = r.specialization;
      soulMd = r.soul_md;
      scopeMd = r.scope_md;
      avatar = r.avatar || '🤖';
      skills = r.skills.map((s) => ({ name: s.name, must_use: s.must_use }));
      scheduleRaw = r.suggested_schedule ?? null;
      // Resolve reports-to by title against existing agents.
      if (r.reports_to_title) {
        const m = (swarm.detail?.agents ?? []).find(
          (a) => a.title.toLowerCase() === r.reports_to_title!.toLowerCase(),
        );
        reportsTo = m?.id ?? '';
      }
      step = 1;
    } catch (e) {
      toasts.error('Recruiter failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }

  async function hire() {
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
      schedule: scheduleRaw ?? null,
    };
    try {
      await swarm.createAgent(swarm.detail.id, body);
      toasts.success(`${name} hired`);
      onclose();
    } catch (e) {
      toasts.error('Hire failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
    }
  }
</script>

<Modal title="Recruit an agent" width={560} {onclose}>
  {#if step === 0}
    <div class="field">
      <label for="rc-role">What role do you want to hire?</label>
      <input
        id="rc-role"
        class="input"
        placeholder="e.g. CTO, Backend Dev, UX Researcher"
        bind:value={role}
        onkeydown={(e) => e.key === 'Enter' && recruit()}
      />
    </div>
    <div class="field">
      <label for="rc-ctx">Any specifics? (optional)</label>
      <textarea id="rc-ctx" class="input" rows="3" bind:value={context} placeholder="Context to guide the recruiter…"></textarea>
    </div>
    <p class="dim small">The recruiter proposes a soul, skills, provider and schedule — you can edit everything before hiring.</p>
  {:else}
    <div class="grid2">
      <div class="field"><label for="rc-name">Name</label><input id="rc-name" class="input" bind:value={name} /></div>
      <div class="field"><label for="rc-title">Title</label><input id="rc-title" class="input" bind:value={title} /></div>
      <div class="field">
        <label for="rc-prov">Provider</label>
        <select id="rc-prov" class="input" bind:value={provider}>
          {#each Array.from(new Set([provider, 'claude', 'codex', 'agy'])) as p (p)}<option value={p}>{p}</option>{/each}
        </select>
      </div>
      <div class="field">
        <label for="rc-rep">Reports to</label>
        <select id="rc-rep" class="input" bind:value={reportsTo}>
          <option value="">— top of org —</option>
          {#each swarm.detail?.agents ?? [] as a (a.id)}<option value={a.id}>{a.name} ({a.title})</option>{/each}
        </select>
      </div>
    </div>
    <div class="field"><label for="rc-spec">Specialization</label><input id="rc-spec" class="input" bind:value={specialization} /></div>
    <div class="field"><label for="rc-soul">Soul</label><textarea id="rc-soul" class="input" rows="3" bind:value={soulMd}></textarea></div>
    <div class="field"><label for="rc-scope">Scope</label><textarea id="rc-scope" class="input" rows="2" bind:value={scopeMd}></textarea></div>
    <div class="field">
      <span class="label">Skills (★ = must use)</span>
      <div class="skills">
        {#each skills as s, i (s.name)}
          <span class="skill-chip" class:must={s.must_use}>
            <button class="link" onclick={() => (skills[i] = { ...s, must_use: !s.must_use })}>{s.must_use ? '★' : '☆'}</button>
            {s.name}
            <button class="link" onclick={() => (skills = skills.filter((_, j) => j !== i))}>×</button>
          </span>
        {/each}
        {#if skills.length === 0}<span class="dim small">no library skills proposed</span>{/if}
      </div>
    </div>
    {#if scheduleRaw}<p class="dim small">Suggested schedule: {scheduleRaw.cadence}{scheduleRaw.at ? ` @ ${scheduleRaw.at}` : ''} — editable later in the agent editor.</p>{/if}
  {/if}

  {#snippet footer()}
    {#if step === 0}
      <button class="btn ghost" onclick={onclose}>Cancel</button>
      <button class="btn primary" onclick={recruit} disabled={!role.trim() || busy}>
        {busy ? 'Recruiting…' : 'Propose agent'}
      </button>
    {:else}
      <button class="btn ghost" onclick={() => (step = 0)}>Back</button>
      <button class="btn primary" onclick={hire} disabled={!name.trim() || busy}>Hire</button>
    {/if}
  {/snippet}
</Modal>

<style>
  .grid2 {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .skills {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
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
  .small {
    font-size: 11px;
  }
</style>
