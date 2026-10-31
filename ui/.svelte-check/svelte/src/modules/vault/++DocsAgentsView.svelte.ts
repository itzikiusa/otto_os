///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import PathField from '../../lib/components/PathField.svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
// Docs agents — fan 1-4 writer agents out over a prompt to author notes into
// the vault (a summarizer consolidates drafts when >1 writer), plus the
// vault's RUN HISTORY (docs runs + per-note refine turns, server-persisted
// in `vault_docs_runs`). Center-stage view: a compact form, the selected
// run's per-agent rows with live status + inline terminals, and the runs
// list below. The selected run lives on the vault store; the LIST is
// refetched from the server on every mount — that is what makes runs
// reappear after a tab/module switch or a full app restart. This view owns
// an event-fed refresh (1.5 s poll while the event socket is down) and stops
// it once nothing is active.
import { onMount } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import Switch from '../../lib/components/Switch.svelte';
import Modal from '../../lib/components/Modal.svelte';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import { api } from '../../lib/api/client';
import { contextApi } from '../../lib/api/context';
import type {
    VaultDocsFindingEvidence,
    VaultDocsReviewSkill,
    VaultDocsRun,
  } from '../../lib/api/types';
import {
    cancelDocsRun,
    docsRun as getDocsRun,
    deleteDocsRun,
    resolveDocsRun,
    retryDocsAgent,
    retryDocsReviewer,
    retryDocsRevision,
    retryDocsSummarizer,
    runDocsAgents,
  } from '../../lib/api/vault';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { kindLabel, runStateLabel, severityLabel } from '../../lib/labels';
