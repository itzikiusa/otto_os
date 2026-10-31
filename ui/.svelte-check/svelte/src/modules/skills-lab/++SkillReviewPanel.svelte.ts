///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
// Skills Lab → Review. A list of skill reviews + a detail pane that mirrors the
// code-review UX: a deterministic static-analysis card, N visible embedded
// agent terminals (SkillReviewAgents), and the summarizer's aggregated report.
// Live-refreshes on the skill_review_updated bus, with a fallback poll while a
// review is running.
import { onDestroy } from 'svelte';
import { toastError } from '../../lib/toastError';
import type { LibrarySkill, BundledSkillView, ProviderSkillInfo, SkillReview } from '../../lib/api/types';
import { skillReviewApi } from '../../lib/api/skillReview';
import { skillLabApi } from '../../lib/api/skillLab';
import { skillReviewBus } from '../../lib/events.svelte';
import type { Poller } from '../../lib/poll';
import { liveQuery } from '../../lib/live';
import { toasts } from '../../lib/toast.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import SkillReviewAgents from './SkillReviewAgents.svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import LazyTerminal from '../../lib/components/LazyTerminal.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { runStatus, type BadgeTone } from '../../lib/status';
import { severityLabel } from '../../lib/labels';
import Badge from '../../lib/components/Badge.svelte';
import { rel } from '../../lib/stores/now.svelte';
import { agentProviders, defaultAgentProvider } from '../../lib/providers';
import { sourceLabel } from './skillGroups';
;

  interface Props {
    wsId: string;
    listCollapsed?: boolean;
    /** Optional skill to pre-select in the New Review form (from the Skills tab). */
    initialTarget?: { name: string; source: string } | null;
    onconsumed?: () => void;
    /** Open this existing review (a skill's Usage → Reviews card). */
    initialReview?: string | null;
    onreviewconsumed?: () => void;
    onphonedetail?: (open: boolean) => void;
  };;

  // `source` is "library" | "bundled" | a provider name (claude/codex/agy) — all
  // valid `skill_source` values the review engine resolves.
  type SkillOpt = { name: string; source: string; label: string };;function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { wsId, listCollapsed = false, initialTarget = null, onconsumed, initialReview = null, onreviewconsumed, onphonedetail }: Props = $props();


  let reviews = $state<SkillReview[]>([]);
  // Inline list-load failure + Retry (instead of a toast over "No skill reviews yet").
  let listError = $state<string | null>(null);
  // First list load: don't flash "No skill reviews yet" before it lands.
  let listLoading = $state(true);
  let selected = $state<SkillReview | null>(null);
  let skillOpts = $state<SkillOpt[]>([]);

  // New-review form.
  let fSkill = $state('');
  let fMode = $state<'static' | 'agents'>('agents');
  // Reviewer providers from the live registry (built-ins + custom, e.g. grok).
  // Selection is a set of provider names; default to the first agent provider.
  let fProviders = $state<Set<string>>(new Set([defaultAgentProvider()]));
  function toggleReviewer(p: string): void {
    const next = new Set(fProviders);
    if (next.has(p)) next.delete(p);
    else next.add(p);
    fProviders = next;
  }
  let fInstructions = $state('');
  let starting = $state(false);
  let phoneListOpen = $state(false);
  const showPhoneList = $derived(
    viewport.isPhone && (phoneListOpen || ((listLoading || !!listError) && reviews.length === 0)),
  );
  $effect(() => {
    onphonedetail?.(viewport.isPhone && !showPhoneList && reviews.length > 0);
  });
   function back(): void {
    phoneListOpen = true;
  }

  // Apply-fixes form.
  let fixProvider = $state(defaultAgentProvider());
  let fixInstructions = $state('');
  let fixTermOpen = $state(false);
  let applying = $state(false);

  const fixRunning = $derived(
    !!selected?.fix_agent && ['pending', 'running', 'waiting'].includes(selected.fix_agent.status),
  );
  // Keep the fallback poll alive while the fixer runs, not just the review.
  const activeReview = $derived(selected && (selected.status === 'running' || fixRunning));
  const canApply = $derived(
    !!selected &&
      selected.status === 'done' &&
      !fixRunning &&
      ((selected.summary?.findings.length ?? 0) > 0 ||
        (selected.summary?.patch_plan.length ?? 0) > 0 ||
        (selected.static_report?.findings.length ?? 0) > 0),
  );

  /** Skill sources that failed on the last load ("Library", "Bundled",
   *  "Provider") — named under the picker with Retry instead of a picker that
   *  silently lists fewer skills. */
  let skillsFailed = $state<string[]>([]);
  let skillsLoading = $state(false);
  async function loadSkills(): Promise<void> {
    skillsLoading = true;
    const [libR, bundledR, providerR] = await Promise.allSettled([
      skillLabApi.listLibrary(),
      skillLabApi.listBundled(),
      skillLabApi.listProvider(),
    ]);
    skillsLoading = false;
    const lib = libR.status === 'fulfilled' ? libR.value : ([] as LibrarySkill[]);
    const bundled = bundledR.status === 'fulfilled' ? bundledR.value : ([] as BundledSkillView[]);
    const provider = providerR.status === 'fulfilled' ? providerR.value : ([] as ProviderSkillInfo[]);
    skillsFailed = [
      libR.status === 'rejected' ? 'Library' : '',
      bundledR.status === 'rejected' ? 'Bundled' : '',
      providerR.status === 'rejected' ? 'Provider' : '',
    ].filter(Boolean);
    const opts: SkillOpt[] = [];
    for (const s of lib) opts.push({ name: s.name, source: 'library', label: `${s.name} · library` });
    const libNames = new Set(lib.map((s) => s.name));
    for (const b of bundled)
      if (!libNames.has(b.name)) opts.push({ name: b.name, source: 'bundled', label: `${b.name} · bundled` });
    for (const p of provider)
      opts.push({ name: p.name, source: p.provider, label: `${p.name} · ${p.provider}` });
    opts.sort((a, b) => a.name.localeCompare(b.name));
    // Keep a pre-filled hand-off target the fresh list doesn't carry.
    const keep = skillOpts.filter((o) => `${o.source}:${o.name}` === fSkill && !opts.some((n) => n.source === o.source && n.name === o.name));
    skillOpts = [...keep, ...opts];
  }

  let listGeneration = 0;
  let disposed = false;
  onDestroy(() => { disposed = true; listGeneration++; detailGeneration++; });

  async function loadList(): Promise<void> {
    const workspace = wsId;
    const generation = ++listGeneration;
    if (!wsId) {
      listLoading = false;
      return;
    }
    listLoading = true;
    try {
      const next = await skillReviewApi.list(workspace);
      if (disposed || generation !== listGeneration || workspace !== wsId) return;
      reviews = next;
      listError = null;
    } catch (e) {
      if (!disposed && generation === listGeneration && workspace === wsId) listError = loadErrorText(e);
    } finally {
      if (!disposed && generation === listGeneration && workspace === wsId) listLoading = false;
    }
  }

  /** `quiet` for background refreshes (poll / bus): a transient failure there
   *  must not raise a toast every 2.5 s — the next tick retries. */
  let detailGeneration = 0;
  let selectionPending: number | null = null;
  let refreshPending: number | null = null;
  async function openReview(id: string, quiet = false): Promise<void> {
    // A poll for the still-visible old review must not supersede a deliberate
    // selection. Coalesce background reads so a slow response can finish.
    if (quiet && (selected?.id !== id || selectionPending === detailGeneration || refreshPending === detailGeneration)) return;
    const generation = quiet ? detailGeneration : ++detailGeneration;
    if (quiet) refreshPending = generation;
    else {
      selectionPending = generation;
      phoneListOpen = false;
    }
    const workspace = wsId;
    try {
      const review = await skillReviewApi.get(id);
      if (generation === detailGeneration && workspace === wsId) selected = review;
    } catch (e) {
      if (generation === detailGeneration && workspace === wsId && !quiet) toastError('Couldn’t open the review', e);
    } finally {
      if (quiet && refreshPending === generation) refreshPending = null;
      if (!quiet && selectionPending === generation) selectionPending = null;
    }
  }

  async function start(): Promise<void> {
    if (!fSkill || starting) return;
    const opt = skillOpts.find((o) => `${o.source}:${o.name}` === fSkill);
    if (!opt) return;
    const providers: string[] = [];
    if (fMode === 'agents') {
      providers.push(...fProviders);
      if (providers.length === 0) providers.push(defaultAgentProvider());
    }
    const workspace = wsId;
    const generation = detailGeneration;
    starting = true;
    try {
      const rev = await skillReviewApi.start(workspace, {
        skill_name: opt.name,
        skill_source: opt.source,
        providers,
        agent_mode: fMode,
        instructions: fInstructions.trim(),
      });
      if (disposed || workspace !== wsId || generation !== detailGeneration) return;
      selected = rev;
      phoneListOpen = false;
      await loadList();
    } catch (e) {
      toastError('Couldn’t start the review', e);
    } finally {
      starting = false;
    }
  }

  async function applyFixes(): Promise<void> {
    if (!selected || applying) return;
    const id = selected.id;
    const workspace = wsId;
    const generation = detailGeneration;
    applying = true;
    try {
      const updated = await skillReviewApi.apply(id, {
        provider: fixProvider,
        instructions: fixInstructions.trim(),
      });
      if (disposed || workspace !== wsId || generation !== detailGeneration || selected?.id !== id) return;
      selected = updated;
      fixTermOpen = true;
      toasts.info('Fixer agent starting…');
    } catch (e) {
      toastError('Couldn’t start the fixer agent', e);
    } finally {
      applying = false;
    }
  }

  async function cancelReview(): Promise<void> {
    if (!selected) return;
    const id = selected.id;
    const workspace = wsId;
    if (
      !(await confirmer.ask(`Stop the review of “${selected.skill_name}”? Agents still running are stopped and their partial findings are not summarized.`, {
        title: 'Stop review',
        confirmLabel: 'Stop review',
        danger: true,
      }))
    )
      return;
    if (disposed || workspace !== wsId || selected?.id !== id) return;
    try {
      const updated = await skillReviewApi.cancel(id);
      // Another review may have been opened while the stop was in flight.
      if (selected?.id === id) selected = updated;
      await loadList();
    } catch (e) {
      toastError('Couldn’t stop the review', e);
    }
  }

  async function deleteReview(rev: SkillReview): Promise<void> {
    // Every other delete in Skills Lab / Evaluator confirms; this one used to
    // fire on a single click of a bare ✕.
    if (
      !(await confirmer.ask(
        `Delete the review of “${rev.skill_name}”? Its findings, agent transcripts and summary are removed. The skill itself is not touched.`,
        { title: 'Delete review', confirmLabel: 'Delete', danger: true },
      ))
    )
      return;
    try {
      await skillReviewApi.remove(rev.id);
      if (selected?.id === rev.id) selected = null;
      await loadList();
    } catch (e) {
      toastError('Couldn’t delete the review', e);
    }
  }

  function newReview(): void {
    detailGeneration++;
    selected = null;
    fSkill = '';
    phoneListOpen = false;
  }

  // --- live refresh -----------------------------------------------------------
  let lastTick = 0;
  $effect(() => {
    const t = skillReviewBus.tick;
    if (t === lastTick) return;
    lastTick = t;
    if (skillReviewBus.workspaceId && skillReviewBus.workspaceId !== wsId) return;
    // A review advanced — refresh the list and, if it's the open one, the detail.
    void loadList();
    if (selected && skillReviewBus.reviewId === selected.id) void openReview(selected.id, true);
  });

  // Fallback poll while the open review is running (covers dropped sockets):
  // `skill_review_updated` (above) drives it while the event socket is up, so
  // this is a 15 s safety net then; 2.5 s only while the socket is down.
  let poll: Poller | null = null;
  $effect(() => {
    if (poll) { poll.stop(); poll = null; }
    if (activeReview && selected) {
      const id = selected.id;
      poll = liveQuery({ run: () => openReview(id, true), on: [], fallbackMs: 2500, safetyMs: 15_000, immediate: false });
    }
  });
  onDestroy(() => { detailGeneration++; if (poll) poll.stop(); });

  // Load on workspace change.
  let loadedWs = '';
  $effect(() => {
    if (wsId && wsId !== loadedWs) {
      // The open review belongs to the previous workspace.
      detailGeneration++;
      if (loadedWs) selected = null;
      loadedWs = wsId;
      void loadSkills();
      // Open on the newest review rather than a blank form — unless the form
      // is already in use (a "Review this skill" hand-off pre-fills fSkill).
      void loadList().then(() => {
        if (!selected && !fSkill && reviews.length > 0) void openReview(reviews[0].id);
      });
    }
  });

  // Consume a cross-tab "review this skill" intent by pre-filling the form.
  let consumedTarget = '';
  $effect(() => {
    if (initialTarget && `${initialTarget.source}:${initialTarget.name}` !== consumedTarget) {
      detailGeneration++;
      consumedTarget = `${initialTarget.source}:${initialTarget.name}`;
      selected = null;
      phoneListOpen = false;
      // Ensure the option exists even before the skill list loads.
      const key = `${initialTarget.source}:${initialTarget.name}`;
      if (!skillOpts.some((o) => `${o.source}:${o.name}` === key)) {
        skillOpts = [
          { name: initialTarget.name, source: initialTarget.source, label: `${initialTarget.name} · ${initialTarget.source}` },
          ...skillOpts,
        ];
      }
      fSkill = key;
      onconsumed?.();
    }
  });

  $effect(() => {
    if (!initialReview) return;
    const id = initialReview;
    onreviewconsumed?.();
    void openReview(id);
  });

  /** "spec_compliance" → "Spec compliance" (scorecard areas are snake_case ids). */
  function areaLabel(a: string): string {
    const t = a.replace(/_/g, ' ').trim();
    return t.charAt(0).toUpperCase() + t.slice(1);
  }

  function verdictClass(v: string): string {
    if (v === 'Ready') return 'verdict-ready';
    if (v === 'Ready with fixes') return 'verdict-fixes';
    return 'verdict-block';
  }
  // Review severity → the shared Badge tone + wording (lib/labels severityLabel).
  const SEV_TONE: Record<string, BadgeTone> = { critical: 'bad', high: 'bad', medium: 'warn', low: 'info' };
  function sevTone(sev: string): BadgeTone {
    return SEV_TONE[sev.toLowerCase()] ?? 'neutral';
  }
  // Spoken at the state edge through the shell's always-mounted announcer: a
  // live region inserted together with its text is not read (WebKit/VoiceOver).
  const skillsFailedText = $derived(skillsFailed.join(', '));
  $effect(() => { if (skillsFailedText) announce(`${skillsFailedText} skills didn’t load. Retry to load them.`, { assertive: true }); });
  const selectedError = $derived(selected?.error ?? '');
  $effect(() => { if (selectedError) announce(selectedError, { assertive: true }); });
