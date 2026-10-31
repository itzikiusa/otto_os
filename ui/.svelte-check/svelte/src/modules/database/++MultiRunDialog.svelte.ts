///<reference types="svelte" />
;
import { announce } from '../../lib/announce.svelte';
import { scrollRegion } from './scroll-region';
import Skeleton from '../../lib/components/Skeleton.svelte';
import Badge from '../../lib/components/Badge.svelte';
// "Run on…" — the SAME script on several targets (connections × databases)
// and/or once per parameter value. Four stages in one sheet:
//   setup    → pick targets, give each placeholder its value list, options;
//   preview  → the daemon's plan: the FINAL statement per run (placeholders
//              substituted, ClickHouse `ON CLUSTER` injected into DDL), write
//              and production flags, per-target cluster override;
//   confirm  → only when a run writes to a production / read-only target:
//              every such target and its final statement, plus a typed phrase;
//   results  → live status per run, n ok / n failed, Stop, per-run result.
// The run executes in the daemon (`POST /db/multi-runs`), so closing the
// sheet does not stop it; "Recent" reopens a run. Each run is also recorded
// in its connection's ordinary History.
import { onDestroy, tick, untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
import { plural } from '../../lib/plural';
import Modal from '../../lib/components/Modal.svelte';
import Icon from '../../lib/components/Icon.svelte';
import EnvBadge from '../../lib/components/EnvBadge.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { api, ApiError } from '../../lib/api/client';
import { mergeMultiRunJob } from '../../lib/api/db-multirun-types';
import { pollWhileVisible, type Poller } from '../../lib/poll';
import { runStatus } from '../../lib/status';
import { toasts } from '../../lib/toast.svelte';
import { database } from '../../lib/stores/database.svelte';
import type {
    Connection,
    DbClusterMode,
    DbMultiRunBrief,
    DbMultiRunItemDetail,
    DbMultiRunJob,
    DbMultiRunParam,
    DbMultiRunPlan,
    DbMultiRunSpec,
    DbParamType,
    DbStartMultiRunReq,
    SchemaNode,
  } from '../../lib/api/types';
import { extractVars, type SplitMode } from '../../lib/database/sql-util';
import { splitModeFor } from '../../lib/database/sql-dialect';
import {
    buildTargets,
    clusterLine,
    confirmPhrase,
    parseValues,
    phraseMatches,
    scopeOptions,
    summaryText,
    targetKey,
  } from './multi-run';
;

  interface Props {
    /** The script to run (the whole buffer, or the selection). */
    statement: string;
    onclose: () => void;
  };;

  type Stage = 'setup' | 'preview' | 'confirm' | 'results';;;
  interface ParamDraft {
    text: string;
    type: DbParamType;
    escape: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { statement, onclose }: Props = $props();

  let stage = $state<Stage>('setup');

  const DB_KINDS = ['mysql', 'postgres', 'redis', 'mongodb', 'clickhouse'];
  const kind = $derived(database.selectedConn?.kind ?? null);
  const isClickhouse = $derived(kind === 'clickhouse');
  // Engine lexing (Redis per line, Postgres dollar quotes / no `#` comments).
  const splitMode = $derived<SplitMode>(splitModeFor(kind));
  /** Same-engine connections — one script, one dialect. */
  const candidates = $derived<Connection[]>(
    database.connections.filter((c) => c.kind === kind && DB_KINDS.includes(c.kind)),
  );

  // ── Targets ───────────────────────────────────────────────────────────────
  /** Picked connections, in pick order, and the scopes picked on each. */
  let order = $state<string[]>([]);
  let picks = $state<Record<string, string[]>>({});
  let scopes = $state<Record<string, { value: string; label: string }[] | 'loading' | { error: string }>>({});

  // Seed: the current connection + its active database.
  untrack(() => {
    const id = database.selectedConnId;
    if (id) {
      order = [id];
      picks = { [id]: database.activeDb ? [database.activeDb] : [] };
      void loadScopes(id);
    }
  });

  async function loadScopes(id: string): Promise<void> {
    scopes = { ...scopes, [id]: 'loading' };
    try {
      const root = await api.get<SchemaNode[]>(`/connections/${encodeURIComponent(id)}/db/schema`);
      scopes = { ...scopes, [id]: scopeOptions(root) };
    } catch (e) {
      scopes = { ...scopes, [id]: { error: loadErrorText(e) } };
    }
  }

  function toggleConn(id: string, on: boolean): void {
    if (on) {
      if (!order.includes(id)) order = [...order, id];
      picks = { ...picks, [id]: picks[id] ?? [] };
      if (!scopes[id]) void loadScopes(id);
    } else {
      order = order.filter((x) => x !== id);
      const { [id]: _drop, ...rest } = picks;
      picks = rest;
    }
    plan = null;
  }

  function toggleScope(id: string, value: string, on: boolean): void {
    const cur = picks[id] ?? [];
    picks = { ...picks, [id]: on ? [...cur, value] : cur.filter((v) => v !== value) };
    plan = null;
  }

  // ClickHouse: one default for every target; per-target overrides from the preview.
  let clusterMode = $state<DbClusterMode>('auto');
  let overrides = $state<Record<string, { mode: DbClusterMode; name?: string }>>({});

  const targets = $derived(buildTargets(order, picks, { mode: clusterMode }, overrides));

  // ── Parameters ────────────────────────────────────────────────────────────
  const placeholders = $derived(extractVars(statement, splitMode));

  let drafts = $state<Record<string, ParamDraft>>(
    untrack(() => {
      const tabVars = database.tab.vars ?? {};
      const out: Record<string, ParamDraft> = {};
      for (const n of extractVars(statement, splitMode)) {
        const v = tabVars[n];
        out[n] = { text: v?.value ?? '', type: v?.type ?? 'string', escape: v?.escape ?? true };
      }
      return out;
    }),
  );
  const params = $derived<DbMultiRunParam[]>(
    placeholders.map((n) => {
      const d = drafts[n] ?? { text: '', type: 'string', escape: true };
      return { name: n, values: parseValues(d.text), type: d.type, escape: d.escape };
    }),
  );
  const missing = $derived(params.filter((p) => p.values.length === 0).map((p) => p.name));
  const combos = $derived(params.reduce((n, p) => n * Math.max(1, p.values.length), 1));
  const runCount = $derived(targets.length * combos);

  // ── Options ───────────────────────────────────────────────────────────────
  let concurrency = $state(1);
  let stopOnError = $state(true);
  let maxRows = $state(500);

  function spec(): DbMultiRunSpec {
    return {
      statement,
      targets,
      params,
      max_rows: maxRows,
      mask: database.tab.mask || undefined,
    };
  }

  // ── Preview ───────────────────────────────────────────────────────────────
  let plan = $state<DbMultiRunPlan | null>(null);
  let planning = $state(false);
  let planError = $state<string | null>(null);
  let notice = $state<string | null>(null);
  /** A cluster override changed after the preview — re-plan before running. */
  let planStale = $state(false);

  const setupBlocker = $derived(
    targets.length === 0
      ? 'Pick at least one target'
      : missing.length > 0
        ? `Give ${missing.map((m) => `:${m}`).join(', ')} at least one value`
        : runCount > 200
          ? `${runCount} runs — the limit is 200`
          : null,
  );

  async function preview(): Promise<void> {
    if (setupBlocker || planning) return;
    planning = true;
    planError = null;
    try {
      plan = await api.post<DbMultiRunPlan>('/db/multi-run/plan', spec());
      notice = null;
      planStale = false;
      stage = 'preview';
    } catch (e) {
      planError = loadErrorText(e);
    } finally {
      planning = false;
    }
  }

  function setOverride(i: number, mode: DbClusterMode, name?: string): void {
    const t = targets[i];
    if (!t) return;
    overrides = { ...overrides, [targetKey(t.connection_id, t.node)]: { mode, name } };
    planStale = true;
  }

  // ── Confirm ───────────────────────────────────────────────────────────────
  let typed = $state('');
  const phrase = $derived(plan ? confirmPhrase(plan) : '');
  const guardedRuns = $derived(plan ? plan.runs.filter((r) => r.needs_confirm) : []);

  // ── Run + results ─────────────────────────────────────────────────────────
  // `$state.raw`: the job (≤200 items + targets) is only ever REPLACED by a
  // poll answer — a deep proxy re-wrapped the whole tree every 800 ms tick.
  let job = $state.raw<DbMultiRunJob | null>(null);
  let starting = $state(false);
  let startError = $state<string | null>(null);
  let poller: Poller | null = null;

  function proceed(): void {
    if (!plan) return;
    if (plan.needs_confirm) {
      typed = '';
      stage = 'confirm';
      return;
    }
    void start(false);
  }

  async function start(confirmWrite: boolean): Promise<void> {
    if (!plan || starting) return;
    starting = true;
    startError = null;
    const body: DbStartMultiRunReq = {
      ...spec(),
      concurrency,
      stop_on_error: stopOnError,
      confirm_write: confirmWrite,
      plan_hash: plan.plan_hash,
    };
    try {
      job = await api.post<DbMultiRunJob>('/db/multi-runs', body);
      stage = 'results';
      watch(job.id);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      if (e instanceof ApiError && msg.startsWith('plan_changed:')) {
        // The daemon re-planned and got different statements — show them.
        stage = 'setup';
        await preview();
        notice = 'The final statements changed since the preview — review them again.';
      } else if (e instanceof ApiError && msg.startsWith('write_blocked:')) {
        stage = 'confirm';
        startError = msg.replace(/^write_blocked:\s*/, '');
      } else {
        startError = msg;
      }
    } finally {
      starting = false;
    }
  }

  function watch(id: string): void {
    poller?.stop();
    // Fast while a run is young (most finish in seconds), then 2 s: a long
    // 200-target run no longer pulls the whole job ~75×/min for its lifetime.
    const since = Date.now();
    poller = pollWhileVisible(
      async (signal) => {
        // Only the runs that changed since the last answer (`?since=`), merged
        // by index — not all ≤200 items + targets every tick.
        const since = job && job.id === id && job.seq !== undefined ? `?since=${job.seq}` : '';
        const j = mergeMultiRunJob(job, await api.get<DbMultiRunJob>(`/db/multi-runs/${encodeURIComponent(id)}${since}`, signal));
        job = j;
        if (j.status !== 'running') poller?.stop();
      },
      {
        get ms() {
          return Date.now() - since > 30_000 ? 2000 : 800;
        },
        floorMs: 500,
        immediate: false,
      },
    );
  }

  async function stop(): Promise<void> {
    if (!job) return;
    try {
      job = await api.post<DbMultiRunJob>(`/db/multi-runs/${encodeURIComponent(job.id)}/cancel`);
      poller?.now();
    } catch (e) {
      toastError('Couldn’t stop the multi-run', e);
    }
  }

  let openItem = $state<number | null>(null);
  let detail = $state<DbMultiRunItemDetail | null>(null);
  let detailError = $state<string | null>(null);
  async function showItem(index: number): Promise<void> {
    if (!job) return;
    if (openItem === index) {
      openItem = null;
      return;
    }
    openItem = index;
    detail = null;
    detailError = null;
    try {
      detail = await api.get<DbMultiRunItemDetail>(
        `/db/multi-runs/${encodeURIComponent(job.id)}/items/${index}`,
      );
    } catch (e) {
      detailError = loadErrorText(e);
    }
  }

  function openInTab(text: string): void {
    database.newTab(text);
    onclose();
  }

  function cell(v: unknown): string {
    if (v === null || v === undefined) return 'NULL';
    if (typeof v === 'object') return JSON.stringify(v);
    return String(v);
  }

  // ── Recent runs (the job outlives the sheet) ──────────────────────────────
  let recent = $state<DbMultiRunBrief[]>([]);
  void api
    .get<DbMultiRunBrief[]>('/db/multi-runs')
    .then((r) => (recent = r))
    .catch(() => (recent = []));

  async function reopen(id: string): Promise<void> {
    try {
      job = await api.get<DbMultiRunJob>(`/db/multi-runs/${encodeURIComponent(id)}`);
      openItem = null;
      stage = 'results';
      if (job.status === 'running') watch(job.id);
    } catch (e) {
      toastError('Couldn’t open the multi-run', e);
    }
  }

  function close(): void {
    if (job?.status === 'running') {
      toasts.info('The multi-run keeps going', 'Reopen it from Run on… → Recent.');
    }
    onclose();
  }

  onDestroy(() => poller?.stop());

  // Every stage change unmounts the footer button that caused it, so focus
  // would fall to <body> (WCAG 2.4.3). Move it to the new stage's first
  // meaningful control — the typed phrase on confirm, the stage's lead
  // (tabindex=-1) elsewhere. Not on mount: the Modal places focus then.
  let mrEl = $state<HTMLDivElement | undefined>();
  let focusedStage: Stage = untrack(() => stage);
  $effect(() => {
    const s = stage;
    if (s === focusedStage) return;
    focusedStage = s;
    void tick().then(() => {
      if (stage !== s) return;
      mrEl?.querySelector<HTMLElement>('[data-stage-focus]')?.focus();
    });
  });

  const title = $derived(
    stage === 'results' ? 'Multi-run' : stage === 'confirm' ? 'Confirm production writes' : 'Run on multiple targets',
  );
  const scriptLines = $derived(statement.split('\n').length);
  $effect(() => { if (notice) announce(notice); });
  $effect(() => { if (planStale) announce('Cluster choice changed — update the preview to see the final statements.'); });
  $effect(() => { if (planError) announce(planError, { assertive: true }); });
  $effect(() => { if (startError) announce(startError, { assertive: true }); });
  $effect(() => { if (detailError) announce(detailError, { assertive: true }); });
  const confirming = $derived(stage === 'confirm' && !!plan);
  $effect(() => { if (confirming && guardedRuns.length) announce(`${plural(guardedRuns.length, 'run')} will write or change the schema on production or read-only targets. Check every target and statement.`, { assertive: true }); });
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },title,"width":920,"onclose":close,footer:() => { async ()/*Ωignore_positionΩ*/ => {
    if(stage === 'setup'){
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":close,});  }
       { svelteHTML.createElement("button", {         "class":`btn primary`,"onclick":preview,"disabled":!!setupBlocker || planning,"title":setupBlocker ?? 'Show the final statement for every run before anything executes',});
        planning ? 'Preparing…' : `Preview ${plural(runCount, 'run')}`;
       }
    } else if (stage === 'preview' && plan){
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (stage = 'setup'),});  }
       { svelteHTML.createElement("button", {          "class":`btn primary`,"onclick":proceed,"disabled":starting || planning || planStale,"title":planStale ? 'Update the preview first' : plan.needs_confirm ? 'Writes to production / read-only targets — you will confirm each one' : 'Run every statement shown',});plan.needs_confirm;
        starting ? 'Starting…' : plan.needs_confirm ? `Review ${plural(guardedRuns.length, 'guarded write')}…` : `Run ${plan.runs.length}`;
       }
    } else if (stage === 'confirm' && plan){
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (stage = 'preview'),});  }
       { svelteHTML.createElement("button", {         "class":`btn primary danger`,"onclick":() => start(true),"disabled":!phraseMatches(typed, phrase) || starting,"title":phraseMatches(typed, phrase) ? 'Run all planned statements, including the writes above' : `Type ${phrase} first`,});
        starting ? 'Starting…' : `Run ${plan.runs.length}`;
       }
    } else if (stage === 'results' && job){
      if(job.status === 'running'){
         { svelteHTML.createElement("button", {     "class":`btn danger`,"onclick":stop,"title":`Stop dispatching runs and cancel the ones in flight`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});} 
         }
      }else{
         { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => { job = null; openItem = null; stage = 'setup'; },});  }
      }
       { svelteHTML.createElement("button", {   "class":`btn primary`,"onclick":close,});  }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
   { const $$_div1 = svelteHTML.createElement("div", {    "class":`mr`,...__sveltets_2_empty({"data-testid":`multi-run`}),});mrEl = $$_div1;
    if(notice && stage !== 'results'){ { svelteHTML.createElement("p", { "class":`mr-notice`,});notice; }}
    if(planError && stage === 'preview'){
       { svelteHTML.createElement("div", { "class":`mr-error`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});} { svelteHTML.createElement("span", {});planError; } }
    }
    if(stage === 'setup'){
      if(recent.length > 0){
         { svelteHTML.createElement("section", {   "class":`mr-sec`,"aria-label":`Recent multi-runs`,});
           { svelteHTML.createElement("h3", { "class":`mr-h`,});  }
           { svelteHTML.createElement("ul", { "class":`mr-recent`,});
               for(let r of __sveltets_2_ensureArray((recent.slice(0, 4)))){r.id;
               { svelteHTML.createElement("li", {});
                 { svelteHTML.createElement("button", {     "class":`mr-link`,"onclick":() => reopen(r.id),"title":`Open this multi-run’s results`,});
                   { const $$_egdaBsutatS6C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS6C({ target: __sveltets_2_any(), props: {    "status":runStatus(r.status),"variant":`text`,}});}
                   { svelteHTML.createElement("span", { "class":`mono mr-clip`,});r.statement_preview; }
                   { svelteHTML.createElement("span", { "class":`mr-dim`,});summaryText(r.summary); }
                 }
               }
            }
           }
         }
      }

       { svelteHTML.createElement("section", {   "class":`mr-sec`,"aria-label":`Script`,});
         { svelteHTML.createElement("h3", {    "class":`mr-h`,"tabindex":-1,...__sveltets_2_empty({"data-stage-focus":true}),});  { svelteHTML.createElement("span", { "class":`mr-dim`,}); plural(scriptLines, 'line'); } }
         {const $$action_0 = __sveltets_2_ensureAction(scrollRegion(svelteHTML.mapElementTag('pre'),('Statement')));{ svelteHTML.createElement("pre", __sveltets_2_union($$action_0), {   "class":`mr-code mono`,});statement; }}
       }

       { svelteHTML.createElement("section", {   "class":`mr-sec`,"aria-label":`Targets`,});
         { svelteHTML.createElement("h3", { "class":`mr-h`,});  { svelteHTML.createElement("span", { "class":`mr-dim`,}); targets.length;  } }
        if(candidates.length === 0){
           { svelteHTML.createElement("p", { "class":`mr-dim`,}); kind ?? '';     }
        }else{
           { svelteHTML.createElement("ul", { "class":`mr-conns`,});
               for(let c of __sveltets_2_ensureArray(candidates)){c.id;
              const on = order.includes(c.id);
              const sc = scopes[c.id];
               { svelteHTML.createElement("li", {  "class":`mr-conn`,});on;
                 { svelteHTML.createElement("label", { "class":`mr-check`,});
                   { svelteHTML.createElement("input", {      "type":`checkbox`,"checked":on,"onchange":(e) => toggleConn(c.id, e.currentTarget.checked),});}
                   { svelteHTML.createElement("span", { "class":`mr-name`,});c.name; }
                   { const $$_egdaBvnE6C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE6C({ target: __sveltets_2_any(), props: {    "env":c.environment,"readOnly":c.read_only,}});}
                 }
                if(on){
                   { svelteHTML.createElement("div", { "class":`mr-scopes`,});
                    if(sc === 'loading'){
                       { svelteHTML.createElement("span", { "class":`mr-dim`,}); { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 11px`,"aria-hidden":`true`,}); }   }
                    } else if (sc && 'error' in sc){
                       { svelteHTML.createElement("span", { "class":`mr-err`,});   sc.error; }
                       { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => loadScopes(c.id),});  }
                    } else if (sc && sc.length > 0){
                         for(let s of __sveltets_2_ensureArray(sc)){s.value;
                         { svelteHTML.createElement("label", { "class":`mr-scope`,});
                           { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":(picks[c.id] ?? []).includes(s.value),"onchange":(e) => toggleScope(c.id, s.value, e.currentTarget.checked),});}
                           { svelteHTML.createElement("span", { "class":`mono`,});s.label; }
                         }
                      }
                    }
                    if((picks[c.id] ?? []).length === 0){
                       { svelteHTML.createElement("span", { "class":`mr-dim`,});          }
                    }
                   }
                }
               }
            }
           }
        }
        if(isClickhouse){
           { svelteHTML.createElement("label", { "class":`mr-inline`,});
             { svelteHTML.createElement("span", {});    }
             { svelteHTML.createElement("select", {     "class":`input`,"bind:value":clusterMode,"aria-label":`ON CLUSTER for DDL`,});/*Ωignore_startΩ*/() => clusterMode = __sveltets_2_any(null);/*Ωignore_endΩ*/
               { svelteHTML.createElement("option", { "value":`auto`,});   }
               { svelteHTML.createElement("option", { "value":`off`,});    }
             }
             { svelteHTML.createElement("span", { "class":`mr-dim`,});                      }
           }
        }
       }

       { svelteHTML.createElement("section", {   "class":`mr-sec`,"aria-label":`Parameters`,});
         { svelteHTML.createElement("h3", { "class":`mr-h`,});  }
        if(placeholders.length === 0){
           { svelteHTML.createElement("p", { "class":`mr-dim`,});
                { svelteHTML.createElement("span", { "class":`mono`,});  }  { svelteHTML.createElement("span", { "class":`mono`,});'{name}'; } 
             { svelteHTML.createElement("span", { "class":`mono`,});'{{name}}'; }         
           }
        }else{
           { svelteHTML.createElement("p", { "class":`mr-dim`,});              }
             for(let n of __sveltets_2_ensureArray(placeholders)){n;
            if(drafts[n]){
            const count = parseValues(drafts[n].text).length;
             { svelteHTML.createElement("div", { "class":`mr-param`,});
               { svelteHTML.createElement("span", {   "class":`mr-pname mono`,"title":n,}); n; }
               { svelteHTML.createElement("textarea", {                "dir":`ltr`,"class":`input mr-pvals mono`,"rows":Math.min(4, Math.max(1, drafts[n].text.split('\n').length)),"bind:value":drafts[n].text,"placeholder":`1, 2, 3`,"spellcheck":`false`,"aria-label":`Values for ${n}`,"oninput":() => (plan = null),});/*Ωignore_startΩ*/() => drafts[n].text = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
               { svelteHTML.createElement("select", {       "class":`input mr-ptype`,"bind:value":drafts[n].type,"aria-label":`Type for ${n}`,"title":`string = quoted literal · number = validated number · raw = verbatim`,});/*Ωignore_startΩ*/() => drafts[n].type = __sveltets_2_any(null);/*Ωignore_endΩ*/
                 { svelteHTML.createElement("option", { "value":`string`,});  }
                 { svelteHTML.createElement("option", { "value":`number`,});  }
                 { svelteHTML.createElement("option", { "value":`raw`,});  }
               }
              if(drafts[n].type === 'string'){
                 { svelteHTML.createElement("label", {   "class":`mr-esc`,"title":`Escape quotes inside the value`,});
                   { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":drafts[n].escape,});/*Ωignore_startΩ*/() => drafts[n].escape = __sveltets_2_any(null);/*Ωignore_endΩ*/} 
                 }
              }
               { svelteHTML.createElement("span", { "class":`mr-dim mr-count`,});plural(count, 'value'); }
             }
            }
          }
        }
       }

       { svelteHTML.createElement("section", {   "class":`mr-sec mr-opts`,"aria-label":`Options`,});
         { svelteHTML.createElement("label", { "class":`mr-inline`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", {     "class":`input`,"bind:value":concurrency,"aria-label":`Concurrency`,});/*Ωignore_startΩ*/() => concurrency = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", { "value":1,});    }
             { svelteHTML.createElement("option", { "value":2,});   }
             { svelteHTML.createElement("option", { "value":4,});   }
             { svelteHTML.createElement("option", { "value":8,});   }
           }
         }
         { svelteHTML.createElement("label", { "class":`mr-inline`,});
           { svelteHTML.createElement("input", {    "type":`checkbox`,"bind:checked":stopOnError,});/*Ωignore_startΩ*/() => stopOnError = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("span", {});     }
         }
         { svelteHTML.createElement("label", { "class":`mr-inline`,});
           { svelteHTML.createElement("span", {});    }
           { svelteHTML.createElement("select", {     "class":`input`,"bind:value":maxRows,"aria-label":`Rows kept per run`,});/*Ωignore_startΩ*/() => maxRows = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", { "value":100,});  }
             { svelteHTML.createElement("option", { "value":500,});  }
             { svelteHTML.createElement("option", { "value":1000,});  }
             { svelteHTML.createElement("option", { "value":5000,});  }
           }
         }
       }

      if(planError){
         { svelteHTML.createElement("div", { "class":`mr-error`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});}
           { svelteHTML.createElement("span", {});planError; }
           { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":preview,});  }
         }
      }
    } else if (stage === 'preview' && plan){
       { svelteHTML.createElement("p", {    "class":`mr-lead`,"tabindex":-1,...__sveltets_2_empty({"data-stage-focus":true}),});
         { svelteHTML.createElement("strong", {});plural(plan.runs.length, 'run'); } 
         { svelteHTML.createElement("strong", {});plural(plan.targets.length, 'target'); }
        if(plan.write_count > 0){  { svelteHTML.createElement("span", { "class":`mr-warn-text`,});plural(plan.write_count, 'write');  }}
         concurrency === 1 ? 'one at a time' : `${concurrency} in parallel`;
         stopOnError ? 'stops at the first failure' : 'continues past failures';
       }
      
          for(let w of __sveltets_2_ensureArray(plan.warnings)){let i = 1;i;
         { svelteHTML.createElement("p", { "class":`mr-warning`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} w; }
      }

      if(isClickhouse){
         { svelteHTML.createElement("section", {   "class":`mr-sec`,"aria-label":`Clusters`,});
           { svelteHTML.createElement("h3", { "class":`mr-h`,});  }
           { svelteHTML.createElement("ul", { "class":`mr-clusters`,});
               for(let t of __sveltets_2_ensureArray(plan.targets)){t.index;
              const c = t.cluster;
              const ov = overrides[targetKey(t.connection_id, t.node)];
               { svelteHTML.createElement("li", { "class":`mr-cl`,});
                 { svelteHTML.createElement("span", { "class":`mr-name`,});t.label; }
                 { const $$_egdaBvnE5C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE5C({ target: __sveltets_2_any(), props: {    "env":t.environment,"readOnly":t.read_only,}});}
                if(c){
                   { svelteHTML.createElement("span", {   "class":`mr-dim`,"title":c.note ?? '',});clusterLine(c); }
                   { svelteHTML.createElement("select", {         "class":`input mr-clsel`,"aria-label":`ON CLUSTER for ${t.label}`,"value":ov?.mode === 'custom' ? `custom:${ov.name ?? ''}` : (ov?.mode ?? 'auto'),"onchange":(e) => {
                      const v = e.currentTarget.value;
                      if (v.startsWith('custom:')) setOverride(t.index, 'custom', v.slice(7));
                      else setOverride(t.index, v as DbClusterMode);
                    },});
                     { svelteHTML.createElement("option", { "value":`auto`,}); c.detected ? ` (${c.detected})` : ''; }
                     { svelteHTML.createElement("option", { "value":`off`,});  }
                       for(let name of __sveltets_2_ensureArray(c.candidates)){name;
                       { svelteHTML.createElement("option", { "value":`custom:${name}`,});  name; }
                    }
                    if(ov?.mode === 'custom' && ov.name && !c.candidates.includes(ov.name)){
                       { svelteHTML.createElement("option", { "value":`custom:${ov.name}`,});  ov.name; }
                    }
                   }
                   { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input mr-clname mono`,"placeholder":`other cluster…`,"aria-label":`Custom cluster for ${t.label}`,"onkeydown":(e) => {
                      if (e.key === 'Enter' && e.currentTarget.value.trim()) setOverride(t.index, 'custom', e.currentTarget.value.trim());
                    },"onblur":(e) => {
                      if (e.currentTarget.value.trim()) setOverride(t.index, 'custom', e.currentTarget.value.trim());
                    },});}
                }
               }
            }
           }
          if(planStale){
             { svelteHTML.createElement("p", { "class":`mr-notice`,});            }
             { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":preview,"disabled":planning,});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}  
             }
          }
         }
      }

       { svelteHTML.createElement("section", {   "class":`mr-sec`,"aria-label":`Final statements`,});
         { svelteHTML.createElement("h3", { "class":`mr-h`,});  }
         { svelteHTML.createElement("ul", { "class":`mr-runs`,});
             for(let r of __sveltets_2_ensureArray(plan.runs)){r.index;
            const t = plan.targets[r.target];
             { svelteHTML.createElement("li", {});
               { svelteHTML.createElement("details", { "open":plan.runs.length <= 6,});
                 { svelteHTML.createElement("summary", { "class":`mr-run-sum`,});
                   { svelteHTML.createElement("span", { "class":`mr-idx mono`,}); r.index + 1; }
                   { svelteHTML.createElement("span", { "class":`mr-name`,});r.label; }
                   { const $$_egdaBvnE7C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE7C({ target: __sveltets_2_any(), props: {    "env":t.environment,"readOnly":t.read_only,}});}
                  if(r.is_write){ { const $$_egdaB7C = __sveltets_2_ensureComponent(Badge); new $$_egdaB7C({ target: __sveltets_2_any(), props: {    "tone":`warn`,"label":`Write`,}});}}
                  if(r.needs_confirm){ { const $$_egdaB7C = __sveltets_2_ensureComponent(Badge); new $$_egdaB7C({ target: __sveltets_2_any(), props: {    "tone":`bad`,"label":`Needs confirm`,}});}}
                  if(r.on_cluster?.length){ { const $$_egdaB7C = __sveltets_2_ensureComponent(Badge); new $$_egdaB7C({ target: __sveltets_2_any(), props: {    "tone":`info`,"label":`ON CLUSTER ×${r.on_cluster.length}`,}});}}
                 }
                 {const $$action_0 = __sveltets_2_ensureAction(scrollRegion(svelteHTML.mapElementTag('pre'),(`Statement for ${r.label}`)));{ svelteHTML.createElement("pre", __sveltets_2_union($$action_0), {   "class":`mr-code mono`,});r.statement; }}
                   for(let s of __sveltets_2_ensureArray(r.cluster_skipped ?? [])){s;
                   { svelteHTML.createElement("p", { "class":`mr-dim mr-small`,});  s; }
                }
               }
             }
          }
         }
       }
      if(startError){ { svelteHTML.createElement("div", { "class":`mr-error`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});} { svelteHTML.createElement("span", {});startError; } }}
    } else if (stage === 'confirm' && plan){
       { svelteHTML.createElement("div", { "class":`mr-danger`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
         { svelteHTML.createElement("span", {});
          plural(guardedRuns.length, 'run');         
                       
         }
       }
       { svelteHTML.createElement("ul", { "class":`mr-runs`,});
           for(let r of __sveltets_2_ensureArray(guardedRuns)){r.index;
          const t = plan.targets[r.target];
           { svelteHTML.createElement("li", { "class":`mr-guarded`,});
             { svelteHTML.createElement("div", { "class":`mr-run-sum`,});
               { svelteHTML.createElement("span", { "class":`mr-idx mono`,}); r.index + 1; }
               { svelteHTML.createElement("span", { "class":`mr-name`,});r.label; }
               { const $$_egdaBvnE5C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE5C({ target: __sveltets_2_any(), props: {    "env":t.environment,"readOnly":t.read_only,}});}
             }
             {const $$action_0 = __sveltets_2_ensureAction(scrollRegion(svelteHTML.mapElementTag('pre'),(`Statement for ${r.label}`)));{ svelteHTML.createElement("pre", __sveltets_2_union($$action_0), {   "class":`mr-code mono`,});r.statement; }}
           }
        }
       }
       { svelteHTML.createElement("label", { "class":`mr-typed`,});
         { svelteHTML.createElement("span", {});  { svelteHTML.createElement("strong", { "class":`mono`,});phrase; }   }
         { svelteHTML.createElement("input", {               "dir":`ltr`,"class":`input mono`,...__sveltets_2_empty({"data-stage-focus":true}),"bind:value":typed,"placeholder":phrase,"spellcheck":`false`,"aria-label":`Type ${phrase} to confirm`,"onkeydown":(e) => {
            if (e.key === 'Enter' && phraseMatches(typed, phrase)) void start(true);
          },});/*Ωignore_startΩ*/() => typed = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
      if(startError){ { svelteHTML.createElement("div", { "class":`mr-error`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});} { svelteHTML.createElement("span", {});startError; } }}
    } else if (stage === 'results' && job){
       { svelteHTML.createElement("div", {    "class":`mr-status`,"tabindex":-1,...__sveltets_2_empty({"data-stage-focus":true}),});
         { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {  "status":runStatus(job.status === 'done' ? (job.summary.failed > 0 ? 'failed' : 'done') : job.status),}});}
         { svelteHTML.createElement("span", { "class":`mr-lead`,});summaryText(job.summary); }
         { svelteHTML.createElement("span", { "class":`mr-dim`,}); job.summary.total; }
         { svelteHTML.createElement("span", {   "class":`mr-dim mr-clip mono`,"title":job.statement_preview,});job.statement_preview; }
       }
      if(job.summary.total > 0){
         { svelteHTML.createElement("div", {             "class":`mr-bar`,"role":`progressbar`,"aria-label":`Multi-run progress`,"aria-valuemin":0,"aria-valuemax":job.summary.total,"aria-valuenow":job.summary.ok + job.summary.failed + job.summary.skipped + job.summary.cancelled,});
           { svelteHTML.createElement("span", {  "class":`ok`,});__sveltets_2_ensureType(String, Number, `${(job.summary.ok / job.summary.total) * 100} `); }
           { svelteHTML.createElement("span", {  "class":`bad`,});__sveltets_2_ensureType(String, Number, `${(job.summary.failed / job.summary.total) * 100} `); }
         }
      }
       { svelteHTML.createElement("ul", { "class":`mr-results`,});
           for(let it of __sveltets_2_ensureArray(job.items)){it.index;
          const t = job.targets[it.target];
           { svelteHTML.createElement("li", {  "class":`mr-res`,});openItem === it.index;
             { svelteHTML.createElement("button", {         "class":`mr-res-row`,"onclick":() => showItem(it.index),"aria-expanded":openItem === it.index,"title":`Show this run’s statement and result`,});
               { svelteHTML.createElement("span", { "class":`mr-idx mono`,}); it.index + 1; }
               { svelteHTML.createElement("span", { "class":`mr-name`,});it.label; }
              if(t){ { const $$_egdaBvnE5C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE5C({ target: __sveltets_2_any(), props: {    "env":t.environment,"readOnly":t.read_only,}});}}
               { const $$_egdaBsutatS5C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS5C({ target: __sveltets_2_any(), props: {    "status":runStatus(it.status),"variant":`text`,}});}
               { svelteHTML.createElement("span", { "class":`mr-dim mr-meta`,});
                if(it.duration_ms != null){it.duration_ms; }
                if(it.rows_affected != null){ it.rows_affected; } else if (it.row_count != null){ plural(it.row_count, 'row');}
               }
              if(it.error){ { svelteHTML.createElement("span", {   "class":`mr-err mr-clip`,"title":it.error,});it.error; }
              } else if (it.message){ { svelteHTML.createElement("span", {   "class":`mr-dim mr-clip`,"title":it.message,});it.message; }}
             }
            if(openItem === it.index){
               { svelteHTML.createElement("div", { "class":`mr-detail`,});
                if(detailError){
                   { svelteHTML.createElement("div", { "class":`mr-error`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":13,}});} { svelteHTML.createElement("span", {});detailError; } }
                } else if (!detail){
                   { const $$_notelekS5C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS5C({ target: __sveltets_2_any(), props: {      "rows":3,"height":20,"label":`result details`,}});}
                }else{
                   {const $$action_0 = __sveltets_2_ensureAction(scrollRegion(svelteHTML.mapElementTag('pre'),('Final statement')));{ svelteHTML.createElement("pre", __sveltets_2_union($$action_0), {   "class":`mr-code mono`,});detail.statement; }}
                   { svelteHTML.createElement("div", { "class":`mr-detail-actions`,});
                     { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => openInTab(detail!.statement),"title":`Open this final statement in a new query tab`,});
                       { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`external`,"size":12,}});}     
                     }
                   }
                  if(detail.result && detail.result.columns.length > 0){
                     {const $$action_0 = __sveltets_2_ensureAction(scrollRegion(svelteHTML.mapElementTag('div'),(`Result of run ${it.index + 1}`)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`mr-table-wrap`,});
                       { svelteHTML.createElement("table", { "class":`mr-table mono`,});
                         { svelteHTML.createElement("thead", {});
                           { svelteHTML.createElement("tr", {});    for(let col of __sveltets_2_ensureArray(detail.result.columns)){let ci = 1;ci; { svelteHTML.createElement("th", {});col.name; }} }
                         }
                         { svelteHTML.createElement("tbody", {});
                              for(let row of __sveltets_2_ensureArray((detail.result.rows.slice(0, 200)))){let ri = 1;ri;
                             { svelteHTML.createElement("tr", {});    for(let v of __sveltets_2_ensureArray(row)){let ci = 1;ci; { svelteHTML.createElement("td", { "title":cell(v),});cell(v); }} }
                          }
                         }
                       }
                     }}
                    if(detail.result.rows.length > 200 || detail.result.truncated){
                       { svelteHTML.createElement("p", { "class":`mr-dim mr-small`,});   Math.min(200, detail.result.rows.length);             }
                    }
                  } else if (it.result_dropped){
                     { svelteHTML.createElement("p", { "class":`mr-dim mr-small`,});               }
                  } else if (it.status === 'ok'){
                     { svelteHTML.createElement("p", { "class":`mr-dim mr-small`,});detail.result?.message ?? 'Done — no rows returned.'; }
                  }
                }
               }
            }
           }
        }
       }
    }
   }

  
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MultiRunDialog__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MultiRunDialog__SvelteComponent_ = ReturnType<typeof MultiRunDialog__SvelteComponent_>;
/*Ωignore_endΩ*/export default MultiRunDialog__SvelteComponent_;