<script lang="ts">
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
  import Icon, { asIcon } from '../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import StatusDot from '../../../lib/components/StatusDot.svelte';
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
  import { askError, sendAsk } from './run';
  import {
    QUICK_ACTIONS,
    REJECT_REASONS,
    applyAssistEvent,
    buildThread,
    busyTurn,
    directionName,
    fixRequest,
    isTerminal,
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
  }
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
    const fetched = await listTurns(artifact.id);
    if (my !== turnsSeq) return;
    turns = mergeTurns(turns, fetched);
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
      loadError = e instanceof Error ? e.message : String(e);
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
      const fixed = t.findings
        .filter((f) => f.fixed !== false && typeof f.rule === 'string' && f.rule)
        .map((f) => ({ rule: f.rule as string, message: typeof f.message === 'string' ? f.message : '' }));
      const list = fixed.length ? fixed : (pendingFix.get(id) ?? []).filter((f) => f.rule).map((f) => ({ rule: f.rule!, message: f.message }));
      for (const f of list.slice(0, 12)) {
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

  async function applyConflict(turn: DesignAssistTurn): Promise<void> {
    if (!turn.version_id) return;
    const ok = await confirmer.ask(
      `Apply Otto’s draft on top of ${head ? `v${head.seq}` : 'the current version'}? It becomes a new version; ${head ? `v${head.seq}` : 'the current version'} and everything before it stay in history.`,
      { title: 'Apply Otto’s draft', confirmLabel: 'Apply on top', danger: false },
    );
    if (!ok) return;
    try {
      const res = await acceptVariant(artifact.id, turn.version_id, true);
      appliedNote = `Applied Otto’s draft as v${res.version.seq}.`;
      void loadRuns().catch(() => {});
    } catch (e) {
      toasts.error('Couldn’t apply the draft', e instanceof Error ? e.message : String(e));
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
      toasts.error('Couldn’t apply the variant', e instanceof Error ? e.message : String(e));
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
    if (el) queueMicrotask(() => (el.scrollTop = el.scrollHeight));
  });

  const placeholder = $derived(sel ? 'Ask Otto to change the selected section…' : 'Ask Otto to change this design…');
  const brief = $derived(typeof artifact.meta?.brief === 'string' ? (artifact.meta.brief as string) : null);
</script>

<div class="otto" data-testid="design-otto-panel">
  <header class="agent">
    <div class="line">
      <ProviderIcon provider={effectiveProvider} size={18} />
      <button class="picker" onclick={() => (pickerOpen = !pickerOpen)} aria-expanded={pickerOpen} title="Choose the agent for the next turn"
        data-testid="design-otto-agent">
        <strong>Otto</strong> <span class="dim">{effectiveProvider}{model ? ` · ${model}` : ''}</span>
        <Icon name={pickerOpen ? 'chevronUp' : 'chevronDown'} size={11} />
      </button>
      <span class="grow"></span>
      {#if liveSession}
        <button class="linkbtn" onclick={() => openSession(liveSession!)} data-testid="design-otto-live">View live session</button>
      {/if}
    </div>
    <div class="state" role="status">
      {#if stale}
        <span class="warnc"><Icon name="warning" size={11} /> Reconnecting — live updates paused</span>
      {:else if running || runningRun}
        <StatusDot status="working" /> {runningRun && !running ? 'Drawing variants' : running?.status === 'starting' ? 'Starting' : 'Working'} on
        {head ? `v${head.seq}` : 'this design'}
      {:else}
        <StatusDot status="idle" /> Ready{#if storedProvider} · resumes its {storedProvider} session{/if}
      {/if}
    </div>
    {#if pickerOpen}
      <div class="agent-settings">
        <label class="fld">
          <span>Agent</span>
          <select class="input" value={effectiveProvider} onchange={(e) => { provider = (e.currentTarget as HTMLSelectElement).value; model = ''; }}
            aria-label="Agent provider">
            {#each providers as p (p)}<option value={p}>{p}</option>{/each}
          </select>
        </label>
        <ModelPicker provider={effectiveProvider} value={model} onchange={(m) => (model = m)} />
        <p class="hint">Applies to your next ask. A turn resumes this design’s earlier session when the agent matches.</p>
      </div>
    {/if}
  </header>

  <div class="thread" bind:this={threadEl} aria-label="Conversation with Otto" data-testid="design-otto-thread">
    {#if phase === 'loading'}
      <Skeleton rows={3} height={48} />
    {:else if phase === 'error'}
      <div class="inline-err" role="alert">
        <Icon name="warning" size={14} />
        <span>Couldn’t load Otto’s turns. <span class="dim">{loadError}</span></span>
        <button class="btn small" onclick={() => void loadAll()}>Retry</button>
      </div>
    {:else if thread.length === 0}
      <div class="intro">
        {#if brief}
          <p class="brief"><Icon name="sparkle" size={12} /> <strong>Brief</strong> {brief}</p>
        {/if}
        <p>Ask Otto to change this design. Each answer is a new version you can compare or undo; variants wait until you apply one.</p>
        <p class="dim">Otto sees the saved version, the story it implements, the brand kit, your references and your team’s approved rules.</p>
      </div>
    {:else}
      {#each thread as item (item.kind === 'turn' ? `t:${item.turn.turn_id}` : `r:${item.run.run_id}`)}
        {#if item.kind === 'turn'}
          <TurnMessage
            turn={item.turn}
            ask={asks.get(item.turn.turn_id)}
            version={versionOf(item.turn.version_id)}
            base={versionOf(item.turn.base_version_id)}
            headSeq={head?.seq ?? null}
            {ruleText}
            {dismissed}
            keptCurrent={keptCurrent.has(item.turn.turn_id)}
            {readonly}
            {busy}
            onopenref={openRef}
            oncompare={compareVersions}
            onfix={fix}
            ondismiss={(f) => dismiss(item.turn, f)}
            onapplyconflict={() => void applyConflict(item.turn)}
            onkeepcurrent={() => keepCurrent(item.turn)}
            onopensession={openSession}
          />
        {:else}
          {@const run = item.run}
          {@const ask = asks.get(run.run_id)}
          <div class="exchange" data-testid="design-variants-run">
            {#if ask}
              <div class="msg human">
                <div class="who"><span class="av" aria-hidden="true">Y</span> <strong>You</strong></div>
                {#if ask.selectionLabel}<span class="chip ctx"><Icon name="target" size={11} /> {ask.selectionLabel}</span>{/if}
                <p class="bubble" title={ask.display && ask.display !== ask.prompt ? ask.prompt : undefined}>{ask.display ?? ask.prompt}</p>
              </div>
            {/if}
            <div class="msg agent">
              <div class="who">
                <ProviderIcon provider={run.turns[0]?.provider || effectiveProvider} size={16} />
                <strong>Otto</strong> <span class="chip agent-label">AGENT</span>
                <span class="grow"></span>
                <span class="st">
                  {#if run.status === 'running'}<StatusDot status="working" /> Drawing{:else if run.status === 'accepted'}Applied{:else}Ready — pick one{/if}
                </span>
              </div>
              <p class="body">
                {run.versions.length + run.turns.filter((t) => !isTerminal(t.status)).length <= 1 && run.status !== 'running'
                  ? 'Otto’s draft, kept aside. The current version is untouched until you apply it.'
                  : `Here are ${Math.max(run.versions.length, run.turns.length)} directions. The current version stays until you apply one.`}
              </p>
              <VariantsTray
                {artifact}
                {run}
                {rejected}
                {applying}
                {readonly}
                onapply={(c) => void applyVariant(run, c)}
                oncompare={compareVariant}
                onreject={(e, c) => rejectMenu(e, run, c)}
              />
            </div>
          </div>
        {/if}
      {/each}
    {/if}
    {#if appliedNote}
      <p class="applied" role="status"><Icon name="check" size={12} /> {appliedNote} <a href="#/design/learned">See what Otto learned</a></p>
    {/if}
  </div>

  <footer class="compose">
    {#if readonlyReason}
      <p class="ro"><Icon name="lock" size={12} /> {readonlyReason}</p>
    {:else}
      <div class="ctx-chips" aria-label="Context Otto will see">
        {#if sel}
          <span class="chip ctx" data-testid="design-otto-selection">
            <Icon name="target" size={11} /> {selLabel}
            <button class="x" onclick={() => (useSelection = false)} aria-label="Don’t focus the selection" title="Don’t focus the selection"><Icon name="x" size={10} /></button>
          </span>
        {/if}
        {#if brandKit}
          <span class="chip ctx" title="The brand kit is always part of Otto’s context brief"><Icon name="palette" size={11} /> {brandKit.title}</span>
        {/if}
        {#each stories as s (s.link.id)}
          <span class="chip ctx" title="The story this design implements is always part of the brief"><Icon name="ticket" size={11} /> {s.label}</span>
        {/each}
        {#each refs.slice(0, 4) as r (r.link.id)}
          <span class="chip ctx" title="Linked as a reference — Otto may cite it as [R…]"><Icon name="link" size={11} /> {r.label}</span>
        {/each}
        {#if refs.length > 4}<span class="chip ctx">+{refs.length - 4} references</span>{/if}
      </div>
      <div class="quick" role="group" aria-label="Quick actions">
        {#each QUICK_ACTIONS as q (q.id)}
          <button class="chip as-btn" disabled={busy || sending} title={q.hint} onclick={() => quick(q.id)} data-testid={`design-quick-${q.id}`}>
            <Icon name={asIcon(q.icon)} size={11} /> {q.label}
          </button>
        {/each}
      </div>
      <div class="box">
        <textarea
          class="input"
          rows="2"
          bind:value={text}
          {placeholder}
          aria-label="Ask Otto"
          onkeydown={onComposerKey}
          data-testid="design-otto-input"
        ></textarea>
        <button class="send btn primary small" disabled={!text.trim() || busy || sending} onclick={sendText}
          aria-label="Send to Otto (⌘Enter)" title="Send (⌘Enter)" data-testid="design-otto-send">
          <Icon name="send" size={12} />
        </button>
      </div>
      {#if dirty}<p class="hint warnc">You have unsaved edits — Otto works on the saved version.</p>{/if}
    {/if}
  </footer>
</div>

<style>
  .otto {
    height: 100%;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .agent {
    padding: 10px 14px 8px;
    border-block-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .line {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .picker {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: 0;
    background: none;
    padding: 2px 4px;
    border-radius: var(--radius-s);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .picker:hover {
    background: var(--hover);
  }
  .grow {
    flex: 1;
  }
  .dim {
    color: var(--text-dim);
  }
  .state {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .warnc {
    color: var(--warning);
  }
  .agent-settings {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-block-start: 6px;
    padding: 8px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .hint {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .linkbtn {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .linkbtn:hover {
    text-decoration: underline;
  }
  .thread {
    flex: 1;
    min-height: 120px;
    overflow-y: auto;
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .intro {
    font-size: var(--fs-s);
  }
  .intro p {
    margin: 0 0 8px;
  }
  .brief {
    padding: 8px 10px;
    border-inline-start: 2px solid var(--border-strong);
    background: var(--surface-2);
    border-radius: var(--radius-s);
    white-space: pre-wrap;
  }
  .inline-err {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .inline-err > :global(svg) {
    color: var(--danger);
  }
  .exchange {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .msg {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .msg.agent {
    border-inline-start: 2px solid var(--border-strong);
    padding-inline-start: 10px;
  }
  .who {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
  }
  .av {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    display: grid;
    place-items: center;
    font-size: var(--fs-xs);
    font-weight: 700;
    background: var(--surface-3);
    color: var(--text-dim);
  }
  .agent-label {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    letter-spacing: 0.04em;
  }
  .st {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .bubble {
    margin: 0;
    align-self: flex-start;
    max-width: 100%;
    padding: 6px 10px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    font-size: var(--fs-s);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .body {
    margin: 0;
    font-size: var(--fs-s);
  }
  .ctx {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .msg .ctx {
    align-self: flex-start;
  }
  .applied {
    margin: 0;
    padding: 8px 10px;
    border-radius: var(--radius-s);
    background: var(--success-soft);
    font-size: var(--fs-s);
  }
  .applied :global(svg) {
    color: var(--success);
    vertical-align: -2px;
  }
  .applied a {
    color: var(--accent-text);
  }
  .compose {
    border-block-start: 1px solid var(--border);
    padding: 8px 14px 12px;
    display: flex;
    flex-direction: column;
    gap: 6px;
    background: var(--surface);
  }
  .ro {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .ctx-chips,
  .quick {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .quick .chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .x {
    display: inline-grid;
    place-items: center;
    border: 0;
    background: none;
    padding: 0;
    margin-inline-start: 2px;
    color: var(--text-dim);
    cursor: pointer;
  }
  .x:hover {
    color: var(--text);
  }
  .box {
    position: relative;
  }
  .box textarea {
    width: 100%;
    resize: vertical;
    min-height: 56px;
    padding-inline-end: 40px;
    box-sizing: border-box;
  }
  .send {
    position: absolute;
    inset-inline-end: 6px;
    inset-block-end: 8px;
  }
  .as-btn {
    cursor: pointer;
    font-family: inherit;
  }
  .as-btn:hover:not(:disabled) {
    color: var(--text);
    border-color: var(--border-strong);
  }
  .as-btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
</style>
