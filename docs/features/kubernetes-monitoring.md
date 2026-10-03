# Kubernetes Monitoring — pod probes, restart classification, health digest

**Kubernetes → Monitor** is a Komodor-style dashboard on top of the
[Kubernetes console](./kubernetes-console.md). It is **opt-in per cluster** and
collects from two sources:

- **Your services' own HTTP endpoints**, fetched from every running pod with
  user-defined probes (`/actuator/info`, `/actuator/prometheus`, `/metrics`,
  anything that speaks JSON or the Prometheus text format). Nothing about
  Otto's own services is hard-wired; presets only fill the form.
- **metrics-server**, whenever the cluster's RBAC allows it (CPU + working-set
  memory per container). It is re-probed every cycle, and a denial is shown
  verbatim so you can forward the exact grant to the cluster admin.

On top of that every cycle diffs the pod list against the previous one and
**classifies restarts**, so an OOM kill never hides behind a rollout.

Contract: `docs/contracts/api.md` → "Monitoring". Design spec:
`docs/superpowers/specs/2026-09-05-k8s-monitoring-dashboard-design.md`.

## Setup

1. **Usage engine on.** Samples live in the embedded ClickHouse that Usage
   tracking already runs (Settings → Usage). Enabling monitoring without it
   returns `409` with that hint.
2. **Kubernetes → Monitor → pick a cluster → Settings.**
3. Choose a **preset** (Go actuator / Spring Boot actuator / plain `/metrics`),
   fix the **port** if your containers do not declare one, and adjust
   **namespaces** — required when the cluster has no default namespace (the
   kubeconfig user may not be allowed to list namespaces cluster-wide).
4. **Test probes**: saves the form, fetches every probe from one pod and shows
   what parsed (samples, labels, parse errors). Fix mappings until the numbers
   look right.
5. Tick **Monitoring on**, Save. The collector starts within 15 seconds and
   runs every `interval_secs` (default 60).

Enable on a staging cluster first. The collector is read-only, but it does
keep one `kubectl port-forward` per pod open when the API server denies the
pod proxy (see Transport).

## What a cycle does

```
sweep pods (kubectl get pods -o json)          → status samples for pods that changed
events (incremental watch from the last version) → OOMKilling / Killing / Unhealthy / Scaling…
metrics-server (re-probed, never cached)        → cpu_millis, mem_working_set_bytes
pick transport (auto: cached per cluster)       → re-sniffed hourly or on failure
scrape every running, non-excluded pod          → probe samples (bounded concurrency)
classify restarts + churn vs previous snapshot  → k8s_events rows
write ClickHouse → status row → WS k8s_monitor_cycle
```

Each cycle is ONE insert per table (all pods' samples in a single NDJSON
body, the wide pod rows in another, all events in a third) — one part per
cycle, so no `async_insert` is needed.

### Storage: wide pod rows, rollups and raw samples

The dashboards never re-aggregate raw samples. Two families of
pre-aggregated tables exist:

**Wide tiers (what the dashboards read).** Each cycle the collector writes
ONE `k8s_pod_cycle` row per pod: its memory, request and 5xx counts, latency
sum and count, and the latency histogram as a `le → count` map. Counters are
written as **increments** since the previous cycle, not as raw values. The
collector keeps the last value of every counter series per cluster. A drop in
value means the counter was reset, and then the new value is the increment
(Prometheus `increase()` semantics). A series seen for the first time adds 0.
After a daemon restart the last values are read back once from `k8s_latest`,
so a restart doesn't lose an interval. Memory is the most authoritative gauge
the pod has (`mem_working_set_bytes` → `mem_sys_bytes` →
`jvm_memory_used_bytes`), **summed** over its series. metrics-server reports
per container, and the collector writes the pod total (`container = ""`), so a
sidecar no longer turns a pod's memory into the average of its containers.

`k8s_pod_cycle` stores nothing (`ENGINE = Null`). Materialized views fold it
into:

| table | grain / row | kept |
|---|---|---|
| `k8s_wl_1m`, `k8s_wl_5m`, `k8s_wl_1h` | one row per workload per bucket: cycles, max pods, memory sum / max / last cycle's total, request / 5xx / latency totals, histogram (`sumMap`) | 2 d / 14 d / retention |
| `k8s_pod_1m`, `k8s_pod_5m`, `k8s_pod_1h` | the same per pod (Fleet `group=pod`, a pod filter, per-pod lines) | 2 d / 14 d / retention |

