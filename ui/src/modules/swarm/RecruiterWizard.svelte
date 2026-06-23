<script lang="ts">
  // Recruiter: name a role → an AI helper proposes a full agent definition
  // (title, reports-to, specialization, soul, skills, provider, schedule) → edit
  // → Hire.
  import { untrack } from 'svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { isAbortError } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import type { AgentSkill, CreateAgentReq, RecruitedAgent } from './types';

  interface Props {
    onclose: () => void;
    /** Open directly on the editable proposal (e.g. hiring from a completed
     *  recruit run whose modal was closed). */
    proposal?: RecruitedAgent | null;
    /** The recruit run this proposal came from — marked hired on success so its
     *  Runs-list "Hire" button disappears. */
    proposalRunId?: string | null;
  }
  let { onclose, proposal = null, proposalRunId = null }: Props = $props();

  // Read the seed proposal once (untrack avoids the props-in-$state warning).
  const seed = untrack(() => proposal);
  const seededReportsTo = untrack(
    () =>
      (seed?.reports_to_title
        ? (swarm.detail?.agents ?? []).find(
            (a) => a.title.toLowerCase() === seed.reports_to_title!.toLowerCase(),
          )?.id
        : '') ?? '',
  );

  let step = $state(seed ? 1 : 0);
  let role = $state('');
  let context = $state('');
  let busy = $state(false);
  let recruitCtl: AbortController | null = null;

  // Editable proposal fields (pre-filled when hiring from a completed run).
  let name = $state(seed?.name ?? '');
  let title = $state(seed?.title ?? '');
  let provider = $state(seed?.suggested_provider ?? 'claude');
  let model = $state(seed?.suggested_model ?? '');
  let reportsTo = $state(seededReportsTo);
  let specialization = $state(seed?.specialization ?? '');
  let soulMd = $state(seed?.soul_md ?? '');
  let scopeMd = $state(seed?.scope_md ?? '');
  let avatar = $state(seed?.avatar ?? '🤖');
  let skills = $state<AgentSkill[]>(
    seed ? seed.skills.map((s) => ({ name: s.name, must_use: s.must_use })) : [],
  );
  let scheduleRaw = $state<RecruitedAgent['suggested_schedule']>(seed?.suggested_schedule ?? null);
  // How many copies to hire at once (e.g. the same role on 2 models / shifts).
  let count = $state(1);

  // Naming theme: the recruiter derives the agent's name from it (a fun, cohesive
  // roster). Defaults to the swarm's saved theme; persisted on recruit.
  const NAMING_THEMES = [
    'Famous footballers',
    'NBA legends',
    'F1 drivers',
    'Greek mythology',
    'Norse mythology',
    'Sci-fi captains',
    'Classical composers',
    'Renowned scientists',
    'Chess grandmasters',
    'Jazz legends',
  ];
  let namingTheme = $state(untrack(() => swarm.detail?.config?.naming_theme ?? ''));

  async function recruit() {
    if (!role.trim() || busy) return;
    recruitCtl = new AbortController();
    busy = true;
    // Remember the theme on the swarm so the whole roster stays consistent.
    if (swarm.detail && (swarm.detail.config?.naming_theme ?? '') !== namingTheme) {
      void swarm.setNamingTheme(swarm.detail.id, namingTheme);
    }
    try {
      const r = await swarm.recruit(
        role.trim(),
        context.trim() || undefined,
        namingTheme || undefined,
        recruitCtl.signal,
      );
      name = r.name;
      title = r.title || role.trim();
      provider = r.suggested_provider || 'claude';
      model = r.suggested_model ?? '';
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
      if (isAbortError(e)) toasts.info('Recruiting stopped');
      else toasts.error('Recruiter failed', e instanceof Error ? e.message : String(e));
    } finally {
      busy = false;
      recruitCtl = null;
    }
  }

  function stopRecruit() {
    // Kill the live recruiter session server-side, then abandon the UI wait.
    if (swarm.detail) void swarm.stopAgentRun(swarm.detail.id);
    recruitCtl?.abort();
  }

  async function hire() {
    if (!name.trim() || busy || !swarm.detail) return;
    busy = true;
    const n = Math.max(1, Math.min(20, Math.floor(count) || 1));
    const base: CreateAgentReq = {
      name: name.trim(),
      provider,
      model: model.trim() || null,
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
      for (let i = 0; i < n; i++) {
        // Suffix names when hiring multiple so they're distinguishable.
        const body = n > 1 ? { ...base, name: `${base.name} ${i + 1}` } : base;
        await swarm.createAgent(swarm.detail.id, body);
      }
      toasts.success(n > 1 ? `Hired ${n}× ${name}` : `${name} hired`);
      if (proposalRunId) swarm.markRecruitHired(proposalRunId);
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
    <div class="field">
      <label for="rc-theme">Naming theme <span class="dim">(optional — names the agent after it)</span></label>
      <select id="rc-theme" class="input" bind:value={namingTheme}>
        <option value="">No theme (sensible name)</option>
        {#each NAMING_THEMES as t (t)}<option value={t}>{t}</option>{/each}
      </select>
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
        <label for="rc-model">Model <span class="dim">(optional)</span></label>
        <input id="rc-model" class="input" bind:value={model} placeholder="default — e.g. opus / sonnet" />
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
      {#if busy}
        <button class="btn" onclick={stopRecruit} title="Stop waiting for the recruiter">Stop</button>
      {/if}
      <button class="btn primary" onclick={recruit} disabled={!role.trim() || busy}>
        {busy ? 'Recruiting…' : 'Propose agent'}
      </button>
    {:else}
      <button class="btn ghost" onclick={() => (step = 0)}>Back</button>
      <label class="count" title="Hire this many copies (e.g. the same role on different models)">
        ×<input class="input num" type="number" min="1" max="20" bind:value={count} />
      </label>
      <button class="btn primary" onclick={hire} disabled={!name.trim() || busy}>
        {busy ? 'Hiring…' : count > 1 ? `Hire ${count}` : 'Hire'}
      </button>
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
  .count {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    color: var(--text-dim);
    font-size: 12px;
    margin-right: auto;
  }
  .count .num {
    width: 52px;
  }
</style>
