///<reference types="svelte" />
;// Test Cases tab — generate test cases, view them grouped by category, allow
// per-case approve/request-changes/edit, bulk-select + bulk-approve, drag-to-
// reorder (persists order_idx), approve a run (triggers skill learning), and
// publish to Confluence.

import { rel } from '../../lib/stores/now.svelte';
import ConfluenceSpaceSelect from '../../lib/components/ConfluenceSpaceSelect.svelte';
import ConfluencePagePicker from '../../lib/components/ConfluencePagePicker.svelte';
import { toastError } from '../../lib/toastError';
import { product } from '../../lib/stores/product.svelte';
import type { Poller } from '../../lib/poll';
import { liveQuery } from '../../lib/live';
import { onStoryChange } from './storyReset.svelte';
import { CaseOrder } from './caseOrder.svelte';
import { toasts } from '../../lib/toast.svelte';
import { runStateLabel } from '../../lib/labels';
import { plural } from '../../lib/plural';
import type {
    ProductTestcaseRunDetail,
    ProductTestcaseRun,
    ProductTestcase,
    TestcaseSteps,
  } from './types';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import Icon, { type IconName } from '../../lib/components/Icon.svelte';
import AgentByline from '../../lib/components/AgentByline.svelte';
import Badge from '../../lib/components/Badge.svelte';
import type { BadgeTone } from '../../lib/status';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // The rewrite/tests/inject run spawns an agent CLI session via the live
  // registry, so the provider must be a real registered agent (built-in or
  // custom like grok) — openai/gemini were never valid here.
  const PROVIDERS = $derived(agentProviders());
  const CATEGORIES = ['happy', 'validation', 'error', 'edge'] as const;
  type Category = (typeof CATEGORIES)[number];

  // Categories are kinds, not states: a neutral label + a kind icon, no hue.
  const CATEGORY_ICONS: Record<Category, IconName> = {
    happy: 'check',
    validation: 'shield',
    error: 'warning',
    edge: 'target',
  };

  const CATEGORY_LABELS: Record<Category, string> = {
    happy: 'Happy Path',
    validation: 'Validation',
    error: 'Error Cases',
    edge: 'Edge Cases',
  };

  // Priority ordering for display within category (used as tiebreaker when
  // within a category group but no explicit order_idx reorder has been applied).
  const PRIORITY_ORDER: Record<string, number> = { high: 0, medium: 1, low: 2 };

  // ── Local UI state ──────────────────────────────────────────────────────────
  let provider = $state<string>(defaultAgentProvider());
  let generating = $state(false);
  let approvingRun = $state(false);
  let bulkApproving = $state(false);
  let publishingRun = $state(false);
  let showPublishForm = $state(false);
  let publishSpaceKey = $state('');
  // Parent page: picked by TITLE (PC-9), stored by id.
  let publishParentId = $state<string | null>(null);
  let publishParentTitle = $state<string | null>(null);

  // ── Bulk-select state ────────────────────────────────────────────────────────
  // Set of testcase ids that are currently checked.
  let selected = $state<Set<string>>(new Set());

  function toggleSelect(id: string): void {
    const next = new Set(selected);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    selected = next;
  }

  function selectAll(): void {
    selected = new Set(activeCases.map((c) => c.id));
  }

  function clearSelection(): void {
    selected = new Set();
  }

  // ── Reorder (drag, ⌥↑/⌥↓ and the Move buttons): caseOrder.svelte.ts ──────
  // `order.ids` mirrors activeCases but can be locally rearranged before
  // persisting; reset whenever activeCases changes, so local drags don't
  // fight with store refreshes until the user drops.
  const order = new CaseOrder({ run: () => activeRun, save: (runId, ids) => product.reorderTestcases(runId, ids) });
  $effect(() => {
    order.ids = activeCases.map((c) => c.id);
  });

  // Which test-case run we're viewing (most recent by default).
  let activeRunId = $state<string | null>(null);

  // Per-case action state, keyed by testcase id.
  interface CaseAction {
    mode: 'idle' | 'approve' | 'changes' | 'edit';
    reviewNote: string;
    /** Inline error under the review note (never a toast for a field error). */
    noteError?: string;
    editTitle: string;
    editCategory: string;
    editPriority: string;
    editPreconditions: string; // newline-separated
    editSteps: string; // newline-separated
    editExpected: string;
    busy: boolean;
  }
  let caseActions = $state<Record<string, CaseAction>>({});

  // ── Polling ──────────────────────────────────────────────────────────────────
  // Shared poll helper (lib/poll.ts): paused while hidden, never stacked,
  // aborted on stop. `pollTimer !== null` still means "polling" to the UI.
  let pollTimer = $state.raw<Poller | null>(null);
  const POLL_INTERVAL_MS = 3000;
  const POLL_MAX_MS = 120_000;
  let pollStartedAt = 0;
  let runsCountAtStart = 0;
  // Consecutive failed reads while waiting (same as PlanTab): after
  // POLL_FAIL_SHOWN the waiting state says why (with Check again), and a
  // timeout after failed reads blames the read, not the agent.
  const POLL_FAIL_SHOWN = 2;
  let pollFailures = $state(0);
  let lastPollError = $state<string | null>(null);

  function clearPoll(): void {
    if (pollTimer !== null) {
      pollTimer.stop();
      pollTimer = null;
    }
  }

  /** One poll read. Resolves `false` on a failed read so the poller backs
   *  off (lib/poll.ts) instead of hammering a daemon that is down. */
  async function pollTestcases(): Promise<boolean> {
    // A `product_changed` nudge also lands here with no poll running: no
    // timeout and no failure count then.
    const polling = pollTimer !== null;
    if (polling && Date.now() - pollStartedAt > POLL_MAX_MS) {
      const cause = pollFailures > 0 ? lastPollError : null;
      clearPoll();
      if (cause) toasts.warn('Couldn’t check for the test cases', cause);
      else toasts.warn('Generate timed out', 'No new test run appeared within 2 minutes.');
      return true;
    }
    try {
      await product.loadTestcases();
      pollFailures = 0;
      lastPollError = null;
      if (product.testcaseRuns.length > runsCountAtStart) {
        clearPoll();
        // Auto-select the newest run.
        if (product.testcaseRuns.length > 0) {
          activeRunId = product.testcaseRuns[0].run.id;
        }
      }
      return true;
    } catch (e) {
      if (!polling) return true;
      pollFailures++;
      lastPollError = loadErrorText(e);
      return false;
    }
  }

  function startPolling(): void {
    clearPoll();
    runsCountAtStart = product.testcaseRuns.length;
    pollStartedAt = Date.now();
    pollFailures = 0;
    lastPollError = null;
    // Immediate first poll, then every POLL_INTERVAL_MS after each settles.
    // `product_changed` (onSectionChange, below) settles it; this chain is
    // a 15 s safety net while events flow, POLL_INTERVAL_MS while they cannot.
    pollTimer = liveQuery({ run: () => pollTestcases(), on: [], fallbackMs: POLL_INTERVAL_MS, safetyMs: 15_000, jitter: 0 });
  }

  /** A failed load — inline with Retry, never as "No test cases yet". */
  let loadError = $state<string | null>(null);
  async function loadRuns(): Promise<void> {
    loadError = null;
    try {
      await product.loadTestcases();
      if (product.testcaseRuns.length > 0 && !activeRunId) {
        activeRunId = product.testcaseRuns[0].run.id;
      }
    } catch (e) {
      loadError = loadErrorText(e);
    }
  }

  // Reset on story change — depends on the story id only (storyReset.svelte.ts).
  onStoryChange(() => product.selectedId, (sid) => {
    activeRunId = null;
    caseActions = {};
    showPublishForm = false;
    selected = new Set();
    order.ids = [];
    clearPoll();
    if (sid) void loadRuns();
  }, clearPoll);

  // Subscribe to `product_changed { section: 'testcases' }` WS events.
  $effect(() => {
    const off = product.onSectionChange('testcases', (_status: string) => {
      void pollTestcases(); // immediate refresh; clears poll if new run appeared
    });
    return off;
  });

  // ── Derived ─────────────────────────────────────────────────────────────────
  const story = $derived(product.detail?.story ?? null);

  const activeRunDetail = $derived<ProductTestcaseRunDetail | null>(
    activeRunId
      ? (product.testcaseRuns.find((r) => r.run.id === activeRunId) ?? null)
      : (product.testcaseRuns[0] ?? null)
  );
  const activeRun = $derived<ProductTestcaseRun | null>(activeRunDetail?.run ?? null);
  const activeCases = $derived<ProductTestcase[]>(activeRunDetail?.cases ?? []);

  // What a Confluence publish actually sends (mirrors `publish_testcases`): only
  // APPROVED cases, on a "Test Cases — <title>" page. A Jira story has no space
  // to fall back to, so its space key is required, and the daemon also comments
  // the page link on the issue — say all of that before the user hits Publish.
  const approvedCount = $derived(activeCases.filter((c) => c.status === 'approved').length);
  const storyIsJira = $derived(story?.source_kind === 'jira');
  const publishBlocked = $derived(
    approvedCount === 0
      ? 'Accept at least one case — only accepted cases are published'
      : storyIsJira && !publishSpaceKey.trim()
        ? 'Pick a space — a Jira story has no Confluence space to publish into'
        : '',
  );

  // Cases in the locally-applied display order (reflects drag-and-drop before
  // the next store reload, which will bring the persisted order_idx back).
  const orderedCases = $derived.by(() => {
    const byId = new Map(activeCases.map((c) => [c.id, c]));
    // order.ids may be stale (set from a previous activeCases snapshot); fall
    // back to any cases not present in order.ids appended at the end.
    const seen = new Set<string>();
    const result: ProductTestcase[] = [];
    for (const id of order.ids) {
      const tc = byId.get(id);
      if (tc) {
        result.push(tc);
        seen.add(id);
      }
    }
    // Append cases that arrived after the local order.ids was set.
    for (const c of activeCases) {
      if (!seen.has(c.id)) result.push(c);
    }
    return result;
  });

  // Cases grouped by category, ordered by the local drag-reordered list (then
  // priority as tiebreaker for cases not yet reordered by the user).
  const groupedCases = $derived.by(() => {
    const result: Record<Category, ProductTestcase[]> = {
      happy: [],
      validation: [],
      error: [],
      edge: [],
    };
    for (const c of orderedCases) {
      const cat = c.category as Category;
      if (cat in result) {
        result[cat].push(c);
      } else {
        // Unknown category — put in edge as fallback.
        result.edge.push(c);
      }
    }
    // Within each category the drag-order is already respected by orderedCases;
    // use priority as a secondary sort only among cases that share the same
    // effective order_idx (i.e. newly inserted rows not yet reordered).
    for (const cat of CATEGORIES) {
      result[cat].sort((a, b) => {
        const oa = order.ids.indexOf(a.id);
        const ob = order.ids.indexOf(b.id);
        if (oa !== ob) {
          // Both present — use the local order.ids position.
          if (oa !== -1 && ob !== -1) return oa - ob;
          // One missing (new row) → push it after the known ones.
          if (oa === -1) return 1;
          if (ob === -1) return -1;
        }
        // Fallback: priority.
        const pa = PRIORITY_ORDER[a.priority] ?? 99;
        const pb = PRIORITY_ORDER[b.priority] ?? 99;
        return pa - pb;
      });
    }
    return result;
  });

  // ── Helpers ──────────────────────────────────────────────────────────────────

  function parseSteps(json: string): TestcaseSteps {
    try {
      return JSON.parse(json) as TestcaseSteps;
    } catch {
      return { preconditions: [], steps: [], expected: '' };
    }
  }

  function defaultAction(): CaseAction {
    return {
      mode: 'idle',
      reviewNote: '',
      editTitle: '',
      editCategory: '',
      editPriority: '',
      editPreconditions: '',
      editSteps: '',
      editExpected: '',
      busy: false,
    };
  }

  // Pure read: returns the stored action or a fresh idle default. It must NOT
  // mutate `caseActions` — getAction is called from the template ({@const ...}),
  // and writing state during render throws `state_unsafe_mutation` (which broke
  // the whole tab: the cases never rendered and it sat on "Loading…" forever).
  // The real entry is created lazily by setAction() on the first user action.
  function getAction(id: string): CaseAction {
    return caseActions[id] ?? defaultAction();
  }

  function setAction(id: string, patch: Partial<CaseAction>): void {
    const current = getAction(id);
    caseActions = { ...caseActions, [id]: { ...current, ...patch } };
  }

  function openEdit(tc: ProductTestcase): void {
    const steps = parseSteps(tc.steps_json);
    setAction(tc.id, {
      mode: 'edit',
      editTitle: tc.title,
      editCategory: tc.category,
      editPriority: tc.priority,
      editPreconditions: steps.preconditions.join('\n'),
      editSteps: steps.steps.join('\n'),
      editExpected: steps.expected,
      reviewNote: tc.review_note ?? '',
    });
  }

  function openChanges(tc: ProductTestcase): void {
    setAction(tc.id, {
      mode: 'changes',
      reviewNote: tc.review_note ?? '',
    });
  }

  function cancelAction(id: string): void {
    setAction(id, { mode: 'idle' });
  }

  async function approveCase(tc: ProductTestcase): Promise<void> {
    setAction(tc.id, { busy: true });
    try {
      await product.updateTestcase(tc.id, { status: 'approved' });
      setAction(tc.id, { mode: 'idle', busy: false });
      toasts.success('Case accepted');
    } catch (e) {
      toastError('Couldn’t accept the case', e);
      setAction(tc.id, { busy: false });
    }
  }

  // ── Bulk approve selected cases ───────────────────────────────────────────
  async function bulkApproveSelected(): Promise<void> {
    if (!activeRun || selected.size === 0 || bulkApproving) return;
    bulkApproving = true;
    try {
      const result = await product.bulkApproveTestcases(activeRun.id, [...selected]);
      clearSelection();
      toasts.success(
        `${plural(result.approved, 'case')} accepted`,
      );
    } catch (e) {
      toastError('Couldn’t accept the selected cases', e);
    } finally {
      bulkApproving = false;
    }
  }

  async function submitChanges(tc: ProductTestcase): Promise<void> {
    const action = getAction(tc.id);
    if (!action.reviewNote.trim()) {
      setAction(tc.id, { noteError: 'Add a review note that says what needs to change.' });
      return;
    }
    setAction(tc.id, { noteError: undefined });
    setAction(tc.id, { busy: true });
    try {
      await product.updateTestcase(tc.id, {
        status: 'changes_requested',
        review_note: action.reviewNote.trim(),
      });
      setAction(tc.id, { mode: 'idle', busy: false });
      toasts.info('Changes requested');
    } catch (e) {
      toastError('Couldn’t update the case', e);
      setAction(tc.id, { busy: false });
    }
  }

  async function submitEdit(tc: ProductTestcase): Promise<void> {
    const action = getAction(tc.id);
    setAction(tc.id, { busy: true });
    const steps: TestcaseSteps = {
      preconditions: action.editPreconditions
        .split('\n')
        .map((s) => s.trim())
        .filter(Boolean),
      steps: action.editSteps
        .split('\n')
        .map((s) => s.trim())
        .filter(Boolean),
      expected: action.editExpected.trim(),
    };
    try {
      await product.updateTestcase(tc.id, {
        title: action.editTitle.trim() || null,
        category: action.editCategory || null,
        priority: action.editPriority || null,
        steps,
        review_note: action.reviewNote.trim() || null,
      });
      setAction(tc.id, { mode: 'idle', busy: false });
      toasts.success('Test case updated');
    } catch (e) {
      toastError('Couldn’t update the case', e);
      setAction(tc.id, { busy: false });
    }
  }

  // ── Generate ─────────────────────────────────────────────────────────────────

  async function generate(): Promise<void> {
    if (generating) return;
    generating = true;
    try {
      await product.generateTests({ provider: provider || null });
      toasts.info('Test generation triggered', 'Waiting for a new run to appear…');
      startPolling();
    } catch (e) {
      toastError('Couldn’t generate test cases', e);
    } finally {
      generating = false;
    }
  }

  // ── Approve run ───────────────────────────────────────────────────────────────

  async function approveRun(): Promise<void> {
    if (!activeRun || approvingRun) return;
    approvingRun = true;
    try {
      await product.approveRun(activeRun.id);
      const approvedCount = activeCases.filter((c) => c.status === 'approved').length;
      toasts.success(
        'Run approved',
        `${plural(approvedCount, 'case')} accepted — skill learning kicked off.`,
      );
    } catch (e) {
      toastError('Couldn’t approve the run', e);
    } finally {
      approvingRun = false;
    }
  }

  // ── Publish ───────────────────────────────────────────────────────────────────

  async function publishTests(): Promise<void> {
    if (!activeRun || publishingRun) return;
    // Pin the run: `activeRun` is derived and may point elsewhere after the awaits.
    const run = activeRun;
    publishingRun = true;
    try {
      // publishTests reloads the runs itself (for confluence_url) — no second fetch.
      await product.publishTests(run.id, {
        space_key: publishSpaceKey.trim() || null,
        parent_id: publishParentId,
      });
      showPublishForm = false;
      const updatedRun = product.testcaseRuns.find((r) => r.run.id === run.id);
      const url = updatedRun?.run.confluence_url ?? run.confluence_url;
      if (url) {
        toasts.success('Published to Confluence', url);
      } else {
        toasts.success('Published to Confluence');
      }
    } catch (e) {
      toastError('Couldn’t publish the test cases', e);
    } finally {
      publishingRun = false;
    }
  }

  // ── Status helpers ────────────────────────────────────────────────────────────

  function statusTone(status: string): BadgeTone {
    switch (status) {
      case 'approved': return 'ok';
      case 'changes_requested': return 'warn';
      case 'rejected': return 'bad';
      case 'published': return 'info';
      default: return 'neutral';
    }
  }

  function statusLabel(status: string): string {
    switch (status) {
      case 'approved': return 'Accepted';
      case 'changes_requested': return 'Changes needed';
      case 'rejected': return 'Rejected';
      case 'draft': return 'Draft';
      case 'published': return 'Published';
      default: return status;
    }
  }

  function priorityClass(p: string): string {
    switch (p) {
      case 'high': return 'pri-high';
      case 'medium': return 'pri-medium';
      case 'low': return 'pri-low';
      default: return 'pri-medium';
    }
  }

  function fmtDate(s: string): string {
    try { return rel(s); } catch { return s; }
  }
