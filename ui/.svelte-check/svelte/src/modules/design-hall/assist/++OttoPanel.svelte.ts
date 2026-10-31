///<reference types="svelte" />
;
import { announceShown } from '../../../lib/announce.svelte';
// The Otto tab of an open design (every studio — the assist pipeline works
// on any text/JSON format):
//
//   ┌ Otto (claude ▾)  ● Working · View live session ┐  agent + live state
//   │ thread: You › Otto (AGENT) … provenance chips   │  turns + variant runs
//   │         variants tray: Apply · Compare · ✕ why  │
//   ├ context: Section · Brand kit · LOY-142 · refs   │  what Otto will see
//   │ quick actions                                   │
//   └ composer (⌘Enter)                               ┘
//
// Agents are visible and ask first: every answer is attributed, a committed
// turn is a NEW version (Compare → Restore the previous one), variants wait
// for the person to apply one, conflicts are set aside, never clobbered.
// Live state comes from `design_assist_updated` / `design_variants_ready`
// (designAssistBus); the full turn is re-read when one finishes. The UI
// posts only the decisions the daemon can't see: a variant rejected with a
// reason, an accessibility fix the person asked for, a finding dismissed, a
// conflict draft set aside. Accepts, drafts and edit-after-draft are
// recorded server-side.
import { untrack } from 'svelte';
import { toastError } from '../../../lib/toastError';
import { loadErrorText } from '../../../lib/loadError';
import Icon, { asIcon } from '../../../lib/components/Icon.svelte';
import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
import AgentChip from '../../../lib/components/AgentChip.svelte';
import StatusDot from '../../../lib/components/StatusDot.svelte';
import LiveWorkingDot from '../../../lib/components/LiveWorkingDot.svelte';
import ModelPicker from '../../../lib/components/ModelPicker.svelte';
import Skeleton from '../../../lib/components/Skeleton.svelte';
import { ctxMenu } from '../../../lib/contextmenu.svelte';
import { confirmer } from '../../../lib/confirm.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { router } from '../../../lib/router.svelte';
import { ws } from '../../../lib/stores/workspace.svelte';
import { designAssistBus, events } from '../../../lib/events.svelte';
import { ApiError } from '../../../lib/api/client';
import { captureSignal, listSignals } from '../../../lib/api/design';
import { agentProvidersWith, defaultAgentProvider } from '../../../lib/providers';
import type { DesignArtifact, DesignAssistTurn, DesignVariantRun, DesignVersion } from '../../../lib/api/types';
import type { CompareSide } from '../CompareModal.svelte';
import type { LinkRow } from '../model';
import { library } from '../library.svelte';
import { asks } from './asks.svelte';
import { acceptVariant, getLearned, listTurns, listVariantRuns } from './api';
import { interruptSession } from '../../../lib/api/interrupt';
import { askError, sendAsk } from './run';
import {
    QUICK_ACTIONS, REJECT_REASONS,
    applyAssistEvent,
    buildThread,
    busyTurn,
    directionName,
    fixRequest,
    isTerminal,
    a11yFixesOf,
    mergeTurns,
    parseBranch,
    promptRequest,
    quickActionRequest,
    rejectSignal,
    type AssistRequest,
    type AssistSelection,
    type Finding,
    type ProvChip,
    type QuickActionId,
    type VariantCard,
  } from './model';
