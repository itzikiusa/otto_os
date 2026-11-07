<!--
  Add or edit one routine: which days and when (in a timezone — yours by
  default), what it does and what it may do, with a live "Next: …" preview in
  your own time. "Every few minutes", "Once" and a raw cron stay one pick
  away under Repeats.
-->
<script lang="ts">
  import { announceShown } from '../../lib/announce.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  import { personalAgents } from '../../lib/stores/personalAgents.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { confirmer } from '../../lib/confirm.svelte';
  import { routineLabel } from '../../lib/personalAgentBadges';
  import type { PersonalAgent, PersonalAgentPermission, PersonalAgentSchedule } from '../../lib/api/types';
  import { browserTz, buildCadence, DAY_CHIPS, daysOf, loadCadence, nextForDays, scheduleForDays, timezones, workdays, type CadenceForm } from './cadence';
  import { joinDirective, nextRunWords, splitDirective } from './agentHome';

  interface Props {
    agent: PersonalAgent;
    /** null = a new routine. */
    routine: PersonalAgentSchedule | null;
    onclose: () => void;
  }
  let { agent, routine, onclose }: Props = $props();
  // A new routine starts from the agent's suggested first routine, if any.
  // svelte-ignore state_referenced_locally
  const draft = routine ? undefined : personalAgents.routineDrafts[agent.id];
  const uid = $props.id();

  // Mounted fresh per open: the routine's values seed the form once.
  // svelte-ignore state_referenced_locally
  const init = routine;
  const parsed = init ? daysOf(init.schedule) : { days: workdays(), at: '09:00' };
  let repeats = $state<'days' | 'interval' | 'once' | 'cron'>(parsed ? 'days' : (loadCadence(init?.schedule ?? {}).cadence as 'interval' | 'once' | 'cron'));
  let days = $state<number[]>(parsed?.days ?? workdays());
  let at = $state(parsed?.at ?? '09:00');
  let other = $state<CadenceForm>(loadCadence(init?.schedule ?? { cadence: 'interval', every_min: 60 }, init?.timezone || browserTz()));
  let tz = $state(init?.timezone || browserTz());
  const split = splitDirective(init?.directive ?? '');
  let name = $state(split.name || (draft?.name ?? ''));
  let directive = $state(split.instruction || (draft?.instruction ?? ''));
  let permission = $state<PersonalAgentPermission>(init?.permission ?? 'directed');
  let busy = $state(false);
  let error = $state('');
  const ZONES = timezones(init?.timezone || browserTz());

  function toggleDay(d: number): void {
    days = days.includes(d) ? days.filter((x) => x !== d) : [...days, d].sort((a, b) => a - b);
  }

  const preview = $derived.by(() => {
    if (!agent.enabled) return 'Won’t run — the agent is paused';
    if (init && !init.enabled) return 'Paused — won’t run until you resume it';
    if (repeats === 'days') {
      const n = nextForDays(days, at, tz);
      if (!n) return '';
      const mine = tz === browserTz();
      return `Next: ${nextRunWords(n)}${mine ? '' : ` your time (${at} ${tz})`}`;
    }
    if (repeats === 'interval') return `Next: ${nextRunWords(new Date(Date.now() + Math.max(5, other.everyMin) * 60_000))}`;
    return '';
  });

  async function save(): Promise<void> {
    if (busy) return;
    error = '';
    if (repeats === 'days' && days.length === 0) return void (error = 'Pick at least one day.');
    if (repeats === 'once' && !other.runAt) return void (error = 'Pick the date and time it runs.');
    if (!directive.trim()) return void (error = 'Say what it should do.');
    const schedule = repeats === 'days' ? scheduleForDays(days, at) : buildCadence({ ...other, cadence: repeats });
    const body = { schedule, timezone: tz, directive: joinDirective(name, directive), permission, enabled: init?.enabled ?? true };
    // Same time as another routine of this agent: usually a slip — ask, don't block.
    const twin = (personalAgents.schedulesByAgent[agent.id] ?? []).find(
      (s) => s.id !== init?.id && s.timezone === tz && JSON.stringify(s.schedule) === JSON.stringify(schedule),
    );
    if (twin) {
      const other = routineLabel(twin.directive);
      const verb = init ? 'save' : 'add';
      const ok = await confirmer.ask(`Same time as “${other}” — ${verb} anyway?`, { title: 'Two routines at the same time', confirmLabel: init ? 'Save anyway' : 'Add anyway' });
      if (!ok) return;
    }
    busy = true;
    try {
      if (init) await personalAgents.updateSchedule(agent.id, init.id, body);
      else {
        await personalAgents.createSchedule(agent.id, body);
        if (draft) {
          const { [agent.id]: _used, ...rest } = personalAgents.routineDrafts;
          personalAgents.routineDrafts = rest;
        }
      }
      onclose();
    } catch (e) {
      error = `Couldn’t save the routine. ${loadErrorText(e)}`;
    } finally {
      busy = false;
    }
  }
</script>