;
async () => {


 { svelteHTML.createElement("span", {   "class":`sr-only`,"role":`status`,});pollTimer !== null && pollFailures >= POLL_FAIL_SHOWN && lastPollError ? `Couldn’t check progress: ${lastPollError}` : ''; }

if(!story){
   { svelteHTML.createElement("div", { "class":`muted`,});   }
}else{
   { svelteHTML.createElement("div", { "class":`tc-tab`,});

    
     { svelteHTML.createElement("section", { "class":`tc-card gen-panel`,});
       { svelteHTML.createElement("div", { "class":`gen-row`,});
         { svelteHTML.createElement("div", { "class":`provider-wrap`,});
           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`tc-provider-sel`,});  }
           { svelteHTML.createElement("select", {         "id":`tc-provider-sel`,"class":`input tc-sel`,"bind:value":provider,"disabled":generating,});/*Ωignore_startΩ*/() => provider = __sveltets_2_any(null);/*Ωignore_endΩ*/
               for(let p of __sveltets_2_ensureArray(PROVIDERS)){p;
               { svelteHTML.createElement("option", { "value":p,});p; }
            }
           }
         }

        
         { svelteHTML.createElement("button", {        "class":`btn`,"onclick":generate,"disabled":generating || pollTimer !== null,});product.testcaseRuns.length === 0;
          if(generating){ } else if (pollTimer !== null){  }else{  }
         }

        if(pollTimer !== null){
           { svelteHTML.createElement("span", { "class":`polling-indicator`,});   }
          if(pollFailures >= POLL_FAIL_SHOWN && lastPollError){
             { svelteHTML.createElement("span", { "class":`poll-error`,});   lastPollError; }
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => pollTimer?.now(),});  }
          }
        }
       }
     }

    
    if(product.testcaseRuns.length > 1){
       { svelteHTML.createElement("div", { "class":`run-selector-row`,});
         { svelteHTML.createElement("span", { "class":`field-label`,});  }
         { svelteHTML.createElement("select", {       "class":`input tc-sel`,"bind:value":activeRunId,"aria-label":`Test case run`,});/*Ωignore_startΩ*/() => activeRunId = __sveltets_2_any(null);/*Ωignore_endΩ*/
             for(let rd of __sveltets_2_ensureArray(product.testcaseRuns)){rd.run.id;
             { svelteHTML.createElement("option", { "value":rd.run.id,});
              fmtDate(rd.run.created_at);  runStateLabel(rd.run.status);  plural(rd.cases.length, 'case');
             }
          }
         }
       }
    }

    
    if((product.loadingTestcases || loadError) && product.testcaseRuns.length === 0){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`test cases`,"loading":product.loadingTestcases,"error":loadError,"empty":true,"onretry":() => void loadRuns(),}});}
    } else if (activeRunDetail && activeRun){
      
       { svelteHTML.createElement("section", { "class":`tc-card run-header`,});
         { svelteHTML.createElement("div", { "class":`rh-row`,});
           { svelteHTML.createElement("div", { "class":`rh-info`,});
            
             { const $$_enilyBtnegA4C = __sveltets_2_ensureComponent(AgentByline); new $$_enilyBtnegA4C({ target: __sveltets_2_any(), props: {    "at":activeRun.created_at,"testid":`testcases-byline`,}});}
             { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {    "tone":statusTone(activeRun.status),"label":statusLabel(activeRun.status),}});}
             { svelteHTML.createElement("span", { "class":`rh-count`,});plural(activeCases.length, 'case'); }
           }

           { svelteHTML.createElement("div", { "class":`rh-actions`,});
            
            if(activeRun.status !== 'approved' && activeRun.status !== 'published'){
               { svelteHTML.createElement("button", {         "class":`btn small primary`,"onclick":approveRun,"disabled":approvingRun,"title":`Approve this run — triggers skill self-improvement from the reviewed cases`,});
                approvingRun ? 'Approving…' : 'Approve run';
               }
            }

            
            if(activeRun.confluence_url){
               { svelteHTML.createElement("a", {         "class":`confluence-link`,"href":activeRun.confluence_url,"target":`_blank`,"rel":`noopener noreferrer`,});
                  
               }
            }else{
               { svelteHTML.createElement("button", {         "class":`btn small`,"aria-expanded":showPublishForm,"onclick":() => (showPublishForm = !showPublishForm),"disabled":publishingRun,});
                  
               }
            }
           }
         }

        
        if(showPublishForm){
           { svelteHTML.createElement("div", { "class":`publish-form`,});
             { svelteHTML.createElement("div", { "class":`pf-row`,});
               { svelteHTML.createElement("label", {   "class":`field-label`,"for":`pf-space`,});  }
              if(story){
                 { const $$_tceleSecapSecneulfnoC4C = __sveltets_2_ensureComponent(ConfluenceSpaceSelect); new $$_tceleSecapSecneulfnoC4C({ target: __sveltets_2_any(), props: {             "id":`pf-space`,"accountId":story.account_id,"value":publishSpaceKey,"disabled":publishingRun,"emptyOption":storyIsJira ? undefined : 'The source page’s space',"onchange":(k) => { publishSpaceKey = k; },}});}
              }
             }
             { svelteHTML.createElement("div", { "class":`pf-row`,});
              if(story){
                 { const $$_rekciPegaPecneulfnoC4C = __sveltets_2_ensureComponent(ConfluencePagePicker); new $$_rekciPegaPecneulfnoC4C({ target: __sveltets_2_any(), props: {               "id":`pf-parent`,"accountId":story.account_id,"spaceKey":publishSpaceKey,"value":publishParentId,"title":publishParentTitle,"disabled":publishingRun,"onchange":(id, t) => { publishParentId = id; publishParentTitle = t; },}});}
              }
             }
             { svelteHTML.createElement("p", {   "class":`pf-summary`,...__sveltets_2_empty({"data-testid":`tc-publish-summary`}),});
               plural(approvedCount, 'accepted case');   
                 story?.title ?? ''; publishParentId ? ` under “${publishParentTitle ?? `page ${publishParentId}`}”` : '';storyIsJira ? `, and comments its link on ${story?.source_key ?? 'the issue'}` : '';
                      
             }
             { svelteHTML.createElement("div", { "class":`pf-actions`,});
               { svelteHTML.createElement("button", {         "class":`btn small primary`,"onclick":publishTests,"disabled":publishingRun || !!publishBlocked,"title":publishBlocked || undefined,});
                publishingRun ? 'Publishing…' : 'Publish to Confluence';
               }
               { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":() => (showPublishForm = false),"disabled":publishingRun,});
                
               }
             }
           }
        }
       }

      
      if(activeCases.length > 0){
         { svelteHTML.createElement("div", { "class":`bulk-toolbar`,});
           { svelteHTML.createElement("label", { "class":`bulk-check-all`,});
             { svelteHTML.createElement("input", {         "type":`checkbox`,"checked":selected.size === activeCases.length && activeCases.length > 0,"indeterminate":selected.size > 0 && selected.size < activeCases.length,"onchange":() => (selected.size === activeCases.length ? clearSelection() : selectAll()),});}
             { svelteHTML.createElement("span", { "class":`field-label`,});
              selected.size > 0 ? `${selected.size} selected` : 'Select all';
             }
           }

          if(selected.size > 0){
             { svelteHTML.createElement("button", {         "class":`btn small primary`,"onclick":bulkApproveSelected,"disabled":bulkApproving,"title":`Accept all selected draft cases`,});
              bulkApproving ? 'Accepting…' : `Accept ${selected.size}`;
             }
             { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":clearSelection,"disabled":bulkApproving,});
              
             }
          }

          if(order.saving){
             { svelteHTML.createElement("span", { "class":`field-label save-indicator`,});  }
          }
         }
      }

      
         for(let cat of __sveltets_2_ensureArray(CATEGORIES)){cat;
        const cases = groupedCases[cat];
        if(cases.length > 0){
           { svelteHTML.createElement("section", { "class":`category-section`,});
             { svelteHTML.createElement("div", { "class":`cat-header`,});
               { svelteHTML.createElement("span", { "class":`cat-label`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":CATEGORY_ICONS[cat],"size":12,}});} CATEGORY_LABELS[cat]; }
               { svelteHTML.createElement("span", { "class":`cat-count`,});cases.length; }
             }

             { svelteHTML.createElement("div", {   "class":`cases-list`,"role":`list`,});
                 for(let tc of __sveltets_2_ensureArray(cases)){tc.id;
                const action = getAction(tc.id);
                const steps = parseSteps(tc.steps_json);

                 { svelteHTML.createElement("div", {                      ...__sveltets_2_empty({"data-case-id":tc.id}),"class":`case-card tc-card`,"role":`listitem`,"draggable":`true`,"ondragstart":() => order.dragStart(tc.id),"ondragover":(e) => order.dragOver(tc.id, e),"ondragleave":() => order.dragLeave(),"ondrop":() => order.drop(tc.id),"ondragend":() => order.dragEnd(),});tc.status === 'approved';selected.has(tc.id);order.dragOverId === tc.id;
                  
                   { svelteHTML.createElement("div", { "class":`case-header`,});
                     { svelteHTML.createElement("div", { "class":`case-title-row`,});
                      
                       { svelteHTML.createElement("span", {     "class":`drag-handle`,"aria-hidden":`true`,"title":`Drag to reorder (or ⌥↑ / ⌥↓)`,});  }
                       { svelteHTML.createElement("input", {               "type":`checkbox`,"class":`case-checkbox`,"checked":selected.has(tc.id),"onchange":() => toggleSelect(tc.id),"onclick":(e) => e.stopPropagation(),"onkeydown":(e) => order.onKey(e, tc.id, cases.map((c) => c.id)),"aria-label":`Select ${tc.title}`,});}
                       { svelteHTML.createElement("span", { "class":`case-title`,});tc.title; }
                       { svelteHTML.createElement("span", { "class":`priority-badge ${priorityClass(tc.priority)}`,});tc.priority; }
                       { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "tone":statusTone(tc.status),"label":statusLabel(tc.status),}});}
                     }

                    
                    if(action.mode === 'idle'){
                       { svelteHTML.createElement("div", { "class":`case-actions`,});
                         { svelteHTML.createElement("button", {         "class":`btn small primary`,"onclick":() => approveCase(tc),"disabled":action.busy || tc.status === 'approved',"title":`Accept this test case`,});
                          tc.status === 'approved' ? 'Accepted' : 'Accept';
                         }
                         { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":() => openChanges(tc),"disabled":action.busy,"title":`Request changes`,});
                           
                         }
                         { svelteHTML.createElement("button", {         "class":`btn small ghost`,"onclick":() => openEdit(tc),"disabled":action.busy,"title":`Edit this test case`,});
                          
                         }
                        if(cases.length > 1){
                          const group = cases.map((c) => c.id);
                           { svelteHTML.createElement("button", {               "class":`btn small ghost`,...__sveltets_2_empty({"data-move":`up`}),"onclick":() => void order.move(tc.id, -1, group, 'up'),"onkeydown":(e) => order.onKey(e, tc.id, group),"disabled":group[0] === tc.id,"title":`Move up (⌥↑)`,"aria-label":`Move ${tc.title} up`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`chevronUp`,"size":12,}});} }
                           { svelteHTML.createElement("button", {               "class":`btn small ghost`,...__sveltets_2_empty({"data-move":`down`}),"onclick":() => void order.move(tc.id, 1, group, 'down'),"onkeydown":(e) => order.onKey(e, tc.id, group),"disabled":group[group.length - 1] === tc.id,"title":`Move down (⌥↓)`,"aria-label":`Move ${tc.title} down`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});} }
                        }
                       }
                    }
                   }

                  
                  if(tc.review_note && action.mode === 'idle'){
                     { svelteHTML.createElement("div", { "class":`review-note`,});
                       { svelteHTML.createElement("span", { "class":`rn-label`,});  }
                       { svelteHTML.createElement("span", { "class":`rn-body`,});tc.review_note; }
                     }
                  }

                  
                  if(action.mode === 'idle'){
                     { svelteHTML.createElement("div", { "class":`steps-section`,});
                      if(steps.preconditions.length > 0){
                         { svelteHTML.createElement("div", { "class":`steps-group`,});
                           { svelteHTML.createElement("div", { "class":`steps-label`,});  }
                           { svelteHTML.createElement("ul", { "class":`steps-list`,});
                              for(let pre of __sveltets_2_ensureArray(steps.preconditions)){
                               { svelteHTML.createElement("li", {});pre; }
                            }
                           }
                         }
                      }

                      if(steps.steps.length > 0){
                         { svelteHTML.createElement("div", { "class":`steps-group`,});
                           { svelteHTML.createElement("div", { "class":`steps-label`,});  }
                           { svelteHTML.createElement("ol", { "class":`steps-list`,});
                              for(let step of __sveltets_2_ensureArray(steps.steps)){
                               { svelteHTML.createElement("li", {});step; }
                            }
                           }
                         }
                      }

                      if(steps.expected){
                         { svelteHTML.createElement("div", { "class":`steps-group`,});
                           { svelteHTML.createElement("div", { "class":`steps-label`,});  }
                           { svelteHTML.createElement("div", { "class":`expected-body`,});steps.expected; }
                         }
                      }
                     }
                  }

                  
                  if(action.mode === 'changes'){
                     { svelteHTML.createElement("div", { "class":`inline-form`,});
                       { svelteHTML.createElement("div", { "class":`if-label`,});   }
                       { svelteHTML.createElement("textarea", {                    "dir":`auto`,"aria-label":`Review note`,"class":`text-area`,"rows":3,"placeholder":`Describe what needs to change…`,"bind:value":caseActions[tc.id].reviewNote,"disabled":action.busy,"aria-invalid":action.noteError ? 'true' : undefined,"aria-describedby":action.noteError ? `rc-note-err-${tc.id}` : undefined,"oninput":() => { if (action.noteError) setAction(tc.id, { noteError: undefined }); },});/*Ωignore_startΩ*/() => caseActions[tc.id].reviewNote = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                      if(action.noteError){ { svelteHTML.createElement("p", {   "class":`field-error`,"id":`rc-note-err-${tc.id}`,});action.noteError; }}
                       { svelteHTML.createElement("div", { "class":`if-actions`,});
                         { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":() => submitChanges(tc),"disabled":action.busy,});
                          action.busy ? 'Saving…' : 'Request changes';
                         }
                         { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":() => cancelAction(tc.id),"disabled":action.busy,});
                          
                         }
                       }
                     }
                  }

                  
                  if(action.mode === 'edit'){
                     { svelteHTML.createElement("div", { "class":`inline-form`,});
                       { svelteHTML.createElement("div", { "class":`edit-grid`,});
                         { svelteHTML.createElement("div", { "class":`edit-field`,});
                           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`ef-title-${tc.id}`,});  }
                           { svelteHTML.createElement("input", {            "dir":`auto`,"id":`ef-title-${tc.id}`,"class":`text-input`,"type":`text`,"bind:value":caseActions[tc.id].editTitle,"disabled":action.busy,});/*Ωignore_startΩ*/() => caseActions[tc.id].editTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                         }
                         { svelteHTML.createElement("div", { "class":`edit-field edit-row`,});
                           { svelteHTML.createElement("div", { "class":`edit-field-sm`,});
                             { svelteHTML.createElement("label", {   "class":`field-label`,"for":`ef-cat-${tc.id}`,});  }
                             { svelteHTML.createElement("select", {         "id":`ef-cat-${tc.id}`,"class":`sel`,"bind:value":caseActions[tc.id].editCategory,"disabled":action.busy,});/*Ωignore_startΩ*/() => caseActions[tc.id].editCategory = __sveltets_2_any(null);/*Ωignore_endΩ*/
                                 for(let c of __sveltets_2_ensureArray(CATEGORIES)){c;
                                 { svelteHTML.createElement("option", { "value":c,});CATEGORY_LABELS[c]; }
                              }
                             }
                           }
                           { svelteHTML.createElement("div", { "class":`edit-field-sm`,});
                             { svelteHTML.createElement("label", {   "class":`field-label`,"for":`ef-pri-${tc.id}`,});  }
                             { svelteHTML.createElement("select", {         "id":`ef-pri-${tc.id}`,"class":`sel`,"bind:value":caseActions[tc.id].editPriority,"disabled":action.busy,});/*Ωignore_startΩ*/() => caseActions[tc.id].editPriority = __sveltets_2_any(null);/*Ωignore_endΩ*/
                               { svelteHTML.createElement("option", { "value":`high`,});  }
                               { svelteHTML.createElement("option", { "value":`medium`,});  }
                               { svelteHTML.createElement("option", { "value":`low`,});  }
                             }
                           }
                         }
                         { svelteHTML.createElement("div", { "class":`edit-field`,});
                           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`ef-pre-${tc.id}`,});    }
                           { svelteHTML.createElement("textarea", {              "dir":`auto`,"id":`ef-pre-${tc.id}`,"class":`text-area`,"rows":3,"bind:value":caseActions[tc.id].editPreconditions,"disabled":action.busy,"placeholder":`One precondition per line`,});/*Ωignore_startΩ*/() => caseActions[tc.id].editPreconditions = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                         }
                         { svelteHTML.createElement("div", { "class":`edit-field`,});
                           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`ef-steps-${tc.id}`,});    }
                           { svelteHTML.createElement("textarea", {              "dir":`auto`,"id":`ef-steps-${tc.id}`,"class":`text-area`,"rows":4,"bind:value":caseActions[tc.id].editSteps,"disabled":action.busy,"placeholder":`One step per line`,});/*Ωignore_startΩ*/() => caseActions[tc.id].editSteps = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                         }
                         { svelteHTML.createElement("div", { "class":`edit-field`,});
                           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`ef-exp-${tc.id}`,});  }
                           { svelteHTML.createElement("textarea", {              "dir":`auto`,"id":`ef-exp-${tc.id}`,"class":`text-area`,"rows":2,"bind:value":caseActions[tc.id].editExpected,"disabled":action.busy,"placeholder":`Expected outcome`,});/*Ωignore_startΩ*/() => caseActions[tc.id].editExpected = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
                         }
                         { svelteHTML.createElement("div", { "class":`edit-field`,});
                           { svelteHTML.createElement("label", {   "class":`field-label`,"for":`ef-note-${tc.id}`,});   }
                           { svelteHTML.createElement("input", {              "dir":`auto`,"id":`ef-note-${tc.id}`,"class":`text-input`,"type":`text`,"bind:value":caseActions[tc.id].reviewNote,"disabled":action.busy,"placeholder":`Optional note`,});/*Ωignore_startΩ*/() => caseActions[tc.id].reviewNote = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                         }
                       }
                       { svelteHTML.createElement("div", { "class":`if-actions`,});
                         { svelteHTML.createElement("button", {       "class":`btn small primary`,"onclick":() => submitEdit(tc),"disabled":action.busy,});
                          action.busy ? 'Saving…' : 'Save changes';
                         }
                         { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":() => cancelAction(tc.id),"disabled":action.busy,});
                          
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

      if(activeCases.length === 0){
         { svelteHTML.createElement("div", { "class":`muted`,});      }
      }

    } else if (!product.loadingTestcases){
       { svelteHTML.createElement("div", { "class":`muted`,});         }
    }else{
        { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {  "what":`test cases`,"loading":true,"empty":true,}});}
    }
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TestCasesTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TestCasesTab__SvelteComponent_ = ReturnType<typeof TestCasesTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default TestCasesTab__SvelteComponent_;