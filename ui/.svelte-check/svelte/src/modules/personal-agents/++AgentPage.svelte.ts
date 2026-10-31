///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
// One personal agent: Overview / Activity / Autonomy / Schedules / Runs /
// Chat / Memory / Context tabs.
import { announce } from '../../lib/announce.svelte';
import { latestOnly } from '../../lib/latest';
import RelTime from '../../lib/components/RelTime.svelte';
import { tabKeys } from '../../lib/tabKeys';
import { tick, untrack } from 'svelte';
import { personalAgents } from '../../lib/stores/personalAgents.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { confirmRunAndDeliver } from '../../lib/confirmOutward';
import { personalAgentsApi } from '../../lib/api/personalAgents';
import { authedText } from '../../lib/api/client';
import { ws } from '../../lib/stores/workspace.svelte';
import { router } from '../../lib/router.svelte';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Modal from '../../lib/components/Modal.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import { runStatus } from '../../lib/status';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import AgentAvatar from './AgentAvatar.svelte';
import AgentEditSheet from './AgentEditSheet.svelte';
import AgentAutonomy from './AgentAutonomy.svelte';
import AgentActivity from './AgentActivity.svelte';
import { lazyComponent } from '../../lib/lazy-component.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { browserTz, buildCadence, cadenceLabel, defaultCadence, loadCadence, WEEKDAYS, type CadenceForm } from './cadence';
import type { PersonalAgentAutonomy, PersonalAgentPermission, PersonalAgentRun, PersonalAgentSchedule } from '../../lib/api/types';
;

  interface Props {
    agentId: string;
  };;

  type Tab = 'overview' | 'activity' | 'autonomy' | 'schedules' | 'runs' | 'chat' | 'memory' | 'context';;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const AgentMemoryInspectorLazy = lazyComponent(() => import('./AgentMemoryInspector.svelte'));
  const AgentDocumentsLazy = lazyComponent(() => import('./AgentDocuments.svelte'));

  let { agentId }: Props = $props();
  /** Prefix for the schedule form's label↔control ids. */
  const uid = $props.id();

  const TABS: { id: Tab; label: string }[] = [
    { id: 'overview', label: 'Overview' },
    { id: 'activity', label: 'Activity' },
    { id: 'autonomy', label: 'Autonomy' },
    { id: 'schedules', label: 'Schedules' },
    { id: 'runs', label: 'Runs' },
    { id: 'chat', label: 'Chat' },
    { id: 'memory', label: 'Memory' },
    { id: 'context', label: 'Context' },
  ];
  // The tab is part of the URL (`#/personal-agents/<id>/<tab>`), so Run now can
  // land on Runs and a reload / back keeps the section.
  const tab = $derived<Tab>(TABS.some((t) => t.id === router.parts[2]) ? (router.parts[2] as Tab) : 'overview');
  let tabTransition = 0;
  async function goTab(t: Tab, event?: MouseEvent): Promise<void> {
    const list = (event?.currentTarget as HTMLElement | undefined)?.closest('[role="tablist"]');
    const seq = ++tabTransition;
    const accepted = await router.goChecked(t === 'overview' ? `personal-agents/${agentId}` : `personal-agents/${agentId}/${t}`);
    await tick();
    await Promise.resolve();
    if (seq !== tabTransition || !list?.isConnected || list.closest('[inert]')) return;
    // hashchange may follow this microtask; focus the accepted target only.
    list.querySelector<HTMLButtonElement>(`[role="tab"][data-tab="${accepted ? t : tab}"]`)?.focus();
  }
  let editing = $state(false);
  let busy = $state(false);
  let error = $state('');

  const agent = $derived(personalAgents.agent(agentId));
  const schedules = $derived(personalAgents.schedulesByAgent[agentId] ?? []);
  const runs = $derived(personalAgents.runsByAgent[agentId] ?? []);

  // The agent may not be cached yet (deep link) — the list load fills it in.
  // Once per (workspace, agent): the lookup is untracked, so a list that comes
  // back WITHOUT the id ("Agent not found") doesn't re-run this and refetch
  // in a loop (ui-performance-2-02).
  let lookedUp = '';
  $effect(() => {
    const wsId = ws.currentId;
    const key = `${wsId}|${agentId}`;
    if (!wsId || key === lookedUp) return;
    if (untrack(() => personalAgents.agent(agentId))) return;
    lookedUp = key;
    // A failed lookup is not "not found": forget it, so a later visit asks again.
    void retryAgents(wsId).then(() => { if (personalAgents.agentsError && lookedUp === key) lookedUp = ''; });
  });
  const retryAgents = (wsId: string | null) => (wsId ? personalAgents.loadAgents(wsId) : Promise.resolve());

  $effect(() => {
    if (tab === 'runs') void personalAgents.loadRuns(agentId);
  });

  // Autonomy summary for the header badges (primary / proactive). Best-effort:
  // the Autonomy tab shows its own load state.
  let autonomy = $state<PersonalAgentAutonomy | null>(null);
  $effect(() => {
    const id = agentId;
    autonomy = null;
    personalAgentsApi
      .autonomy(id)
      .then((a) => { if (id === agentId) autonomy = a; })
      .catch(() => {});
  });

  // Reset: memories + conversations + schedules + run history (confirmed by
  // typing the agent's name — the server checks it too).
  let memoryReload = $state(0);
  async function resetAgent(): Promise<void> {
    if (!agent) return;
    const name = agent.name;
    const typed = await confirmer.promptText(
      `Reset “${name}”? It forgets everything it learned (memory), its chat is closed, and its schedules and run history are deleted. Its persona, rules, goals and model stay. Type the agent’s name to confirm.`,
      { title: 'Reset agent', confirmLabel: 'Reset agent', placeholder: name, danger: true },
    );
    if (typed === null) return;
    if (typed.trim() !== name.trim()) {
      toasts.error('Not reset', 'The name didn’t match.');
      return;
    }
    try {
      await personalAgentsApi.reset(agentId, typed);
      chatSessionId = '';
      memoryReload += 1;
      void personalAgents.loadSchedules(agentId);
      void personalAgents.loadRuns(agentId);
      toasts.success(`Reset ${name}`);
    } catch (e) {
      toasts.error(`Couldn’t reset ${name}`, loadErrorText(e));
    }
  }

  /** Stop a running run — asks first, naming the schedule or the start time. */
  async function stopRun(r: PersonalAgentRun): Promise<void> {
    const what = scheduleName(r);
    const when = new Date(r.started_at).toLocaleString();
    const ok = await confirmer.ask(
      `Stop the ${what} run that started ${when}? Its agent session is stopped and nothing is delivered. The agent’s schedules keep running.`,
      { title: `Stop ${what}?`, danger: true, confirmLabel: 'Stop run' },
    );
    if (!ok) return;
    try {
      await personalAgentsApi.cancelRun(r.id);
      toasts.info('Stopping the run…');
    } catch (e) {
      toasts.error('Couldn’t stop the run', loadErrorText(e));
    }
    void personalAgents.loadRuns(agentId);
  }

  // --- Chat: one interactive session pinned to the agent's provider/model/cwd.
  let chatSessionId = $state('');
  let chatError = $state('');
  // Bumped by Retry: clearing the (already empty) session id is no change, so
  // the effect never re-ran and Retry left "Opening…" up forever.
  let chatAttempt = $state(0);
  // The get-or-create in flight, if any. Chat → Runs → Chat during the first
  // (multi-second) create used to fire a SECOND create — two Claude sessions,
  // one orphaned. The server serialises per agent too; this avoids the request.
  let chatPending: Promise<void> | null = null;
  $effect(() => {
    void chatAttempt;
    if (tab !== 'chat' || chatSessionId) return;
    const id = agentId;
    if (chatPending) return;
    chatError = '';
    announce(`Opening ${untrack(() => agent?.name) ?? 'the agent'}’s chat session…`);
    chatPending = personalAgentsApi
      .chatSession(id)
      .then((r) => {
        if (id === agentId) chatSessionId = r.session_id;
      })
      .catch((e) => {
        if (id === agentId) chatError = loadErrorText(e);
      })
      .finally(() => {
        chatPending = null;
      });
  });

  // --- Schedules form -------------------------------------------------------
  let schedFormOpen = $state(false);
  let schedEditId = $state<string | null>(null);
  let sf = $state<CadenceForm>(defaultCadence());
  let sfTimezone = $state(browserTz());
  let sfDirective = $state('');
  let sfEnabled = $state(true);
  let sfPermission = $state<PersonalAgentPermission>('directed');

  function openSchedCreate(): void {
    schedEditId = null;
    sf = defaultCadence();
    sfTimezone = browserTz();
    sfDirective = '';
    sfEnabled = true;
    sfPermission = 'directed';
    schedFormOpen = true;
    error = '';
  }

  function openSchedEdit(s: PersonalAgentSchedule): void {
    schedEditId = s.id;
    sf = loadCadence(s.schedule, s.timezone || browserTz());
    sfTimezone = s.timezone || browserTz();
    sfDirective = s.directive;
    sfEnabled = s.enabled;
    sfPermission = s.permission ?? 'directed';
    schedFormOpen = true;
    error = '';
  }

  async function saveSchedule(): Promise<void> {
    if (sf.cadence === 'once' && !sf.runAt) {
      error = 'Pick the date and time it runs.';
      return;
    }
    busy = true;
    error = '';
    const body = {
      schedule: buildCadence(sf),
      timezone: sfTimezone.trim() || 'UTC',
      directive: sfDirective,
      enabled: sfEnabled,
      permission: sfPermission,
    };
    try {
      if (schedEditId) await personalAgents.updateSchedule(agentId, schedEditId, body);
      else await personalAgents.createSchedule(agentId, body);
      schedFormOpen = false;
    } catch (e) {
      // Stays inline next to the form: it's usually a field problem (cron, tz).
      error = `Couldn’t save the schedule. ${loadErrorText(e)}`;
    } finally {
      busy = false;
    }
  }

  async function toggleSchedule(s: PersonalAgentSchedule): Promise<void> {
    try {
      await personalAgents.updateSchedule(agentId, s.id, { enabled: !s.enabled });
    } catch (e) {
      toasts.error(s.enabled ? 'Couldn’t pause the schedule' : 'Couldn’t enable the schedule', loadErrorText(e));
    }
  }

  async function deleteSchedule(s: PersonalAgentSchedule): Promise<void> {
    const what = cadenceLabel(s.schedule, s.timezone);
    if (!(await confirmer.ask(`Delete the schedule “${what}”? Past runs stay in Runs.`, { title: 'Delete schedule', danger: true, confirmLabel: 'Delete' }))) return;
    try {
      await personalAgents.deleteSchedule(agentId, s.id);
    } catch (e) {
      toasts.error('Couldn’t delete the schedule', loadErrorText(e));
    }
  }

  async function runNow(scheduleId?: string): Promise<void> {
    // Outward-facing: the run delivers its report the moment it finishes.
    if (agent && !(await confirmRunAndDeliver(agent.name, agent.delivery))) return;
    busy = true;
    try {
      await personalAgents.runNow(agentId, scheduleId);
      goTab('runs');
    } catch (e) {
      toasts.error(`Couldn’t run ${agent?.name ?? 'the agent'}`, loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  async function toggleAgent(): Promise<void> {
    if (!agent) return;
    const was = agent.enabled;
    busy = true;
    try {
      await personalAgents.setEnabled(agentId, !was);
    } catch (e) {
      toasts.error(was ? `Couldn’t pause ${agent.name}` : `Couldn’t enable ${agent.name}`, loadErrorText(e));
    } finally {
      busy = false;
    }
  }

  async function removeAgent(): Promise<void> {
    if (!agent) return;
    const name = agent.name;
    if (!(await confirmer.ask(`Delete personal agent “${name}”? Its schedules, memory and run history go with it.`, { title: 'Delete personal agent', danger: true, confirmLabel: 'Delete' }))) return;
    try {
      await personalAgents.remove(agentId);
      toasts.success(`Deleted ${name}`);
      router.go('personal-agents');
    } catch (e) {
      toasts.error(`Couldn’t delete ${name}`, loadErrorText(e));
    }
  }

  // --- Runs / report viewer -------------------------------------------------
  let reportRun = $state<PersonalAgentRun | null>(null);
  let reportText = $state('');
  let reportError = $state('');
  let reportLoading = $state(false);

  const reportSeq = latestOnly();
  async function viewReport(run: PersonalAgentRun): Promise<void> {
    reportRun = run;
    reportLoading = true;
    reportText = '';
    reportError = '';
    // Only the report last asked for may land (A's slow text used to fill
    // the modal opened for B).
    const t = reportSeq.begin();
    try {
      const text = await authedText(personalAgentsApi.reportPath(run.id));
      if (t.current) reportText = text;
    } catch (e) {
      if (t.current) reportError = loadErrorText(e);
    } finally {
      if (t.current) reportLoading = false;
    }
  }

  function duration(r: PersonalAgentRun): string {
    if (!r.finished_at) return '…';
    const ms = Date.parse(r.finished_at) - Date.parse(r.started_at);
    if (!Number.isFinite(ms) || ms < 0) return '';
    if (ms < 60_000) return `${Math.round(ms / 1000)}s`;
    return `${Math.round(ms / 60_000)}m`;
  }

  function scheduleName(r: PersonalAgentRun): string {
    if (r.mode === 'proactive') {
      const g = autonomy?.goals.find((x) => x.id === r.goal_id);
      return g ? `Goal · ${g.text}` : 'Standing goal';
    }
    if (!r.schedule_id) return 'Manual';
    const s = schedules.find((x) => x.id === r.schedule_id);
    return s ? cadenceLabel(s.schedule, s.timezone) : 'Deleted schedule';
  }

  /** Where each run's report goes, in words (outward destinations named). */
  function deliveryLabel(): string {
    const d = agent?.delivery ?? {};
    const str = (k: string): string => (typeof d[k] === 'string' ? (d[k] as string) : '');
    switch (d.type) {
      case 'slack': return str('chat_id') ? `Slack · ${str('chat_id')}` : 'Slack · the integration’s channel';
      case 'telegram': return str('chat_id') ? `Telegram · ${str('chat_id')}` : 'Telegram · the integration’s chat';
      case 'email': return `Email · ${str('to') || 'no address set'}`;
      case 'webhook': return `Webhook · ${str('url') || 'no URL set'}`;
      default: return 'None — reports stay on this page';
    }
  }
  const runsLoaded = $derived(personalAgents.runsByAgent[agentId] !== undefined);
  /** A failed runs load is shown inline with Retry — never as "No runs yet". */
  const runsErr = $derived(personalAgents.runsError[agentId] ?? null);

  const canEditDocuments = $derived(auth.can('scheduled_tasks', 'edit') &&
    (auth.isRoot || ['editor', 'admin'].includes(ws.workspaces.find((w) => w.id === agent?.workspace_id)?.my_role ?? 'viewer')));
  /** The agent runs AS its owner, so what it runs — memory and context
   *  documents, memory items, standing-goal texts and rules — is the owner's
   *  (or a ws admin's / root's) call; the server refuses everyone else 403
   *  (PC-3, access-control-4-04). An ownerless preset is adopted on the first
   *  write. Toggles, budget, primary and "Run now" stay `canEditDocuments`. */
  const myRole = $derived(ws.workspaces.find((w) => w.id === agent?.workspace_id)?.my_role ?? 'viewer');
  const canEditOwnerFields = $derived(canEditDocuments && (auth.isRoot || myRole === 'admin' ||
    !agent?.created_by || agent.created_by === auth.me?.id));
  const ownerNote = $derived(canEditDocuments && !canEditOwnerFields
    ? 'Runs as its owner; only the owner or a workspace admin can change this.'
    : '');
;
async () => {

 { svelteHTML.createElement("div", { "class":`agent-page`,});
 { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":agent ? agent.name : 'Personal agent',"tabsPlacement":`below`,leading:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"title":`Back to Personal Agents`,"aria-label":`Back to Personal Agents`,"onclick":() => router.go('personal-agents'),});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});}
     }
    if(agent){ { const $$_ratavAtnegA0C = __sveltets_2_ensureComponent(AgentAvatar); new $$_ratavAtnegA0C({ target: __sveltets_2_any(), props: {      "avatar":agent.avatar,"name":agent.name,"size":24,}});}}
  };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
    if(agent){
       { svelteHTML.createElement("span", {   "class":`chip prov`,"title":agent.model ? `${agent.provider} · ${agent.model}` : agent.provider,});
         { const $$_nocIredivorP1C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP1C({ target: __sveltets_2_any(), props: {    "provider":agent.provider,"size":12,}});}agent.provider;agent.model ? ` · ${agent.model}` : '';
       }
      if(!agent.enabled){ { svelteHTML.createElement("span", {   "class":`chip`,"title":`Schedules don’t fire while paused. Run now and chat still work.`,});  }}
      if(agent.browser){ { svelteHTML.createElement("span", {   "class":`chip`,"title":`Runs and chat can use the Otto browser tool`,});  }}
      if(autonomy?.primary){ { svelteHTML.createElement("span", {   "class":`chip pa-accent`,"title":`Your primary assistant — routes specialist work to your other agents`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`star`,"size":12,}});}   }}
      if(autonomy?.proactive.enabled){ { svelteHTML.createElement("span", {   "class":`chip`,"title":`Works its standing goals in the background, read-only`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":12,}});}  }}
    }
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
    if(agent){
      if(agent.enabled){
         { svelteHTML.createElement("button", {         "class":`btn small primary`,...__sveltets_2_empty({"data-icon":`play`}),"disabled":busy,"aria-label":`Run now ${agent.name}`,"onclick":() => runNow(),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}   }
         { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-icon":`clock`}),"disabled":busy,"aria-label":`Pause ${agent.name}`,"onclick":toggleAgent,"title":`Stop its schedules from firing`,});  }
      }else{
         { svelteHTML.createElement("button", {         "class":`btn small primary`,...__sveltets_2_empty({"data-icon":`check`}),"disabled":busy,"aria-label":`Enable ${agent.name}`,"onclick":toggleAgent,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  }
         { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-icon":`play`}),"disabled":busy,"aria-label":`Run once ${agent.name}`,"onclick":() => runNow(),"title":`Run it once without enabling its schedules`,});  }
      }
       { svelteHTML.createElement("button", {     "class":`btn small`,...__sveltets_2_empty({"data-icon":`edit`}),"onclick":() => (editing = true),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}  }
       { svelteHTML.createElement("button", {             "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-1`}),...__sveltets_2_empty({"data-icon":`undo`}),...__sveltets_2_empty({"data-label":`Reset agent…`}),"onclick":resetAgent,"aria-label":`Reset agent`,"title":`Reset agent — forget memory, chat, schedules and runs`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`undo`,"size":14,}});}
       }
       { svelteHTML.createElement("button", {             "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-2`}),...__sveltets_2_empty({"data-icon":`trash`}),...__sveltets_2_empty({"data-label":`Delete agent…`}),"onclick":removeAgent,"aria-label":`Delete agent`,"title":`Delete agent`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
       }
    }
  };return __sveltets_2_any(0)},tabs:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("label", { "class":`tab-picker`,});
       { svelteHTML.createElement("span", { "class":`tab-picker-label`,});  }
       { svelteHTML.createElement("select", {       "class":`input`,"aria-label":`Agent sections`,"value":tab,"onchange":(e) => void goTab((e.currentTarget as HTMLSelectElement).value as Tab),});
           for(let t of __sveltets_2_ensureArray(TABS)){t.id; { svelteHTML.createElement("option", { "value":t.id,});t.label; }}
       }
     }
     { svelteHTML.createElement("div", {         "class":`tabs`,"role":`tablist`,"aria-label":`Agent sections`,"tabindex":-1,"onkeydown":tabKeys({ activate: 'click' }),});
         for(let t of __sveltets_2_ensureArray(TABS)){t.id;
         { svelteHTML.createElement("button", {              "class":`tab`,"role":`tab`,...__sveltets_2_empty({"data-tab":t.id}),"aria-selected":tab === t.id,"tabindex":tab === t.id ? 0 : -1,"onclick":(event) => void goTab(t.id, event),});tab === t.id;t.label; }
      }
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, badge, actions, tabs} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
  
  
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"width":`readable`,"fill":tab === 'chat',}});
 { svelteHTML.createElement("div", {  "class":`agent-body`,});tab === 'chat';
  if(!agent){
    if(personalAgents.loadingAgents){
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {      "rows":3,"height":64,"label":`the agent`,}});}
    } else if (personalAgents.agentsError){
      
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "what":`this agent`,"variant":`page`,"error":personalAgents.agentsError,"empty":true,"onretry":() => void retryAgents(ws.currentId),}});}
    }else{
       { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {             "variant":`page`,"icon":`user`,"title":`Agent not found`,"body":`It may have been deleted, or it belongs to another workspace.`,"actionLabel":`Back to Personal Agents`,"onaction":() => router.go('personal-agents'),}});}
    }
  } else if (tab === 'overview'){
     { svelteHTML.createElement("div", { "class":`overview`,});
       { svelteHTML.createElement("section", { "class":`pa-panel`,});
         { svelteHTML.createElement("div", { "class":`card-head`,});
           { svelteHTML.createElement("h2", {});  }
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => (editing = true),});  }
         }
        if(agent.soul_md.trim()){
           { svelteHTML.createElement("pre", { "class":`soul`,});agent.soul_md; }
        }else{
           { svelteHTML.createElement("p", { "class":`muted`,});               }
        }
       }
       { svelteHTML.createElement("section", { "class":`pa-panel`,});
         { svelteHTML.createElement("h2", {});  }
         { svelteHTML.createElement("dl", { "class":`props`,});
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { "class":`prov-dd`,}); { const $$_nocIredivorP7C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP7C({ target: __sveltets_2_any(), props: {    "provider":agent.provider,"size":12,}});} { svelteHTML.createElement("span", { "class":`mono`,});agent.provider; } }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", { });!!agent.model;agent.model || 'Provider default'; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {   "class":`clip`,"title":deliveryLabel(),});deliveryLabel(); }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});agent.browser ? 'On' : 'Off'; }
           { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});agent.enabled ? 'Enabled' : 'Paused — schedules don’t fire'; }
           { svelteHTML.createElement("dt", {});  }
          if(agent.cwd){
             { svelteHTML.createElement("dd", {     "class":`mono path`,"dir":`ltr`,"title":agent.cwd,});agent.cwd; }
          }else{
            
             { svelteHTML.createElement("dd", { "title":`…/personal/${agent.id}/`,});     }
          }
           { svelteHTML.createElement("dt", {});  }
           { svelteHTML.createElement("dd", {});
            schedules.length === 0 ? 'None' : schedules.length;
            if(schedules.length > 0){ if(agent.enabled){   { const $$_emiTleR7C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR7C({ target: __sveltets_2_any(), props: {    "iso":personalAgents.nextRunAt(agent.id),"fallback":`not scheduled`,}});}}else{ }
            }
             { svelteHTML.createElement("button", {   "class":`link`,"onclick":() => goTab('schedules'),});  }
           }
         }
       }
     }
  } else if (tab === 'activity'){
    agentId; { { const $$_ytivitcAtnegA3C = __sveltets_2_ensureComponent(AgentActivity); new $$_ytivitcAtnegA3C({ target: __sveltets_2_any(), props: { agentId,}});}}
  } else if (tab === 'autonomy'){
    agentId; { { const $$_ymonotuAtnegA3C = __sveltets_2_ensureComponent(AgentAutonomy); new $$_ymonotuAtnegA3C({ target: __sveltets_2_any(), props: {        agentId,"editable":canEditDocuments,"ownerEditable":canEditOwnerFields,ownerNote,"onsaved":(a) => (autonomy = a),}});}}
  } else if (tab === 'schedules'){
    if(schedules.length > 0 || schedFormOpen){
       { svelteHTML.createElement("div", { "class":`section-head`,});
         { svelteHTML.createElement("p", { "class":`hint`,});         if(!agent.enabled){' They don’t fire while the agent is paused.';} }
        if(!schedFormOpen){
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":openSchedCreate,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
        }
       }
    }
    if(schedFormOpen){
       { svelteHTML.createElement("div", { "class":`form pa-panel`,});
         { svelteHTML.createElement("h2", {});schedEditId ? 'Edit schedule' : 'New schedule'; }
        if(error){ {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: error, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`err`,});error; }}}
         { svelteHTML.createElement("div", { "class":`field-row`,});
           { svelteHTML.createElement("div", { "class":`field`,});
             { svelteHTML.createElement("label", { "for":`${uid}-cadence`,});  }
             { svelteHTML.createElement("select", {     "id":`${uid}-cadence`,"class":`input`,"bind:value":sf.cadence,});/*Ωignore_startΩ*/() => sf.cadence = __sveltets_2_any(null);/*Ωignore_endΩ*/
               { svelteHTML.createElement("option", { "value":`interval`,});  }
               { svelteHTML.createElement("option", { "value":`daily`,});  }
               { svelteHTML.createElement("option", { "value":`weekly`,});  }
               { svelteHTML.createElement("option", { "value":`cron`,});  }
               { svelteHTML.createElement("option", { "value":`once`,});  }
             }
           }
          if(sf.cadence === 'interval'){
             { svelteHTML.createElement("div", { "class":`field`,});
               { svelteHTML.createElement("label", { "for":`${uid}-every`,});    }
               { svelteHTML.createElement("input", {          "id":`${uid}-every`,"class":`input`,"type":`number`,"min":`5`,"bind:value":sf.everyMin,});/*Ωignore_startΩ*/() => sf.everyMin = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
          } else if (sf.cadence === 'once'){
             { svelteHTML.createElement("div", { "class":`field`,});
               { svelteHTML.createElement("label", { "for":`${uid}-run-at`,});      }
               { svelteHTML.createElement("input", {        "id":`${uid}-run-at`,"class":`input`,"type":`datetime-local`,"bind:value":sf.runAt,});/*Ωignore_startΩ*/() => sf.runAt = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
          } else if (sf.cadence === 'cron'){
             { svelteHTML.createElement("div", { "class":`field`,});
               { svelteHTML.createElement("label", { "for":`${uid}-cron`,});    }
               { svelteHTML.createElement("input", {          "id":`${uid}-cron`,"class":`input`,"bind:value":sf.cronExpr,"placeholder":`0 9 * * 1`,"dir":`ltr`,});/*Ωignore_startΩ*/() => sf.cronExpr = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
          }else{
             { svelteHTML.createElement("div", { "class":`field`,});
               { svelteHTML.createElement("label", { "for":`${uid}-at`,});  }
               { svelteHTML.createElement("input", {          "id":`${uid}-at`,"class":`input`,"bind:value":sf.at,"placeholder":`09:00`,"dir":`ltr`,});/*Ωignore_startΩ*/() => sf.at = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
            if(sf.cadence === 'weekly'){
               { svelteHTML.createElement("div", { "class":`field`,});
                 { svelteHTML.createElement("label", { "for":`${uid}-weekday`,});  }
                 { svelteHTML.createElement("select", {     "id":`${uid}-weekday`,"class":`input`,"bind:value":sf.weekday,});/*Ωignore_startΩ*/() => sf.weekday = __sveltets_2_any(null);/*Ωignore_endΩ*/
                      for(let d of __sveltets_2_ensureArray(WEEKDAYS)){let i = 1;d; { svelteHTML.createElement("option", { "value":i,});d; }}
                 }
               }
            }
          }
          if(sf.cadence !== 'interval'){
             { svelteHTML.createElement("div", { "class":`field`,});
               { svelteHTML.createElement("label", { "for":`${uid}-tz`,});  }
               { svelteHTML.createElement("input", {          "id":`${uid}-tz`,"class":`input`,"bind:value":sfTimezone,"placeholder":`e.g. Europe/London`,"dir":`ltr`,});/*Ωignore_startΩ*/() => sfTimezone = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             }
          }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`${uid}-directive`,});     }
           { svelteHTML.createElement("textarea", {           "dir":`auto`,"id":`${uid}-directive`,"class":`input`,"bind:value":sfDirective,"rows":4,"placeholder":`Produce the daily recap…`,});/*Ωignore_startΩ*/() => sfDirective = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         }
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`${uid}-permission`,});    }
           { svelteHTML.createElement("select", {     "id":`${uid}-permission`,"class":`input`,"bind:value":sfPermission,});/*Ωignore_startΩ*/() => sfPermission = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", { "value":`directed`,});        }
             { svelteHTML.createElement("option", { "value":`read_only`,});         }
           }
         }
         { svelteHTML.createElement("label", { "class":`chk`,}); { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":sfEnabled,});/*Ωignore_startΩ*/() => sfEnabled = __sveltets_2_any(null);/*Ωignore_endΩ*/}  }
         { svelteHTML.createElement("div", { "class":`actions`,});
           { svelteHTML.createElement("button", {     "class":`btn`,"disabled":busy,"onclick":() => { schedFormOpen = false; error = ''; },});  }
           { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy,"onclick":saveSchedule,});busy ? 'Saving…' : schedEditId ? 'Save schedule' : 'Add schedule'; }
         }
       }
    }
     { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {            "what":`this agent’s schedules`,"variant":`compact`,"loading":personalAgents.schedulesLoading[agentId] ?? false,"error":personalAgents.schedulesError[agentId] ?? null,"empty":schedules.length === 0,"onretry":() => void personalAgents.loadSchedules(agentId),}});}
    if(schedules.length === 0 && !schedFormOpen && !personalAgents.schedulesLoading[agentId] && !personalAgents.schedulesError[agentId]){
       { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {             "icon":`calendar`,"title":`No schedules`,"body":`This agent only runs when you press Run now, or when you chat with it. Add a schedule to run it on a cadence.`,"actionLabel":`Add schedule`,"actionIcon":`plus`,"onaction":openSchedCreate,}});}
    }else{
       { svelteHTML.createElement("ul", { "class":`rows`,});
           for(let s of __sveltets_2_ensureArray(schedules)){s.id;
           { svelteHTML.createElement("li", { "class":`rowline`,});
             { svelteHTML.createElement("div", { "class":`rowmain`,});
               { svelteHTML.createElement("div", { "class":`rowtitle`,});
                 { svelteHTML.createElement("strong", {});cadenceLabel(s.schedule, s.timezone); }
                if(!s.enabled){ { svelteHTML.createElement("span", { "class":`chip`,});  }}
                if(s.permission === 'read_only'){ { svelteHTML.createElement("span", {   "class":`chip`,"title":`Its runs can read and report, but can’t send, post, write or change anything`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});}  }}
               }
              
               { svelteHTML.createElement("span", { "class":`meta`,});if(s.enabled && agent.enabled){  { const $$_emiTleR7C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR7C({ target: __sveltets_2_any(), props: {  "iso":s.next_run_at,}});} }  { const $$_emiTleR7C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR7C({ target: __sveltets_2_any(), props: {    "iso":s.last_run_at,"fallback":`never`,}});} }
               { svelteHTML.createElement("p", { "class":`directive`,});s.directive || 'No directive — runs on the persona alone.'; }
             }
             { svelteHTML.createElement("div", { "class":`rowactions`,});
               { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":busy,"aria-label":`Run now ${cadenceLabel(s.schedule, s.timezone)}`,"onclick":() => runNow(s.id),});  }
               { svelteHTML.createElement("button", {     "class":`btn small`,"aria-label":`${s.enabled ? 'Pause' : 'Enable'} ${cadenceLabel(s.schedule, s.timezone)}`,"onclick":() => toggleSchedule(s),});s.enabled ? 'Pause' : 'Enable'; }
               { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => openSchedEdit(s),});  }
               { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => deleteSchedule(s),"aria-label":`Delete schedule ${cadenceLabel(s.schedule, s.timezone)}`,"title":`Delete schedule`,});
                 { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});}
               }
             }
           }
        }
       }
    }
  } else if (tab === 'runs'){
     { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL3 = new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`this agent’s runs`,"loading":!runsLoaded,"error":runsErr,"empty":runs.length === 0,"rows":4,"onretry":() => void personalAgents.loadRuns(agentId),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
         { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {       "icon":`play`,"title":`No runs yet`,"body":agent.enabled
            ? 'Runs appear here with their report and delivery status — from a schedule firing, or from Run once above.'
            : 'Runs appear here with their report and delivery status. Enable the agent to let its schedules fire, or use Run once above.',}});}
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL3.$$prop_def;/*Ωignore_endΩ*/
      
       { svelteHTML.createElement("ul", { "class":`rows`,});
           for(let r of __sveltets_2_ensureArray(runs)){r.id;
           { svelteHTML.createElement("li", { "class":`run`,});
             { const $$_egdaBsutatS6C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS6C({ target: __sveltets_2_any(), props: {    "status":runStatus(r.status),"variant":`text`,}});}
             { svelteHTML.createElement("span", { "class":`run-when`,}); { const $$_emiTleR7C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR7C({ target: __sveltets_2_any(), props: {  "iso":r.started_at,}});} }
             { svelteHTML.createElement("span", {   "class":`chip clip-chip`,"title":`What started this run`,});scheduleName(r); }
            if(r.read_only){ { svelteHTML.createElement("span", {   "class":`chip`,"title":`This run could read and report, but not change anything`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});}  }}
            if(duration(r)){ { svelteHTML.createElement("span", { "class":`meta`,});duration(r); }}
             { svelteHTML.createElement("span", {   "class":`run-sum`,"title":r.summary || r.error || undefined,});r.summary || r.error || 'No summary'; }
            if((r.attempts ?? 1) > 1){ { svelteHTML.createElement("span", { "class":`chip pa-warn`,});r.attempts;  }}
            if(r.delivered){ { svelteHTML.createElement("span", { "class":`chip ok`,});  }}
            if(r.skipped_delivery){ { svelteHTML.createElement("span", {   "class":`chip`,"title":`The report didn’t change since the last run, so nothing was sent`,});  }}
            if(r.delivery_error){ { svelteHTML.createElement("span", {   "class":`chip bad`,"title":r.delivery_error,});  }}
            if(r.report_rel){
               { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => viewReport(r),});  }
            }
            if(r.session_id){
               { svelteHTML.createElement("button", {     "class":`btn small`,"title":`Open the agent session this run drove`,"onclick":() => ws.navigateToSession(r.session_id ?? ''),});  }
            }
            if(r.status === 'running'){
               { svelteHTML.createElement("button", {       "class":`btn small danger`,"title":`Stop this run (its agent session is stopped)`,"aria-label":`Stop ${scheduleName(r)}`,"onclick":() => void stopRun(r),});  }
            }
            
            if(r.status === 'error' && r.error && r.summary){
               { svelteHTML.createElement("p", {   "class":`run-err`,"role":`note`,});r.error; }
            }
            if(r.delivery_error){
               { svelteHTML.createElement("p", {   "class":`run-err warn`,"role":`note`,});  r.delivery_error; }
            }
           }
        }
       }
     LoadState}
  } else if (tab === 'chat'){
    if(chatError){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`the chat session`,"error":chatError,"empty":true,"onretry":() => { chatError = ''; chatSessionId = ''; chatAttempt += 1; },}});}
    } else if (chatSessionId){
       { svelteHTML.createElement("div", { "class":`chatwrap`,});
          { const $$_lanimreTyzaL4C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL4C({ target: __sveltets_2_any(), props: {  "sessionId":chatSessionId,"autoFocus":true,}});}
       }
    }else{
       { svelteHTML.createElement("p", { "class":`muted`,}); agent.name;   }
    }
  } else if ((tab === 'memory' || tab === 'context') && agent){
    if(tab === 'memory'){
      if(AgentMemoryInspectorLazy.error){
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":`this agent’s memory`,"error":AgentMemoryInspectorLazy.error,"onretry":() => AgentMemoryInspectorLazy.retry(),}});}
      } else if (AgentMemoryInspectorLazy.component){
         { const $$_tnenopmoc_yzaLrotcepsnIyromeMtnegA3C = __sveltets_2_ensureComponent(AgentMemoryInspectorLazy.component); new $$_tnenopmoc_yzaLrotcepsnIyromeMtnegA3C({ target: __sveltets_2_any(), props: {     agentId,"editable":canEditOwnerFields,"reloadKey":memoryReload,}});}
      }else{
          { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {  "what":`this agent’s memory`,"loading":true,}});}
      }
    }
    `${agentId}:${tab}:${memoryReload}`; {
      if(AgentDocumentsLazy.error){
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {      "what":tab === 'memory' ? 'this agent’s document' : 'this agent’s context',"error":AgentDocumentsLazy.error,"onretry":() => AgentDocumentsLazy.retry(),}});}
      } else if (AgentDocumentsLazy.component){
         { const $$_tnenopmoc_yzaLstnemucoDtnegA3C = __sveltets_2_ensureComponent(AgentDocumentsLazy.component); new $$_tnenopmoc_yzaLstnemucoDtnegA3C({ target: __sveltets_2_any(), props: {         agentId,"workspaceId":agent.workspace_id,"kind":tab === 'context' ? 'context' : 'memory',"editable":canEditOwnerFields,"sharedWorkspace":!!agent.cwd.trim(),}});}
      }else{
          { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {  "what":tab === 'memory' ? 'this agent’s document' : 'this agent’s context',"loading":true,}});}
      }
    }
  }

  if(editing && agent){
     { const $$_teehStidEtnegA3C = __sveltets_2_ensureComponent(AgentEditSheet); new $$_teehStidEtnegA3C({ target: __sveltets_2_any(), props: {   agent,"onclose":() => (editing = false),}});}
  }

  if(reportRun){
    
     { const $$_ladoM3C = __sveltets_2_ensureComponent(Modal); new $$_ladoM3C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Report`,"width":760,"onclose":() => (reportRun = null),}});
      if(reportLoading){
         { const $$_notelekS4C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS4C({ target: __sveltets_2_any(), props: {      "rows":6,"height":18,"label":`the report`,}});}
      } else if (reportError){
         { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {      "what":`the report`,"error":reportError,"empty":true,"onretry":() => reportRun && viewReport(reportRun),}});}
      }else{
         { svelteHTML.createElement("pre", { "class":`report`,});reportText; }
      }
     Modal}
  }
 }
 PageBody}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentPage__SvelteComponent_ = ReturnType<typeof AgentPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentPage__SvelteComponent_;