import TurnMessage from './TurnMessage.svelte';
import VariantsTray from './VariantsTray.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
;

  interface Props {
    artifact: DesignArtifact;
    versions: DesignVersion[];
    head: DesignVersion | null;
    /** Outgoing links (Uses) — the context Otto is given besides the selection. */
    uses: LinkRow[];
    /** The focused node/section (Site Studio section, 3D object…), if any. */
    selection?: AssistSelection | null;
    /** Why the person can't ask Otto here (no edit access, imported, binary). */
    readonlyReason?: string | null;
    /** Unsaved local edits — Otto works on the saved version. */
    dirty?: boolean;
    oncompare: (left: CompareSide, right: CompareSide) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { artifact, versions, head, uses, selection = null, readonlyReason = null, dirty = false, oncompare }: Props = $props();

  const readonly = $derived(!!readonlyReason);

  // ── State ─────────────────────────────────────────────────────────────────
  let turns = $state<DesignAssistTurn[]>([]);
  let runs = $state<DesignVariantRun[]>([]);
  let phase = $state<'loading' | 'ready' | 'error'>('loading');
  let loadError = $state<string | null>(null);
  let sending = $state(false);
  let text = $state('');
  let applying = $state<string | null>(null);
  /** version id → reason label (this person's 👎 on a variant). */
  let rejected = $state<Record<string, string>>({});
  let dismissed = $state<Set<string>>(new Set());
  let keptCurrent = $state<Set<string>>(new Set());
  let appliedNote = $state<string | null>(null);
  let ruleTexts = $state<Record<string, string>>({});
  /** Findings a fix turn was started for (posted as `a11y_fix` when it lands). */
  const pendingFix = new Map<string, Finding[]>();

  // Agent picker: the stored assist session's provider (a main turn resumes it
  // when the provider matches), else the workspace/app default.
  const storedProvider = $derived.by(() => {
    const a = artifact.meta?.assist as { provider?: unknown } | undefined;
    return typeof a?.provider === 'string' && a.provider ? a.provider : null;
  });
  let provider = $state('');
  let model = $state('');
  let pickerOpen = $state(false);
  const providers = $derived(agentProvidersWith(provider || storedProvider));
  const effectiveProvider = $derived(provider || storedProvider || defaultAgentProvider());

  // The selection chip is removable per ask; a new selection brings it back.
  let useSelection = $state(true);
  $effect(() => {
    void selection?.node_id;
    useSelection = true;
  });
  const sel = $derived(useSelection ? selection : null);

  // ── Loading (sequence-guarded) ────────────────────────────────────────────
  let turnsSeq = 0;
  let runsSeq = 0;
  async function loadTurns(): Promise<void> {
    const my = ++turnsSeq;
    const known = new Set(turns.map((t) => t.turn_id)); // authoritative for these
    const fetched = await listTurns(artifact.id);
    if (my !== turnsSeq) return;
    turns = mergeTurns(turns, fetched, known);
  }
  async function loadRuns(): Promise<void> {
    const my = ++runsSeq;
    const r = await listVariantRuns(artifact.id);
    if (my !== runsSeq) return;
    runs = r;
  }
  async function loadAll(): Promise<void> {
    try {
      await Promise.all([loadTurns(), loadRuns()]);
      phase = 'ready';
      loadError = null;
    } catch (e) {
      if (phase !== 'ready') phase = 'error';
      loadError = loadErrorText(e);
    }
  }
  async function loadRejections(): Promise<void> {
    try {
      const s = await listSignals({ artifact_id: artifact.id, kind: 'variant_rejected', limit: 200 });
      const next: Record<string, string> = {};
      for (const x of s) {
        const p = x.payload ?? {};
        if (p.source !== 'variant_tray' || !x.version_id) continue;
        next[x.version_id] = REJECT_REASONS.find((r) => r.id === p.reason)?.label ?? 'Other';
      }
      rejected = { ...next, ...rejected };
    } catch {
      /* best effort: the tray just doesn't mark earlier rejections */
    }
  }
  async function loadRules(): Promise<void> {
    try {
      const l = await getLearned(artifact.workspace_id);
      ruleTexts = Object.fromEntries(l.active.map((r) => [r.key, r.rule]));
    } catch {
      /* keys are shown instead of rule text */
    }
  }

  $effect(() => {
    void artifact.id;
    untrack(() => {
      turns = [];
      runs = [];
      phase = 'loading';
      void loadAll();
      void loadRejections();
      void loadRules();
    });
  });
  $effect(() => {
    const t = designAssistBus.resyncTick;
    if (t === 0) return;
    untrack(() => void loadAll());
  });

  // ── Live ──────────────────────────────────────────────────────────────────
  let runsTimer: ReturnType<typeof setTimeout> | null = null;
  function runsSoon(): void {
    if (runsTimer) clearTimeout(runsTimer);
    runsTimer = setTimeout(() => {
      runsTimer = null;
      void loadRuns().catch(() => {});
    }, 300);
  }
  $effect(() => () => {
    if (runsTimer) clearTimeout(runsTimer);
  });

  let seen = designAssistBus.seq;
  $effect(() => {
    const now = designAssistBus.seq;
    untrack(() => {
      const evs = designAssistBus.since(seen);
      seen = now;
      let refetch = false;
      const finished: string[] = [];
      for (const ev of evs) {
        if (ev.artifact_id !== artifact.id) continue;
        if (ev.type === 'design_variants_ready') {
          runsSoon();
          continue;
        }
        const r = applyAssistEvent(turns, ev, new Date().toISOString());
        turns = r.turns;
        if (ev.mode === 'variant' || parseBranch(ev.branch)) runsSoon();
        if (r.refetch) {
          refetch = true;
          finished.push(ev.turn_id);
        }
      }
      if (refetch) void loadTurns().then(() => afterFinish(finished), () => {});
    });
  });

  /** A fix turn the person asked for landed → the accessibility fixes it made
   *  are accepted (`a11y_fix`, the learning extractor's input). */
  function afterFinish(ids: string[]): void {
    for (const id of ids) {
      const t = turns.find((x) => x.turn_id === id);
      const ask = asks.get(id);
      if (!t || t.status !== 'done' || ask?.intent !== 'fix_a11y') continue;
      for (const f of a11yFixesOf(t, pendingFix.get(id))) {
        captureSignal({
          artifact_id: artifact.id,
          kind: 'a11y_fix',
          version_id: t.version_id ?? undefined,
          session_id: t.session_id ?? undefined,
          payload: { source: 'otto_panel', rule: f.rule, disposition: 'accepted', turn_id: t.turn_id, finding: f.message.slice(0, 200) },
        });
      }
      pendingFix.delete(id);
    }
  }

  // ── Derived view ──────────────────────────────────────────────────────────
  const thread = $derived(buildThread(turns, runs));
  const running = $derived(busyTurn(turns));
  const runningRun = $derived(runs.find((r) => r.status === 'running') ?? null);
  const busy = $derived(!!running || !!runningRun);
  const versionOf = (id: string | null) => (id ? (versions.find((v) => v.id === id) ?? null) : null);
  const liveSession = $derived(running?.session_id ?? runningRun?.turns.find((t) => t.session_id && !isTerminal(t.status))?.session_id ?? null);

  // Stop: interrupt the live turn's agent (Esc into its PTY) — the turn then
  // settles through its normal status events; the session itself is kept.
  let stopping = $state(false);
  async function stopTurn(): Promise<void> {
    const sid = liveSession;
    if (!sid || stopping) return;
    stopping = true;
    try {
      await interruptSession(sid);
      toasts.info('Stopped Otto', 'The agent’s current turn was interrupted.');
    } catch (e) {
      toastError('Couldn’t stop Otto', e);
    } finally {
      stopping = false;
    }
  }
  const stale = $derived(events.state !== 'connected');

  // Context Otto is given besides the ask (the server builds it from links;
  // the chips say what it will see — they aren't removable because the
  // context brief always includes them).
  const brandKit = $derived.by(() => {
    const p = library.projectOf(artifact.project_id);
    if (p?.brand_kit_id) return { id: p.brand_kit_id, title: library.hitOf(p.brand_kit_id)?.artifact.title ?? 'Brand kit' };
    const t = uses.find((r) => r.link.rel === 'uses_tokens' && r.other);
    return t?.other ? { id: t.other.id, title: t.other.title } : null;
  });
  const stories = $derived(uses.filter((r) => r.link.rel === 'implements' && r.link.dst_kind === 'story'));
  const refs = $derived(uses.filter((r) => (r.link.rel === 'references' || r.link.rel === 'derived_from') && r.other));

  // ── Sending ───────────────────────────────────────────────────────────────
  function ctx() {
    return { selection: sel, provider: provider || undefined, model: model || undefined };
  }
  const selLabel = $derived(sel ? `Section: ${sel.label ?? sel.node_id}` : null);

  async function send(req: AssistRequest, onSent?: (key: string) => void): Promise<void> {
    if (readonly || sending) return;
    if (busy) {
      toasts.warn('Otto is still working', 'One turn per design at a time. Wait for it to finish, or open its live session.');
      return;
    }
    if (dirty) {
      const ok = await confirmer.ask(
        `You have unsaved edits. Otto works on the saved version${head ? ` v${head.seq}` : ''}, so its result won’t include them — you’ll choose between the two when you save.`,
        { title: 'Ask Otto without your edits?', confirmLabel: 'Ask Otto', danger: false },
      );
      if (!ok) return;
    }
    sending = true;
    appliedNote = null;
    try {
      const sent = await sendAsk(artifact.id, req, selLabel);
      if (sent.kind === 'turn') {
        turns = mergeTurns(turns, [sent.turn]);
        onSent?.(sent.turn.turn_id);
      } else {
        runs = [sent.run, ...runs.filter((r) => r.run_id !== sent.run.run_id)];
        onSent?.(sent.run.run_id);
      }
      text = '';
    } catch (e) {
      toasts.error('Otto couldn’t start', askError(e));
    } finally {
      sending = false;
    }
  }

  function sendText(): void {
    if (!text.trim()) return;
    void send(promptRequest(text, ctx()));
  }
  function quick(id: QuickActionId): void {
    void send(quickActionRequest(id, { ...ctx(), text }));
  }
  function fix(findings: Finding[], a11y: boolean): void {
    const req = fixRequest(findings, a11y, ctx());
    void send(req, (key) => {
      if (a11y) pendingFix.set(key, findings);
    });
  }
  function onComposerKey(e: KeyboardEvent): void {
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      sendText();
    }
  }

  // ── Findings / conflicts ──────────────────────────────────────────────────
  function dismiss(turn: DesignAssistTurn, f: Finding): void {
    dismissed = new Set([...dismissed, `${turn.turn_id}:${f.index}`]);
    captureSignal({
      artifact_id: artifact.id,
      kind: 'critique_finding',
      version_id: turn.base_version_id ?? undefined,
      payload: { source: 'otto_panel', disposition: 'dismissed', rule: f.rule ?? undefined, severity: f.severity, turn_id: turn.turn_id, finding: f.message.slice(0, 200) },
    });
  }

  /** Turns whose conflict draft is being applied — a double-click (or a
   *  second click during the confirm) must not create two forced versions. */
  let applyingConflict = $state<ReadonlySet<string>>(new Set());
  async function applyConflict(turn: DesignAssistTurn): Promise<void> {
    if (!turn.version_id || applyingConflict.has(turn.turn_id)) return;
    applyingConflict = new Set([...applyingConflict, turn.turn_id]);
    try {
      await applyConflictNow(turn, turn.version_id);
    } finally {
      const next = new Set(applyingConflict);
      next.delete(turn.turn_id);
      applyingConflict = next;
    }
  }
  async function applyConflictNow(turn: DesignAssistTurn, versionId: string): Promise<void> {
    const ok = await confirmer.ask(
      `Apply Otto’s draft on top of ${head ? `v${head.seq}` : 'the current version'}? It becomes a new version; ${head ? `v${head.seq}` : 'the current version'} and everything before it stay in history.`,
      { title: 'Apply Otto’s draft', confirmLabel: 'Apply on top', danger: false },
    );
    if (!ok) return;
    try {
      const res = await acceptVariant(artifact.id, versionId, true);
      appliedNote = `Applied Otto’s draft as v${res.version.seq}.`;
      void loadRuns().catch(() => {});
    } catch (e) {
      toastError('Couldn’t apply the draft', e);
    }
  }
  function keepCurrent(turn: DesignAssistTurn): void {
    keptCurrent = new Set([...keptCurrent, turn.turn_id]);
    const v = versionOf(turn.version_id);
    captureSignal({
      artifact_id: artifact.id,
      kind: 'variant_rejected',
      version_id: turn.version_id ?? undefined,
      payload: { source: 'assist_conflict', turn_id: turn.turn_id, rejected_version_id: turn.version_id, rejected_seq: v?.seq, kept_version_id: head?.id },
    });
  }

  // ── Variants ──────────────────────────────────────────────────────────────
  async function applyVariant(run: DesignVariantRun, card: VariantCard): Promise<void> {
    if (!card.version || applying) return;
    const name = directionName(card.direction);
    if (dirty) {
      const ok = await confirmer.ask(
        `You have unsaved edits. Applying “${name}” saves it as a new version; your edits stay in the editor and you’ll choose between the two when you save.`,
        { title: 'Apply variant', confirmLabel: `Apply ${name}`, danger: false },
      );
      if (!ok) return;
    }
    applying = card.version.id;
    try {
      let res;
      try {
        res = await acceptVariant(artifact.id, card.version.id);
      } catch (e) {
        if (!(e instanceof ApiError && e.status === 409 && /moved/i.test(e.message))) throw e;
        const ok = await confirmer.ask(
          `Someone saved a newer version after Otto drew these variants. Apply “${name}” on top? The current version stays in history — nothing is lost.`,
          { title: 'Apply variant', confirmLabel: 'Apply on top', danger: false },
        );
        if (!ok) return;
        res = await acceptVariant(artifact.id, card.version.id, true);
      }
      appliedNote = `Applied ${name} as v${res.version.seq}. Otto noted your pick — repeated choices become team rules you approve.`;
      runs = runs.map((r) => (r.run_id === run.run_id ? { ...r, status: 'accepted' as const, accepted_version_id: res.accepted_version_id } : r));
      void loadRuns().catch(() => {});
    } catch (e) {
      toastError('Couldn’t apply the variant', e);
    } finally {
      applying = null;
    }
  }
  function compareVariant(card: VariantCard): void {
    if (!card.version || !head) return;
    oncompare({ artifact, versionId: head.id }, { artifact, versionId: card.version.id });
  }
  function rejectMenu(e: MouseEvent, run: DesignVariantRun, card: VariantCard): void {
    ctxMenu.show(e, [
      { label: `Why not ${directionName(card.direction)}?`, disabled: true },
      ...REJECT_REASONS.map((r) => ({
        label: r.label,
        action: () => {
          if (!card.version) return;
          rejected = { ...rejected, [card.version.id]: r.label };
          captureSignal(rejectSignal({ artifactId: artifact.id, card, runId: run.run_id, reason: r.id }));
          toasts.info('Thanks — noted', 'This feeds your team’s design memory. Nothing changes until you approve a rule.');
        },
      })),
    ]);
  }

  // ── Misc ──────────────────────────────────────────────────────────────────
  function openRef(c: ProvChip): void {
    if (c.artifactId) router.go(`design/a/${encodeURIComponent(c.artifactId)}`);
  }
  function compareVersions(left: string, right: string): void {
    oncompare({ artifact, versionId: left }, { artifact, versionId: right });
  }
  function openSession(id: string): void {
    ws.navigateToSession(id);
  }
  const ruleText = (k: string) => ruleTexts[k] ?? null;

  // Keep the newest message in view as the thread grows.
  let threadEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    void thread.length;
    void running?.status;
    const el = threadEl;
    if (el) requestAnimationFrame(() => (el.scrollTop = el.scrollHeight));
  });

  const placeholder = $derived(sel ? 'Ask Otto to change the selected section…' : 'Ask Otto to change this design…');
  const brief = $derived(typeof artifact.meta?.brief === 'string' ? (artifact.meta.brief as string) : null);
