<script lang="ts">
  // Swarm settings: three tabs —
  //   • Standing goals — the swarm's quality bar, applied to every task (PUT set)
  //   • Team skills    — library skills every agent inherits (config.skills)
  //   • Triggers       — channel rules that auto-launch swarm work
  import Modal from '../../lib/components/Modal.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import GoalEditor from './GoalEditor.svelte';
  import SkillPicker from './SkillPicker.svelte';
  import { swarm } from '../../lib/stores/swarm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import type { CreateGoalReq, CreateTriggerReq, Swarm, SwarmChannelTrigger, SwarmGoal } from './types';

  let { onclose }: { onclose: () => void } = $props();

  const detail = $derived(swarm.detail);
  type Tab = 'goals' | 'skills' | 'triggers';
  let tab = $state<Tab>('goals');

  // -- Standing goals (edited as a local draft set, PUT on Save) --------------
  let draftGoals = $state<CreateGoalReq[]>([]);
  let goalsLoaded = $state(false);
  let savingGoals = $state(false);
  let goalEditorOpen = $state(false);
  let goalEditIndex = $state<number>(-1);

  $effect(() => {
    const sid = detail?.id;
    if (!sid || goalsLoaded) return;
    goalsLoaded = true;
    void swarm.loadStandingGoals(sid).then(() => {
      draftGoals = swarm.standingGoals.map((g) => ({
        title: g.title,
        description: g.description,
        metric: g.metric ?? undefined,
        comparator: g.comparator ?? undefined,
        target_value: g.target_value ?? undefined,
        block_value: g.block_value ?? undefined,
        verify_cmd: g.verify_cmd ?? undefined,
        max_retries: g.max_retries,
        blocking: g.blocking,
        order_idx: g.order_idx,
      }));
    });
  });

  function addGoal() {
    goalEditIndex = -1;
    goalEditorOpen = true;
  }
  function editGoal(i: number) {
    goalEditIndex = i;
    goalEditorOpen = true;
  }
  function onGoalSubmit(req: CreateGoalReq) {
    if (goalEditIndex >= 0) draftGoals = draftGoals.map((g, i) => (i === goalEditIndex ? req : g));
    else draftGoals = [...draftGoals, req];
  }
  function removeGoal(i: number) {
    draftGoals = draftGoals.filter((_, j) => j !== i);
  }
  async function saveGoals() {
    if (!detail) return;
    savingGoals = true;
    try {
      await swarm.putStandingGoals(detail.id, draftGoals);
      toasts.success('Standing goals saved');
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      savingGoals = false;
    }
  }

  // -- Team skills (config.skills) -------------------------------------------
  const teamSkills = $derived(((detail?.config.skills ?? []) as unknown[]).map((s) => String(s)));
  async function setTeamSkills(next: string[]) {
    if (!detail) return;
    const cfg = { ...(detail.config ?? {}), skills: next };
    try {
      await swarm.updateSwarm(detail.id, { config: cfg } as Partial<Swarm>);
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    }
  }

  // -- Triggers --------------------------------------------------------------
  let triggersLoaded = $state(false);
  let triggerForm = $state<CreateTriggerReq | null>(null);
  let triggerEditId = $state<string | null>(null);
  let savingTrigger = $state(false);

  $effect(() => {
    const sid = detail?.id;
    if (!sid || triggersLoaded) return;
    triggersLoaded = true;
    void swarm.loadTriggers(sid);
  });

  function newTrigger() {
    triggerEditId = null;
    triggerForm = { channel: 'slack', match_chat: '', keyword: '', repo_path: '', auto_start: true, reply: true, enabled: true };
  }
  function editTrigger(t: SwarmChannelTrigger) {
    triggerEditId = t.id;
    triggerForm = {
      channel: t.channel,
      match_chat: t.match_chat,
      keyword: t.keyword,
      repo_path: t.repo_path ?? '',
      auto_start: t.auto_start,
      reply: t.reply,
      enabled: t.enabled,
    };
  }
  async function saveTrigger() {
    if (!detail || !triggerForm) return;
    savingTrigger = true;
    try {
      const body: CreateTriggerReq = {
        channel: triggerForm.channel,
        match_chat: triggerForm.match_chat?.trim() || undefined,
        keyword: triggerForm.keyword?.trim() || undefined,
        repo_path: triggerForm.repo_path?.trim() || undefined,
        auto_start: triggerForm.auto_start,
        reply: triggerForm.reply,
        enabled: triggerForm.enabled,
      };
      if (triggerEditId) await swarm.updateTrigger(triggerEditId, body);
      else await swarm.createTrigger(detail.id, body);
      toasts.success(triggerEditId ? 'Trigger updated' : 'Trigger added');
      triggerForm = null;
      triggerEditId = null;
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    } finally {
      savingTrigger = false;
    }
  }
  async function toggleEnabled(t: SwarmChannelTrigger) {
    try {
      await swarm.updateTrigger(t.id, { enabled: !t.enabled });
    } catch (e) {
      toasts.error('Update failed', e instanceof Error ? e.message : String(e));
    }
  }
  async function delTrigger(t: SwarmChannelTrigger) {
    if (await confirmer.ask(`Delete this ${t.channel} trigger?`, { title: 'Delete trigger?' })) {
      try {
        await swarm.deleteTrigger(t.id);
      } catch (e) {
        toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
      }
    }
  }

  const TABS: { id: Tab; label: string; icon: string }[] = [
    { id: 'goals', label: 'Standing goals', icon: 'check' },
    { id: 'skills', label: 'Team skills', icon: 'zap' },
    { id: 'triggers', label: 'Triggers', icon: 'comment' },
  ];
