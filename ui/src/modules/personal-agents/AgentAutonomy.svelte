<script lang="ts">
  // Autonomy tab: permission modes (Proactive / Directed / Scheduled), standing
  // goals + daily budget, custom rules, and "your agent" (primary). One form,
  // one Save — the daemon derives which rules it can enforce.
  import { untrack } from 'svelte';
  import { personalAgentsApi } from '../../lib/api/personalAgents';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { router } from '../../lib/router.svelte';
  import { guardUnsaved } from '../../lib/leaveGuard';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import Icon from '../../lib/components/Icon.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import RelTime from '../../lib/components/RelTime.svelte';
  import type { PersonalAgentAutonomy, PersonalAgentRule } from '../../lib/api/types';

  interface Props {
    agentId: string;
    editable: boolean;
    /** Told after a successful save (the page header shows mode badges). */
    onsaved?: (a: PersonalAgentAutonomy) => void;
  }
  let { agentId, editable, onsaved }: Props = $props();
  /** Prefix for the label↔control ids. */
  const uid = $props.id();

  interface GoalDraft { id?: string; text: string; enabled: boolean; last_run_at: string | null }
  interface RuleDraft { id?: string; text: string; enforce: PersonalAgentRule['enforce'] }

  let loading = $state(true);
  let loadError = $state<string | null>(null);
  let saving = $state(false);
  let saved = $state<PersonalAgentAutonomy | null>(null);

  let proactiveOn = $state(false);
  let runsPerDay = $state(4);
  let maxMinutes = $state(15);
  let goals = $state<GoalDraft[]>([]);
  let rules = $state<RuleDraft[]>([]);
  let primary = $state(false);

  const schedules = $derived(personalAgents.schedulesByAgent[agentId] ?? []);
  const readOnlySchedules = $derived(schedules.filter((s) => s.permission === 'read_only').length);

  function apply(a: PersonalAgentAutonomy): void {
    saved = a;
    proactiveOn = a.proactive.enabled;
    runsPerDay = a.proactive.runs_per_day;
    maxMinutes = a.proactive.max_minutes;
    goals = a.goals.map((g) => ({ ...g }));
    rules = a.rules.map((r) => ({ ...r }));
    primary = a.primary;
  }

  async function load(): Promise<void> {
    loading = true;
    loadError = null;
    try {
      apply(await personalAgentsApi.autonomy(agentId));
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    void agentId;
    untrack(() => void load());
  });

  const dirty = $derived.by(() => {
    if (!saved) return false;
    const cur = JSON.stringify({
      p: [proactiveOn, Number(runsPerDay), Number(maxMinutes)],
      g: goals.map((g) => [g.id ?? '', g.text.trim(), g.enabled]).filter((g) => g[1] !== ''),
      r: rules.map((r) => [r.id ?? '', r.text.trim()]).filter((r) => r[1] !== ''),
      primary,
    });
    const was = JSON.stringify({
      p: [saved.proactive.enabled, saved.proactive.runs_per_day, saved.proactive.max_minutes],
      g: saved.goals.map((g) => [g.id, g.text, g.enabled]),
      r: saved.rules.map((r) => [r.id, r.text]),
      primary: saved.primary,
    });
    return cur !== was;
  });
  $effect(() => router.guard(() => !saving));
  $effect(() => guardUnsaved(() => dirty, { what: 'this agent’s autonomy settings' }));

  async function save(): Promise<void> {
    if (saving) return;
    saving = true;
    try {
      const next = await personalAgentsApi.saveAutonomy(agentId, {
        proactive: {
          enabled: proactiveOn,
          runs_per_day: Math.min(24, Math.max(1, Math.round(Number(runsPerDay) || 1))),
          max_minutes: Math.min(60, Math.max(1, Math.round(Number(maxMinutes) || 1))),
        },
        goals: goals.map((g) => ({ id: g.id, text: g.text, enabled: g.enabled })),
        rules: rules.map((r) => ({ id: r.id, text: r.text })),
        primary,
      });
      apply(next);
      onsaved?.(next);
      toasts.success('Saved');
    } catch (e) {
      toasts.error('Couldn’t save the agent’s autonomy settings', loadErrorText(e));
    } finally {
      saving = false;
    }
  }

  let workingGoal = $state<string | null>(null);
  async function workNow(goalId: string): Promise<void> {
    workingGoal = goalId;
    try {
      await personalAgentsApi.runGoal(agentId, goalId);
      toasts.info('Working on the goal — read-only. Findings land in Runs.');
      router.go(`personal-agents/${agentId}/activity`);
    } catch (e) {
      toasts.error('Couldn’t start the goal run', loadErrorText(e));
    } finally {
      workingGoal = null;
    }
  }

  function enforceLabel(r: RuleDraft): string {
    if (!r.enforce) return 'Instructions only';
    const terms = r.enforce.terms.map((t) => `“${t}”`).join(', ');
    return r.enforce.kind === 'deny'
      ? `Enforced: blocks actions mentioning ${terms}`
      : `Enforced: asks you before actions mentioning ${terms}`;
  }