;
async () => {

 { svelteHTML.createElement("div", { "class":`review-wrap`,});
 { svelteHTML.createElement("div", {    "class":`lab-review`,...__sveltets_2_empty({"data-testid":`skill-review`}),});listCollapsed && !viewport.isPhone;
   { svelteHTML.createElement("aside", {    "class":`lr-side`,"id":`reviews-list`,});viewport.isPhone && !showPhoneList;
    
     { svelteHTML.createElement("div", { "class":`lr-side-head`,});
       { svelteHTML.createElement("span", { "class":`lr-side-title`,});  }
       { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":newReview,"aria-pressed":!selected,"title":`New review`,...__sveltets_2_empty({"data-testid":`new-skill-review`}),}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
     }
    if((listError || listLoading) && reviews.length === 0){
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {            "what":`reviews`,"variant":`compact`,"loading":listLoading,"error":listError,"empty":true,"onretry":loadList,}});}
    } else if (reviews.length === 0){
       { svelteHTML.createElement("p", { "class":`lr-empty`,});         }
    }else{
       { svelteHTML.createElement("ul", { "class":`lr-list`,});
           for(let r of __sveltets_2_ensureArray(reviews)){r.id;
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("button", {      "class":`lr-item`,"aria-current":selected?.id === r.id ? 'true' : undefined,"onclick":() => openReview(r.id),});selected?.id === r.id;
               { svelteHTML.createElement("span", { "class":`lr-item-top`,});
                 { svelteHTML.createElement("span", {   "class":`lr-item-name`,"title":r.skill_name,});r.skill_name; }
                 { svelteHTML.createElement("span", {   "class":`rp-status-pill`,...__sveltets_2_empty({"data-status":r.status}),}); { const $$_egdaBsutatS8C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS8C({ target: __sveltets_2_any(), props: {    "status":runStatus(r.status),"variant":`text`,}});} }
               }
               { svelteHTML.createElement("span", { "class":`lr-item-meta`,});
                 { svelteHTML.createElement("span", {});sourceLabel(r.skill_source);r.static_report ? ` · ${r.static_report.verdict}` : ''; }
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                 { svelteHTML.createElement("span", { "title":new Date(r.created_at).toLocaleString(),});rel(r.created_at); }
               }
             }
           }
        }
       }
    }
   }

   { svelteHTML.createElement("main", {  "class":`lr-main`,});viewport.isPhone && showPhoneList;
    if(!selected){
      
       { svelteHTML.createElement("div", { "class":`lr-form card`,});
         { svelteHTML.createElement("h3", {});   }
         { svelteHTML.createElement("p", { "class":`lr-hint`,});
                  { svelteHTML.createElement("strong", {});  }   
               { svelteHTML.createElement("code", {});  }        
                  
         }
         { svelteHTML.createElement("label", { "class":`lr-field`,});
           { svelteHTML.createElement("span", {});   }
           { svelteHTML.createElement("select", {     "class":`input`,"bind:value":fSkill,...__sveltets_2_empty({"data-testid":`skill-review-select`}),});/*Ωignore_startΩ*/() => fSkill = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"","disabled":true,});   }
               for(let o of __sveltets_2_ensureArray(skillOpts)){o.source + ':' + o.name;
               { svelteHTML.createElement("option", { "value":o.source + ':' + o.name,});o.label; }
            }
           }
         }
        if(skillsFailed.length > 0){
           { svelteHTML.createElement("span", { "class":`lr-skills-failed`,});
             { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}
            skillsFailed.join(', ');   
             { svelteHTML.createElement("span", { "aria-hidden":`true`,});  }
             { svelteHTML.createElement("button", {       "class":`btn small ghost`,"type":`button`,"onclick":() => void loadSkills(),"disabled":skillsLoading,});skillsLoading ? 'Retrying…' : 'Retry'; }
           }
        }
         { svelteHTML.createElement("fieldset", { "class":`lr-field`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("label", { "class":`lr-radio`,}); { svelteHTML.createElement("input", {     "type":`radio`,"value":`static`,});fMode = __sveltets_2_any(null);}       }
           { svelteHTML.createElement("label", { "class":`lr-radio`,}); { svelteHTML.createElement("input", {     "type":`radio`,"value":`agents`,});fMode = __sveltets_2_any(null);}       }
         }
        if(fMode === 'agents'){
           { svelteHTML.createElement("fieldset", { "class":`lr-field`,});
             { svelteHTML.createElement("span", {});  }
               for(let p of __sveltets_2_ensureArray(agentProviders())){p;
               { svelteHTML.createElement("label", { "class":`lr-check`,});
                 { svelteHTML.createElement("input", {      "type":`checkbox`,"checked":fProviders.has(p),"onchange":() => toggleReviewer(p),});} p;
               }
            }
           }
        }
         { svelteHTML.createElement("label", { "class":`lr-field`,});
           { svelteHTML.createElement("span", {});   { svelteHTML.createElement("span", { "class":`hint-inline`,});  } }
           { svelteHTML.createElement("textarea", {            "dir":`auto`,"class":`input lr-textarea`,"rows":3,"bind:value":fInstructions,"placeholder":`Extra context for the reviewers — e.g. “check recent commits: they fix the previous review round”, known issues from earlier implementations…`,...__sveltets_2_empty({"data-testid":`skill-review-instructions`}),});/*Ωignore_startΩ*/() => fInstructions = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
         }
         { svelteHTML.createElement("div", { "class":`lr-actions`,});
           { svelteHTML.createElement("span", { "class":`dim lr-cost`,});fMode === 'static' ? 'No agents — runs instantly' : `${plural(Math.max(1, fProviders.size), 'review agent')} + a summarizer`; }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {         "class":`btn primary`,"disabled":!fSkill || starting,"title":!fSkill ? 'Pick a skill to review' : undefined,"onclick":start,...__sveltets_2_empty({"data-testid":`start-skill-review`}),});
            starting ? 'Starting…' : 'Start review';
           }
         }
       }
    }else{
      
       { svelteHTML.createElement("div", { "class":`lr-detail`,});
         { svelteHTML.createElement("div", { "class":`lr-detail-head`,});
           { svelteHTML.createElement("div", {});
             { svelteHTML.createElement("h3", {});selected.skill_name; }
             { svelteHTML.createElement("span", { "class":`chip lr-src`,});sourceLabel(selected.skill_source); }
             { svelteHTML.createElement("span", {   "class":`rp-status-pill`,...__sveltets_2_empty({"data-status":selected.status}),}); { const $$_egdaBsutatS7C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS7C({ target: __sveltets_2_any(), props: {  "status":runStatus(selected.status),}});} }
           }
           { svelteHTML.createElement("div", { "class":`grow`,}); }
          if(selected.status === 'running'){
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":cancelReview,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}   }
          }
           { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => selected && deleteReview(selected),"aria-label":`Delete this review`,"title":`Delete this review`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":14,}});} }
         }

        if(selected.error){
           { svelteHTML.createElement("p", { "class":`lr-error`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} selected.error; }
        }

        if(selected.instructions){
           { svelteHTML.createElement("p", {   "class":`lr-instructions`,"title":`Additional instructions this review ran with`,});
             { svelteHTML.createElement("strong", {});  } selected.instructions;
           }
        }

        
        if(selected.static_report){
          const sr = selected.static_report;
           { svelteHTML.createElement("section", {   "class":`card lr-static`,...__sveltets_2_empty({"data-testid":`static-report`}),});
             { svelteHTML.createElement("div", { "class":`lr-verdict ${verdictClass(sr.verdict)}`,});
               { svelteHTML.createElement("strong", {});  }
               { svelteHTML.createElement("span", { "class":`lr-verdict-badge`,});sr.verdict; }
               { svelteHTML.createElement("span", { "class":`lr-avg`,}); sr.average_score.toFixed(1);  }
             }
             { svelteHTML.createElement("table", { "class":`lr-score`,});
               { svelteHTML.createElement("tbody", {});
                   for(let row of __sveltets_2_ensureArray(sr.scorecard)){row.area;
                   { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("td", { "class":`lr-area`,});areaLabel(row.area); } { svelteHTML.createElement("td", { "class":`lr-num`,});row.score;  } { svelteHTML.createElement("td", { "class":`lr-notes`,});row.notes; } }
                }
               }
             }
            if(sr.findings.length > 0){
               { svelteHTML.createElement("ul", { "class":`lr-findings`,});
                    for(let f of __sveltets_2_ensureArray(sr.findings)){let i = 1;i + f.code;
                   { svelteHTML.createElement("li", { "class":`rp-finding`,});
                     { const $$_egdaB7C = __sveltets_2_ensureComponent(Badge); new $$_egdaB7C({ target: __sveltets_2_any(), props: {    "tone":sevTone(f.severity),"label":severityLabel(f.severity.toLowerCase()),}});}
                     { svelteHTML.createElement("span", { "class":`mono rp-loc`,});f.code; }
                     { svelteHTML.createElement("span", { "class":`rp-finding-body`,}); { svelteHTML.createElement("strong", {});f.title; }  f.fix; }
                   }
                }
               }
            }
           }
        }

        
        if(selected.agents.length > 0){
           { svelteHTML.createElement("section", { "class":`lr-agents-sec`,});
             { svelteHTML.createElement("h4", {});  }
             { const $$_stnegAweiveRllikS5C = __sveltets_2_ensureComponent(SkillReviewAgents); new $$_stnegAweiveRllikS5C({ target: __sveltets_2_any(), props: {      "review":selected,"view":selected.status === 'running' ? 'running' : 'done',"onretried":(r) => (selected = r),}});}
           }
        }

        
        if(selected.summary){
          const sm = selected.summary;
           { svelteHTML.createElement("section", {   "class":`card lr-summary`,...__sveltets_2_empty({"data-testid":`summary-report`}),});
             { svelteHTML.createElement("div", { "class":`lr-verdict ${verdictClass(sm.verdict)}`,});
               { svelteHTML.createElement("strong", {});  }
               { svelteHTML.createElement("span", { "class":`lr-verdict-badge`,});sm.verdict; }
               { svelteHTML.createElement("span", { "class":`lr-avg`,}); sm.average_score.toFixed(1);  }
             }
            if(sm.patch_plan.length > 0){
               { svelteHTML.createElement("h5", {});  }
               { svelteHTML.createElement("ol", { "class":`lr-plan`,});
                    for(let step of __sveltets_2_ensureArray(sm.patch_plan)){let i = 1;i; { svelteHTML.createElement("li", {});step; }}
               }
            }
            if(sm.findings.length > 0){
               { svelteHTML.createElement("h5", {}); sm.findings.length;  }
               { svelteHTML.createElement("ul", { "class":`lr-findings`,});
                    for(let f of __sveltets_2_ensureArray(sm.findings)){let i = 1;i + f.code;
                   { svelteHTML.createElement("li", { "class":`rp-finding`,});
                     { const $$_egdaB7C = __sveltets_2_ensureComponent(Badge); new $$_egdaB7C({ target: __sveltets_2_any(), props: {    "tone":sevTone(f.severity),"label":severityLabel(f.severity.toLowerCase()),}});}
                     { svelteHTML.createElement("span", { "class":`mono rp-loc`,});f.code; }
                     { svelteHTML.createElement("span", { "class":`rp-finding-body`,}); { svelteHTML.createElement("strong", {});f.title; }f.fix ? ` — ${f.fix}` : ''; }
                   }
                }
               }
            }
           }
        }

        
        if(selected.status === 'done' && (canApply || selected.fix_agent)){
           { svelteHTML.createElement("section", {   "class":`card lr-fix`,...__sveltets_2_empty({"data-testid":`apply-fixes`}),});
             { svelteHTML.createElement("h4", {});  }
            if(selected.fix_agent){
              const fx = selected.fix_agent;
               { svelteHTML.createElement("div", { "class":`lr-fix-row`,});
                 { svelteHTML.createElement("span", { "class":`lr-fix-name`,});  }
                 { svelteHTML.createElement("span", { "class":`chip`,});fx.provider; }
                 { svelteHTML.createElement("span", { "class":`grow`,}); }
                if(fx.session_id){
                   { svelteHTML.createElement("button", {     "class":`btn small ghost`,"aria-expanded":fixTermOpen,"onclick":() => (fixTermOpen = !fixTermOpen),});
                    fixTermOpen ? 'Hide session' : 'Open session';
                   }
                }
                 { svelteHTML.createElement("span", {   "class":`rp-status-pill`,...__sveltets_2_empty({"data-status":fx.status}),}); { const $$_egdaBsutatS7C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS7C({ target: __sveltets_2_any(), props: {  "status":runStatus(fx.status),}});} }
               }
              if(fx.note){
                 { svelteHTML.createElement("p", { "class":`lr-fix-note`,});fx.note; }
              }
              if(fx.status === 'waiting'){
                 { svelteHTML.createElement("p", { "class":`lr-fix-waiting`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}        { svelteHTML.createElement("strong", {});  }   }
              }
              if(fx.session_id && fixTermOpen){
                 { svelteHTML.createElement("div", { "class":`lr-fix-term`,});
                   { const $$_lanimreTyzaL6C = __sveltets_2_ensureComponent(LazyTerminal); new $$_lanimreTyzaL6C({ target: __sveltets_2_any(), props: {    "sessionId":fx.session_id,"preferDom":true,"resumeOnOpen":false,}});}
                 }
              }
            }
            if(canApply){
              if(selected.skill_source === 'bundled'){
                 { svelteHTML.createElement("p", { "class":`lr-hint`,});                  }
              }else{
                 { svelteHTML.createElement("p", { "class":`lr-hint`,});
                              sourceLabel(selected.skill_source);       
                 }
                 { svelteHTML.createElement("div", { "class":`lr-fix-form`,});
                   { svelteHTML.createElement("select", {       "class":`input`,"bind:value":fixProvider,"title":`Fixer agent`,"aria-label":`Fixer agent`,});/*Ωignore_startΩ*/() => fixProvider = __sveltets_2_any(null);/*Ωignore_endΩ*/
                       for(let p of __sveltets_2_ensureArray(agentProviders())){p; { svelteHTML.createElement("option", { "value":p,});p; }}
                   }
                   { svelteHTML.createElement("input", {            "dir":`auto`,"type":`text`,"class":`input lr-fix-input`,"aria-label":`Extra instructions for the fixer`,"bind:value":fixInstructions,"placeholder":`Extra instructions for the fixer (optional)…`,});/*Ωignore_startΩ*/() => fixInstructions = __sveltets_2_any(null);/*Ωignore_endΩ*/}
                   { svelteHTML.createElement("button", {       "class":`btn primary`,"disabled":applying,"onclick":applyFixes,...__sveltets_2_empty({"data-testid":`apply-fixes-btn`}),});
                    applying ? 'Starting…' : selected.fix_agent ? 'Run again' : 'Apply fixes with agent';
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
 }


};
return { props: {} as any as Props, exports: {} as any as { back: typeof back }, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SkillReviewPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SkillReviewPanel__SvelteComponent_ = ReturnType<typeof SkillReviewPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default SkillReviewPanel__SvelteComponent_;