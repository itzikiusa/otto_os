<script lang="ts">
  // What Otto learned from your team (`#/design/learned[/pending|rules|memory|
  // signals|settings][/<edit id>]`). Learning v1 is suggest-only: a
  // deterministic extractor turns repeated design signals (≥ 3 across ≥ 2
  // designs) into PENDING edits of the workspace skill `design-team-style`;
  // a person approves, rejects or rolls back each through otto-improve's edit
  // flow (every decision asks first). Tabs:
  //   Pending   proposals + the patterns still forming (candidates)
  //   Rules     the approved rules (roll back) + the decision history
  //   Memory    design memories (`otto-memory` collection `design`)
  //   Signals   the raw log the rules come from
  //   Settings  the workspace setting `design_learning` (off / suggest only)
  // The evidence pane shows the signals behind the selected rule.
  import { untrack } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { router } from '../../lib/router.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { auth } from '../../lib/stores/auth.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { designBus } from '../../lib/events.svelte';
  import { ApiError } from '../../lib/api/client';
  import { downloadText } from '../../lib/components/exporters';
  import { listSignals } from '../../lib/api/design';
  import type {
    DesignLearnedEdit,
    DesignLearnedResp,
    DesignLearnedRule,
    DesignSignal,
    Memory,
  } from '../../lib/api/types';
  import { signalSummary, signalTone, type LearnedTab } from './model';
  import { library } from './library.svelte';
  import { extractRules, getLearned, listDesignMemories, setLearningMode } from './assist/api';
  import {
    assistSignalSummary,
    candidateKindLabel,
    candidateProgress,
    editHeadline,
    editStatusLabel,
    exportRulesMarkdown,
    signalKindLabel,
  } from './assist/model';
  import { approveRule, rejectRule, rollbackRule } from './assist/ruleActions';
  import EvidencePane from './assist/EvidencePane.svelte';

  interface Props {
    tab: LearnedTab;
  }
  let { tab }: Props = $props();

  const TABS: { id: LearnedTab; label: string }[] = [
    { id: 'pending', label: 'Pending' },
    { id: 'rules', label: 'Rules' },
    { id: 'memory', label: 'Memory' },
    { id: 'signals', label: 'Signals' },
    { id: 'settings', label: 'Settings' },
  ];

  const wsId = $derived(ws.currentId);
  const canEdit = $derived(auth.can('design', 'edit'));

  // ── Learned rules (sequence-guarded) ──────────────────────────────────────
  let learned = $state<DesignLearnedResp | null>(null);
  let phase = $state<'loading' | 'ready' | 'error'>('loading');
  let error = $state<string | null>(null);
  let seq = 0;
  async function load(): Promise<void> {
    const w = wsId;
    if (!w) return;
    const my = ++seq;
    if (!learned) phase = 'loading';
    try {
      const l = await getLearned(w);
      if (my !== seq) return;
      learned = l;
      phase = 'ready';
      error = null;
    } catch (e) {
      if (my !== seq) return;
      error = e instanceof Error ? e.message : String(e);
      phase = learned ? 'ready' : 'error';
    }
  }

  // ── Signals (the log + evidence lookup) ──────────────────────────────────
  const KINDS = [
    'variant_accepted',
    'variant_rejected',
    'variant_chosen',
    'agent_draft',
    'edit_after_draft',
    'a11y_fix',
    'critique_finding',
    'brand_correction',
    'review_comment',
    'rule_feedback',
    'status_change',
    'shipped',
    'restored',
    'reference_added',
    'forked',
  ] as const;
  let kind = $state<'' | (typeof KINDS)[number]>('');
  let signals = $state<DesignSignal[]>([]);
  let sigLoading = $state(true);
  let sigError = $state<string | null>(null);
  let sigSeq = 0;
  async function loadSignals(): Promise<void> {
    const w = wsId;
    if (!w) return;
    const my = ++sigSeq;
    sigLoading = true;
    try {
      const s = await listSignals({ workspace_id: w, limit: 500 });
      if (my !== sigSeq) return;
      signals = s;
      sigError = null;
    } catch (e) {
      if (my === sigSeq) sigError = e instanceof Error ? e.message : String(e);
    } finally {
      if (my === sigSeq) sigLoading = false;
    }
  }
  const byId = $derived(Object.fromEntries(signals.map((s) => [s.id, s])) as Record<string, DesignSignal>);
  const shownSignals = $derived(kind ? signals.filter((s) => s.kind === kind) : signals);

  // ── Memory ────────────────────────────────────────────────────────────────
  let memories = $state<Memory[] | null>(null);
  let memError = $state<string | null>(null);
  let memQ = $state('');
  async function loadMemory(): Promise<void> {
    const w = wsId;
    if (!w) return;
    try {
      memories = await listDesignMemories(w);
      memError = null;
    } catch (e) {
      memError =
        e instanceof ApiError && e.status === 403
          ? 'Design memory lives in Otto memory, which needs Product view access.'
          : e instanceof Error
            ? e.message
            : String(e);
    }
  }
  const shownMemories = $derived.by(() => {
    const q = memQ.trim().toLowerCase();
    const list = memories ?? [];
    return q ? list.filter((m) => `${m.title} ${m.body} ${m.kind} ${m.tags.join(' ')}`.toLowerCase().includes(q)) : list;
  });

  $effect(() => {
    void wsId;
    void designBus.resyncTick;
    untrack(() => {
      void load();
      void loadSignals();
      memories = null;
      if (!library.loaded) void library.load();
    });
  });
  $effect(() => {
    if (tab === 'memory' && memories === null) untrack(() => void loadMemory());
  });
  let seen = designBus.seq;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      const evs = designBus.since(seen);
      seen = now;
      const mine = evs.filter((e) => e.type === 'design_learning_update' && e.workspace_id === wsId);
      if (!mine.length) return;
      void loadSignals();
      void load();
    });
  });

  // ── Selection (evidence) — `#/design/learned/<tab>/<edit id | rule key>` ──
  const selectedKey = $derived(router.parts[3] || null);
  function select(key: string | null): void {
    router.go(key ? `design/learned/${tab}/${encodeURIComponent(key)}` : `design/learned/${tab}`);
  }
  const selection = $derived.by<{ title: string; rationale: string; evidence: string[] } | null>(() => {
    if (!learned || !selectedKey) return null;
    if (tab === 'pending') {
      const e = learned.pending.find((x) => x.edit_id === selectedKey);
      if (e) return { title: editHeadline(e), rationale: e.rationale, evidence: e.evidence };
      const c = learned.candidates.find((x) => x.key === selectedKey);
      if (c) return { title: c.rule, rationale: c.rationale, evidence: c.signal_ids };
    }
    if (tab === 'rules') {
      const r = learned.active.find((x) => x.key === selectedKey);
      if (r) {
        const e = learned.history.find((h) => h.edit_id === r.edit_id);
        return { title: r.rule, rationale: e?.rationale ?? '', evidence: r.evidence.length ? r.evidence : (e?.evidence ?? []) };
      }
      const h = learned.history.find((x) => x.edit_id === selectedKey);
      if (h) return { title: editHeadline(h), rationale: h.rationale, evidence: h.evidence };
    }
    return null;
  });

  // ── Actions ───────────────────────────────────────────────────────────────
  let busy = $state<string | null>(null);
  async function approve(e: DesignLearnedEdit): Promise<void> {
    busy = e.edit_id;
    if (await approveRule(e, learned?.skill)) await load();
    busy = null;
  }
  async function reject(e: DesignLearnedEdit): Promise<void> {
    busy = e.edit_id;
    if (await rejectRule(e)) await load();
    busy = null;
  }
  async function rollback(r: DesignLearnedRule): Promise<void> {
    busy = r.key;
    if (await rollbackRule(r)) await load();
    busy = null;
  }
  function ruleMenu(ev: MouseEvent, r: DesignLearnedRule): void {
    ctxMenu.show(ev, [
      { label: 'View evidence', icon: 'eye', action: () => select(r.key) },
      { separator: true },
      { label: 'Roll back…', icon: 'refresh', danger: true, disabled: !canEdit || !r.edit_id, action: () => void rollback(r) },
    ]);
  }

  let extracting = $state(false);
  async function lookNow(): Promise<void> {
    const w = wsId;
    if (!w || extracting) return;
    extracting = true;
    try {
      const r = await extractRules(w);
      if (r.mode === 'off') toasts.info('Learning is off', 'Turn it on in Settings to get rule proposals.');
      else if (r.proposed.length) toasts.success(`${r.proposed.length} new rule${r.proposed.length === 1 ? '' : 's'} to review`, 'Nothing applies until you approve it.');
      else toasts.info('No new rules', `${r.candidates.filter((c) => !c.ready).length} pattern(s) are still forming.`);
      await load();
    } catch (e) {
      toasts.error('Couldn’t look for rules', e instanceof Error ? e.message : String(e));
    } finally {
      extracting = false;
    }
  }

  let savingMode = $state(false);
  async function setMode(mode: 'suggest' | 'off'): Promise<void> {
    const w = wsId;
    if (!w || savingMode || learned?.mode === mode) return;
    savingMode = true;
    try {
      const updated = await setLearningMode(w, mode);
      ws.workspaces = ws.workspaces.map((x) => (x.id === updated.id ? { ...x, ...updated } : x));
      if (learned) learned = { ...learned, mode };
      toasts.success(mode === 'off' ? 'Learning is off' : 'Learning is on — suggest only');
    } catch (e) {
      toasts.error(
        'Couldn’t change learning mode',
        e instanceof ApiError && e.status === 403 ? 'Only workspace admins can change this.' : e instanceof Error ? e.message : String(e),
      );
    } finally {
      savingMode = false;
    }
  }
  function modeMenu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Suggest only', checked: learned?.mode !== 'off', action: () => void setMode('suggest') },
      { label: 'Off', checked: learned?.mode === 'off', action: () => void setMode('off') },
    ]);
  }

  function exportRules(): void {
    if (!learned) return;
    const name = ws.current?.name ?? 'workspace';
    downloadText(exportRulesMarkdown(learned, name), `design-team-rules-${name.replace(/[^\w.-]+/g, '-')}.md`, 'text/markdown');
  }

  // ── Tabs ──────────────────────────────────────────────────────────────────
  function go(t: LearnedTab): void {
    router.go(`design/learned/${t}`);
  }
  let tablist = $state<HTMLDivElement | null>(null);
  function onTabKey(e: KeyboardEvent): void {
    const i = TABS.findIndex((t) => t.id === tab);
    let j = -1;
    if (e.key === 'ArrowRight') j = (i + 1) % TABS.length;
    else if (e.key === 'ArrowLeft') j = (i + TABS.length - 1) % TABS.length;
    else if (e.key === 'Home') j = 0;
    else if (e.key === 'End') j = TABS.length - 1;
    if (j < 0) return;
    e.preventDefault();
    go(TABS[j].id);
    tablist?.querySelectorAll<HTMLButtonElement>('[role="tab"]')[j]?.focus();
  }
  const count = (t: LearnedTab): number | null => {
    if (t === 'pending') return learned?.pending.length ?? null;
    if (t === 'rules') return learned?.active.length ?? null;
    if (t === 'memory') return memories?.length ?? null;
    if (t === 'signals') return signals.length;
    return null;
  };

  const titleOf = (id: string) => library.hitOf(id)?.artifact.title ?? 'A design';
  function byLabel(s: DesignSignal): string {
    if (s.actor_kind === 'agent') return 'Otto';
    if (s.actor_kind === 'system') return 'System';
    return s.actor_id && s.actor_id === auth.me?.id ? 'You' : 'Teammate';
  }
  function whenLabel(iso: string): string {
    const d = new Date(iso);
    if (d.toDateString() === new Date().toDateString()) return `Today ${d.toLocaleTimeString(undefined, { hour: 'numeric', minute: '2-digit' })}`;
    return d.toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  }
  const sumOf = (s: DesignSignal) => assistSignalSummary(s.kind, s.payload) ?? signalSummary(s);
  const forming = $derived((learned?.candidates ?? []).filter((c) => !c.ready));
  const withEvidence = $derived((tab === 'pending' || tab === 'rules') && !!selection);
