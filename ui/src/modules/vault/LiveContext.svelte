<script module lang="ts">
  // Shared 60 s cache of the cross-module reads, so flipping between notes of
  // one bundle does not refetch the repo directory / monitor overview per note.
  const TTL_MS = 60_000;
  const cache = new Map<string, { at: number; p: Promise<unknown> }>();
  function cached<T>(key: string, load: () => Promise<T>): Promise<T> {
    const hit = cache.get(key);
    if (hit && Date.now() - hit.at < TTL_MS) return hit.p as Promise<T>;
    const p = load();
    p.catch(() => cache.delete(key)); // failures retry on the next note
    cache.set(key, { at: Date.now(), p });
    return p;
  }
</script>

<script lang="ts">
  // LIVE CONTEXT for a typed vault note: matches the note's entity hints
  // (service / workload names, repo paths, DB names, collections, dashboards)
  // against the rest of Otto through the modules' existing read APIs — K8s
  // monitor workloads, the repo directory + status, DB connections and
  // dashboards, API-client collections — and links into those views. Read
  // only: nothing here changes another module's state beyond navigating.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
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
  import { monitorPath, resourcesPath } from '../kubernetes/viewState';

  let { hints, wsId }: { hints: EntityHints; wsId: string } = $props();

  interface WorkloadHit { cluster: K8sMonitorOverviewRow['cluster']; row: K8sMonitorWorkloadRow }
  interface RepoHit { repo: RepoDirectoryEntry; status: RepoStatusResp | null; prs: number | null }
  interface Ctx {
    workloads: WorkloadHit[];
    repos: RepoHit[];
    connections: Connection[];
    collections: ApiCollection[];
    dashboards: DbDashboard[];
    /** Sources that failed for a reason other than "not enabled / no access". */
    failed: string[];
  }

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

  async function k8sHits(): Promise<WorkloadHit[]> {
    if (!hints.services.length) return [];
    const overview = await cached('k8s:overview', () => k8sApi.monitorOverview('1h'));
    const want = hints.k8sCluster ? norm(hints.k8sCluster) : null;
    const clusters = overview
      .filter((r) => r.enabled && (!want || norm(r.cluster.name) === want || r.cluster.id === hints.k8sCluster))
      .slice(0, 6);
    const per = await Promise.all(
      clusters.map(async (r) => {
        try {
          const w = await cached<K8sMonitorWorkloadsResp>(`k8s:wl:${r.cluster.id}:${hints.k8sNamespace ?? ''}`, () =>
            k8sApi.monitorWorkloads(r.cluster.id, '1h', hints.k8sNamespace ?? undefined),
          );
          return matchByName(hints.services, w.workloads, (x) => x.workload).map((row) => ({ cluster: r.cluster, row }));
        } catch {
          return [];
        }
      }),
    );
    return per.flat().slice(0, 8);
  }

  async function repoHits(): Promise<RepoHit[]> {
    if (!hints.repos.length) return [];
    const dir = await cached<RepoDirectory>('git:directory', () => api.bg.get<RepoDirectory>('/git/repos/directory'));
    const matched = matchRepos(hints.repos, dir.repos).slice(0, 3);
    return Promise.all(
      matched.map(async (repo, i) => {
        const status = await api.bg.get<RepoStatusResp>(`/repos/${repo.id}/status`).catch(() => null);
        // Open PRs ask the forge — only for the best match, never blocking the card.
        const prs = i === 0 && repo.remote_url
          ? await api.bg.get<{ items: unknown[] }>(`/repos/${repo.id}/prs?state=open`).then((r) => r.items.length, () => null)
          : null;
        return { repo, status, prs };
      }),
    );
  }

  async function load(): Promise<void> {
    const mine = ++seq;
    loading = true;
    const failed: string[] = [];
    const nameHints = [...hints.databases, ...hints.services];
    const [workloads, repos, connections, collections, dashboards] = await Promise.all([
      source('Kubernetes', failed, k8sHits),
      source('Git', failed, repoHits),
      source('Connections', failed, async () =>
        hints.databases.length || hints.services.length
          ? matchByName(nameHints, await cached(`conn:${wsId}`, () => api.bg.get<Connection[]>(`/workspaces/${wsId}/connections`)), (c) => c.name)
          : [],
      ),
      source('API collections', failed, async () =>
        hints.collections.length
          ? matchByName(hints.collections, await cached(`coll:${wsId}`, () => api.bg.get<ApiCollection[]>(`/workspaces/${wsId}/api-client/collections`)), (c) => c.name)
          : [],
      ),
      source('Dashboards', failed, async () =>
        hints.dashboards.length || hints.services.length
          ? matchByName([...hints.dashboards, ...hints.services], await cached(`dash:${wsId}`, () => api.bg.get<DbDashboard[]>(`/workspaces/${wsId}/db/dashboards`)), (d) => d.name)
          : [],
      ),
    ]);
    if (mine !== seq) return;
    ctx = { workloads, repos, connections, collections, dashboards, failed };
    loading = false;
  }

  // Keyed on the hints' VALUE: a poll that re-delivers the same note builds a
  // new (equal) hints object and must not refetch repo status per tick.
  const key = $derived(`${wsId}|${JSON.stringify(hints)}`);
  $effect(() => {
    void key;
    untrack(() => void load());
  });

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
</script>

