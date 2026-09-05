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
open one `kubectl port-forward` per pod per cycle when the API-server proxy is
denied (see Transport).

## What a cycle does

```
sweep pods (kubectl get pods -o json)          → status samples per pod
events (last cycle → now)                       → OOMKilling / Killing / Unhealthy / Scaling…
metrics-server (re-probed, never cached)        → cpu_millis, mem_working_set_bytes
pick transport (auto: proxy? else port-forward) → one decision per cycle
scrape every running, non-excluded pod          → probe samples (bounded concurrency)
classify restarts + churn vs previous snapshot  → k8s_events rows
write ClickHouse → status row → WS k8s_monitor_cycle
```

Status series written from the sweep alone: `restarts_total`, `ready`,
`phase_running`, `mem_limit_bytes`, `cpu_request_millis`, `pod_age_seconds`.

## Probes

| field | meaning |
|---|---|
| `port` | container port to hit; blank = the container's first declared port |
| `path` | must start with `/` |
| `format` | `json` (field mappings), `prometheus` (text format), `health` (records `up` = 1 on 2xx) |
| `mappings` (json) | `field` is a dotted path (`memory_stats.sys`, `items.0.x`); `metric` emits a sample, `label` attaches a pod-level label (e.g. `build_info.version → version`, used for **version drift**) |
| `unit` | `number`, `bytes`, `bytes_human` (`"27 MB"`, `512Mi`, `1.5GiB` — all binary multiples), `duration_human` (`1m30s`, `250ms`), `percent` |
| `include` / `exclude` (prometheus) | series-name globs (`http_*`, `*_bucket`); empty include = everything |
| `timeout_ms` | 100..30000, default 3000 |

Limits: 10 probes, 200 mappings, 500 distinct series per pod per cycle (the
status shows `series_capped` when a probe overflows — tighten the globs).

**Exclusions** skip the scrape but keep the pod in the sweep: `pod` / `namespace`
/ `workload` globs (`*` `?`; workload globs match `kind:name`, e.g.
`cronjob:*`) and `label` selectors (`app=frb,tier!=web,batch`).

## Transport

`auto` tries `GET /api/v1/namespaces/…/pods/<pod>:<port>/proxy<path>` once per
cycle. If the API server answers, every probe goes through the proxy (one
short `kubectl` call each). If it is denied — Rancher-managed clusters
typically deny `pods/proxy` — the collector falls back to
`kubectl port-forward pod/<pod> 0:<port>` with `concurrency` parallel
forwards (default 8, max 32) and a plain HTTP GET on loopback. 230 pods at
concurrency 8 take roughly 30 seconds; if `cycle_ms` exceeds the interval,
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

Restart counters are per pod, so a rollout never inflates "restarts"; it is
counted separately as **churn**. The first cycle after enabling has no baseline
and records nothing.

## Dashboard

- **Overview** (`#/kubernetes/monitor`): one card per cluster — health badge
  (`healthy` / `degraded` / `incident`), pods, unplanned restarts by class,
  memory vs limits, requests/s and 5xx %, workloads running mixed versions,
  the collector line, and the metrics-server RBAC hint with a Copy button.
- **Workloads**: sortable table (memory, restarts, churn, req/s, 5xx, p95 or
  avg latency, versions) with sparklines; click a row for the memory and
  request-rate series of the window. Latency is `p95` when the probe exports
  histogram buckets, `avg` (`_sum/_count`) otherwise.
- **Events**: classified restarts / churn newest first, filterable by class;
  `Raw cluster events` shows the kept Kubernetes events.
- **Insights**: the latest report of the workspace's **Kubernetes watchdog**
  agent (below) with run history, Run now, and the verdict badge.
- **Settings**: everything in Setup.

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
`GET …/monitor/events`, `GET …/monitor/health`, WS `k8s_monitor_cycle`. Full
shapes in `docs/contracts/api.md`.

## Limits and known gaps

- **CPU without metrics-server** needs the process to export it. For Go
  services, registering the default process/Go collectors in the shared web
  package adds `process_cpu_seconds_total` and `go_memstats_*` to
  `/actuator/prometheus`; add `process_*` to the probe's include list.
- Counter rates are `max − min` over the window per (pod, labels); a counter
  reset inside the window under-counts that pod rather than spiking negative.
- Kubernetes keeps events for about an hour on EKS; a 60 s interval loses
  nothing, an interval above 1 h can miss the `Unhealthy`/`Killing` pair.
- Port-forward transport spawns one `kubectl` per pod per cycle; on very large
  namespaces prefer granting `pods/proxy`.

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
(`monitor_*`). UI: `cd ui && npx playwright test e2e/k8s-monitor.spec.ts --project=desktop-browser`.