</script>

<PageHeader title="What Otto learned" subtitle="Rules Otto proposes from your team’s choices. Nothing applies until you approve it."
  crumbs={[{ label: 'Design Hall', onclick: () => router.go('design') }]}>
  {#snippet tabs()}
    <div class="segmented" role="tablist" aria-label="Learning" bind:this={tablist}>
      {#each TABS as t (t.id)}
        {@const n = count(t.id)}
        <button role="tab" aria-selected={tab === t.id} class:active={tab === t.id} tabindex={tab === t.id ? 0 : -1}
          onclick={() => go(t.id)} onkeydown={onTabKey} data-testid={`design-learned-tab-${t.id}`}>
          {t.label}{#if n != null}<span class="n" class:hot={t.id === 'pending' && n > 0}>{n}</span>{/if}
        </button>
      {/each}
    </div>
  {/snippet}
  {#snippet actions()}
    <button class="btn small" onclick={modeMenu} aria-haspopup="menu" disabled={!learned || savingMode} data-testid="design-learning-mode"
      title="Workspace learning mode">
      Learning: {learned?.mode === 'off' ? 'Off' : 'Suggest only'} <Icon name="chevronDown" size={12} />
    </button>
    <button class="btn small primary" onclick={() => void lookNow()} disabled={!wsId || !canEdit || extracting || learned?.mode === 'off'}
      data-testid="design-learned-extract" title="Look at recent signals and propose any rule that is ready">
      <Icon name="bulb" size={12} /> {extracting ? 'Looking…' : 'Look for rules now'}
    </button>
  {/snippet}
</PageHeader>

<PageBody>
  {#if !wsId}
    <EmptyState variant="page" icon="bulb" title="Pick a workspace" body="Team rules are learned per workspace." />
  {:else if tab === 'signals'}
    <div class="filters" role="group" aria-label="Filter by kind">
      <button class="pill-toggle" class:on={kind === ''} aria-pressed={kind === ''} onclick={() => (kind = '')}>All</button>
      {#each KINDS as k (k)}
        <button class="pill-toggle" class:on={kind === k} aria-pressed={kind === k} onclick={() => (kind = k)}>{signalKindLabel(k)}</button>
      {/each}
    </div>
    {#if sigLoading && !signals.length}
      <Skeleton rows={6} height={32} />
    {:else if sigError}
      <div class="err" role="alert">
        <Icon name="warning" size={14} /> Couldn’t load signals. <span class="dim">{sigError}</span>
        <button class="btn small" onclick={() => void loadSignals()}>Retry</button>
      </div>
    {:else if shownSignals.length === 0}
      <EmptyState variant="page" icon="bulb" title={kind ? `No “${signalKindLabel(kind)}” signals yet` : 'No signals yet'}
        body="Applying or rejecting variants, fixes you accept, edits after an agent draft, approvals and shipping are captured here as signals." />
    {:else}
      <div class="table-wrap">
        <table data-testid="design-signals">
          <thead>
            <tr><th>When</th><th>Kind</th><th>Signal</th><th>Design</th><th>By</th></tr>
          </thead>
          <tbody>
            {#each shownSignals as s (s.id)}
              <tr>
                <td class="when" title={new Date(s.created_at).toLocaleString()}>{whenLabel(s.created_at)} <span class="dim">· {rel(s.created_at)}</span></td>
                <td><span class="kind tone-{signalTone(s.kind)}">{signalKindLabel(s.kind)}</span></td>
                <td class="sum">{sumOf(s)}</td>
                <td class="art"><a href={`#/design/a/${encodeURIComponent(s.artifact_id)}`}>{titleOf(s.artifact_id)}</a></td>
                <td class="by">{byLabel(s)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
      <p class="foot dim">Signals store version references and short summaries — never design content. They stay on this Mac.</p>
    {/if}
  {:else if tab === 'memory'}
    <p class="lead dim">Stored in Otto memory (collection: <code>design</code>) · local only. Otto recalls matching memories in every design turn.</p>
    {#if memError}
      <div class="err" role="alert">
        <Icon name="warning" size={14} /> {memError}
        <button class="btn small" onclick={() => void loadMemory()}>Retry</button>
      </div>
    {:else if memories === null}
      <Skeleton rows={4} height={32} />
    {:else if memories.length === 0}
      <EmptyState variant="page" icon="book" title="No design memories yet"
        body="Atomic preferences (“avoid”, “prefer”, “pattern”) land here when agents or people save them to the design collection." />
    {:else}
      <label class="search"><Icon name="search" size={13} />
        <input class="input" type="search" placeholder="Search memories" aria-label="Search design memories" bind:value={memQ} /></label>
      <div class="table-wrap">
        <table>
          <thead><tr><th>Kind</th><th>Memory</th><th>Source</th><th>Created</th></tr></thead>
          <tbody>
            {#each shownMemories as m (m.id)}
              <tr>
                <td><span class="kind">{m.kind}</span></td>
                <td class="sum"><strong>{m.title}</strong>{#if m.body && m.body !== m.title}<div class="dim">{m.body}</div>{/if}</td>
                <td class="dim">{m.source_kind}</td>
                <td class="dim" title={new Date(m.created_at).toLocaleString()}>{rel(m.created_at)}</td>
              </tr>
            {/each}
          </tbody>
        </table>
      </div>
    {/if}
  {:else if tab === 'settings'}
    {#if phase === 'loading'}
      <Skeleton rows={3} height={56} />
    {:else if phase === 'error' || !learned}
      <div class="err" role="alert"><Icon name="warning" size={14} /> Couldn’t load learning settings. <span class="dim">{error}</span>
        <button class="btn small" onclick={() => void load()}>Retry</button></div>
    {:else}
      <div class="settings card" data-testid="design-learned-settings">
        <div class="set">
          <div><strong>Learning mode</strong><p class="dim">How Otto turns your team’s signals into rules.</p></div>
          <div class="segmented" role="group" aria-label="Learning mode">
            <button aria-pressed={learned.mode === 'off'} class:active={learned.mode === 'off'} disabled={savingMode} onclick={() => void setMode('off')}>Off</button>
            <button aria-pressed={learned.mode !== 'off'} class:active={learned.mode !== 'off'} disabled={savingMode} onclick={() => void setMode('suggest')}>Suggest only</button>
          </div>
        </div>
        <div class="set">
          <div><strong>How rules are found</strong>
            <p class="dim">A deterministic pass over the last 90 days of signals — no model reads them. A pattern becomes a proposal at 3 signals across 2 designs. It runs after every variant you apply, and when you click “Look for rules now”.</p></div>
        </div>
        <div class="set">
          <div><strong>Where rules live</strong>
            <p class="dim">Skill <code>{learned.skill}</code> · <span class="path" title={learned.skill_path}>{learned.skill_path}</span> · {learned.active.length} rule{learned.active.length === 1 ? '' : 's'}</p></div>
          <button class="btn small" onclick={exportRules}><Icon name="download" size={12} /> Export rules (.md)</button>
        </div>
        <div class="set">
          <div><strong>Auto-apply</strong><p class="dim">Not available: every rule needs a person’s approval, and each one can be rolled back under Rules.</p></div>
        </div>
      </div>
    {/if}
  {:else if phase === 'loading'}
    <Skeleton rows={4} height={72} />
  {:else if phase === 'error' || !learned}
    <div class="err" role="alert">
      <Icon name="warning" size={14} /> Couldn’t load what Otto learned. <span class="dim">{error}</span>
      <button class="btn small" onclick={() => void load()}>Retry</button>
    </div>
  {:else}
    <div class="split" class:with-evidence={withEvidence}>
      <div class="list">
        {#if learned.mode === 'off'}
          <p class="callout" role="status"><Icon name="info" size={13} /> Learning is off for this workspace, so Otto doesn’t propose rules.
            <button class="linkbtn" onclick={() => go('settings')}>Turn it on</button></p>
        {/if}
        {#if tab === 'pending'}
          <p class="lead dim">Otto proposes rules from what your team applies, rejects and edits. Nothing applies until you approve it.</p>
          {#if learned.pending.length === 0}
            <div class="none card">
              <strong>No rules waiting for you</strong>
              <p class="dim">A rule is proposed when the same choice repeats 3 times across 2 designs.</p>
            </div>
          {/if}
          {#each learned.pending as e (e.edit_id)}
            <article class="rule-card card" class:sel={selectedKey === e.edit_id} data-testid="design-pending-rule">
              <div class="top">
                <span class="chip accent">New</span>
                <span class="dim">Proposed {rel(e.created_at)} · target: skill {learned.skill}</span>
              </div>
              {#each e.rules as r (r.key)}<p class="rule-text">{r.rule}</p>{/each}
              {#if !e.rules.length}<p class="rule-text">{editHeadline(e)}</p>{/if}
              {#if e.rationale}<p class="ev-line"><Icon name="clock" size={12} /> {e.rationale}</p>{/if}
              <div class="acts">
                <button class="btn small primary" disabled={!canEdit || busy === e.edit_id} onclick={() => void approve(e)} data-testid="design-rule-approve">
                  <Icon name="check" size={12} /> Approve
                </button>
                <button class="btn small ghost" disabled={!canEdit || busy === e.edit_id} onclick={() => void reject(e)} data-testid="design-rule-reject">Reject</button>
                <span class="grow"></span>
                <button class="btn small ghost" onclick={() => select(selectedKey === e.edit_id ? null : e.edit_id)} aria-pressed={selectedKey === e.edit_id}>
                  Evidence ({e.evidence.length})
                </button>
              </div>
            </article>
          {/each}
          {#if forming.length}
            <h2 class="sub">Patterns Otto is watching</h2>
            <ul class="forming">
              {#each forming as c (c.key)}
                {@const p = candidateProgress(c)}
                <li>
                  <button class="row-btn" class:sel={selectedKey === c.key} onclick={() => select(c.key)}>
                    <span class="rule-sm">{c.rule}</span>
                    <span class="dim small">{candidateKindLabel(c.kind)} · {p.text}</span>
                    <span class="bar" aria-hidden="true"><span style:inline-size={`${p.pct}%`}></span></span>
                  </button>
                </li>
              {/each}
            </ul>
          {/if}
        {:else}
          {#if learned.active.length === 0}
            <div class="none card">
              <strong>No team rules yet</strong>
              <p class="dim">Rules you approve under Pending show here, with the evidence behind them. Otto follows them in every design turn and says so.</p>
            </div>
          {:else}
            <div class="rules card">
              {#each learned.active as r (r.key)}
                <div class="rule-row" class:sel={selectedKey === r.key} data-testid="design-active-rule">
                  <button class="row-btn" onclick={() => select(r.key)}>
                    <span class="rule-sm">{r.rule}</span>
                    <span class="meta">
                      <span class="chip"><Icon name="file" size={11} /> skill: {learned.skill}</span>
                      {#if r.applied_at}<span class="dim small">approved {rel(r.applied_at)}</span>{/if}
                      <span class="dim small">{r.evidence.length} signal{r.evidence.length === 1 ? '' : 's'}</span>
                    </span>
                  </button>
                  <button class="icon-btn" onclick={(ev) => ruleMenu(ev, r)} aria-label="Rule actions" title="Rule actions" aria-haspopup="menu"
                    disabled={busy === r.key}>
                    <Icon name="more" size={14} />
                  </button>
                </div>
              {/each}
            </div>
          {/if}
          {#if learned.history.length}
            <h2 class="sub">Decisions</h2>
            <div class="table-wrap">
              <table>
                <thead><tr><th>Rule</th><th>Decision</th><th>When</th></tr></thead>
                <tbody>
                  {#each learned.history as h (h.edit_id)}
                    {@const st = editStatusLabel(h.status)}
                    <tr>
                      <td class="sum"><button class="linkbtn plain" onclick={() => select(h.edit_id)}>{editHeadline(h)}</button></td>
                      <td><span class="kind tone-{st.tone}">{st.label}</span></td>
                      <td class="dim">{rel(h.applied_at ?? h.created_at)}</td>
                    </tr>
                  {/each}
                </tbody>
              </table>
            </div>
          {/if}
        {/if}
      </div>
      {#if withEvidence && selection}
        <div class="side">
          <EvidencePane title={selection.title} rationale={selection.rationale} evidence={selection.evidence} signals={byId} onclose={() => select(null)} />
        </div>
      {/if}
    </div>
  {/if}
</PageBody>

<style>
  .n {
    color: var(--text-dim);
    margin-inline-start: 4px;
    font-variant-numeric: tabular-nums;
  }
  .n.hot {
    color: var(--warning);
    font-weight: 600;
  }
  .lead {
    margin: 0 0 12px;
    font-size: var(--fs-s);
  }
  .dim {
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-xs);
  }
  .grow {
    flex: 1;
  }
  .split {
    display: grid;
    grid-template-columns: minmax(0, 1fr);
    gap: 20px;
    container-type: inline-size;
  }
  .split.with-evidence {
    grid-template-columns: minmax(0, 1fr) minmax(280px, 380px);
  }
  @media (max-width: 1000px) {
    .split.with-evidence {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 10px;
    min-width: 0;
  }
  .side {
    min-width: 0;
  }
  .callout {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin: 0;
    padding: 8px 12px;
    border-radius: var(--radius-s);
    background: var(--info-soft);
    font-size: var(--fs-s);
  }
  .none {
    padding: 16px;
  }
  .none p {
    margin: 4px 0 0;
    font-size: var(--fs-s);
  }
  .rule-card {
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .rule-card.sel,
  .rule-row.sel,
  .row-btn.sel {
    border-color: var(--accent);
    box-shadow: 0 0 0 1px var(--accent);
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-xs);
  }
  .rule-text {
    margin: 0;
    font-size: var(--fs-m);
    font-weight: 600;
    line-height: 1.4;
  }
  .ev-line {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-style: italic;
  }
  .ev-line :global(svg) {
    vertical-align: -2px;
  }
  .acts {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    margin-block-start: 4px;
  }
  .sub {
    margin: 12px 0 0;
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text-dim);
  }
  .forming {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .row-btn {
    width: 100%;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 4px;
    text-align: start;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    cursor: pointer;
  }
  .row-btn:hover {
    background: var(--hover);
  }
  .rule-sm {
    font-size: var(--fs-s);
    font-weight: 500;
  }
  .bar {
    display: block;
    inline-size: 100%;
    block-size: 4px;
    border-radius: 999px;
    background: var(--surface-2);
    overflow: hidden;
  }
  .bar > span {
    display: block;
    block-size: 100%;
    background: var(--accent);
  }
  .rules {
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .rule-row {
    display: flex;
    align-items: center;
    gap: 6px;
    padding-inline-end: 10px;
    border-block-end: 1px solid var(--border);
  }
  .rule-row:last-child {
    border-block-end: 0;
  }
  .rule-row .row-btn {
    border: 0;
    border-radius: 0;
    background: transparent;
  }
  .rule-row .row-btn:hover {
    background: var(--hover);
  }
  .rule-row.sel {
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
  }
  .filters {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin-block-end: 14px;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-block-end: 10px;
    max-width: 360px;
    color: var(--text-dim);
  }
  .search input {
    flex: 1;
  }
  .table-wrap {
    overflow-x: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--fs-s);
  }
  th {
    position: sticky;
    inset-block-start: 0;
    text-align: start;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
    padding: 8px 12px;
    background: var(--surface);
    border-block-end: 1px solid var(--border);
  }
  td {
    padding: 7px 12px;
    border-block-end: 1px solid var(--border);
    white-space: nowrap;
  }
  tbody tr:last-child td {
    border-block-end: 0;
  }
  tbody tr:hover {
    background: var(--hover);
  }
  .linkbtn.plain {
    color: var(--text);
    text-align: start;
  }
  td.sum {
    white-space: normal;
    min-width: 240px;
  }
  td.art a {
    color: var(--text);
    text-decoration: none;
  }
  td.art a:hover {
    color: var(--accent-text);
  }
  .kind {
    font-size: var(--fs-xs);
    font-weight: 500;
    padding: 1px 7px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .tone-ok {
    color: var(--success);
    background: var(--success-soft);
  }
  .tone-bad {
    color: var(--danger);
    background: var(--danger-soft);
  }
  .tone-warn {
    color: var(--warning);
    background: var(--warning-soft);
  }
  .tone-info {
    color: var(--info);
    background: var(--info-soft);
  }
  .foot {
    margin: 12px 0 0;
    font-size: var(--fs-xs);
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--fs-s);
  }
  .err > :global(svg) {
    color: var(--danger);
  }
  .settings {
    max-width: 880px;
    display: flex;
    flex-direction: column;
  }
  .set {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    padding: 14px 16px;
    border-block-end: 1px solid var(--border);
  }
  .set:last-child {
    border-block-end: 0;
  }
  .set p {
    margin: 2px 0 0;
    font-size: var(--fs-s);
  }
  .set > div:first-child {
    min-width: 0;
  }
  .path {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    overflow-wrap: anywhere;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
  }
  .linkbtn {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
  }
  .linkbtn:hover {
    text-decoration: underline;
  }
  @media (max-width: 640px) {
    .set {
      flex-direction: column;
      align-items: flex-start;
    }
  }
</style>