import Badge from '../../lib/components/Badge.svelte';
import type { BadgeTone } from '../../lib/status';
import { DOCS_TEMPLATES } from './docsTemplates';
import { vault } from './vault.svelte';
import type { Poller } from '../../lib/poll';
import { liveQuery } from '../../lib/live';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  interface AgentRow {
    provider: string;
    model: string;
  }

  interface ReviewerRow extends AgentRow {
    skill: VaultDocsReviewSkill;
    focus: string;
  }

  const REVIEW_METHODS: { value: VaultDocsReviewSkill; label: string }[] = [
    { value: 'vault-docs-review', label: 'Generic — complete bundle' },
    { value: 'vault-api-review', label: 'API contracts and flows' },
    { value: 'vault-data-review', label: 'Datastores and impact' },
    { value: 'vault-runtime-review', label: 'Runtime, workers and messaging' },
    { value: 'vault-evidence-review', label: 'Evidence and coverage' },
  ];

  const newReviewer = (): ReviewerRow => ({
    provider: defaultAgentProvider(),
    model: '',
    skill: 'vault-docs-review',
    focus: '',
  });

  // -- form ---------------------------------------------------------------------
  let prompt = $state('');
  let targetDir = $state('');
  let agents = $state<AgentRow[]>([{ provider: defaultAgentProvider(), model: '' }]);
  let sumProvider = $state(defaultAgentProvider());
  let reviewEnabled = $state(false);
  let reviewers = $state<ReviewerRow[]>([newReviewer()]);
  let maxReviewIterations = $state(3);
  let starting = $state(false);

  // Prepared prompts — pick a template, fill the repo path, Insert. The
  // template's skills ride the run (RunReq.skills) and show as chips below.
  let tplId = $state('');
  let tplRepo = $state('');
  // Infra/library repo: no HTTP API of its own, flows are partial — the
  // template pivots its scope to packages + exported interfaces + consumers.
  let tplInfra = $state(false);
  let tplSkills = $state<string[]>([]);
  const tpl = $derived(DOCS_TEMPLATES.find((t) => t.id === tplId) ?? null);

  function applyTemplate(): void {
    if (!tpl) return;
    prompt = tpl.build(tplRepo.trim(), tplInfra);
    tplSkills = [...tpl.skills];
  }

  /** Everything the run will inject (template skills + okf on OKF vaults). */
  const runSkills = $derived([
    ...tplSkills,
    ...(vault.current?.okf && !tplSkills.includes('okf-authoring') ? ['okf-authoring'] : []),
  ]);

  // Skill viewer — chips are clickable so what gets injected is inspectable.
  let skillView = $state<{ name: string; body: string } | null>(null);
  async function viewSkill(name: string): Promise<void> {
    try {
      const s = await contextApi.getSkill(name);
      skillView = { name, body: s.body };
    } catch (e) {
      toastError('Couldn’t open the skill', e);
    }
  }

  const providers = $derived(agentProviders());
  const run = $derived(vault.docsRun);
  const isActive = (r: VaultDocsRun) =>
    r.state === 'running' ||
    r.state === 'summarizing' ||
    r.state === 'reviewing' ||
    r.state === 'revising';
  const active = $derived(run != null && isActive(run));
  /** Anything (selected or listed) still moving → the poll keeps ticking. */
  const anyActive = $derived((run != null && isActive(run)) || vault.docsRuns.some(isActive));

  // Prefill (and re-prefill on "Docs agent here" from a folder's context menu).
  $effect(() => {
    targetDir = vault.docsAgentsDir;
  });

  // One-shot prompt/skills prefill ("Review + fix docs", "Send to agent…",
  // group actions, "Send to agent to fix" on a findings run). Consuming it
  // also deselects any run so the form is what the user lands on. `autorun`
  // (group-bar actions) launches immediately — the bar's button IS the send.
  $effect(() => {
    const p = vault.docsAgentsPrefill;
    if (!p) return;
    vault.docsAgentsPrefill = null;
    prompt = p.prompt;
    tplSkills = p.skills;
    targetDir = vault.docsAgentsDir;
    if (p.agents?.length) {
      agents = p.agents.map((a) => ({ provider: a.provider, model: a.model ?? '' }));
    }
    vault.docsRun = null;
    if (p.autorun) void start();
  });

  function addAgent(): void {
    if (agents.length < 4) agents = [...agents, { provider: defaultAgentProvider(), model: '' }];
  }

  function removeAgent(i: number): void {
    if (agents.length > 1) agents = agents.filter((_, x) => x !== i);
  }

  function addReviewer(): void {
    if (reviewers.length < 4) reviewers = [...reviewers, newReviewer()];
  }

  function removeReviewer(i: number): void {
    if (reviewers.length > 1) reviewers = reviewers.filter((_, x) => x !== i);
  }

  function evidenceLabel(evidence: VaultDocsFindingEvidence): string {
    const source = evidence.repo_path || evidence.doc_path || 'Evidence';
    const location = evidence.line ? `:${evidence.line}` : evidence.section ? ` · ${evidence.section}` : '';
    return `${source}${location}`;
  }

  function reviewStateLabel(r: VaultDocsRun): string {
    if (r.review.state === 'clean') return 'Review complete';
    if (r.review.state === 'exhausted') return 'Review limit reached';
    if (r.review.state === 'error') return 'Review failed';
    if (r.review.state === 'cancelled') return 'Review canceled';
    if (r.review.state === 'interrupted') return 'Review interrupted';
    if (r.review.state === 'pending') return 'Review queued';
    return `Review round ${Math.max(r.review.current_iteration, 1)} of ${r.review.max_iterations}`;
  }

  /** `needs_review` → “Needs review”: raw enum values never reach the screen. */
  const displayState = (state: string): string => {
    const t = state.replaceAll('_', ' ');
    return t.charAt(0).toUpperCase() + t.slice(1);
  };

  function reviewIterationLimit(value: number): number {
    return Number.isFinite(value) ? Math.min(10, Math.max(1, Math.round(value))) : 3;
  }

  // -- run + polling ---------------------------------------------------------------
  // lib/poll: one request in flight at a time, paused while hidden.
  let poller: Poller | null = null;

  function stopPoll(): void {
    poller?.stop();
    poller = null;
  }

  function startPoll(): void {
    stopPoll();
    // A docs run advances as its agent sessions do: re-read on their status
    // events (coalesced), with a 10 s safety net for run-level transitions no
    // event names; the 1.5 s cadence only while the event socket is down.
    poller = liveQuery({
      run: () => poll(),
      on: ['session_status', 'session_created', 'session_removed'],
      fallbackMs: 1500,
      safetyMs: 10_000,
      debounceMs: 1000,
      maxWaitMs: 5000,
      minIntervalMs: 3000,
      immediate: false,
    });
  }

  // Status refresh failures were swallowed forever: the run then looked frozen.
  // One miss is transient (the next tick retries); three in a row are shown
  // inline until a refresh succeeds again.
  let pollFailures = 0;
  let pollError = $state('');

  async function poll(): Promise<void> {
    const r = vault.docsRun;
    try {
      if (r && isActive(r)) {
        const next = await getDocsRun(r.id);
        // Async guard: ignore if the user selected another run meanwhile.
        if (vault.docsRun?.id === next.id) vault.docsRun = next;
        if (!isActive(next)) {
          // The run wrote (or trashed drafts of) notes — reflect it everywhere.
          void vault.refreshTree();
          void vault.refreshStatus();
        }
      }
      // Keep the history list in step (it also carries refine turns that
      // complete server-side without this view's involvement).
      await vault.refreshDocsRuns();
      pollFailures = 0;
      pollError = '';
    } catch (e) {
      // Transient misses retry on the next tick; a streak is surfaced.
      if (++pollFailures >= 3) pollError = loadErrorText(e);
    }
    if (!anyActive) stopPoll();
  }

  async function start(): Promise<void> {
    if (!prompt.trim() || starting || !vault.current) return;
    const workspace = vault.wsId, id = vault.current.id, generation = vault.lookupGeneration;
    const current = () => vault.wsId === workspace && vault.current?.id === id && vault.lookupGeneration === generation;
    starting = true;
    try {
      const started = await runDocsAgents(workspace, id, {
        prompt: prompt.trim(),
        target_dir: targetDir.trim(),
        agents: agents.map((a) => ({ provider: a.provider, model: a.model.trim() || undefined })),
        summarizer: agents.length > 1 ? { provider: sumProvider } : undefined,
        skills: tplSkills.length ? tplSkills : undefined,
        review: reviewEnabled
          ? {
              max_iterations: reviewIterationLimit(maxReviewIterations),
              reviewers: reviewers.map((reviewer) => ({
                provider: reviewer.provider,
                model: reviewer.model.trim() || undefined,
                skill: reviewer.skill,
                focus: reviewer.focus.trim() || undefined,
              })),
            }
          : undefined,
      });
      if (!current()) return;
      vault.docsRun = started;
      openTerminals = new Set();
      void vault.refreshDocsRuns();
      startPoll();
    } catch (e) {
      toastError('Couldn’t start the docs agent', e);
    } finally {
      starting = false;
    }
  }

  // Per-slot retry (writers by index, 'sum' = summarizer) — kills the stuck
  // session server-side; a fresh one re-spawns with the same prompt.
  let retrying = $state<Record<string, boolean>>({});
  async function retry(target: number | 'sum'): Promise<void> {
    const r = vault.docsRun;
    if (!r || retrying[String(target)]) return;
    retrying = { ...retrying, [String(target)]: true };
    try {
      if (target === 'sum') await retryDocsSummarizer(r.id);
      else await retryDocsAgent(r.id, target);
      startPoll();
    } catch (e) {
      toastError('Couldn’t retry the run', e);
    } finally {
      retrying = { ...retrying, [String(target)]: false };
    }
  }

  async function retryReviewer(iteration: number, index: number): Promise<void> {
    const r = vault.docsRun;
    const key = `reviewer-${iteration}-${index}`;
    if (!r || retrying[key]) return;
    retrying = { ...retrying, [key]: true };
    try {
      await retryDocsReviewer(r.id, iteration, index);
      startPoll();
    } catch (e) {
      toastError('Couldn’t retry the reviewer', e);
    } finally {
      retrying = { ...retrying, [key]: false };
    }
  }

  async function retryRevision(iteration: number): Promise<void> {
    const r = vault.docsRun;
    const key = `revision-${iteration}`;
    if (!r || retrying[key]) return;
    retrying = { ...retrying, [key]: true };
    try {
      await retryDocsRevision(r.id, iteration);
      startPoll();
    } catch (e) {
      toastError('Couldn’t retry the revision', e);
    } finally {
      retrying = { ...retrying, [key]: false };
    }
  }

  let cancelling = $state(false);
  async function cancel(): Promise<void> {
    const r = vault.docsRun;
    if (!r || cancelling) return;
    // Stopping a multi-agent run throws away its in-progress work — ask first.
    const ok = await confirmer.ask('Stop this documentation run? Agents still working stop now and their unfinished work is discarded.', {
      title: 'Stop the run?', confirmLabel: 'Stop run', cancelLabel: 'Keep running', danger: true,
    });
    if (!ok || vault.docsRun?.id !== r.id) return;
    cancelling = true;
    try {
      await cancelDocsRun(r.id);
      await poll();
    } catch (e) {
      toastError('Couldn’t cancel the run', e);
    } finally {
      cancelling = false;
    }
  }

  /** Back to the form (keeps the prompt so a tweak-and-rerun is one edit). */
  function newRun(): void {
    vault.docsRun = null;
    openTerminals = new Set();
  }

  // -- done_with_findings dispositions ------------------------------------------
  // "Send to agent to fix" turns the run's outstanding findings into a
  // prefilled fix prompt; "Mark OK"/"Mark fixed" resolve the run durably.
  let resolving = $state(false);

  async function resolveRun(outcome: 'ok' | 'fixed'): Promise<void> {
    const r = vault.docsRun;
    if (!r || resolving) return;
    resolving = true;
    try {
      const resolved = await resolveDocsRun(r.id, outcome);
      if (vault.docsRun?.id !== r.id) return;
      vault.docsRun = resolved;
      void vault.refreshDocsRuns();
      toasts.success(
        'Review resolved',
        outcome === 'ok' ? 'Findings accepted as-is' : 'Findings marked as fixed',
      );
    } catch (e) {
      toastError('Couldn’t resolve the review', e);
    } finally {
      resolving = false;
    }
  }

  // History cleanup — terminal runs only (the server 409s on active ones).
  let deleting = $state<string | null>(null);

  async function deleteRun(r: VaultDocsRun): Promise<void> {
    if (deleting) return;
    const ok = await confirmer.ask('Delete this run from the history? Its log and findings go with it; documents it already wrote are kept.', {
      title: 'Delete the run?', confirmLabel: 'Delete', danger: true,
    });
    if (!ok || deleting) return;
    deleting = r.id;
    try {
      await deleteDocsRun(r.id);
      if (vault.docsRun?.id === r.id) vault.docsRun = null;
      vault.docsRuns = vault.docsRuns.filter((x) => x.id !== r.id);
    } catch (e) {
      toastError('Couldn’t delete the run', e);
    } finally {
      deleting = null;
    }
  }

  function fixWithAgent(): void {
    const r = vault.docsRun;
    if (!r) return;
    const round = r.review.rounds.at(-1);
    const findings = (round?.reviewers ?? []).flatMap((rv) => rv.findings);
    if (findings.length === 0) {
      toasts.error('Couldn’t fix the findings', 'This run has no recorded findings');
      return;
    }
    const lines = findings.map((f, i) => {
      const ev = f.evidence
        .map((e) =>
          [
            e.repo_path ? `${e.repo_path}${e.line != null ? ':' + e.line : ''}` : null,
            e.doc_path ? `${e.doc_path}${e.section ? ' §' + e.section : ''}` : null,
          ]
            .filter(Boolean)
            .join(' → '),
        )
        .filter(Boolean)
        .join('; ');
      return (
        `${i + 1}. [${f.severity}/${f.category}] ${f.summary}\n` +
        `   missed: ${f.missed_item}\n` +
        `   fix required: ${f.required_fix}` +
        (ev ? `\n   evidence: ${ev}` : '')
      );
    });
    const scope = r.target_dir ? ` (bundle \`${r.target_dir}/\`)` : '';
    vault.docsAgentsPrefill = {
      prompt:
        `FIX the documentation in this vault: a review pass finished with UNRESOLVED ` +
        `findings${scope}. Address EVERY finding below by editing the affected notes in ` +
        `place — verify each fix against the CURRENT source code, use REAL examples ` +
        `(actual field names from the code), and do not rewrite unaffected content.\n\n` +
        `FINDINGS:\n${lines.join('\n')}\n\n` +
        `Finish with a one-line summary listing the notes you changed.`,
      skills: ['vault-repo-docs'],
    };
  }

  /** Show one run (live or history) — the poll follows the selection. */
  function selectRun(r: VaultDocsRun): void {
    vault.docsRun = r;
    openTerminals = new Set();
    if (isActive(r)) startPoll();
  }

  // Inline live terminals — multiple may be open at once, keyed by session id.
  // History runs may reference sessions retention has since pruned — verify
  // the session still exists before mounting a dead terminal.
  let openTerminals = $state<Set<string>>(new Set());
  async function toggleTerminal(sessionId: string | null): Promise<void> {
    if (!sessionId) return;
    const next = new Set(openTerminals);
    if (next.has(sessionId)) {
      next.delete(sessionId);
      openTerminals = next;
      return;
    }
    try {
      await api.get(`/sessions/${sessionId}`);
    } catch {
      toasts.error('Couldn’t open the agent session', 'This agent session no longer exists (cleaned up by retention).');
      return;
    }
    next.add(sessionId);
    openTerminals = next;
  }

  onMount(() => {
    // Refetch the persisted list, then re-surface the most recent ACTIVE run
    // when nothing is selected — a run launched before a tab switch (or a
    // daemon restart, as `interrupted` history) is visible again immediately.
    void vault.refreshDocsRuns().then(() => {
      if (!vault.docsRun) {
        const live = vault.docsRuns.find(isActive);
        if (live) vault.docsRun = live;
      }
      const r = vault.docsRun;
      if ((r && isActive(r)) || vault.docsRuns.some(isActive)) startPoll();
    });
    const r = vault.docsRun;
    if (r && isActive(r)) startPoll();
    return () => stopPoll();
  });

  /** Run / agent / review state → the shared Badge tone. */
  function stTone(state: string): BadgeTone {
    if (state === 'running' || state === 'summarizing' || state === 'reviewing' || state === 'revising') return 'accent';
    if (state === 'done' || state === 'clean' || state === 'revised') return 'ok';
    if (state === 'done_with_findings' || state === 'exhausted') return 'warn';
    if (state === 'error') return 'bad';
    return 'neutral';
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`docs-agents`,});
   { svelteHTML.createElement("div", { "class":`inner`,});
     { svelteHTML.createElement("h2", {}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":14,}});}   }
    if(pollError){
       { svelteHTML.createElement("div", {   "class":`poll-err`,"role":`alert`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});}
         { svelteHTML.createElement("span", {});            pollError; }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void poll(),});  } 
       }
    }

    if(!run){
      
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`da-template`,});   }
         { svelteHTML.createElement("div", { "class":`tpl-row`,});
           { svelteHTML.createElement("select", {     "id":`da-template`,"class":`input`,"bind:value":tplId,});/*Ωignore_startΩ*/() => tplId = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",});     }
               for(let t of __sveltets_2_ensureArray(DOCS_TEMPLATES)){t.id;
               { svelteHTML.createElement("option", { "value":t.id,});t.label; }
            }
           }
          if(tpl?.needsRepo){
             { const $$_dleiFhtaP4C = __sveltets_2_ensureComponent(PathField); const $$_dleiFhtaP4 = new $$_dleiFhtaP4C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },value:tplRepo,}});/*Ωignore_startΩ*/() => tplRepo = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_dleiFhtaP4.$$bindings = 'value'; { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`input tpl-repo`,"bind:value":tplRepo,"placeholder":`e.g. ~/code/payments-service`,"aria-label":`Repository folder`,});/*Ωignore_startΩ*/() => tplRepo = __sveltets_2_any(null);/*Ωignore_endΩ*/} PathField}
          }
           { svelteHTML.createElement("button", {      "class":`tpl-use`,"disabled":!tpl || (tpl.needsRepo && !tplRepo.trim()),"onclick":applyTemplate,});  }
         }
        if(tpl?.needsRepo){
           { svelteHTML.createElement("label", {     "class":`tpl-infra`,"title":`Library/infrastructure repo: no HTTP API of its own, flows are partial — documents packages, exported interfaces, consumers and config instead`,});
             { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":tplInfra,});/*Ωignore_startΩ*/() => tplInfra = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                       
           }
        }
        if(tpl){
           { svelteHTML.createElement("div", { "class":`tpl-hint`,});tpl.hint;        }
        }
       }

       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`da-prompt`,});    }
         { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`da-prompt`,"class":`input da-prompt`,"bind:value":prompt,"rows":4,"placeholder":`e.g. Document the deploy pipeline: triggers, stages, rollback, and the runbook for a failed release.`,});/*Ωignore_startΩ*/() => prompt = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`da-target`,});      }
         { svelteHTML.createElement("input", {          "dir":`ltr`,"id":`da-target`,"class":`input`,"bind:value":targetDir,"placeholder":`e.g. runbooks/deploys`,});/*Ωignore_startΩ*/() => targetDir = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }

       { svelteHTML.createElement("div", {     "class":`field`,"role":`group`,"aria-labelledby":`da-writers`,});
         { svelteHTML.createElement("span", {   "class":`field-caption`,"id":`da-writers`,});  agents.length;  }
            for(let agent of __sveltets_2_ensureArray(agents)){let i = 1;i;
           { svelteHTML.createElement("div", { "class":`agent-row`,});
             { svelteHTML.createElement("select", {     "class":`input`,"bind:value":agent.provider,"aria-label":`Writer agent ${i + 1} provider`,});/*Ωignore_startΩ*/() => agent.provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
                 for(let p of __sveltets_2_ensureArray(providers)){p;
                 { svelteHTML.createElement("option", { "value":p,});p; }
              }
             }
             { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`input model`,"bind:value":agent.model,"placeholder":`Model (optional)`,"aria-label":`Writer agent ${i + 1} model`,});/*Ωignore_startΩ*/() => agent.model = __sveltets_2_any(null);/*Ωignore_endΩ*/}
             { svelteHTML.createElement("button", {           "class":`icon-btn`,"title":`Remove agent`,"aria-label":`Remove agent`,"disabled":agents.length <= 1,"onclick":() => removeAgent(i),});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
             }
           }
        }
        if(agents.length < 4){
           { svelteHTML.createElement("button", {   "class":`add-agent`,"onclick":addAgent,});   }
        }
       }

      if(agents.length > 1){
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("label", { "for":`da-summarizer`,});   agents.length;     }
           { svelteHTML.createElement("select", {     "id":`da-summarizer`,"class":`input sum-select`,"bind:value":sumProvider,});/*Ωignore_startΩ*/() => sumProvider = __sveltets_2_any(null);/*Ωignore_endΩ*/
               for(let p of __sveltets_2_ensureArray(providers)){p;
               { svelteHTML.createElement("option", { "value":p,});p; }
            }
           }
         }
      }

       { svelteHTML.createElement("section", {  "class":`review-config`,});reviewEnabled;
         { svelteHTML.createElement("div", { "class":`review-config-head`,});
           { svelteHTML.createElement("div", {});
             { svelteHTML.createElement("strong", {});  }
             { svelteHTML.createElement("span", {});          }
           }
           { const $$_hctiwS4C = __sveltets_2_ensureComponent(Switch); new $$_hctiwS4C({ target: __sveltets_2_any(), props: {      "checked":reviewEnabled,"onchange":(next) => (reviewEnabled = next),"label":`Review outcomes`,}});}
         }

        if(reviewEnabled){
           { svelteHTML.createElement("div", { "class":`review-settings`,});
             { svelteHTML.createElement("div", { "class":`review-setting-title`,});
               { svelteHTML.createElement("span", {});  reviewers.length;  }
               { svelteHTML.createElement("label", { "class":`iteration-field`,});
                  
                 { svelteHTML.createElement("input", {               "class":`input iteration-input`,"type":`number`,"min":`1`,"max":`10`,"bind:value":maxReviewIterations,"onchange":() => (maxReviewIterations = reviewIterationLimit(maxReviewIterations)),"aria-label":`Maximum review iterations`,});/*Ωignore_startΩ*/() => maxReviewIterations = __sveltets_2_any(null);/*Ωignore_endΩ*/}
               }
             }
                for(let reviewer of __sveltets_2_ensureArray(reviewers)){let i = 1;i;
               { svelteHTML.createElement("div", { "class":`reviewer-config-row`,});
                 { svelteHTML.createElement("span", { "class":`reviewer-number`,});i + 1; }
                 { svelteHTML.createElement("div", { "class":`reviewer-fields`,});
                   { svelteHTML.createElement("div", { "class":`reviewer-main-fields`,});
                     { svelteHTML.createElement("select", {     "class":`input`,"bind:value":reviewer.provider,"aria-label":`Reviewer provider`,});/*Ωignore_startΩ*/() => reviewer.provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
                         for(let p of __sveltets_2_ensureArray(providers)){p;
                         { svelteHTML.createElement("option", { "value":p,});p; }
                      }
                     }
                     { svelteHTML.createElement("input", {          "dir":`ltr`,"class":`input`,"bind:value":reviewer.model,"placeholder":`Model (optional)`,"aria-label":`Reviewer model`,});/*Ωignore_startΩ*/() => reviewer.model = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                     { svelteHTML.createElement("select", {     "class":`input review-method`,"bind:value":reviewer.skill,"aria-label":`Review method`,});/*Ωignore_startΩ*/() => reviewer.skill = __sveltets_2_any(null);/*Ωignore_endΩ*/
                         for(let method of __sveltets_2_ensureArray(REVIEW_METHODS)){method.value;
                         { svelteHTML.createElement("option", { "value":method.value,});method.label; }
                      }
                     }
                     { svelteHTML.createElement("button", {           "class":`icon-btn`,"title":`Remove reviewer`,"aria-label":`Remove reviewer`,"disabled":reviewers.length <= 1,"onclick":() => removeReviewer(i),});
                       { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});}
                     }
                   }
                   { svelteHTML.createElement("input", {          "dir":`auto`,"class":`input review-focus`,"bind:value":reviewer.focus,"placeholder":`Optional focus — e.g. request/response bodies`,"aria-label":`Review focus`,});/*Ωignore_startΩ*/() => reviewer.focus = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                 }
               }
            }
            if(reviewers.length < 4){
               { svelteHTML.createElement("button", {   "class":`add-agent`,"onclick":addReviewer,});   }
            }
             { svelteHTML.createElement("p", { "class":`review-hint`,});
                          
             }
           }
        }
       }

      if(runSkills.length > 0){
         { svelteHTML.createElement("div", {     "class":`field`,"role":`group`,"aria-labelledby":`da-skills`,});
           { svelteHTML.createElement("span", {   "class":`field-caption`,"id":`da-skills`,});         }
           { svelteHTML.createElement("div", { "class":`skill-chips`,});
               for(let s of __sveltets_2_ensureArray(runSkills)){s;
               { svelteHTML.createElement("button", {     "class":`skill-chip`,"title":`Open ${s}`,"onclick":() => void viewSkill(s),});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`function`,"size":12,}});}
                s;
               }
            }
           }
         }
      }

       { svelteHTML.createElement("div", { "class":`form-actions`,});
         { svelteHTML.createElement("button", {     "class":`primary`,"disabled":!prompt.trim() || starting,"onclick":() => void start(),});
          starting ? 'Starting…' : 'Run';
         }
       }
    }else{
      
       { svelteHTML.createElement("div", { "class":`run-head`,});
         { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"tone":stTone(run.state),"title":run.state === 'interrupted' ? 'The app/daemon restarted mid-run' : undefined,}});
          if(active){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); }}
          displayState(run.state);
         Badge}
         { svelteHTML.createElement("span", { "class":`kind-chip`,});kindLabel(run.kind); }
         { svelteHTML.createElement("span", {   "class":`run-meta`,"title":run.prompt,});
          if(run.kind === 'refine'){
             { svelteHTML.createElement("button", {   "class":`note-link`,"onclick":() => void vault.open(run.note_path),});
              run.note_path;
             }
             run.prompt;
          }else{
            run.prompt;run.target_dir ? ` → ${run.target_dir}/` : '';
          }
         }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
        if(active && run.kind === 'docs'){
           { svelteHTML.createElement("button", {     "class":`ghost`,"disabled":cancelling,"onclick":() => void cancel(),});
            cancelling ? 'Canceling…' : 'Cancel';
           }
        }
        if(run.state === 'done_with_findings'){
           { svelteHTML.createElement("button", {      "class":`ghost`,"title":`Prefill a fix run from this run’s outstanding findings`,"onclick":fixWithAgent,});     }
           { svelteHTML.createElement("button", {        "class":`ghost`,"disabled":resolving,"title":`The findings were addressed — resolve this run as done`,"onclick":() => void resolveRun('fixed'),});  }
           { svelteHTML.createElement("button", {        "class":`ghost`,"disabled":resolving,"title":`Accept the findings as-is — resolve this run as done`,"onclick":() => void resolveRun('ok'),});  }
        }
        
         { svelteHTML.createElement("button", {   "class":`ghost`,"onclick":newRun,});  }
       }

      if(run.error){
         { svelteHTML.createElement("div", {   "class":`err`,"role":`alert`,});run.error; }
      }

       { svelteHTML.createElement("div", { "class":`rows`,});
           for(let agent of __sveltets_2_ensureArray(run.agents)){agent.index;
           { svelteHTML.createElement("div", { "class":`agent-card`,});
             { svelteHTML.createElement("div", { "class":`agent-top`,});
               { svelteHTML.createElement("span", { "class":`agent-name`,});agent.name; }
               { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"variant":`outline`,}});agent.provider;agent.model ? ' · ' + agent.model : ''; Badge}
               { svelteHTML.createElement("span", { "class":`grow`,}); }
              if(agent.session_id){
                 { svelteHTML.createElement("button", {   "class":`ghost small`,"onclick":() => void toggleTerminal(agent.session_id),});
                  openTerminals.has(agent.session_id) ? 'Hide' : 'Open';
                 }
              }
              
              if(active && (agent.state === 'running' || agent.state === 'pending' || (agent.state === 'error' && run.state === 'running'))){
                 { svelteHTML.createElement("button", {         "class":`ghost small`,"title":agent.state === 'error'
                    ? "Re-spawn this writer’s turn in a fresh session"
                    : "Kill this writer’s session and restart its turn fresh","disabled":retrying[String(agent.index)],"onclick":() => void retry(agent.index),});
                  retrying[String(agent.index)] ? 'Retrying…' : 'Retry';
                 }
              }
               { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"tone":stTone(agent.state),}});
                if(agent.state === 'running'){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); }}
                runStateLabel(agent.state);
               Badge}
             }
            if(agent.error){
               { svelteHTML.createElement("p", { "class":`agent-err`,});agent.error; }
            }
            if(agent.drafts.length > 0 && active){
               { svelteHTML.createElement("p", { "class":`drafts`,});
                plural(agent.drafts.length, 'draft');
                 { svelteHTML.createElement("span", { "class":`mono`,});agent.drafts.join(' · '); }
               }
            }
            if(agent.session_id && openTerminals.has(agent.session_id)){
               { svelteHTML.createElement("div", { "class":`term`,});
                agent.session_id; {
                   { const $$_lanimreTyzaL5C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL5C({ target: __sveltets_2_any(), props: {    "sessionId":agent.session_id,"preferDom":true,"resumeOnOpen":false,}});}
                }
               }
            }
           }
        }

        
        if(run.kind !== 'refine'){
           { svelteHTML.createElement("div", { "class":`agent-card`,});
           { svelteHTML.createElement("div", { "class":`agent-top`,});
             { svelteHTML.createElement("span", { "class":`agent-name`,});  }
             { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"variant":`outline`,}});
              run.summarizer.provider;run.summarizer.model ? ' · ' + run.summarizer.model : '';
             Badge}
             { svelteHTML.createElement("span", { "class":`grow`,}); }
            if(run.summarizer.session_id){
               { svelteHTML.createElement("button", {     "class":`ghost small`,"onclick":() => void toggleTerminal(run.summarizer.session_id),});
                openTerminals.has(run.summarizer.session_id) ? 'Hide' : 'Open';
               }
            }
            if(run.state === 'summarizing' && (run.summarizer.state === 'running' || run.summarizer.state === 'pending')){
               { svelteHTML.createElement("button", {         "class":`ghost small`,"title":`Kill the summarizer’s session and restart the consolidation fresh`,"disabled":retrying['sum'],"onclick":() => void retry('sum'),});
                retrying['sum'] ? 'Retrying…' : 'Retry';
               }
            }
             { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"tone":stTone(run.summarizer.state),}});
              if(run.summarizer.state === 'running'){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); }}
              runStateLabel(run.summarizer.state);
             Badge}
           }
          if(run.summarizer.error){
             { svelteHTML.createElement("p", { "class":`agent-err`,});run.summarizer.error; }
          }
            if(run.summarizer.session_id && openTerminals.has(run.summarizer.session_id)){
               { svelteHTML.createElement("div", { "class":`term`,});
                run.summarizer.session_id; {
                   { const $$_lanimreTyzaL5C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL5C({ target: __sveltets_2_any(), props: {    "sessionId":run.summarizer.session_id,"preferDom":true,"resumeOnOpen":false,}});}
                }
               }
            }
           }
        }
       }

      if(run.kind === 'docs' && run.review && run.review.state !== 'skipped'){
         { svelteHTML.createElement("section", { "class":`review-ledger`,});
           { svelteHTML.createElement("div", { "class":`review-progress`,});
             { svelteHTML.createElement("div", {});
               { svelteHTML.createElement("span", { "class":`review-eyebrow`,});  }
               { svelteHTML.createElement("strong", {});reviewStateLabel(run); }
             }
             { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"tone":stTone(run.review.state),}});
              if(run.review.state === 'reviewing' || run.review.state === 'revising'){
                 { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); }
              }
              runStateLabel(run.review.state);
             Badge}
           }

          if(run.review.state === 'clean'){
             { svelteHTML.createElement("div", { "class":`review-outcome clean`,});
                       run.review.current_iteration;
             }
          } else if (run.review.state === 'exhausted'){
             { svelteHTML.createElement("div", { "class":`review-outcome exhausted`,});
               { svelteHTML.createElement("strong", {});   }    plural(run.review.max_iterations, 'iteration');      
                
             }
          } else if (run.review.outcome){
             { svelteHTML.createElement("div", { "class":`review-outcome`,}); displayState(run.review.outcome); }
          }

           { svelteHTML.createElement("div", { "class":`review-rounds`,});
               for(let round of __sveltets_2_ensureArray(run.review.rounds)){round.iteration;
               { svelteHTML.createElement("article", {  "class":`review-round`,});round.iteration === run.review.current_iteration;
                 { svelteHTML.createElement("header", { "class":`review-round-head`,});
                   { svelteHTML.createElement("span", { "class":`round-number`,});round.iteration; }
                   { svelteHTML.createElement("div", {});
                     { svelteHTML.createElement("strong", {}); round.iteration; }
                     { svelteHTML.createElement("span", {});
                      plural(round.reviewers.length, 'reviewer');
                     }
                   }
                   { svelteHTML.createElement("span", { "class":`grow`,}); }
                   { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"tone":stTone(round.state),}});runStateLabel(round.state); Badge}
                 }

                 { svelteHTML.createElement("div", { "class":`reviewer-list`,});
                     for(let reviewer of __sveltets_2_ensureArray(round.reviewers)){reviewer.index;
                     { svelteHTML.createElement("div", { "class":`reviewer-card`,});
                       { svelteHTML.createElement("div", { "class":`agent-top`,});
                         { svelteHTML.createElement("span", { "class":`agent-name`,});reviewer.skill; }
                         { const $$_egdaB8C = __sveltets_2_ensureComponent(Badge); new $$_egdaB8C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"variant":`outline`,}});
                          reviewer.provider;reviewer.model ? ' · ' + reviewer.model : '';
                         Badge}
                         { svelteHTML.createElement("span", { "class":`grow`,}); }
                        if(reviewer.session_id){
                           { svelteHTML.createElement("button", {     "class":`ghost small`,"onclick":() => void toggleTerminal(reviewer.session_id),});
                            openTerminals.has(reviewer.session_id) ? 'Hide' : 'Open';
                           }
                        }
                        if(active &&
                          (reviewer.state === 'pending' ||
                            reviewer.state === 'running' ||
                            reviewer.state === 'error')){
                           { svelteHTML.createElement("button", {       "class":`ghost small`,"disabled":retrying[`reviewer-${round.iteration}-${reviewer.index}`],"onclick":() => void retryReviewer(round.iteration, reviewer.index),});
                            retrying[`reviewer-${round.iteration}-${reviewer.index}`]
                              ? 'Retrying…'
                              : 'Retry reviewer';
                           }
                        }
                         { const $$_egdaB8C = __sveltets_2_ensureComponent(Badge); new $$_egdaB8C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"tone":stTone(reviewer.state),}});
                          if(reviewer.state === 'running'){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); }}
                          runStateLabel(reviewer.state);
                         Badge}
                       }
                      if(reviewer.focus){
                         { svelteHTML.createElement("p", { "class":`review-focus-label`,}); reviewer.focus; }
                      }
                      if(reviewer.error){
                         { svelteHTML.createElement("p", { "class":`agent-err`,});reviewer.error; }
                      }
                      if(reviewer.findings.length === 0 && reviewer.state === 'done'){
                         { svelteHTML.createElement("p", { "class":`clean-verdict`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}   }
                      } else if (reviewer.findings.length > 0){
                         { svelteHTML.createElement("div", { "class":`finding-list`,});
                              for(let finding of __sveltets_2_ensureArray(reviewer.findings)){let findingIndex = 1;`${reviewer.index}-${findingIndex}`;
                             { svelteHTML.createElement("div", { "class":`finding`,});
                               { svelteHTML.createElement("div", { "class":`finding-head`,});
                                 { svelteHTML.createElement("span", { "class":`severity sev-${finding.severity}`,});severityLabel(finding.severity); }
                                 { svelteHTML.createElement("span", { "class":`finding-category`,});finding.category; }
                               }
                               { svelteHTML.createElement("strong", {});finding.summary; }
                               { svelteHTML.createElement("p", {}); { svelteHTML.createElement("span", {});  } finding.missed_item; }
                               { svelteHTML.createElement("p", {}); { svelteHTML.createElement("span", {});  } finding.required_fix; }
                              if(finding.evidence.length > 0){
                                 { svelteHTML.createElement("div", { "class":`evidence-list`,});
                                    for(let evidence of __sveltets_2_ensureArray(finding.evidence)){
                                     { svelteHTML.createElement("span", { "class":`evidence`,});evidenceLabel(evidence); }
                                  }
                                 }
                              }
                             }
                          }
                         }
                      }
                      if(reviewer.session_id && openTerminals.has(reviewer.session_id)){
                         { svelteHTML.createElement("div", { "class":`term`,});
                          reviewer.session_id; {
                             { const $$_lanimreTyzaL8C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL8C({ target: __sveltets_2_any(), props: {    "sessionId":reviewer.session_id,"preferDom":true,"resumeOnOpen":false,}});}
                          }
                         }
                      }
                     }
                  }
                 }

                if(round.revision.state !== 'skipped'){
                   { svelteHTML.createElement("div", { "class":`revision-card`,});
                     { svelteHTML.createElement("div", { "class":`agent-top`,});
                       { svelteHTML.createElement("span", { "class":`revision-mark`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});} }
                       { svelteHTML.createElement("span", { "class":`agent-name`,});   }
                       { svelteHTML.createElement("span", { "class":`grow`,}); }
                      if(round.revision.session_id){
                         { svelteHTML.createElement("button", {     "class":`ghost small`,"onclick":() => void toggleTerminal(round.revision.session_id),});
                          openTerminals.has(round.revision.session_id) ? 'Hide' : 'Open';
                         }
                      }
                      if(active &&
                        (round.revision.state === 'pending' ||
                          round.revision.state === 'running' ||
                          round.revision.state === 'error')){
                         { svelteHTML.createElement("button", {       "class":`ghost small`,"disabled":retrying[`revision-${round.iteration}`],"onclick":() => void retryRevision(round.iteration),});
                          retrying[`revision-${round.iteration}`] ? 'Retrying…' : 'Retry revision';
                         }
                      }
                       { const $$_egdaB7C = __sveltets_2_ensureComponent(Badge); new $$_egdaB7C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"tone":stTone(round.revision.state),}});
                        if(round.revision.state === 'running'){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); }}
                        runStateLabel(round.revision.state);
                       Badge}
                     }
                    if(round.revision.error){
                       { svelteHTML.createElement("p", { "class":`agent-err`,});round.revision.error; }
                    }
                    if(round.revision.changed_paths.length > 0){
                       { svelteHTML.createElement("div", { "class":`changed-paths`,});
                           for(let path of __sveltets_2_ensureArray(round.revision.changed_paths)){path;
                           { svelteHTML.createElement("span", {});path; }
                        }
                       }
                    }
                    if(round.revision.session_id && openTerminals.has(round.revision.session_id)){
                       { svelteHTML.createElement("div", { "class":`term`,});
                        round.revision.session_id; {
                           { const $$_lanimreTyzaL7C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL7C({ target: __sveltets_2_any(), props: {    "sessionId":round.revision.session_id,"preferDom":true,"resumeOnOpen":false,}});}
                        }
                       }
                    }
                   }
                }
               }
            }
           }
         }
      }

      if(run.written.length > 0){
         { svelteHTML.createElement("div", { "class":`written`,});
           { svelteHTML.createElement("span", { "class":`written-title`,});
            plural(run.written.length, 'note'); 
           }
             for(let p of __sveltets_2_ensureArray(run.written)){p;
             { svelteHTML.createElement("button", {   "class":`written-link`,"onclick":() => void vault.open(p),});p; }
             { svelteHTML.createElement("button", {     "class":`written-link`,"title":`Review edits to ${p} during this run`,"onclick":() => void vault.openHistory(p, run.started_at),});  }
          }
         }
      }
    }

    
    if(vault.docsRuns.length > 0){
       { svelteHTML.createElement("div", { "class":`runs-section`,});
         { svelteHTML.createElement("span", { "class":`runs-title`,});  }
           for(let r of __sveltets_2_ensureArray(vault.docsRuns)){r.id;
           { svelteHTML.createElement("div", { "class":`run-row-wrap`,});
             { svelteHTML.createElement("button", {        "class":`run-row`,"onclick":() => selectRun(r),"title":r.prompt,});run?.id === r.id;
               { const $$_egdaB5C = __sveltets_2_ensureComponent(Badge); new $$_egdaB5C({ target: __sveltets_2_any(), props: {   children:() => { return __sveltets_2_any(0); },"tone":stTone(r.state),"title":r.state === 'interrupted' ? 'The app/daemon restarted mid-run' : undefined,}});
                if(isActive(r)){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 10px`,"aria-hidden":`true`,}); }}
                displayState(r.state);
               Badge}
               { svelteHTML.createElement("span", { "class":`kind-chip`,});kindLabel(r.kind); }
               { svelteHTML.createElement("span", { "class":`run-row-text`,});
                r.kind === 'refine' ? `${r.note_path} — ${r.prompt}` : r.prompt;
               }
               { svelteHTML.createElement("span", { "class":`run-row-when`,});new Date(r.started_at).toLocaleString(); }
             }
            if(!isActive(r)){
               { svelteHTML.createElement("button", {           "class":`run-del reveal-on-hover`,"title":`Delete this run from history`,"aria-label":`Delete this run from history`,"disabled":deleting === r.id,"onclick":() => void deleteRun(r),});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
               }
            }
           }
        }
       }
    }
   }
 }

if(skillView){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Skill ${skillView.name}`,"width":760,"onclose":() => (skillView = null),}});
     { svelteHTML.createElement("pre", { "class":`skill-body`,});skillView.body; }
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DocsAgentsView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DocsAgentsView__SvelteComponent_ = ReturnType<typeof DocsAgentsView__SvelteComponent_>;
/*Ωignore_endΩ*/export default DocsAgentsView__SvelteComponent_;