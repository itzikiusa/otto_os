///<reference types="svelte" />
;
  // Shared cache of the cross-module reads, so flipping between notes of one
  // bundle does not refetch the repo directory / monitor overview / repo status
  // per note. TTL is per key: 60 s for directories and overviews, 15 s for a
  // repo's `git status`, 120 s for the forge's open-PR list (rate limited).
  const TTL_MS = 60_000;
  const STATUS_TTL_MS = 15_000;
  const PRS_TTL_MS = 120_000;
  const cache = new Map<string, { at: number; ttl: number; p: Promise<unknown> }>();
  /** A shared read, deduplicated per key for `ttl`. The fetch itself is NOT
   *  bound to the caller's signal: a superseded panel (flipping notes) used to
   *  abort the in-flight fetch every other note was about to share, so the next
   *  note refetched it — the k8s overview went out twice in one window. The
   *  caller's `signal` only stops ITS wait; the fetch completes into the cache. */
  function cached<T>(key: string, load: () => Promise<T>, ttl = TTL_MS, signal?: AbortSignal): Promise<T> {
    const hit = cache.get(key);
    let p: Promise<T>;
    if (hit && Date.now() - hit.at < hit.ttl) {
      p = hit.p as Promise<T>;
    } else {
      p = load();
      const entry = { at: Date.now(), ttl, p };
      p.catch(() => { if (cache.get(key) === entry) cache.delete(key); }); // failures retry on the next note
      cache.set(key, entry);
    }
    return signal ? untilAborted(p, signal) : p;
  }
  /** `p`, or an AbortError as soon as `signal` aborts (`p` keeps running). */
  function untilAborted<T>(p: Promise<T>, signal: AbortSignal): Promise<T> {
    const aborted = () => new DOMException('The operation was aborted.', 'AbortError');
    if (signal.aborted) return Promise.reject(aborted());
    return new Promise<T>((resolve, reject) => {
      const onAbort = () => reject(aborted());
      signal.addEventListener('abort', onAbort, { once: true });
      p.then(
        (v) => { signal.removeEventListener('abort', onAbort); resolve(v); },
        (e) => { signal.removeEventListener('abort', onAbort); reject(e); },
      );
    });
  }
  /** Whole-panel results per (workspace, hints): re-mounting the panel (an
   *  Edit → Read toggle, flipping back to a note) within this window renders
   *  instantly with zero requests. */
  const RESULT_TTL_MS = 15_000;
  const results = new Map<string, { at: number; ctx: unknown }>();
  /** Open-PR counts the user asked for (opt-in forge call), per repo. */
  const prCounts = new Map<string, { at: number; n: number }>();
;;

import { plural } from '../../lib/plural';
// LIVE CONTEXT for a typed vault note: matches the note's entity hints
// (service / workload names, repo paths, DB names, collections, dashboards)
// against the rest of Otto through the modules' existing read APIs — K8s
// monitor workloads, the repo directory + status, DB connections and
// dashboards, API-client collections — and links into those views. Read
// only: nothing here changes another module's state beyond navigating.
import { untrack } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import EnvBadge from '../../lib/components/EnvBadge.svelte';
import { api, ApiError } from '../../lib/api/client';
import { k8sApi } from '../../lib/api/k8s';
import { router } from '../../lib/router.svelte';
import { git } from '../../lib/stores/git.svelte';
import { database } from '../../lib/stores/database.svelte';
import type {
    ApiCollection,
    Connection,
    DbDashboard,
    K8sMonitorOverviewRow,
    K8sMonitorWorkloadRow,
    K8sMonitorWorkloadsResp,
    RepoDirectory,
    RepoDirectoryEntry,
    RepoStatusResp,
  } from '../../lib/api/types';
import { matchByName, matchRepos, norm, workloadRouteKind, type EntityHints } from './structuredNote';
import { monitorPath, resourcesPath } from '../../lib/k8s/viewState';
;

  interface WorkloadHit { cluster: K8sMonitorOverviewRow['cluster']; row: K8sMonitorWorkloadRow };;
  interface RepoHit { repo: RepoDirectoryEntry; status: RepoStatusResp | null; prs: number | null };;
  interface Ctx {
    workloads: WorkloadHit[];
    repos: RepoHit[];
    connections: Connection[];
    collections: ApiCollection[];
    dashboards: DbDashboard[];
    /** Sources that failed for a reason other than "not enabled / no access". */
    failed: string[];
  };