They use `index_granularity = 1024`, and each sort key has a coarse time
prefix (hour, day or week), so a per-workload read touches about a thousand
rows. The workloads tab, the health digest, the overview, the sparklines and
the Fleet table and charts (memory, req/s, 5xx %, latency) read these tiers.
Increments are additive, so a window **total** is stitched together. The whole
5-minute buckets come from the 5-minute tier, and the ragged start and the
open edge come from the minute tier. Over 1 h that is ≤ 5 + 12 + 5 rows per
workload instead of 60. A span that ended before the tier's current bucket
can't change any more, so its answer is cached until the next bucket boundary.
As a result, the 24 h baselines are computed once an hour, not on every
cycle. Workload memory in a chart or a Fleet row is the mean workload
**total** per cycle. Summing per-pod means would double-count pods replaced
during the window.

Upgrading: the wide tables and views are created idempotently on the first
collector start. They are not back-filled, because they fill from the next
cycle. Until 24 h have passed, baselines cover only the hours collected since
the upgrade.

**Per-series tiers (generic charts).** Views fold every `k8s_samples` insert
into tiers keyed by series (`cluster, namespace, workload, metric, pod,
labels`) that store `min / max / sum / count / last` per bucket. The generic
`/monitor/series?metric=<anything>` chart and Fleet → Requests (per path)
read them:

| table | what | kept |
|---|---|---|
| `k8s_samples` | raw scraped values | 2 days (short drill-downs only) |
| `k8s_samples_1m` | 1-minute rollup | 2 days |
| `k8s_samples_5m` | 5-minute rollup | 14 days |
| `k8s_samples_1h` | 1-hour rollup | the retention (≤ 90 days) |
| `k8s_latest` | last value per series (current memory, counter seeding) | 1 day |
| `k8s_pods_1h` | hourly pod inventory (Fleet filters, pod lists) | the retention |
| `k8s_events` | classified restarts / churn / k8s events | the retention |

Every read plans the **coarsest** tier that still gives the window 24
buckets and divides the chart step: a 1 h window reads the minute tier, 6 h
the 5-minute tier, 24 h and 7 d the hour tier; only windows under 24 minutes
(or sub-minute chart steps) read raw rows. The window start snaps down to
the tier's bucket and rates divide by the seconds actually covered. Charts use
steps aligned to the tiers (whole minutes, 5 minutes, and **whole hours from a
24 h window up**, which keeps day charts on the hour tier), so `step_secs` in a
response can be larger than requested. The per-series counter math is
`max − min` per series, which under-counts a window with a counter reset. The
wide tiers don't have that problem.

The ClickHouse integration test (`k8s_monitor_clickhouse.rs`) checks every
query against raw rows. The per-series tiers are compared with raw
`max − min`. The wide tiers are compared with a raw-increment reference that
includes a counter reset, a two-container pod and the same workload name in
two namespaces. The test also measures rows read per refresh (`EXPLAIN
ESTIMATE`) before and after.

Versions (drift, the per-pod `version`) come from the collector's pod
snapshot, not from a ClickHouse scan.

