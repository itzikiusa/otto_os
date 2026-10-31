///<reference types="svelte" />
;
import { viewport } from '../../lib/stores/viewport.svelte';
import { plural } from '../../lib/plural';
import { loadErrorText } from '../../lib/loadError';
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
import { toastError } from '../../lib/toastError';
import { onTabKey } from '../../lib/tabKeys';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
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
;

  interface Props {
    tab: LearnedTab;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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
      error = loadErrorText(e);
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
      if (my === sigSeq) sigError = loadErrorText(e);
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
  let memSeq = 0;
  async function loadMemory(): Promise<void> {
    const w = wsId;
    if (!w) return;
    // Newest call for the CURRENT workspace wins: the old workspace's
    // memories must never land in the new one's list (S18-24).
    const my = ++memSeq;
    const current = () => my === memSeq && wsId === w;
    try {
      const rows = await listDesignMemories(w);
      if (!current()) return;
      memories = rows;
      memError = null;
    } catch (e) {
      if (!current()) return;
      memError =
        e instanceof ApiError && e.status === 403
          ? 'Design memory lives in Otto memory, which needs Product view access.'
          : loadErrorText(e);
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
      else if (r.proposed.length) toasts.success(`${plural(r.proposed.length, 'new rule')} to review`, 'Nothing applies until you accept it.');
      else toasts.info('No new rules', `${r.candidates.filter((c) => !c.ready).length} pattern(s) are still forming.`);
      await load();
    } catch (e) {
      toastError('Couldn’t look for rules', e);
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
        e instanceof ApiError && e.status === 403 ? 'Only workspace admins can change this.' : loadErrorText(e),
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
;
async () => {

 { const $$_redaeHegaP0C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP0 = new $$_redaeHegaP0C({ target: __sveltets_2_any(), props: {   "title":`What Otto learned`,"crumbs":viewport.isPhone ? [] : [{ label: 'Design Hall', onclick: () => router.go('design') }],leading:() => { async ()/*Ωignore_positionΩ*/ => {
    if(viewport.isPhone){ { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => router.go('design'),"aria-label":`Back to Design Hall`,"title":`Back to Design Hall`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});} }}
  };return __sveltets_2_any(0)},tabs:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("div", {     "class":`segmented`,"role":`tablist`,"aria-label":`Learning`,});
         for(let t of __sveltets_2_ensureArray(TABS)){t.id;
        const n = count(t.id);
         { svelteHTML.createElement("button", {            "role":`tab`,"aria-selected":tab === t.id,"tabindex":tab === t.id ? 0 : -1,"onclick":() => go(t.id),"onkeydown":onTabKey,...__sveltets_2_empty({"data-testid":`design-learned-tab-${t.id}`}),});tab === t.id;
          t.label;if(n != null){ { svelteHTML.createElement("span", {  "class":`n`,});t.id === 'pending' && n > 0;n; }}
         }
      }
     }
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
     { svelteHTML.createElement("button", {           "class":`btn small`,"onclick":modeMenu,"aria-haspopup":`menu`,"disabled":!learned || savingMode,...__sveltets_2_empty({"data-testid":`design-learning-mode`}),"title":`Workspace learning mode`,});
       learned?.mode === 'off' ? 'Off' : 'Suggest only';  { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
     }
     { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":() => void lookNow(),"disabled":!wsId || !canEdit || extracting || learned?.mode === 'off',...__sveltets_2_empty({"data-testid":`design-learned-extract`}),"title":learned?.mode === 'off'
        ? 'Learning is off — switch it to Suggest only to look for rules'
        : !canEdit
          ? 'You need edit access to this workspace to look for rules'
          : 'Look at recent signals and propose any rule that is ready',});
       { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`bulb`,"size":12,}});} extracting ? 'Looking…' : 'Look for rules now';
     }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, tabs, actions} = $$_redaeHegaP0.$$prop_def;/*Ωignore_endΩ*/
  
  
  
 PageHeader}

 { const $$_ydoBegaP0C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP0C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
  if(!wsId){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`bulb`,"title":`Pick a workspace`,"body":`Team rules are learned per workspace.`,}});}
  } else if (tab === 'signals'){
     { svelteHTML.createElement("div", {     "class":`filters`,"role":`group`,"aria-label":`Filter by kind`,});
       { svelteHTML.createElement("button", {      "class":`pill-toggle`,"aria-pressed":kind === '',"onclick":() => (kind = ''),});kind === '';  }
         for(let k of __sveltets_2_ensureArray(KINDS)){k;
         { svelteHTML.createElement("button", {      "class":`pill-toggle`,"aria-pressed":kind === k,"onclick":() => (kind = k),});kind === k;signalKindLabel(k); }
      }
     }
    if(sigLoading && !signals.length){
       { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":6,"height":32,}});}
    } else if (sigError){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`signals`,"error":sigError,"empty":true,"onretry":() => void loadSignals(),}});}
    } else if (shownSignals.length === 0){
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`bulb`,"title":kind ? `No “${signalKindLabel(kind)}” signals yet` : 'No signals yet',"body":`Applying or rejecting variants, fixes you accept, edits after an agent draft, approvals and shipping are captured here as signals.`,}});}
    }else{
       { svelteHTML.createElement("div", { "class":`table-wrap`,});
         { svelteHTML.createElement("table", { ...__sveltets_2_empty({"data-testid":`design-signals`}),});
           { svelteHTML.createElement("thead", {});
             { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } }
           }
           { svelteHTML.createElement("tbody", {});
               for(let s of __sveltets_2_ensureArray(shownSignals)){s.id;
               { svelteHTML.createElement("tr", {});
                 { svelteHTML.createElement("td", {   "class":`when`,"title":new Date(s.created_at).toLocaleString(),});whenLabel(s.created_at);  { svelteHTML.createElement("span", { "class":`dim`,}); rel(s.created_at); } }
                 { svelteHTML.createElement("td", {}); { svelteHTML.createElement("span", { "class":`kind tone-${signalTone(s.kind)}`,});signalKindLabel(s.kind); } }
                 { svelteHTML.createElement("td", { "class":`sum`,});sumOf(s); }
                 { svelteHTML.createElement("td", { "class":`art`,}); { svelteHTML.createElement("a", { "href":`#/design/a/${encodeURIComponent(s.artifact_id)}`,});titleOf(s.artifact_id); } }
                 { svelteHTML.createElement("td", { "class":`by`,});byLabel(s); }
               }
            }
           }
         }
       }
       { svelteHTML.createElement("p", { "class":`foot dim`,});                }
    }
  } else if (tab === 'memory'){
     { svelteHTML.createElement("p", { "class":`lead dim`,});      { svelteHTML.createElement("code", {});  }            }
    if(memError){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`design memory`,"error":memError,"empty":true,"onretry":() => void loadMemory(),}});}
    } else if (memories === null){
       { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":4,"height":32,}});}
    } else if (memories.length === 0){
       { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {        "variant":`page`,"icon":`book`,"title":`No design memories yet`,"body":`Atomic preferences (“avoid”, “prefer”, “pattern”) land here when agents or people save them to the design collection.`,}});}
    }else{
       { svelteHTML.createElement("label", { "class":`search`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":13,}});}
         { svelteHTML.createElement("input", {            "dir":`auto`,"class":`input`,"type":`search`,"placeholder":`Filter memories…`,"aria-label":`Filter design memories`,"bind:value":memQ,});/*Ωignore_startΩ*/() => memQ = __sveltets_2_any(null);/*Ωignore_endΩ*/} }
       { svelteHTML.createElement("div", { "class":`table-wrap`,});
         { svelteHTML.createElement("table", {});
           { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } } }
           { svelteHTML.createElement("tbody", {});
               for(let m of __sveltets_2_ensureArray(shownMemories)){m.id;
               { svelteHTML.createElement("tr", {});
                 { svelteHTML.createElement("td", {}); { svelteHTML.createElement("span", { "class":`kind`,});m.kind; } }
                 { svelteHTML.createElement("td", { "class":`sum`,}); { svelteHTML.createElement("strong", {});m.title; }if(m.body && m.body !== m.title){ { svelteHTML.createElement("div", { "class":`dim`,});m.body; }} }
                 { svelteHTML.createElement("td", { "class":`dim`,});m.source_kind; }
                 { svelteHTML.createElement("td", {   "class":`dim`,"title":new Date(m.created_at).toLocaleString(),});rel(m.created_at); }
               }
            }
           }
         }
       }
    }
  } else if (tab === 'settings'){
    if(phase === 'loading'){
       { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":3,"height":56,}});}
    } else if (phase === 'error' || !learned){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`learning settings`,"error":error || 'No data came back.',"empty":true,"onretry":() => void load(),}});}
    }else{
       { svelteHTML.createElement("div", {   "class":`settings card`,...__sveltets_2_empty({"data-testid":`design-learned-settings`}),});
         { svelteHTML.createElement("div", { "class":`set`,});
           { svelteHTML.createElement("div", {}); { svelteHTML.createElement("strong", {});  } { svelteHTML.createElement("p", { "class":`dim`,});        } }
           { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Learning mode`,});
             { svelteHTML.createElement("button", {      "aria-pressed":learned.mode === 'off',"disabled":savingMode,"onclick":() => void setMode('off'),});learned.mode === 'off';  }
             { svelteHTML.createElement("button", {      "aria-pressed":learned.mode !== 'off',"disabled":savingMode,"onclick":() => void setMode('suggest'),});learned.mode !== 'off';  }
           }
         }
         { svelteHTML.createElement("div", { "class":`set`,});
           { svelteHTML.createElement("div", {}); { svelteHTML.createElement("strong", {});    }
             { svelteHTML.createElement("p", { "class":`dim`,});                                         } }
         }
         { svelteHTML.createElement("div", { "class":`set`,});
           { svelteHTML.createElement("div", {}); { svelteHTML.createElement("strong", {});   }
             { svelteHTML.createElement("p", { "class":`dim`,});  { svelteHTML.createElement("code", {});learned.skill; }   { svelteHTML.createElement("span", {   "class":`path`,"title":learned.skill_path,});learned.skill_path; }  plural(learned.active.length, 'rule'); } }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":exportRules,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});}    }
         }
         { svelteHTML.createElement("div", { "class":`set`,});
           { svelteHTML.createElement("div", {}); { svelteHTML.createElement("strong", {});  } { svelteHTML.createElement("p", { "class":`dim`,});                 } }
         }
       }
    }
  } else if (phase === 'loading'){
     { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":4,"height":72,}});}
  } else if (phase === 'error' || !learned){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`what Otto learned`,"error":error || 'No data came back.',"empty":true,"onretry":() => void load(),}});}
  }else{
     { svelteHTML.createElement("div", {  "class":`split`,});withEvidence;
       { svelteHTML.createElement("div", { "class":`list`,});
        if(learned.mode === 'off'){
           { svelteHTML.createElement("p", { "class":`callout`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":13,}});}           
             { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":() => go('settings'),});   } }
        }
        if(tab === 'pending'){
           { svelteHTML.createElement("p", { "class":`lead dim`,});                 }
          if(learned.pending.length === 0){
             { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {      "icon":`bulb`,"title":`No rules waiting for you`,"body":`Otto proposes a rule when the same choice repeats 3 times across 2 designs. Accepted rules apply to every design turn.`,}});}
          }
             for(let e of __sveltets_2_ensureArray(learned.pending)){e.edit_id;
             { svelteHTML.createElement("article", {    "class":`rule-card card`,...__sveltets_2_empty({"data-testid":`design-pending-rule`}),});selectedKey === e.edit_id;
               { svelteHTML.createElement("div", { "class":`top`,});
                 { svelteHTML.createElement("span", { "class":`chip accent`,});  }
                 { svelteHTML.createElement("span", { "class":`dim`,}); rel(e.created_at);    learned.skill; }
               }
                 for(let r of __sveltets_2_ensureArray(e.rules)){r.key; { svelteHTML.createElement("p", { "class":`rule-text`,});r.rule; }}
              if(!e.rules.length){ { svelteHTML.createElement("p", { "class":`rule-text`,});editHeadline(e); }}
              if(e.rationale){ { svelteHTML.createElement("p", { "class":`ev-line`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});} e.rationale; }}
               { svelteHTML.createElement("div", { "class":`acts`,});
                
                 { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":!canEdit || busy === e.edit_id,"onclick":() => void reject(e),...__sveltets_2_empty({"data-testid":`design-rule-reject`}),});  }
                 { svelteHTML.createElement("button", {       "class":`btn small primary`,"disabled":!canEdit || busy === e.edit_id,"onclick":() => void approve(e),...__sveltets_2_empty({"data-testid":`design-rule-approve`}),});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} 
                 }
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                 { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => select(selectedKey === e.edit_id ? null : e.edit_id),"aria-pressed":selectedKey === e.edit_id,});
                   e.evidence.length;
                 }
               }
             }
          }
          if(forming.length){
             { svelteHTML.createElement("h2", { "class":`sub`,});    }
             { svelteHTML.createElement("ul", { "class":`forming`,});
                 for(let c of __sveltets_2_ensureArray(forming)){c.key;
                const p = candidateProgress(c);
                 { svelteHTML.createElement("li", {});
                   { svelteHTML.createElement("button", {    "class":`row-btn`,"onclick":() => select(c.key),});selectedKey === c.key;
                     { svelteHTML.createElement("span", { "class":`rule-sm`,});c.rule; }
                     { svelteHTML.createElement("span", { "class":`dim small`,});candidateKindLabel(c.kind);  p.text; }
                     { svelteHTML.createElement("span", {   "class":`bar`,"aria-hidden":`true`,}); { svelteHTML.createElement("span", { });__sveltets_2_ensureType(String, Number, `${p.pct}%`); } }
                   }
                 }
              }
             }
          }
        }else{
          if(learned.active.length === 0){
             { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {          "icon":`book`,"title":`No team rules yet`,"body":`Rules you accept under Pending show here, with the evidence behind them. Otto follows them in every design turn and says so.`,"actionLabel":count('pending') ? 'Review pending rules' : undefined,"onaction":() => go('pending'),}});}
          }else{
             { svelteHTML.createElement("div", { "class":`rules card`,});
                 for(let r of __sveltets_2_ensureArray(learned.active)){r.key;
                 { svelteHTML.createElement("div", {    "class":`rule-row`,...__sveltets_2_empty({"data-testid":`design-active-rule`}),});selectedKey === r.key;
                   { svelteHTML.createElement("button", {   "class":`row-btn`,"onclick":() => select(r.key),});
                     { svelteHTML.createElement("span", { "class":`rule-sm`,});r.rule; }
                     { svelteHTML.createElement("span", { "class":`meta`,});
                       { svelteHTML.createElement("span", { "class":`chip`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}  learned.skill; }
                      if(r.applied_at){ { svelteHTML.createElement("span", { "class":`dim small`,}); rel(r.applied_at); }}
                       { svelteHTML.createElement("span", { "class":`dim small`,});plural(r.evidence.length, 'signal'); }
                     }
                   }
                   { svelteHTML.createElement("button", {           "class":`icon-btn`,"onclick":(ev) => ruleMenu(ev, r),"aria-label":`Rule actions`,"title":`Rule actions`,"aria-haspopup":`menu`,"disabled":busy === r.key,});
                     { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
                   }
                 }
              }
             }
          }
          if(learned.history.length){
             { svelteHTML.createElement("h2", { "class":`sub`,});  }
             { svelteHTML.createElement("div", { "class":`table-wrap`,});
               { svelteHTML.createElement("table", {});
                 { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } { svelteHTML.createElement("th", {});  } } }
                 { svelteHTML.createElement("tbody", {});
                     for(let h of __sveltets_2_ensureArray(learned.history)){h.edit_id;
                    const st = editStatusLabel(h.status);
                     { svelteHTML.createElement("tr", {});
                       { svelteHTML.createElement("td", { "class":`sum`,}); { svelteHTML.createElement("button", {   "class":`linkbtn plain`,"onclick":() => select(h.edit_id),});editHeadline(h); } }
                       { svelteHTML.createElement("td", {}); { svelteHTML.createElement("span", { "class":`kind tone-${st.tone}`,});st.label; } }
                       { svelteHTML.createElement("td", { "class":`dim`,});rel(h.applied_at ?? h.created_at); }
                     }
                  }
                 }
               }
             }
          }
        }
       }
      if(withEvidence && selection){
         { svelteHTML.createElement("div", { "class":`side`,});
           { const $$_enaPecnedivE3C = __sveltets_2_ensureComponent(EvidencePane); new $$_enaPecnedivE3C({ target: __sveltets_2_any(), props: {          "title":selection.title,"rationale":selection.rationale,"evidence":selection.evidence,"signals":byId,"onclose":() => select(null),}});}
         }
      }
     }
  }
 PageBody}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LearnedPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LearnedPage__SvelteComponent_ = ReturnType<typeof LearnedPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default LearnedPage__SvelteComponent_;