;type $$ComponentProps =  { hints: EntityHints; wsId: string };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { hints, wsId }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();




  let ctx = $state<Ctx | null>(null);
  let loading = $state(false);
  let seq = 0;

  /** 403/404 mean "feature off or no access" — an absent source, not an error. */
  const absent = (e: unknown): boolean => e instanceof ApiError && (e.status === 403 || e.status === 404);

  async function source<T>(name: string, failed: string[], run: () => Promise<T[]>): Promise<T[]> {
    try {
      return await run();
    } catch (e) {
      if (!absent(e)) failed.push(name);
      return [];
    }
  }

  async function k8sHits(signal: AbortSignal): Promise<WorkloadHit[]> {
    if (!hints.services.length) return [];
    const overview = await cached('k8s:overview', () => k8sApi.monitorOverview('1h'), TTL_MS, signal);
    const want = hints.k8sCluster ? norm(hints.k8sCluster) : null;
    const clusters = overview
      .filter((r) => r.enabled && (!want || norm(r.cluster.name) === want || r.cluster.id === hints.k8sCluster))
      .slice(0, 6);
    const per = await Promise.all(
      clusters.map(async (r) => {
        try {
          const w = await cached<K8sMonitorWorkloadsResp>(
            `k8s:wl:${r.cluster.id}:${hints.k8sNamespace ?? ''}`,
            () => k8sApi.monitorWorkloads(r.cluster.id, '1h', hints.k8sNamespace ?? undefined),
            TTL_MS,
            signal,
          );
          return matchByName(hints.services, w.workloads, (x) => x.workload).map((row) => ({ cluster: r.cluster, row }));
        } catch {
          return [];
        }
      }),
    );
    return per.flat().slice(0, 8);
  }

  async function repoHits(signal: AbortSignal): Promise<RepoHit[]> {
    if (!hints.repos.length) return [];
    const dir = await cached<RepoDirectory>('git:directory', () => api.bg.get<RepoDirectory>('/git/repos/directory'), TTL_MS, signal);
    const matched = matchRepos(hints.repos, dir.repos).slice(0, 3);
    return Promise.all(
      matched.map(async (repo) => {
        const status = await cached(
          `git:status:${repo.id}`,
          () => api.bg.get<RepoStatusResp>(`/repos/${repo.id}/status`),
          STATUS_TTL_MS,
          signal,
        ).catch(() => null);
        // Open PRs ask the forge (rate limited): opt-in from the card's chip,
        // unless a recent answer is already cached.
        return { repo, status, prs: cachedPrs(repo.id) };
      }),
    );
  }

  /** A still-fresh cached open-PR count, without fetching. */
  function cachedPrs(repoId: string): number | null {
    const hit = prCounts.get(repoId);
    return hit && Date.now() - hit.at < PRS_TTL_MS ? hit.n : null;
  }
  let prLoading = $state<string | null>(null);
  async function loadPrs(h: RepoHit): Promise<void> {
    if (prLoading) return;
    prLoading = h.repo.id;
    try {
      const n = await cached(
        `git:prs:${h.repo.id}`,
        () => api.bg.get<{ items: unknown[] }>(`/repos/${h.repo.id}/prs?state=open`).then((r) => r.items.length),
        PRS_TTL_MS,
      );
      prCounts.set(h.repo.id, { at: Date.now(), n });
      if (ctx) {
        ctx = { ...ctx, repos: ctx.repos.map((r) => (r.repo.id === h.repo.id ? { ...r, prs: n } : r)) };
        const prior = results.get(key);
        if (prior) results.set(key, { at: prior.at, ctx });
      }
    } catch {
      /* the chip stays clickable; the Pull requests button still works */
    } finally {
      prLoading = null;
    }
  }

  let controller: AbortController | null = null;

  async function load(force = false): Promise<void> {
    const mine = ++seq;
    controller?.abort(); // superseded loads stop instead of finishing unseen
    const ctl = new AbortController();
    controller = ctl;
    const signal = ctl.signal;
    const rkey = key;
    const prior = results.get(rkey);
    if (!force && prior && Date.now() - prior.at < RESULT_TTL_MS) {
      ctx = prior.ctx as Ctx;
      loading = false;
      return;
    }
    loading = true;
    const failed: string[] = [];
    const nameHints = [...hints.databases, ...hints.services];
    const [workloads, repos, connections, collections, dashboards] = await Promise.all([
      source('Kubernetes', failed, () => k8sHits(signal)),
      source('Git', failed, () => repoHits(signal)),
      source('Connections', failed, async () =>
        hints.databases.length || hints.services.length
          ? matchByName(nameHints, await cached(`conn:${wsId}`, () => api.bg.get<Connection[]>(`/workspaces/${wsId}/connections`), TTL_MS, signal), (c) => c.name)
          : [],
      ),
      source('API collections', failed, async () =>
        hints.collections.length
          ? matchByName(hints.collections, await cached(`coll:${wsId}`, () => api.bg.get<ApiCollection[]>(`/workspaces/${wsId}/api-client/collections`), TTL_MS, signal), (c) => c.name)
          : [],
      ),
      source('Dashboards', failed, async () =>
        hints.dashboards.length || hints.services.length
          ? matchByName([...hints.dashboards, ...hints.services], await cached(`dash:${wsId}`, () => api.bg.get<DbDashboard[]>(`/workspaces/${wsId}/db/dashboards`), TTL_MS, signal), (d) => d.name)
          : [],
      ),
    ]);
    if (mine !== seq || signal.aborted) return;
    ctx = { workloads, repos, connections, collections, dashboards, failed };
    if (!failed.length) results.set(rkey, { at: Date.now(), ctx });
    loading = false;
  }

  function refresh(): void {
    cache.clear();
    results.clear();
    prCounts.clear();
    void load(true);
  }

  // Load only once the panel is (nearly) on screen: a typed note whose live
  // context sits below the fold costs no requests until it is scrolled to.
  let host = $state<HTMLElement | null>(null);
  let visible = $state(typeof IntersectionObserver === 'undefined');
  $effect(() => {
    if (visible || !host) return;
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          visible = true;
          io.disconnect();
        }
      },
      { rootMargin: '200px' },
    );
    io.observe(host);
    return () => io.disconnect();
  });

  // Keyed on the hints' VALUE: a poll that re-delivers the same note builds a
  // new (equal) hints object and must not refetch repo status per tick.
  const key = $derived(`${wsId}|${JSON.stringify(hints)}`);
  $effect(() => {
    void key;
    if (!visible) return;
    untrack(() => void load());
  });
  $effect(() => () => controller?.abort());

  const total = $derived(
    ctx ? ctx.workloads.length + ctx.repos.length + ctx.connections.length + ctx.collections.length + ctx.dashboards.length : 0,
  );

  function health(r: K8sMonitorWorkloadRow): 'ok' | 'warn' | 'bad' {
    if (r.pods > 0 && r.ready === 0) return 'bad';
    if (r.ready < r.pods || r.err_pct >= 5 || r.mem_pct >= 90) return 'warn';
    return 'ok';
  }

  function openWorkload(h: WorkloadHit): void {
    router.go(resourcesPath(h.cluster.id, workloadRouteKind(h.row.kind), h.row.namespace, h.row.workload));
  }
  function openRepo(id: string, sub?: string): void {
    git.openRepoTab(id, sub);
    router.go('git');
  }
  function openDashboard(d: DbDashboard): void {
    database.selectedDashboardId = d.id;
    database.mainTab = 'dashboards';
    router.go('database');
  }