</script>

<LoadState what="this agent’s autonomy settings" {loading} error={loadError} rows={4} onretry={() => void load()}>
  <div class="autonomy">
    <section class="pa-panel" aria-labelledby="modes-h">
      <h2 id="modes-h">Permission modes</h2>
      <p class="hint">Every run says which mode it ran in. Read-only is enforced by Otto, not just asked of the agent.</p>
      <div class="modes">
        <div class="mode" class:on={proactiveOn}>
          <div class="mode-head">
            <Icon name="eye" size={14} />
            <strong>Proactive</strong>
            <span class="chip">Read-only</span>
          </div>
          <p class="mode-body">Works its standing goals in the background and reports findings. It can read, search and browse, but can’t send, post, write or change anything.</p>
          <label class="chk"><input type="checkbox" bind:checked={proactiveOn} disabled={!editable || saving} /> Work on standing goals in the background</label>
          <div class="field-row">
            <div class="field">
              <label for="{uid}-runs">Runs per day (max)</label>
              <input id="{uid}-runs" class="input" type="number" min="1" max="24" bind:value={runsPerDay} disabled={!editable || saving} />
            </div>
            <div class="field">
              <label for="{uid}-minutes">Minutes per run (max)</label>
              <input id="{uid}-minutes" class="input" type="number" min="1" max="60" bind:value={maxMinutes} disabled={!editable || saving} />
            </div>
          </div>
        </div>
        <div class="mode on">
          <div class="mode-head">
            <Icon name="hand" size={14} />
            <strong>Directed</strong>
          </div>
          <p class="mode-body">Run now, chat and tasks the Assistant hands it. Actions go through your normal approvals and auto-approve rules.</p>
        </div>
        <div class="mode on">
          <div class="mode-head">
            <Icon name="calendar" size={14} />
            <strong>Scheduled</strong>
          </div>
          <p class="mode-body">
            Each schedule has its own permission set.
            {schedules.length === 0
              ? 'No schedules yet.'
              : `${readOnlySchedules} of ${schedules.length} ${schedules.length === 1 ? 'is' : 'are'} read-only.`}
          </p>
          <button class="link" onclick={() => router.go(`personal-agents/${agentId}/schedules`)}>Manage schedules</button>
        </div>
      </div>
      <p class="note" role="note">
        <Icon name="shield" size={12} />
        Account, credential and sharing actions always ask you first — auto-approve rules never skip them.
      </p>
    </section>

    <section class="pa-panel" aria-labelledby="goals-h">
      <div class="card-head">
        <h2 id="goals-h">Standing goals</h2>
        {#if editable}
          <button class="btn small" disabled={saving} onclick={() => (goals = [...goals, { text: '', enabled: true, last_run_at: null }])}>
            <Icon name="plus" size={12} /> Add goal
          </button>
        {/if}
      </div>
      <p class="hint">Proactive runs pick the goal worked least recently, within the daily budget above. Findings go to Runs — never delivered.</p>
      {#if goals.length === 0}
        <p class="muted">No standing goals. Add one, e.g. “Watch the release branch CI and flag new failures”.</p>
      {:else}
        <ul class="list">
          {#each goals as g, i (g.id ?? `new-${i}`)}
            <li class="item">
              <input type="checkbox" bind:checked={g.enabled} disabled={!editable || saving} aria-label="Goal enabled" title="Enabled" />
              <input class="input grow" bind:value={g.text} disabled={!editable || saving} placeholder="What should it keep an eye on?" aria-label="Goal" />
              <span class="meta">Worked <RelTime iso={g.last_run_at} fallback="never" /></span>
              {#if editable && g.id}
                <button class="btn small" disabled={workingGoal !== null || dirty || saving} title={dirty ? 'Save first' : 'Start a read-only run on this goal now'} onclick={() => g.id && workNow(g.id)}>Work on it now</button>
              {/if}
              {#if editable}
                <button class="icon-btn" disabled={saving} aria-label="Remove goal" title="Remove goal" onclick={() => (goals = goals.filter((_, j) => j !== i))}>
                  <Icon name="trash" size={14} />
                </button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="pa-panel" aria-labelledby="rules-h">
      <div class="card-head">
        <h2 id="rules-h">Rules</h2>
        {#if editable}
          <button class="btn small" disabled={saving} onclick={() => (rules = [...rules, { text: '', enforce: null }])}>
            <Icon name="plus" size={12} /> Add rule
          </button>
        {/if}
      </div>
      <p class="hint">Plain language, added to the agent’s instructions. “Ask before…” or “Never…” plus a target (prod, a “quoted name”, a #channel) is also enforced on its actions.</p>
      {#if rules.length === 0}
        <p class="muted">No rules. Try “Ask before touching prod”.</p>
      {:else}
        <ul class="list">
          {#each rules as r, i (r.id ?? `new-${i}`)}
            <li class="item rule">
              <input class="input grow" bind:value={r.text} disabled={!editable || saving} placeholder="e.g. Never post in #general" aria-label="Rule" />
              {#if r.id && saved?.rules.some((x) => x.id === r.id && x.text === r.text.trim())}
                <span class="chip" class:pa-enf={!!r.enforce} title="Derived by Otto from the rule’s wording">
                  {#if r.enforce}<Icon name="lock" size={12} />{/if}{enforceLabel(r)}
                </span>
              {:else}
                <span class="meta">Save to see what Otto enforces</span>
              {/if}
              {#if editable}
                <button class="icon-btn" disabled={saving} aria-label="Remove rule" title="Remove rule" onclick={() => (rules = rules.filter((_, j) => j !== i))}>
                  <Icon name="trash" size={14} />
                </button>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </section>

    <section class="pa-panel" aria-labelledby="primary-h">
      <h2 id="primary-h">Your agent</h2>
      <p class="hint">Your primary assistant: it handles general requests and routes specialist work to your other agents through a shared room. One per workspace.</p>
      <label class="chk"><input type="checkbox" bind:checked={primary} disabled={!editable || saving} /> Make this my primary agent</label>
    </section>

    {#if editable}
      <div class="actions">
        <button class="btn" disabled={!dirty || saving} onclick={() => saved && apply(saved)}>Discard</button>
        <button class="btn primary" disabled={!dirty || saving} onclick={save}>{saving ? 'Saving…' : 'Save'}</button>
      </div>
    {/if}
  </div>
</LoadState>

<style>
  .autonomy { display: flex; flex-direction: column; gap: 12px; }
  .pa-panel { border: 1px solid var(--border); background: var(--surface); border-radius: var(--radius-m); padding: 12px 14px; color: var(--text); min-width: 0; }
  .pa-panel h2 { margin: 0 0 6px; font-size: var(--fs-l); font-weight: 600; }
  .card-head { display: flex; align-items: center; justify-content: space-between; gap: 8px; flex-wrap: wrap; }
  .hint, .muted { color: var(--text-dim); font-size: var(--fs-s); margin: 0 0 8px; }
  .modes { display: grid; grid-template-columns: repeat(auto-fit, minmax(220px, 1fr)); gap: 10px; }
  .mode { border: 1px solid var(--border); border-radius: var(--radius-m); padding: 10px; display: flex; flex-direction: column; gap: 6px; background: var(--bg); }
  .mode.on { border-color: color-mix(in srgb, var(--accent) 35%, var(--border)); }
  .mode-head { display: flex; align-items: center; gap: 6px; font-size: var(--fs-m); }
  .mode-body { margin: 0; color: var(--text-dim); font-size: var(--fs-s); }
  .note { display: flex; align-items: center; gap: 6px; margin: 10px 0 0; font-size: var(--fs-s); color: var(--text-dim); }
  .list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 6px; }
  .item { display: flex; align-items: center; gap: 8px; flex-wrap: wrap; }
  .grow { flex: 1; min-width: 16ch; }
  .meta { color: var(--text-dim); font-size: var(--fs-s); }
  .pa-enf { color: var(--accent-text); border-color: var(--accent-line); display: inline-flex; align-items: center; gap: 4px; }
  .field-row { display: flex; gap: 10px; flex-wrap: wrap; }
  .field-row > .field { flex: 1; min-width: 120px; margin-bottom: 0; }
  .chk { display: flex; align-items: center; gap: 6px; font-size: var(--fs-m); }
  .link { border: 0; background: none; padding: 0; font: inherit; font-size: var(--fs-s); color: var(--accent-text); cursor: pointer; align-self: flex-start; }
  .link:hover { text-decoration: underline; }
  .actions { display: flex; gap: 8px; justify-content: flex-end; }
</style>