;
async () => {

 { svelteHTML.createElement("div", {   "class":`otto`,...__sveltets_2_empty({"data-testid":`design-otto-panel`}),});
   { svelteHTML.createElement("header", { "class":`agent`,});
     { svelteHTML.createElement("div", { "class":`line`,});
       { const $$_nocIredivorP3C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP3C({ target: __sveltets_2_any(), props: {    "provider":effectiveProvider,"size":18,}});}
       { svelteHTML.createElement("button", {         "class":`picker`,"onclick":() => (pickerOpen = !pickerOpen),"aria-expanded":pickerOpen,"title":`Choose the agent for the next turn`,...__sveltets_2_empty({"data-testid":`design-otto-agent`}),});
         { svelteHTML.createElement("strong", {});  }  { svelteHTML.createElement("span", { "class":`dim`,});effectiveProvider;model ? ` · ${model}` : ''; }
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":pickerOpen ? 'chevronUp' : 'chevronDown',"size":12,}});}
       }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
      if(liveSession){
         { svelteHTML.createElement("button", {     "class":`linkbtn`,"onclick":() => openSession(liveSession!),...__sveltets_2_empty({"data-testid":`design-otto-live`}),});   }
         { svelteHTML.createElement("button", {         "class":`linkbtn`,"onclick":() => void stopTurn(),"disabled":stopping,"aria-busy":stopping,...__sveltets_2_empty({"data-testid":`design-otto-stop`}),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});} stopping ? 'Stopping…' : 'Stop'; }
      }
     }
     { svelteHTML.createElement("div", {   "class":`state`,"role":`status`,});
      if(stale){
         { svelteHTML.createElement("span", { "class":`warnc`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}      }
      } else if (running || runningRun){
         { const $$_toDgnikroWeviL3C = __sveltets_2_ensureComponent(LiveWorkingDot); new $$_toDgnikroWeviL3C({ target: __sveltets_2_any(), props: {}});} runningRun && !running ? 'Drawing variants' : running?.status === 'starting' ? 'Starting' : 'Working'; 
        head ? `v${head.seq}` : 'this design';
      }else{
         { const $$_toDsutatS3C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS3C({ target: __sveltets_2_any(), props: {  "status":`idle`,}});} `Ready${storedProvider ? ` · resumes its ${storedProvider} session` : ''}`;
      }
     }
    if(pickerOpen){
       { svelteHTML.createElement("div", { "class":`agent-settings`,});
         { svelteHTML.createElement("label", { "class":`field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", {       "class":`input`,"value":effectiveProvider,"onchange":(e) => { provider = (e.currentTarget as HTMLSelectElement).value; model = ''; },"aria-label":`Agent provider`,});
               for(let p of __sveltets_2_ensureArray(providers)){p; { svelteHTML.createElement("option", { "value":p,});p; }}
           }
         }
         { const $$_rekciPledoM3C = __sveltets_2_ensureComponent(ModelPicker); new $$_rekciPledoM3C({ target: __sveltets_2_any(), props: {      "provider":effectiveProvider,"value":model,"onchange":(m) => (model = m),}});}
         { svelteHTML.createElement("p", { "class":`hint`,});                }
       }
    }
   }

   { const $$_div1 = svelteHTML.createElement("div", {      "class":`thread`,"aria-label":`Conversation with Otto`,...__sveltets_2_empty({"data-testid":`design-otto-thread`}),});threadEl = $$_div1;
    if(phase === 'loading'){
       { const $$_notelekS2C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS2C({ target: __sveltets_2_any(), props: {    "rows":3,"height":48,}});}
    } else if (phase === 'error'){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`Otto’s turns`,"error":loadError || 'No details were reported.',"empty":true,"onretry":() => void loadAll(),}});}
    } else if (thread.length === 0){
       { svelteHTML.createElement("div", { "class":`intro`,});
        if(brief){
           { svelteHTML.createElement("p", { "class":`brief`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});}  { svelteHTML.createElement("strong", {});  } brief; }
        }
         { svelteHTML.createElement("p", {});                       }
         { svelteHTML.createElement("p", { "class":`dim`,});                   }
       }
    }else{
         for(let item of __sveltets_2_ensureArray(thread)){item.kind === 'turn' ? `t:${item.turn.turn_id}` : `r:${item.run.run_id}`;
        if(item.kind === 'turn'){
           { const $$_egasseMnruT2C = __sveltets_2_ensureComponent(TurnMessage); new $$_egasseMnruT2C({ target: __sveltets_2_any(), props: {                               "turn":item.turn,"ask":asks.get(item.turn.turn_id),"version":versionOf(item.turn.version_id),"base":versionOf(item.turn.base_version_id),"headSeq":head?.seq ?? null,ruleText,dismissed,"keptCurrent":keptCurrent.has(item.turn.turn_id),readonly,busy,"onopenref":openRef,"oncompare":compareVersions,"onfix":fix,"ondismiss":(f) => dismiss(item.turn, f),"onapplyconflict":() => void applyConflict(item.turn),"onkeepcurrent":() => keepCurrent(item.turn),"onopensession":openSession,}});}
        }else{
          const run = item.run;
          const ask = asks.get(run.run_id);
           { svelteHTML.createElement("div", {   "class":`exchange`,...__sveltets_2_empty({"data-testid":`design-variants-run`}),});
            if(ask){
               { svelteHTML.createElement("div", { "class":`msg human`,});
                 { svelteHTML.createElement("div", { "class":`who`,}); { svelteHTML.createElement("span", {   "class":`av`,"aria-hidden":`true`,});  }  { svelteHTML.createElement("strong", {});  } }
                if(ask.selectionLabel){ { svelteHTML.createElement("span", { "class":`chip ctx`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`target`,"size":12,}});} ask.selectionLabel; }}
                 { svelteHTML.createElement("p", {   "class":`bubble`,"title":ask.display && ask.display !== ask.prompt ? ask.prompt : undefined,});ask.display ?? ask.prompt; }
               }
            }
             { svelteHTML.createElement("div", { "class":`msg agent`,});
               { svelteHTML.createElement("div", { "class":`who`,});
                 { const $$_nocIredivorP5C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP5C({ target: __sveltets_2_any(), props: {    "provider":run.turns[0]?.provider || effectiveProvider,"size":16,}});}
                 { svelteHTML.createElement("strong", {});  }  { const $$_pihCtnegA5C = __sveltets_2_ensureComponent(AgentChip); new $$_pihCtnegA5C({ target: __sveltets_2_any(), props: {}});}
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                 { svelteHTML.createElement("span", { "class":`st`,});
                  if(run.status === 'running'){ { const $$_toDgnikroWeviL6C = __sveltets_2_ensureComponent(LiveWorkingDot); new $$_toDgnikroWeviL6C({ target: __sveltets_2_any(), props: {  "label":`Drawing`,}});}} else if (run.status === 'accepted'){ }else{   }
                 }
               }
               { svelteHTML.createElement("p", { "class":`body`,});
                run.versions.length + run.turns.filter((t) => !isTerminal(t.status)).length <= 1 && run.status !== 'running'
                  ? 'Otto’s draft, kept aside. The current version is untouched until you apply it.'
                  : `Here are ${Math.max(run.versions.length, run.turns.length)} directions. The current version stays until you apply one.`;
               }
               { const $$_yarTstnairaV4C = __sveltets_2_ensureComponent(VariantsTray); new $$_yarTstnairaV4C({ target: __sveltets_2_any(), props: {            artifact,run,rejected,applying,readonly,"onapply":(c) => void applyVariant(run, c),"oncompare":compareVariant,"onreject":(e, c) => rejectMenu(e, run, c),}});}
             }
           }
        }
      }
    }
    if(appliedNote){
       {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),(appliedNote)));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`applied`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} appliedNote;  { svelteHTML.createElement("a", { "href":`#/design/learned`,});    } }}
    }
   }

   { svelteHTML.createElement("footer", { "class":`compose`,});
    if(readonlyReason){
       { svelteHTML.createElement("p", { "class":`ro`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});} readonlyReason; }
    }else{
       { svelteHTML.createElement("div", {   "class":`ctx-chips`,"aria-label":`Context Otto will see`,});
        if(sel){
           { svelteHTML.createElement("span", {   "class":`chip ctx`,...__sveltets_2_empty({"data-testid":`design-otto-selection`}),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`target`,"size":12,}});}  { svelteHTML.createElement("span", {   "class":`ct`,"title":selLabel,});selLabel; }
             { svelteHTML.createElement("button", {       "class":`x`,"onclick":(e) => { const box = (e.currentTarget as HTMLElement).closest('footer')?.querySelector<HTMLTextAreaElement>('textarea'); useSelection = false; box?.focus(); },"aria-label":`Don’t focus the selection`,"title":`Don’t focus the selection`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
           }
        }
        if(brandKit){
           { svelteHTML.createElement("span", {   "class":`chip ctx`,"title":`The brand kit is always part of Otto’s context brief`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`palette`,"size":12,}});}  { svelteHTML.createElement("span", { "class":`ct`,});brandKit.title; } }
        }
           for(let s of __sveltets_2_ensureArray(stories)){s.link.id;
           { svelteHTML.createElement("span", {   "class":`chip ctx`,"title":`The story this design implements is always part of the brief`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`ticket`,"size":12,}});}  { svelteHTML.createElement("span", { "class":`ct`,});s.label; } }
        }
           for(let r of __sveltets_2_ensureArray((refs.slice(0, 4)))){r.link.id;
           { svelteHTML.createElement("span", {   "class":`chip ctx`,"title":`Linked as a reference — Otto may cite it as [R…]`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`link`,"size":12,}});}  { svelteHTML.createElement("span", { "class":`ct`,});r.label; } }
        }
        if(refs.length > 4){ { svelteHTML.createElement("span", { "class":`chip ctx`,}); refs.length - 4;  }}
       }
       { svelteHTML.createElement("div", {     "class":`quick scroll-thin`,"role":`group`,"aria-label":`Quick actions`,});
           for(let q of __sveltets_2_ensureArray(QUICK_ACTIONS)){q.id;
           { svelteHTML.createElement("button", {         "class":`chip as-btn`,"disabled":busy || sending,"title":q.hint,"onclick":() => quick(q.id),...__sveltets_2_empty({"data-testid":`design-quick-${q.id}`}),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":asIcon(q.icon),"size":12,}});} q.label;
           }
        }
       }
       { svelteHTML.createElement("div", { "class":`box`,});
         { svelteHTML.createElement("textarea", {               "dir":`auto`,"class":`input`,"rows":2,"bind:value":text,placeholder,"aria-label":`Ask Otto`,"onkeydown":onComposerKey,...__sveltets_2_empty({"data-testid":`design-otto-input`}),});/*Ωignore_startΩ*/() => text = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         { svelteHTML.createElement("button", {             "class":`send btn primary small`,"disabled":!text.trim() || busy || sending,"onclick":sendText,"aria-label":`Send to Otto`,"title":`Send to Otto`,"aria-keyshortcuts":`Meta+Enter`,...__sveltets_2_empty({"data-testid":`design-otto-send`}),});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`send`,"size":12,}});}
         }
       }
      if(dirty){ { svelteHTML.createElement("p", { "class":`hint warnc`,});           }}
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const OttoPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type OttoPanel__SvelteComponent_ = ReturnType<typeof OttoPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default OttoPanel__SvelteComponent_;