{#snippet zoneOptions()}
  {#each ZONES.top as z (z)}<option value={z}>{z}{z === browserTz() ? ' (yours)' : ''}</option>{/each}
  {#each ZONES.regions as [region, zones] (region)}
    <optgroup label={region}>
      {#each zones as z (z)}<option value={z}>{z.slice(z.indexOf('/') + 1).replaceAll('_', ' ')}</option>{/each}
    </optgroup>
  {/each}
{/snippet}

<Modal title={init ? 'Edit routine' : `New routine for ${agent.name}`} width={520} {onclose}>
  <form class="rf" onsubmit={(e) => { e.preventDefault(); void save(); }} data-testid="pa-routine-form">
    {#if error}<p class="rf-err" use:announceShown={{ text: error, assertive: true }}>{error}</p>{/if}
    {#if !init}<p class="rf-note" data-testid="pa-routine-chat-hint">Or just tell {agent.name} in chat: “every weekday at 9, brief me”.</p>{/if}
    <div class="field">
      <label for="{uid}-name">Name <span class="rf-opt">(optional)</span></label>
      <input id="{uid}-name" class="input" dir="auto" maxlength="60" bind:value={name} placeholder="e.g. Morning PR brief" />
    </div>
    <div class="field">
      <label for="{uid}-what">What should it do?</label>
      <textarea id="{uid}-what" class="input" rows="3" dir="auto" bind:value={directive} placeholder="e.g. Brief me on my open pull requests and anything failing"></textarea>
    </div>
    <div class="field">
      <label for="{uid}-repeats">Repeats</label>
      <select id="{uid}-repeats" class="input" bind:value={repeats}>
        <option value="days">On these days</option>
        <option value="interval">Every few minutes</option>
        <option value="once">Once</option>
        <option value="cron">Custom (cron)</option>
      </select>
    </div>
    {#if repeats === 'days'}
      <fieldset class="rf-days">
        <legend>Days</legend>
        {#each DAY_CHIPS as d, i (d)}
          <button type="button" class="rf-day" aria-pressed={days.includes(i)} onclick={() => toggleDay(i)}>{d}</button>
        {/each}
      </fieldset>
      <div class="rf-row">
        <div class="field">
          <label for="{uid}-at">At</label>
          <input id="{uid}-at" class="input" type="time" dir="ltr" bind:value={at} />
        </div>
        <div class="field rf-grow">
          <label for="{uid}-tz">Timezone</label>
          <select id="{uid}-tz" class="input" bind:value={tz}>
            {@render zoneOptions()}
          </select>
        </div>
      </div>
    {:else if repeats === 'interval'}
      <div class="field">
        <label for="{uid}-every">Every (minutes, at least 5)</label>
        <input id="{uid}-every" class="input" type="number" min="5" bind:value={other.everyMin} />
      </div>
    {:else if repeats === 'once'}
      <div class="rf-row">
        <div class="field">
          <label for="{uid}-once">On</label>
          <input id="{uid}-once" class="input" type="datetime-local" bind:value={other.runAt} />
        </div>
        <div class="field rf-grow">
          <label for="{uid}-tz2">Timezone</label>
          <select id="{uid}-tz2" class="input" bind:value={tz}>
            {@render zoneOptions()}
          </select>
        </div>
      </div>
    {:else}
      <div class="rf-row">
        <div class="field rf-grow">
          <label for="{uid}-cron">Cron (5 fields)</label>
          <input id="{uid}-cron" class="input" dir="ltr" bind:value={other.cronExpr} placeholder="0 9 * * 0-4" />
        </div>
        <div class="field rf-grow">
          <label for="{uid}-tz3">Timezone</label>
          <select id="{uid}-tz3" class="input" bind:value={tz}>
            {@render zoneOptions()}
          </select>
        </div>
      </div>
    {/if}
    <div class="field">
      <label for="{uid}-may">It can</label>
      <select id="{uid}-may" class="input" bind:value={permission}>
        <option value="directed">Make changes (asks you first)</option>
        <option value="read_only">Only read and report</option>
      </select>
    </div>
    {#if preview}<p class="rf-next" data-testid="pa-routine-preview">{preview}</p>{/if}
    {#if !agent.enabled}<p class="rf-note">{agent.name} is paused — saving a routine resumes it.</p>{/if}
  </form>
  {#snippet footer()}
    <button type="button" class="btn" disabled={busy} onclick={onclose}>Cancel</button>
    <button type="button" class="btn primary" disabled={busy} onclick={() => void save()}>{busy ? 'Saving…' : init ? 'Save routine' : 'Add routine'}</button>
  {/snippet}
</Modal>

<style>
  .rf { display: flex; flex-direction: column; gap: 12px; }
  .rf :global(.field) { margin: 0; }
  .rf-row { display: flex; gap: 12px; flex-wrap: wrap; }
  .rf-row > .field { min-inline-size: 120px; }
  .rf-grow { flex: 1; min-inline-size: 180px; }
  .rf-days { display: flex; flex-wrap: wrap; gap: 6px; border: 0; padding: 0; margin: 0; }
  .rf-days legend { font-size: var(--fs-s); color: var(--text-dim); margin-block-end: 4px; }
  .rf-day {
    min-inline-size: 44px; padding: 6px 8px; border-radius: var(--radius-m); border: 1px solid var(--border);
    background: var(--surface); color: var(--text); font: inherit; font-size: var(--fs-s); cursor: pointer;
  }
  .rf-day[aria-pressed='true'] { border-color: var(--accent-line-strong); background: var(--accent-soft); color: var(--accent-text); font-weight: 600; }
  .rf-day:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 1px; }
  .rf-next { margin: 0; font-size: var(--fs-m); color: var(--text); }
  .rf-note { margin: 0; font-size: var(--fs-s); color: var(--text-dim); }
  .rf-err { margin: 0; color: var(--danger); font-size: var(--fs-s); }
  .rf-opt { color: var(--text-dim); font-weight: normal; }
</style>