</script>

<Modal title="Swarm settings{detail ? ` — ${detail.name}` : ''}" width={640} {onclose}>
  <div class="tabs">
    {#each TABS as t (t.id)}
      <button class="tab" class:active={tab === t.id} onclick={() => (tab = t.id)}>
        <Icon name={t.icon} size={13} /> {t.label}
      </button>
    {/each}
  </div>

  {#if tab === 'goals'}
    <p class="hint">Standing goals are the swarm's quality bar — verified on every task in addition to the task's own goals.</p>
    <div class="bar">
      <button class="btn small" onclick={addGoal}><Icon name="plus" size={12} /> Add standing goal</button>
      <span class="grow"></span>
      <button class="btn small primary" onclick={saveGoals} disabled={savingGoals}>
        {savingGoals ? 'Saving…' : 'Save standing goals'}
      </button>
    </div>
    {#if draftGoals.length === 0}
      <EmptyState icon="check" title="No standing goals" body="Add goals the whole swarm must hit — e.g. tests pass, no new clippy warnings." />
    {:else}
      <div class="list">
        {#each draftGoals as g, i (i)}
          <div class="row-item">
            <div class="ri-main">
              <span class="ri-title">{g.title}</span>
              {#if g.blocking}<span class="blocking">blocking</span>{/if}
              {#if g.metric}<span class="dim">· {g.metric}{g.comparator ? ` ${g.comparator}` : ''}{g.target_value != null ? ` ${g.target_value}` : ''}</span>{/if}
            </div>
            <button class="icon-btn small" onclick={() => editGoal(i)} aria-label="Edit"><Icon name="edit" size={13} /></button>
            <button class="icon-btn small" onclick={() => removeGoal(i)} aria-label="Remove"><Icon name="trash" size={13} /></button>
          </div>
        {/each}
      </div>
    {/if}
  {:else if tab === 'skills'}
    <p class="hint">Team skills are added to every agent in this swarm, on top of each agent's own skills.</p>
    <SkillPicker selected={teamSkills} onchange={setTeamSkills} />
  {:else if tab === 'triggers'}
    <p class="hint">Triggers auto-launch swarm work when a matching message arrives on a channel.</p>
    {#if triggerForm}
      <div class="form">
        <div class="grid">
          <div class="field">
            <label for="t-channel">Channel</label>
            <select id="t-channel" class="input" bind:value={triggerForm.channel}>
              <option value="slack">Slack</option>
              <option value="telegram">Telegram</option>
              <option value="webhook">Webhook</option>
            </select>
          </div>
          <div class="field">
            <label for="t-chat">Match chat / channel <span class="dim">(blank = any)</span></label>
            <input id="t-chat" class="input" bind:value={triggerForm.match_chat} placeholder="e.g. #builds or chat id" />
          </div>
          <div class="field">
            <label for="t-kw">Keyword <span class="dim">(blank = any)</span></label>
            <input id="t-kw" class="input" bind:value={triggerForm.keyword} placeholder="e.g. /swarm" />
          </div>
          <div class="field">
            <label for="t-repo">Repo path <span class="dim">(optional)</span></label>
            <input id="t-repo" class="input" bind:value={triggerForm.repo_path} placeholder="/path/to/repo" />
          </div>
        </div>
        <div class="toggles">
          <label class="row"><input type="checkbox" bind:checked={triggerForm.auto_start} /> Auto-start the swarm</label>
          <label class="row"><input type="checkbox" bind:checked={triggerForm.reply} /> Reply on the channel</label>
          <label class="row"><input type="checkbox" bind:checked={triggerForm.enabled} /> Enabled</label>
        </div>
        <div class="form-actions">
          <button class="btn small ghost" onclick={() => { triggerForm = null; triggerEditId = null; }}>Cancel</button>
          <button class="btn small primary" onclick={saveTrigger} disabled={savingTrigger}>
            {triggerEditId ? 'Save trigger' : 'Add trigger'}
          </button>
        </div>
      </div>
    {:else}
      <div class="bar">
        <span class="grow"></span>
        <button class="btn small" onclick={newTrigger}><Icon name="plus" size={12} /> Add trigger</button>
      </div>
      {#if swarm.triggers.length === 0}
        <EmptyState icon="comment" title="No triggers" body="Add a trigger to launch swarm work from a Slack/Telegram message or a webhook." />
      {:else}
        <div class="list">
          {#each swarm.triggers as t (t.id)}
            <div class="row-item">
              <div class="ri-main">
                <span class="tchip">{t.channel}</span>
                <span class="ri-title">{t.keyword || 'any'}</span>
                <span class="dim">{t.match_chat ? `in ${t.match_chat}` : 'any chat'}</span>
                {#if t.auto_start}<span class="dim">· auto-start</span>{/if}
                {#if t.reply}<span class="dim">· reply</span>{/if}
              </div>
              <button class="toggle" class:on={t.enabled} onclick={() => toggleEnabled(t)} title={t.enabled ? 'Enabled — click to disable' : 'Disabled — click to enable'}>
                {t.enabled ? 'On' : 'Off'}
              </button>
              <button class="icon-btn small" onclick={() => editTrigger(t)} aria-label="Edit"><Icon name="edit" size={13} /></button>
              <button class="icon-btn small" onclick={() => delTrigger(t)} aria-label="Delete"><Icon name="trash" size={13} /></button>
            </div>
          {/each}
        </div>
      {/if}
    {/if}
  {/if}

  {#snippet footer()}
    <button class="btn ghost" onclick={onclose}>Close</button>
  {/snippet}
</Modal>

{#if goalEditorOpen}
  <GoalEditor
    goal={goalEditIndex >= 0 ? (draftGoals[goalEditIndex] as unknown as SwarmGoal) : null}
    onsubmit={onGoalSubmit}
    onclose={() => { goalEditorOpen = false; goalEditIndex = -1; }}
  />
{/if}

<style>
  .tabs {
    display: flex;
    gap: 4px;
    border-bottom: 1px solid var(--border);
    margin-bottom: 12px;
  }
  .tab {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: none;
    border-bottom: 2px solid transparent;
    background: transparent;
    color: var(--text-dim);
    padding: 6px 10px;
    font-size: 12.5px;
    cursor: pointer;
  }
  .tab:hover {
    color: var(--text);
  }
  .tab.active {
    color: var(--accent);
    border-bottom-color: var(--accent);
  }
  .hint {
    font-size: 12px;
    color: var(--text-dim);
    margin: 0 0 10px;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-bottom: 10px;
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .row-item {
    display: flex;
    align-items: center;
    gap: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 10px;
    background: var(--surface);
  }
  .ri-main {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    font-size: 12.5px;
    flex-wrap: wrap;
  }
  .ri-title {
    font-weight: 600;
  }
  .blocking {
    font-size: 10px;
    color: var(--status-exited);
    border: 1px solid color-mix(in srgb, var(--status-exited) 40%, transparent);
    border-radius: 999px;
    padding: 0 6px;
  }
  .tchip {
    font-size: 10px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--accent);
    border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
    border-radius: 999px;
    padding: 0 6px;
  }
  .toggle {
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text-dim);
    border-radius: 999px;
    padding: 1px 10px;
    font-size: 11px;
    cursor: pointer;
  }
  .toggle.on {
    background: #7ee787;
    color: #0a0a0a;
    border-color: #7ee787;
    font-weight: 600;
  }
  .form {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 10px;
  }
  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .field label {
    font-size: 12px;
    color: var(--text-dim);
  }
  .toggles {
    display: flex;
    flex-wrap: wrap;
    gap: 12px;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12.5px;
  }
  .form-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