;
async () => {



 { const $$_section0 = svelteHTML.createElement("section", {        "class":`live`,"aria-label":`Live context`,"aria-busy":loading,...__sveltets_2_empty({"data-testid":`vault-live-context`}),});host = $$_section0;
   { svelteHTML.createElement("h4", {});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`radar`,"size":13,}});}  
    if(ctx && !loading){
       { svelteHTML.createElement("button", {       "class":`refresh`,"title":`Refresh live context`,"aria-label":`Refresh live context`,"onclick":refresh,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}
       }
    }
   }
  if(!visible && !ctx){
     { svelteHTML.createElement("p", { "class":`dim`,});         }
  } else if (loading && !ctx){
    
     { svelteHTML.createElement("p", { "class":`dim`,});       }
  } else if (ctx){
    if(ctx.failed.length){
       { svelteHTML.createElement("p", {   "class":`warn`,"role":`alert`,});
          ctx.failed.join(', ');
         { svelteHTML.createElement("button", {   "class":`link`,"onclick":refresh,});  }
       }
    }
    if(total === 0 && !ctx.failed.length){
       { svelteHTML.createElement("p", { "class":`dim`,});         { svelteHTML.createElement("code", {});  }  { svelteHTML.createElement("code", {});  }  { svelteHTML.createElement("code", {});  }   { svelteHTML.createElement("code", {});  }       }
    }
     { svelteHTML.createElement("div", { "class":`grid`,});
         for(let h of __sveltets_2_ensureArray(ctx.workloads)){h.cluster.id + h.row.namespace + h.row.workload;
        const st = health(h.row);
         { svelteHTML.createElement("div", { "class":`lc-card`,});
           { svelteHTML.createElement("div", { "class":`head`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`helm`,"size":13,}});} { svelteHTML.createElement("span", { "class":`t`,});h.row.workload; } { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {    "tone":st === 'ok' ? 'ok' : st === 'warn' ? 'warn' : 'bad',"label":st === 'ok' ? 'Healthy' : st === 'warn' ? 'Degraded' : 'Down',}});} }
           { svelteHTML.createElement("div", { "class":`sub`,});h.cluster.name;  h.row.namespace;  h.row.kind; }
           { svelteHTML.createElement("div", { "class":`stats`,});
             { svelteHTML.createElement("span", {});h.row.ready; h.row.pods;  }
             { svelteHTML.createElement("span", {});h.row.mem_pct.toFixed(0);  }
            if(h.row.rps > 0){ { svelteHTML.createElement("span", {});h.row.rps.toFixed(1);   h.row.err_pct.toFixed(1);  }}
           }
           { svelteHTML.createElement("div", { "class":`acts`,});
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => openWorkload(h),});  }
             { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => router.go(monitorPath(h.cluster.id)),});  }
           }
         }
      }
         for(let h of __sveltets_2_ensureArray(ctx.repos)){h.repo.id;
         { svelteHTML.createElement("div", { "class":`lc-card`,});
           { svelteHTML.createElement("div", { "class":`head`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`branch`,"size":13,}});} { svelteHTML.createElement("span", { "class":`t`,});h.repo.name; }if(h.repo.workspace_name){ { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {  "label":h.repo.workspace_name,}});}} }
           { svelteHTML.createElement("div", {   "class":`sub`,"title":h.repo.path,});h.repo.path; }
           { svelteHTML.createElement("div", { "class":`stats`,});
            if(h.status){
               { svelteHTML.createElement("span", {});h.status.branch; }
              if(h.status.ahead || h.status.behind){ { svelteHTML.createElement("span", {}); h.status.ahead; h.status.behind; }}
               { svelteHTML.createElement("span", { });h.status.changes.length > 0;h.status.changes.length ? `${h.status.changes.length} changed` : 'Clean'; }
            }else{
               { svelteHTML.createElement("span", { "class":`dim`,});  }
            }
            if(h.prs !== null){
               { svelteHTML.createElement("span", {});plural(h.prs, 'open PR'); }
            } else if (h.repo.remote_url){
               { svelteHTML.createElement("button", {           "class":`link prs`,...__sveltets_2_empty({"data-testid":`vault-live-prs`}),"title":`Count open pull requests (asks the forge)`,"disabled":prLoading === h.repo.id,"onclick":() => void loadPrs(h),});prLoading === h.repo.id ? 'Counting PRs…' : 'Count open PRs'; }
            }
           }
           { svelteHTML.createElement("div", { "class":`acts`,});
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => openRepo(h.repo.id),});  }
            if(h.repo.remote_url){ { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => openRepo(h.repo.id, 'prs'),});  }}
           }
         }
      }
         for(let c of __sveltets_2_ensureArray(ctx.connections)){c.id;
         { svelteHTML.createElement("div", { "class":`lc-card`,});
           { svelteHTML.createElement("div", { "class":`head`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`db`,"size":13,}});} { svelteHTML.createElement("span", { "class":`t`,});c.name; } { const $$_egdaBvnE4C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE4C({ target: __sveltets_2_any(), props: {  "env":c.environment,}});} }
           { svelteHTML.createElement("div", { "class":`sub`,});c.kind;c.read_only ? ' · read-only' : ''; }
           { svelteHTML.createElement("div", { "class":`acts`,}); { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go(`database/${encodeURIComponent(c.id)}`),});   } }
         }
      }
         for(let c of __sveltets_2_ensureArray(ctx.collections)){c.id;
         { svelteHTML.createElement("div", { "class":`lc-card`,});
           { svelteHTML.createElement("div", { "class":`head`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`send`,"size":13,}});} { svelteHTML.createElement("span", { "class":`t`,});c.name; } { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {  "label":`API collection`,}});} }
           { svelteHTML.createElement("div", { "class":`acts`,}); { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go('api'),});   } }
         }
      }
         for(let d of __sveltets_2_ensureArray(ctx.dashboards)){d.id;
         { svelteHTML.createElement("div", { "class":`lc-card`,});
           { svelteHTML.createElement("div", { "class":`head`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chart`,"size":13,}});} { svelteHTML.createElement("span", { "class":`t`,});d.name; } { const $$_egdaB4C = __sveltets_2_ensureComponent(Badge); new $$_egdaB4C({ target: __sveltets_2_any(), props: {  "label":`Dashboard`,}});} }
           { svelteHTML.createElement("div", { "class":`sub`,});plural(d.layout.length, 'panel'); }
           { svelteHTML.createElement("div", { "class":`acts`,}); { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => openDashboard(d),});  } }
         }
      }
     }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LiveContext__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LiveContext__SvelteComponent_ = ReturnType<typeof LiveContext__SvelteComponent_>;
/*Ωignore_endΩ*/export default LiveContext__SvelteComponent_;