The keeps above are capped by the cluster's `retention_days`; the hour
tiers, the pod inventory and events follow the largest retention among
enabled clusters (the tables' TTL), and a cluster that keeps fewer days is
trimmed with a `DELETE` at most once a day, not per cycle. Changing a TTL
never rewrites existing parts.

**Upgrading a raw-only install** (before the per-series rollups) is
automatic: on the first collector start the rollup tables are created and
back-filled from the raw rows already there (one `(cluster, day)` partition
per statement, two ClickHouse threads), the views are created only after
that — every collector loop waits on the same lock, so nothing is counted
twice and an interrupted backfill simply reruns — and raw days older than 2
days are dropped.

Status series written from the sweep alone: `restarts_total`, `ready`,
`phase_running`, `mem_limit_bytes`, `cpu_request_millis` (and
`pod_age_seconds`). They feed only the generic series picker, so a pod's rows
are written when one of the first five changed and for every pod at least
every 15 minutes (and on the loop's first cycle) — not 6 rows per pod per
cycle. Restarts, limits and pod counts on the dashboards come from events,
the wide rows and the snapshot.

Events are read incrementally: after the first full list per namespace the
collector resumes a short watch (`watch=1&resourceVersion=<last>`, closed by
the API server after 1 s) through the cluster's `kubectl proxy`, so only new
events cross the wire; an expired version relists. metrics-server is read per
namespace in parallel through the same proxy. Without the proxy both fall
back to `kubectl`.

## Probes

| field | meaning |
|---|---|
| `port` | container port to hit; blank = the container's first declared port |
| `path` | must start with `/` |
| `format` | `json` (field mappings), `prometheus` (text format), `health` (records `up` = 1 on 2xx) |
| `mappings` (json) | `field` is a dotted path (`memory_stats.sys`, `items.0.x`); `metric` emits a sample, `label` attaches a pod-level label (e.g. `build_info.version → version`, used for **version drift**) |
| `unit` | `number`, `bytes`, `bytes_human` (`"27 MB"`, `512Mi`, `1.5GiB` — all binary multiples), `duration_human` (`1m30s`, `250ms`), `percent` |
| `include` / `exclude` (prometheus) | series-name globs (`http_*`, `*_bucket`); empty include = everything |
| labels kept (prometheus) | only `code` / `status` / `status_code` / `le` survive ingest; every other label (`path`, `method`, `handler`…) is folded away and values summed. A gateway with hundreds of paths shrinks from ~1500 rows per pod per cycle to a few dozen, which is what keeps the dashboard queries fast |
| `timeout_ms` | 100..30000, default 3000 |

Limits: 10 probes, 200 mappings, and `series_cap` distinct series per pod per cycle (default 1500, 100..10000 in Settings; `_bucket` series are dropped first when a body overflows it — the
status shows `series_capped` when a probe overflows — tighten the globs).

**Exclusions** skip the scrape but keep the pod in the sweep: `pod` / `namespace`
/ `workload` globs (`*` `?`; workload globs match `kind:name`, e.g.
`cronjob:*`) and `label` selectors (`app=frb,tier!=web,batch`).

## Transport

`auto` tries `GET /api/v1/namespaces/…/pods/<pod>:<port>/proxy<path>` (see
below for how the answer is judged and cached). If the API server proxies it,
every probe goes through the proxy. The
collector keeps ONE long-lived `kubectl proxy` per monitored cluster (on a
Unix socket in a private `0700` temp directory, accepting only pod-proxy GETs)
and sends each probe to it as a pooled HTTP request — no process per pod
(the same proxy also serves the read-only events and metrics-server lists). The
proxy is restarted when it exits or its credentials change (kubeconfig or
token overlay rewritten), and stops with the loop; if it cannot start, each
probe falls back to a short `kubectl get --raw`. `auto` sniffs up to 3 pods (spread over the target list) and only an
**API-server** refusal selects port-forward: a Kubernetes `Status` answer
with reason `Forbidden`/`Unauthorized`, or a broken hop to the API server.
The app's own answer — a 404 from `/actuator/info`, a 500 — proves the proxy
works and keeps it; a pod that vanished or is not listening is inconclusive
and the next pod is tried (all inconclusive ⇒ proxy, re-sniffed next cycle).
A conclusive decision is cached per cluster and re-sniffed hourly, on a
transport/config change, or when every pod failed over it. If the proxy is
denied — Rancher-managed clusters typically deny `pods/proxy` — the collector
uses `kubectl port-forward pod/<pod> 0:<port>` with `concurrency` parallel
pods (default 8, max 32) and a plain HTTP GET on loopback. Forwards are kept
open across cycles (one per pod/port, closed when the pod leaves the target
list or the forward breaks) with one shared HTTP client, so a steady cluster
spawns no processes per cycle. If a port-forward cycle still exceeds the
interval, the status line says so (`port-forward transport: cycle took …`);
raise concurrency or the interval.

## Restart classes

| class | rule |
|---|---|
| `oom` | container `lastState.terminated.reason == OOMKilled`, or an `OOMKilling` event |
| `probe` | `Unhealthy` (liveness) followed by `Killing` within 2 minutes |
| `crash` | `Error` / `ContainerCannotRun` / non-zero exit / `CrashLoopBackOff` |
| `planned` (churn) | new ReplicaSet (`rollout`), `ScalingReplicaSet` (`scale`), Evicted/Preempted (`drain`), or an Otto `k8s.action.*` on the workload in the last 5 minutes (`otto:<user>`) |
| `completed` (churn) | a Job's pod finished with exit 0 |
| `unknown` | counter rose / pod changed but nothing matched; the raw reason is kept |
| `version` (new version came up) | the workload's dominant build version (the `version` label from a JSON probe, e.g. `build_info.version`) changed between cycles; shown as "New version `<from> → <to>`" in Events, listed under `deployments` in the health digest, and reported by the watchdog even when everything is healthy |

Restart counters are per pod, so a rollout never inflates "restarts"; it is
counted separately as **churn**. The first cycle after enabling has no baseline
and records nothing.

## Dashboard

Reads are cached per (cluster, window, namespace) and invalidated by the
collector's cycle timestamp, so a tab switch or the watchdog's poll never
re-runs ClickHouse aggregations for unchanged data; Fleet answers are reused
for 15 s and then until any collector writes again. Identical requests that
arrive together (the Home box and an open Monitor page, two windows) run the
ClickHouse queries once — the rest wait for that answer. The queries behind
the workloads table run concurrently.

The pages refresh on collection cycles, not on a timer: cycle events are
coalesced to at most one refresh every 30 s however many clusters are
monitored, nothing refreshes while the window is hidden (one refresh runs
when it comes back), and a page that just opened does not re-load on the
first event.


- **Overview** (`#/kubernetes/monitor`): one card per cluster — health badge
  (`healthy` / `degraded` / `incident`), pods, unplanned restarts by class,
  memory vs limits, requests/s and 5xx %, workloads running mixed versions,
  the collector line, and the metrics-server RBAC hint with a Copy button.
- **Per-cluster Monitor** (`#/kubernetes/<id>/monitor[/<tab>]`): the Monitor
  half of the cluster workspace. A **Resources | Monitor** switch in the
  header moves between the console and the Monitor of the same cluster; the
  cluster stays selected, the console's cached rows stay (switching back
  paints at once and refreshes quietly), and picking a namespace in the
  Monitor sets it in Resources too. The old
  `#/kubernetes/monitor/<id>/<tab>` links redirect here.
- **Remembered view**: namespace, filter, sort, the expanded workload row, the
  Events class filter and the scroll position are kept per cluster in the
  k8s store (persisted in `localStorage` under `otto_k8s_ui:<user>:<cluster>`),
  so the Monitor comes back as it was after a module switch, Resources ↔
  Monitor or a reload.
- **Workloads**: sortable table (memory, restarts, churn, req/s, 5xx, p95 or
  avg latency, versions) with sparklines; click a row for the memory and
  request-rate series of the window. Latency is `p95` when the probe exports
  histogram buckets, `avg` (`_sum/_count`) otherwise. The expanded row lists
  the pods behind the workload — click a pod to open its drawer (Metrics tab)
  in Resources — and **Open pods in Resources** (also on the row's context
  menu) opens the Pods table filtered to the workload.
- **Events**: classified restarts / churn newest first, filterable by class;
  `Raw cluster events` shows the kept Kubernetes events. Click an event's pod
  to open that pod's drawer on its Events tab.
- **From the console**: a workload drawer (and a pod's, via its owner) has a
  **Monitor** button that opens this view with that workload expanded, and
  the pod Metrics tab shows a **History (Monitor)** section (last hour of the
  pod's memory and request-rate series) with **Open in Monitor**.
- **Insights**: the latest report of the workspace's **Kubernetes watchdog**
  agent (below) with run history, Run now, and the verdict badge.
- **Settings**: everything in Setup, plus **Keep request path labels** (off by
  default) — retains per-route `path` + `method` on the request / latency
  counters (never on histogram buckets) so the Fleet dashboard's Requests tab
  can drill down to a route. It multiplies request rows per pod by the number
  of routes, so enable it per cluster where you need it.

### Fleet dashboard (`#/kubernetes/monitor/fleet`)

One dashboard over **every** cluster, read from **ClickHouse only** — it never
calls a cluster and ignores the collector's live pod snapshot, so it answers
"what happened over the window" even for a cluster that is unreachable right
now. Filters are **cluster** (toggle pills; none = all), **namespace**,
**workload** and **pod** (the pod list appears once the selection is narrow),
plus the window; every choice, the table grouping / sort and the events sort
are **persisted per device** and survive a reload.

- **Overview**: KPI tiles (unplanned restarts with the OOM / crash split,
  planned churn, latest memory, req/s + 5xx %) and five charts — restarts by
  class (stacked per class), memory, req/s, 5xx %, avg latency — one line per
  cluster / namespace / workload / pod (your pick).
- **Table**: one row per workload or per pod — pods, restarts (with the class
  breakdown), OOM, churn, memory (latest / peak), req/s, 5xx %, p95-or-avg
  latency — **sortable by every column**, sorted server-side, paged. Clicking a
  workload row narrows the filters to it and switches to its pods; clicking a
  pod row opens its events.
- **Events**: the cross-cluster restart / churn / version timeline, class
  filter, sortable by time, cluster, namespace, workload, pod, class or
  reason, paged.
- **Requests**: per-route req/s, 5xx % and avg latency — needs *Keep request
  path labels* on at least one cluster; the tab links to each cluster's
  settings until then.

Routes: `GET /k8s/monitor/fleet/{filters,table,series,series/batch,events,requests}` — see
the contract for the exact shapes and the identifier / sort-key allow-lists.

Health badge: `incident` when a pod is in CrashLoopBackOff / Failed, or an
OOM/crash coincides with an error-rate spike; `degraded` on any unplanned
restart, memory ≥ 85 % of limit or +25 % over the window, 5xx ≥ 3× the 24 h
baseline (and ≥ 1 %), p95 ≥ 3× baseline, or ≥ 20 % scrape failures.

## The watchdog agent

Personal Agents → New agent → template **Kubernetes watchdog**. It creates an
agent whose persona tells it to call `k8s_list_clusters` + `k8s_health` for
every monitored cluster every 15 minutes, separate unplanned restarts from
planned churn, correlate error spikes with rollouts, pull at most three pod
logs for unexplained crashes, and end with `Verdict: HEALTHY | DEGRADED |
INCIDENT`. Delivery (Slack / Telegram / email / webhook) and notify-on-change
work like any other personal agent, so quiet cycles stay silent.

`k8s_health(cluster_id, window)` is also available to any agent through the
Otto MCP server (read-only). It returns the collector status, pod counts,
classified restarts with pod + memory-limit detail, churn by workload, memory
outliers, error-rate and latency spikes vs baseline, version drift, and the
thresholds used — every list capped at 20 entries.

## API surface

`GET/PUT /k8s/clusters/{id}/monitor`, `POST …/monitor/test`, `POST …/monitor/run`,
`GET /k8s/monitor/overview`, `GET …/monitor/workloads`, `GET …/monitor/series`,
`GET …/monitor/events`, `GET …/monitor/health`, the ClickHouse-only fleet
routes `GET /k8s/monitor/fleet/{filters,table,series,events,requests}`, WS
`k8s_monitor_cycle`. Full shapes in `docs/contracts/api.md`.

## Limits and known gaps

- **CPU without metrics-server** needs the process to export it. For Go
  services, registering the default process/Go collectors in the shared web
  package adds `process_cpu_seconds_total` and `go_memstats_*` to
  `/actuator/prometheus`; add `process_*` to the probe's include list.
- Dashboard rates are sums of reset-aware increments: a counter reset is
  counted correctly. A series' first cycle (a new pod, a new label set, a
  version label change) adds 0. The generic per-metric chart and Fleet →
  Requests still use per-series `max − min`, which under-counts a window
  with a reset. Label collapse (only `code` / `status` / `status_code` / `le`
  are kept) sums the dropped-label series into one counter, so the collapsed
  counter jumps when a new dropped-label series appears. That was already
  true before the wide tiers.
- A Fleet workload row's `pods` is the most pods seen in one cycle (or the
  number of pods with events, if larger), not every pod name seen during the
  window.
- Kubernetes keeps events for about an hour on EKS; a 60 s interval loses
  nothing, an interval above 1 h can miss the `Unhealthy`/`Killing` pair.
- Port-forward transport keeps one `kubectl port-forward` process per scraped
  pod open; on very large namespaces prefer granting `pods/proxy`.

## Troubleshooting

| symptom | cause / fix |
|---|---|
| Card says `metrics-server: forbidden: … cannot list resource "pods" in API group "metrics.k8s.io"` | Cluster RBAC. Copy the message to the admin; the collector picks it up on the next cycle with no restart. |
| `pods_failed` ≈ `pods_seen` | Wrong port or path. Run **Test probes** on one pod; check `body_preview`. |
| `series_capped on N probe(s)` | A prometheus probe exports too many series. Add `include` globs or exclude `*_bucket`. |
| Cycle slower than the interval | Raise `concurrency` (port-forward) or the interval; check the collector line for the transport in use. |
| 409 when enabling | Usage engine (ClickHouse) is off or still starting — Settings → Usage. |
| Insights tab says no watchdog | Create one from the template; the tab finds agents by the template marker in their persona. |

## Development

`OTTO_K8S_E2E=1 cargo test -p otto-k8s --test monitor_minikube` runs one real
cycle against the current kubeconfig context (intended for minikube) using an
in-memory sink; without the variable the test is a no-op. Router-level tests
run through the fake kubectl in `crates/otto-k8s/tests/fake_kubectl.rs`
(`monitor_*`). UI: `cd ui && OTTO_E2E_BIN=target/debug/ottod npx playwright test desktop-k8s-monitor --project=desktop-browser`.