<section class="live" aria-label="Live context" aria-busy={loading}>
  <h4>
    <Icon name="radar" size={13} /> Live context
    {#if ctx && !loading}
      <button class="refresh" title="Refresh live context" aria-label="Refresh live context" onclick={() => { cache.clear(); void load(); }}>
        <Icon name="refresh" size={12} />
      </button>
    {/if}
  </h4>
  {#if loading && !ctx}
    <p class="dim" role="status">Looking for matching workloads, repos and connections…</p>
  {:else if ctx}
    {#if ctx.failed.length}
      <p class="warn" role="alert">
        Couldn’t read {ctx.failed.join(', ')}.
        <button class="link" onclick={() => { cache.clear(); void load(); }}>Retry</button>
      </p>
    {/if}
    {#if total === 0 && !ctx.failed.length}
      <p class="dim">Nothing in Otto matches this note yet. Add <code>service</code>, <code>repository</code>, <code>connections</code> or <code>dashboards</code> to its frontmatter to link it.</p>
    {/if}
    <div class="grid">
      {#each ctx.workloads as h (h.cluster.id + h.row.namespace + h.row.workload)}
        {@const st = health(h.row)}
        <div class="lc-card">
          <div class="head"><Icon name="helm" size={13} /><span class="t">{h.row.workload}</span><span class="pill {st}">{st === 'ok' ? 'Healthy' : st === 'warn' ? 'Degraded' : 'Down'}</span></div>
          <div class="sub">{h.cluster.name} · {h.row.namespace} · {h.row.kind}</div>
          <div class="stats">
            <span>{h.row.ready}/{h.row.pods} ready</span>
            <span>{h.row.mem_pct.toFixed(0)}% mem</span>
            {#if h.row.rps > 0}<span>{h.row.rps.toFixed(1)} rps · {h.row.err_pct.toFixed(1)}% err</span>{/if}
          </div>
          <div class="acts">
            <button class="btn small" onclick={() => openWorkload(h)}>Open workload</button>
            <button class="btn small ghost" onclick={() => router.go(monitorPath(h.cluster.id))}>K8s Monitor</button>
          </div>
        </div>
      {/each}
      {#each ctx.repos as h (h.repo.id)}
        <div class="lc-card">
          <div class="head"><Icon name="branch" size={13} /><span class="t">{h.repo.name}</span>{#if h.repo.workspace_name}<span class="pill">{h.repo.workspace_name}</span>{/if}</div>
          <div class="sub" title={h.repo.path}>{h.repo.path}</div>
          <div class="stats">
            {#if h.status}
              <span>{h.status.branch}</span>
              {#if h.status.ahead || h.status.behind}<span>↑{h.status.ahead} ↓{h.status.behind}</span>{/if}
              <span class:warnText={h.status.changes.length > 0}>{h.status.changes.length ? `${h.status.changes.length} changed` : 'Clean'}</span>
            {:else}
              <span class="dim">Status unavailable</span>
            {/if}
            {#if h.prs !== null}<span>{h.prs} open PR{h.prs === 1 ? '' : 's'}</span>{/if}
          </div>
          <div class="acts">
            <button class="btn small" onclick={() => openRepo(h.repo.id)}>Open repo</button>
            {#if h.repo.remote_url}<button class="btn small ghost" onclick={() => openRepo(h.repo.id, 'prs')}>Pull requests</button>{/if}
          </div>
        </div>
      {/each}
      {#each ctx.connections as c (c.id)}
        <div class="lc-card">
          <div class="head"><Icon name="db" size={13} /><span class="t">{c.name}</span><span class="pill" class:prodPill={c.environment === 'prod'}>{c.environment}</span></div>
          <div class="sub">{c.kind}{c.read_only ? ' · read-only' : ''}</div>
          <div class="acts"><button class="btn small" onclick={() => router.go(`database/${encodeURIComponent(c.id)}`)}>Open in Database</button></div>
        </div>
      {/each}
      {#each ctx.collections as c (c.id)}
        <div class="lc-card">
          <div class="head"><Icon name="send" size={13} /><span class="t">{c.name}</span><span class="pill">API collection</span></div>
          <div class="acts"><button class="btn small" onclick={() => router.go('api')}>Open API client</button></div>
        </div>
      {/each}
      {#each ctx.dashboards as d (d.id)}
        <div class="lc-card">
          <div class="head"><Icon name="chart" size={13} /><span class="t">{d.name}</span><span class="pill">Dashboard</span></div>
          <div class="sub">{d.layout.length} panel{d.layout.length === 1 ? '' : 's'}</div>
          <div class="acts"><button class="btn small" onclick={() => openDashboard(d)}>Open dashboard</button></div>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  .live { margin-block-start: 12px; }
  h4 {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 0 6px;
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .refresh {
    margin-inline-start: auto;
    background: none;
    border: 0;
    color: var(--text-dim);
    cursor: pointer;
    padding: 2px;
    border-radius: var(--radius-s);
  }
  .refresh:hover { color: var(--text); }
  p { margin: 4px 0; font-size: var(--fs-s); }
  .dim { color: var(--text-dim); }
  .warn { color: var(--warning); }
  .link { background: none; border: 0; padding: 0; color: var(--accent-text); cursor: pointer; font: inherit; text-decoration: underline; }
  .grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 8px; }
  .lc-card {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 8px 10px;
    background: var(--surface);
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .head { display: flex; align-items: center; gap: 6px; min-width: 0; }
  .t { font-weight: 600; font-size: var(--fs-s); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; flex: 1; min-width: 0; }
  .sub { font-size: var(--fs-xs); color: var(--text-dim); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .stats { display: flex; flex-wrap: wrap; gap: 4px 10px; font-size: var(--fs-xs); color: var(--text-dim); }
  .warnText { color: var(--warning); }
  .acts { display: flex; flex-wrap: wrap; gap: 6px; margin-block-start: 2px; }
  .pill { font-size: var(--fs-xs); padding: 1px 6px; border-radius: 999px; border: 1px solid var(--border); color: var(--text-dim); white-space: nowrap; }
  .pill.ok { color: var(--success); border-color: var(--success); background: var(--success-soft); }
  .pill.warn { color: var(--warning); border-color: var(--warning); background: var(--warning-soft); }
  .pill.bad, .pill.prodPill { color: var(--danger); border-color: var(--danger); background: var(--danger-soft); }
</style>
