# Kubernetes Console — clusters, workloads, logs, exec & Argo actions

The **Kubernetes console** is a k9s-class view over any cluster you can already
reach with `kubectl`: register a kubeconfig context (or paste a kubeconfig, or
import an EKS cluster from the AWS console), then browse namespaces, nodes,
pods, workloads, services, config, storage, autoscalers, **Argo Rollouts** and
**Argo CD Applications**; read describe / manifest / events; tail or **follow
logs**; open a **shell in a pod** or a full **k9s** tab as a normal Otto
terminal session; and run the everyday operational verbs — restart, scale,
rollout undo/pause/resume, promote/abort/retry a rollout, sync/refresh/redeploy
an Argo CD app, trigger a CronJob — each as a plain `kubectl` command.

Design decisions that shape everything below (the build contract is
[`docs/design/aws-k8s-consoles.md`](../design/aws-k8s-consoles.md)):

- **Only `kubectl`.** The daemon shells out to the `kubectl` binary with
  `-o json` and normalises in Rust. There is no kube-rs client, no `argocd`
  CLI and no `kubectl-argo-rollouts` plugin — Argo is driven by patching the
  CRDs exactly the way those tools do. Whatever auth your kubeconfig uses
  (client certs, OIDC, `aws eks get-token`, `gke-gcloud-auth-plugin`, …) works
  unchanged because kubectl is the one doing it.
- **Otto never edits your kubeconfig.** Every call is
  `kubectl [--kubeconfig <file>] --context <ctx> …`, so the current-context in
  `~/.kube/config` is never switched. Kubeconfigs Otto owns (pasted / EKS)
  live in `<data_dir>/kube/<id>.yaml` with mode `0600`.
- **Missing tools install themselves.** If `kubectl` (or `k9s`) is absent the
  UI offers a one-click install — Homebrew when present, else a direct
  download into `<data_dir>/bin` (already on the daemon's `PATH`). Never `sudo`.
- **RBAC on two levels.** Otto's `kubernetes` feature grant decides what a user
  may *ask*; the cluster's own RBAC decides what kubectl may *do*. A cluster
  denial surfaces as `403 cluster RBAC: …` rather than being hidden by a probe.

---

## 1. Overview & where it lives

| Piece | Location |
|---|---|
| Sidebar entry / routes | **Kubernetes** → `#/kubernetes` (clusters), `#/kubernetes/<clusterId>` (workspace), `#/kubernetes/<clusterId>/<kind>[/<ns>/<name>]` |
| UI module | `ui/src/modules/kubernetes/`, store `ui/src/lib/stores/k8s.svelte.ts`, API client `ui/src/lib/api/k8s.ts` |
| Backend crate | `crates/otto-k8s` — `cli.rs` (kubectl runner), `install.rs`, `clusters.rs`, `resources.rs`, `logs.rs`, `actions.rs`, `sessions.rs`, `http.rs` |
| Persistence | `k8s_clusters` table (migration `0114_k8s_clusters.sql`), repo `crates/otto-state/src/k8s_clusters.rs` |
| Files on disk | `<data_dir>/kube/<id>.yaml` (Otto-owned kubeconfigs, 0600), `<data_dir>/bin/{kubectl,k9s}` (auto-installed binaries) |
| REST | `/api/v1/k8s/*` — see [`docs/contracts/api.md`](../contracts/api.md) "Kubernetes console" |
| WS events | `k8s_cluster_updated`, `k8s_install_updated` — [`docs/contracts/ws.md`](../contracts/ws.md) |
| RBAC | feature `kubernetes` (View / Edit / Admin) — `crates/otto-server/src/policy.rs` |
| Agent tools | `k8s_list_clusters`, `k8s_get_resources`, `k8s_describe`, `k8s_logs`, `k8s_top`, `k8s_action` — [`./mcp-control-plane.md`](./mcp-control-plane.md) |

`<data_dir>` is `~/Library/Application Support/Otto`.

---

## 2. Setup

### kubectl / k9s auto-install

Opening the module calls `GET /k8s/status`, which locates each binary with the
ladder `which` → `/opt/homebrew/bin` → `/usr/local/bin` → `~/.local/bin` →
`<data_dir>/bin` and reports `{ installed, version, path }`. When `kubectl`
is missing the page shows a first-run panel; **Install now** (Admin) calls
`POST /k8s/install { tool: "kubectl" }` and polls `/k8s/status` every 1.5 s
(the same event also arrives on the WebSocket as `k8s_install_updated`).

The installer job runs in the background, one slot per tool:

1. If the binary already runs, the job finishes immediately.
2. If `brew` is on `PATH`: `brew install kubectl` / `brew install k9s`
   (`HOMEBREW_NO_AUTO_UPDATE=1`).
3. Otherwise a direct download with the system `curl -fsSL`:
   - kubectl: `https://dl.k8s.io/release/<stable.txt>/bin/darwin/{arm64|amd64}/kubectl`
     → `<data_dir>/bin/kubectl`, `chmod 755`;
   - k9s: `https://github.com/derailed/k9s/releases/latest/download/k9s_Darwin_{arm64|amd64}.tar.gz`
     → extract `k9s` into `<data_dir>/bin`.
4. The binary must answer `kubectl version --client -o json` / `k9s version -s`
   before the job reports `done`; otherwise it is `failed` with an `error` and
   the captured `log_tail` (8 KiB, collapsible in the UI).

Every kubectl invocation uses the *located* path, so a freshly installed
binary is picked up without a daemon restart. k9s is optional — the k9s button
shows **Install k9s** until it exists.

### Adding a cluster — three paths

**Pick a context from your kubeconfig.** `GET /k8s/discover` runs
`kubectl --kubeconfig <file> config view -o json` against `~/.kube/config` and
every entry of `$KUBECONFIG`, and returns only `contexts[]` joined with
`clusters[].cluster.server` — names, users, namespaces and API endpoints, never
certificate or token material. Selecting one (or several) creates rows via
`POST /k8s/clusters { name, source: "kubeconfig", kubeconfig_path?, context_name,
default_namespace?, environment?, color? }`. Leave `kubeconfig_path` empty for
kubectl's default resolution; set it (`~` allowed) to pin a specific file.

**Paste a kubeconfig.** `POST /k8s/clusters/import { name, kubeconfig_yaml,
context_name?, default_namespace?, environment? }` writes the YAML to
`<data_dir>/kube/<id>.yaml` (dir `0700`, file `0600`), validates it by running
`kubectl --kubeconfig <file> config view -o json`, resolves `context_name` to
the file's `current-context` when omitted (and `default_namespace` to that
context's namespace), and only then inserts the row (`source: "imported"`).
An invalid file or unknown context is a `400` and the file is removed. Pasted
YAML is capped at 1 MiB.

**From EKS.** In the AWS console, an EKS cluster's **Open in Kubernetes**
button (requires `aws_eks:Edit` *and* `kubernetes:Admin`) runs
`aws eks update-kubeconfig` into `<data_dir>/kube/<id>.yaml` and creates a row
with `source: "eks"` linked to the AWS account. For such clusters every kubectl
call also receives the account's AWS environment (`AWS_PROFILE`, or the
access-key variables read from the Keychain, plus `AWS_REGION`) so the
kubeconfig's `aws eks get-token` exec plugin can mint a token — see
[`./aws-console.md`](./aws-console.md).

Cluster cards carry an **environment** pill (`dev` / `staging` / `prod` — prod
gets the red treatment connections use), a colour dot, the server version and
capability chips. **Test** (`POST /k8s/clusters/{id}/test`) runs
`kubectl version -o json --request-timeout=8s` and reports latency + the
server version. `PATCH` / `DELETE` are Admin; deleting an `imported`/`eks`
cluster also removes its Otto-owned kubeconfig file (a user's own file is never
touched, and the Otto-managed path cannot be re-pointed via `PATCH`).

### Capabilities

`GET /k8s/clusters/{id}/capabilities` probes, concurrently:

| Probe | Sets |
|---|---|
| `kubectl version -o json` | `server_version` |
| `kubectl get --raw /apis/metrics.k8s.io/v1beta1` | `metrics_server` |
| `kubectl api-resources --api-group=argoproj.io -o name` | `argo_rollouts` (`rollouts.argoproj.io` present), `argocd` (`applications.argoproj.io` present) |

The result is cached in the row (`capabilities`) and reused until
`?refresh=true`. It drives which resource kinds the left rail shows (Argo
Rollouts / ArgoCD Apps only when detected) and whether CPU/MEM columns are
fetched. `kubectl auth can-i --list` is deliberately **not** run (too slow on
large clusters); a denied verb simply comes back as a `403`.

---

## 3. The cluster workspace

### Namespaces, nodes, resource kinds

The top bar has a namespace filter (**All namespaces** ⇒ kubectl `-A`,
remembered per cluster), a free-text filter, refresh + 10 s auto-refresh, the
cluster switcher and the k9s button. Namespaces are always lowercased (they
are DNS labels; phones auto-capitalize). When the kubeconfig user can't
`get namespaces` (Rancher project-scoped users), the picker lists the
namespaces the cluster is known to have — the default plus every one read
successfully — and any other name can be typed; a cluster-scope 403 on
"All namespaces" says so and points back at the picker. The left rail lists the kinds:
Pods, Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs, CronJobs,
Services, Ingresses, ConfigMaps, Secrets, PVCs, HPAs, Nodes, Events, and —
when capabilities say so — Argo Rollouts and ArgoCD Apps.

`GET /k8s/clusters/{id}/resources?kind=&ns=&label=&q=` runs
`kubectl get <resource> -o json [-n ns | -A] [-l label]` and normalises every
item into a flat `K8sRow { name, namespace, kind, status, ready, restarts,
age_seconds, node, ip, cpu, mem, images, labels, extra, health }` (see the
contract for the per-kind `extra` columns). `-o wide` is never used; the
daemon derives the columns itself so they are stable across kubectl versions.
`q` is a case-insensitive substring match over the visible columns, applied
daemon-side.

`health` colours the status cell: `ok`, `warn`, `bad`, `progressing`. For
pods: `Terminating` when `deletionTimestamp` is set; a container waiting /
terminated reason such as `CrashLoopBackOff`, `ImagePullBackOff`,
`CreateContainerConfigError`, `Error`, `OOMKilled` or `Evicted` overrides the
phase and is `bad`; `Init:<done>/<n>`, `Pending`, `ContainerCreating`,
`PodInitializing` are `progressing`; `Running` with every container ready, or
`Completed`, is `ok`; a running pod with an unready container is `warn`.
Workloads use readiness math (`desired / updated / ready / available`) — a
Deployment whose `Progressing` condition is `False` (`ProgressDeadlineExceeded`)
is `Failed`/`bad` even if some replicas are still up.

`GET /k8s/clusters/{id}/nodes` merges `get nodes` (Ready condition,
`SchedulingDisabled`, roles from `node-role.kubernetes.io/*`, kubelet version,
capacity) with node usage from metrics-server when available.

### Metrics

When `metrics_server` is true, pod rows get `cpu` (millicores) and `mem`
(bytes) merged from `kubectl get --raw /apis/metrics.k8s.io/v1beta1/[namespaces/<ns>/]pods`
— the same data `kubectl top pods` prints, already JSON. Kubernetes `Quantity`
strings (`250m`, `1500u`, `128Mi`, `1Gi`, `1e3`, …) are parsed in Rust.
`GET /k8s/clusters/{id}/metrics?ns=` returns per-pod and per-container usage
for the Metrics tab; `available:false` when the API is absent.

### Detail drawer

`GET /k8s/clusters/{id}/resource?kind=&ns=&name=` returns three things in one
round trip (describe and events run concurrently):

- `manifest` — `kubectl get … -o json` with `managedFields` stripped and, for
  Secrets, every `data` / `stringData` value (and the
  `last-applied-configuration` annotation) replaced by `"<redacted>"`;
- `describe` — `kubectl describe …` verbatim (Secret data sections dropped);
- `events` — `kubectl get events --field-selector involvedObject.uid=<uid>`,
  newest first.

The pod drawer adds **Containers**
(`GET …/pods/{ns}/{name}/containers`: init containers first, with
`running` / `waiting:<Reason>` / `terminated:<Reason>` state and restart
counts), **Logs**, **Terminal** and **Metrics** tabs. `Esc` closes it; `l`, `s`,
`d` on a selected row open logs, shell and describe (k9s muscle memory).

Workload drawers (Deployment, StatefulSet, DaemonSet, ReplicaSet, Job,
Rollout) add **Pods** and **Logs** tabs, driven by the row's `extra.selector`
(`spec.selector.matchLabels` as `k=v,k=v`; the manifest's selector is the
fallback when the row is gone):

- **Pods** — `GET …/resources?kind=pods&ns=&label=<selector>` every 10 s:
  ready, status, restarts, CPU/MEM (when metrics-server answers) and age per
  pod, with totals on top. A row opens that pod's own drawer; its Logs /
  Shell buttons open it straight on those tabs. The row menu offers the same
  as **Pods** and **Logs (all pods)**.
- **Logs** — one stream across every matching pod (next section).

### Logs

`GET …/pods/{ns}/{name}/logs?container=&tail=500&since=&previous=&follow=&timestamps=`
returns `text/plain`.

- One-shot: `kubectl logs <pod> -n <ns> [-c c] --tail=N [--since=10m |
  --since-time=<RFC3339>] [--previous] [--timestamps]` with a 60 s budget and
  a **5 MiB tail cap** (the newest lines are kept; a first line
  `[otto: output truncated …]` marks it). `tail=-1` drops `--tail`.
- `follow=true`: the daemon spawns `kubectl logs -f` and streams its stdout as
  a chunked response that stays open. The child process lives inside the
  response body, so when the browser tab closes or the follow toggle is
  switched off the stream is dropped and **kubectl is killed** — no orphaned
  followers. If kubectl exits with an error and produced no lines, its
  (redacted) stderr is appended as a `[kubectl] …` trailer so an RBAC or auth
  failure is visible in the log pane.

The UI consumes the stream with `fetch` + `ReadableStream`, supports search,
timestamps and download. Agents get the same data via the `k8s_logs` MCP tool
(text tail, never `follow`).

**Workload-level logs** — `GET /k8s/clusters/{id}/logs?ns=&selector=&…` (same
options) runs `kubectl logs -l <selector> --prefix --max-log-requests=100
[--all-containers | -c <c>]`, so every line starts with
`[pod/<pod>/<container>] `. The Logs tab of a workload drawer parses that
prefix into a colored pod tag: click it (or use the pod dropdown) to keep one
pod's lines, ⌥-click or **Open pod** to jump to that pod's drawer, and the
container filter defaults to all containers of the pod template. Download
writes the currently filtered lines.

### Exec (shell in a pod) and k9s

`POST /k8s/clusters/{id}/exec { workspace_id, ns, pod, container?, command? }`
(Edit) opens a PTY session through `Spawner::spawn_command(provider = "k8s")`
running

```
kubectl [--kubeconfig <file>] --context <ctx> -n <ns> exec -it <pod> [-c <container>] -- \
  sh -c 'command -v bash >/dev/null && exec bash || exec sh'
```

(or your `command`). It is a normal Otto session — it appears in the session
list titled `"<pod> · <ns>"`, streams over the terminal WebSocket, can be
shared, and is audited as `k8s.exec`. `POST /k8s/clusters/{id}/k9s
{ workspace_id, ns? }` does the same with `k9s --kubeconfig … --context … [-n ns]`
(the cluster's `default_namespace` when `ns` is omitted) and returns `400 k9s
not installed …` until k9s exists. Streams and PTYs omit `--request-timeout`
on purpose — kubectl applies it to the HTTP client and would cut a long
session.

---

## 4. Actions

`POST /k8s/clusters/{id}/actions { action, kind, ns, name, params? }` (Edit)
plans the kubectl argv first, then runs it, and always writes an audit row
`k8s.action.<action>` (success *and* failure, with the params — they carry no
secrets). Resource names are `<plural>/<name>`; the Argo CRDs are fully
qualified (`rollouts.argoproj.io/…`, `applications.argoproj.io/…`) so a CRD
with a clashing short name can never be hit.

**Typed confirmation.** `delete_pod`, `scale` to `0`, `rollout_undo` and
`argocd_sync` with `prune: true` require `params.confirm_name` to equal the
resource name; the UI collects it with `confirmer.promptText`, and the
`k8s_action` MCP tool only sets it after an explicit user confirmation.

| Action | Applies to | What kubectl does | Notes |
|---|---|---|---|
| `restart` | deployments, statefulsets, daemonsets | `rollout restart <kind>/<name>` | Adds the `kubectl.kubernetes.io/restartedAt` annotation to the pod template → rolling restart. |
| `restart` | rollouts | `patch … --type merge -p {"spec":{"restartAt":"<now>"}}` | Argo Rollouts' restart: pods are replaced in place without changing the rollout's revision. |
| `scale` | deployments, statefulsets, rollouts, replicasets | `scale <kind>/<name> --replicas=<n>` | `params.replicas` 0..10000; `0` needs confirm. |
| `delete_pod` | pods | `delete pods/<name> [--grace-period=<g> [--force]]` | `--force` is added automatically when `grace` is `0` (kubectl requires it). |
| `rollout_status` | deployments, statefulsets, daemonsets | `rollout status … --timeout=5s` | Returns `ok:false` + kubectl's output when the rollout has not finished within 5 s — it is a status query, not an error. |
| `rollout_undo` | deployments, statefulsets, daemonsets | `rollout undo … [--to-revision=<r>]` | Confirm required. |
| `rollout_pause` / `rollout_resume` | deployments | `rollout pause|resume …` | |
| `rollout_pause` / `rollout_resume` | rollouts | `patch … -p {"spec":{"paused":true|false}}` | |
| `rollout_promote` | rollouts | `patch … --subresource=status -p {"status":{"pauseConditions":null}}`, then (`params.full`) `--subresource=status -p {"status":{"promoteFull":true}}`, then `-p {"spec":{"paused":false}}` | Mirrors `kubectl argo rollouts promote [--full]`: clearing `pauseConditions` moves past the current pause step; an indefinite `pause: {}` step also sets `spec.paused`, which is why the spec is unpaused too; `promoteFull` skips every remaining step. Needs kubectl ≥ 1.24 (`--subresource`). |
| `rollout_abort` / `rollout_retry` | rollouts | `patch … --subresource=status -p {"status":{"abort":true|false}}` | Abort rolls back to the stable ReplicaSet; retry (`abort:false`) restarts the aborted update. |
| `argocd_sync` | applications | `patch … -p {"operation":{"initiatedBy":{"username":"otto"},"sync":{"revision":"<rev>","prune":<bool>,"syncStrategy":{"hook":{}}}}}` | The same `operation` stanza `argocd --core app sync` writes; the controller picks it up. `revision` defaults to `.spec.source.targetRevision`. Prune needs confirm. |
| `argocd_refresh` | applications | `annotate … argocd.argoproj.io/refresh=normal|hard --overwrite` | `hard` (`params.hard`) also invalidates the manifest cache. |
| `argocd_terminate_op` | applications | `patch … --type json -p [{"op":"remove","path":"/operation"}]` | Stops a running sync. |
| `argocd_app_restart` | applications | For every Deployment / StatefulSet / DaemonSet / Rollout in the app's `.status.resources[]` (optionally filtered by `params.resource_kind` / `resource_name`), the `restart` verb above in that resource's namespace | "Redeploy" for GitOps-managed workloads without touching git. Runs in **this** cluster's context — the app's destination is assumed to be the cluster hosting it (the common Argo CD layout); apps deploying to a *different* cluster restart nothing useful. |
| `cronjob_trigger` | cronjobs | `create job --from=cronjob/<name> <name>-manual-<unix ts>` | Job name trimmed to the 63-char label limit. |
| `cronjob_suspend` / `cronjob_resume` | cronjobs | `patch … -p {"spec":{"suspend":true|false}}` | |

Row actions live in the right-click `ctxMenu` and the drawer's toolbar; the
Argo CD sync dialog collects revision + prune.

---

## 5. RBAC

The `kubernetes` feature has three rungs; the policy table enforces them
server-side (`crates/otto-server/src/policy.rs`, "Kubernetes console") and the
UI mirrors them with `auth.can('kubernetes', cap)`.

| Capability | Allows |
|---|---|
| **View** | `GET /k8s/status`, `/k8s/discover`, list/get clusters, `POST …/test`, capabilities, namespaces, nodes, resources, resource detail, containers, logs (incl. follow), metrics |
| **Edit** | everything in View + `POST …/exec`, `POST …/k9s`, `POST …/actions` |
| **Admin** | everything in Edit + `POST /k8s/clusters`, `POST /k8s/clusters/import`, `PATCH`/`DELETE /k8s/clusters/{id}`, `POST /k8s/install` |

Clusters are a **global library** (no workspace axis), like connections: a
grant applies to every registered cluster. Importing an EKS cluster from the
AWS console additionally requires `aws_eks:Edit`. Root holds Admin everywhere.

Independently, the **cluster's** RBAC applies to whatever identity the
kubeconfig context carries. kubectl's `Forbidden` responses become
`403 forbidden` with `cluster RBAC: <kubectl's first line>` so the UI can show
the real reason (e.g. `pods is forbidden: User "dev" cannot list resource
"pods" in the namespace "kube-system"`).

Audit rows (`audit_log`): `k8s.install`, `k8s.exec`, `k8s.k9s`,
`k8s.action.<action>` — actor, cluster id as target, and a detail blob with
cluster / context / environment / kind / ns / name / params / ok / error.

---

## 6. API / contract reference

Every route, DTO and the action table are documented verbatim in
[`docs/contracts/api.md`](../contracts/api.md) — section **"Kubernetes console
(`/k8s/*`)"** — and guarded by `crates/otto-server/tests/route_inventory.rs`
(every registered path must appear there) and `tests/policy_coverage.rs`
(every path must have a policy entry). WS events are in
[`docs/contracts/ws.md`](../contracts/ws.md). TypeScript mirrors live under the
`// Kubernetes console` banner of `ui/src/lib/api/types.ts`.

Quick map:

```
GET    /k8s/status                                   POST /k8s/install
GET    /k8s/discover
GET    /k8s/clusters                                 POST /k8s/clusters
POST   /k8s/clusters/import
GET    /k8s/clusters/{id}    PATCH  DELETE
POST   /k8s/clusters/{id}/test                       GET  /k8s/clusters/{id}/capabilities?refresh=
GET    /k8s/clusters/{id}/namespaces                 GET  /k8s/clusters/{id}/nodes
GET    /k8s/clusters/{id}/resources?kind=&ns=&label=&q=
GET    /k8s/clusters/{id}/resource?kind=&ns=&name=
GET    /k8s/clusters/{id}/pods/{ns}/{name}/containers
GET    /k8s/clusters/{id}/pods/{ns}/{name}/logs?container=&tail=&since=&previous=&follow=&timestamps=
GET    /k8s/clusters/{id}/metrics?ns=
POST   /k8s/clusters/{id}/exec                       POST /k8s/clusters/{id}/k9s
POST   /k8s/clusters/{id}/actions
```

Every kubectl call is built as `["--kubeconfig", <path>]? + ["--context", <ctx>,
"--request-timeout", "20s"] + <verb args>` (`crates/otto-k8s/src/cli.rs`
`base_args`), runs with `kill_on_drop` and a 30 s wall clock (60 s for logs;
none for streams), and never goes through a shell.

---

## 7. Capabilities & limitations

- ✅ Any cluster kubectl can reach — kubeconfig contexts, pasted kubeconfigs,
  EKS via the AWS console; exec-plugin auth (EKS, GKE, OIDC) just works.
- ✅ 16 resource kinds incl. Argo Rollouts and Argo CD Applications, with
  k9s-style health colouring, metrics columns and a describe/manifest/events
  drawer.
- ✅ Streaming `logs -f` with a kill-on-disconnect guarantee; exec and k9s as
  first-class Otto sessions (shareable, audited).
- ✅ Every mutating verb is a transparent `kubectl` command (documented above),
  typed-confirm for destructive ones, audited with params.
- ✅ Self-installing `kubectl` / `k9s`; never `sudo`, never writes
  `~/.kube/config`.
- ⚠️ **macOS only** for the installer (darwin download URLs); on other hosts
  install kubectl yourself — discovery/usage is platform-neutral.
- ⚠️ Argo verbs need the CRDs to be reachable by the context's identity;
  `rollout_promote/abort/retry` need kubectl ≥ 1.24 (`--subresource=status`)
  and a cluster that allows status writes for your user.
- ⚠️ `argocd_app_restart` restarts in the *hosting* cluster's context — apps
  whose destination is another cluster are not followed.
- ⚠️ No `auth can-i` pre-check: buttons are shown according to Otto's grant;
  cluster RBAC denials appear when you click (as a clear `403`).
- ⚠️ Non-follow logs are capped at 5 MiB / 60 s; use `follow` or a narrower
  `since` for more.
- ⚠️ Metrics require metrics-server; without it CPU/MEM columns are hidden and
  `/metrics` returns `available:false`.
- ⚠️ Port-forward, apply/edit of arbitrary manifests and Helm are out of scope
  (use the exec shell or a terminal session).

---

## 8. Security model

- **Secrets never leave the daemon.** Secret rows expose `type` + key *names*
  only; the manifest view replaces every value with `<redacted>`; describe
  drops the data section. Kubeconfig contents are never returned by any route
  — discovery reads only context names and API endpoints.
- **Otto-owned kubeconfigs** are `0600` in a `0700` directory and are deleted
  with the row; a user's own kubeconfig is never modified or deleted.
- **Error messages are redacted** (`otto_core::redact` + an AWS access-key
  scrubber) before leaving the daemon — kubectl echoes bearer tokens and
  exec-plugin output on auth failures.
- **No shell.** kubectl is spawned with an argv, never a shell string; resource
  names starting with `-` or containing whitespace are rejected before they
  reach kubectl.
- **EKS credentials** come from the linked AWS account (profile name, or
  Keychain-held access keys injected as env vars for the single kubectl
  process) — never written to `~/.aws`.
- **Audit** for installs, sessions and every action (including refused ones).
- The daemon listens on loopback only; every route passes the session /
  token auth and the policy table like the rest of the API.

---

## 9. Troubleshooting

| Symptom | Cause / fix |
|---|---|
| First-run panel says `kubectl not installed` although you have it | The daemon's `PATH` does not include it. It searches `which`, `/opt/homebrew/bin`, `/usr/local/bin`, `~/.local/bin`, `<data_dir>/bin` — symlink your binary into one of those, or click **Install** (a second copy in `<data_dir>/bin` is harmless). |
| Install job `failed` with a brew error | Read the log tail; brew failures (e.g. an outdated Xcode CLT) fall through to the direct download automatically — retry if the download step also failed (network / GitHub rate limit). |
| `Test` says `connected` but pods list is `403 cluster RBAC: …` | Your context's identity may reach the API but not that namespace/resource. Pick another namespace or ask for RBAC; Otto shows kubectl's exact reason. |
| EKS cluster: `error: You must be logged in to the server (Unauthorized)` or `login required` | The linked AWS account's SSO session expired — use **Sign in** on the AWS account card, then retry. For key-based accounts check the account's region and that the IAM identity is mapped in the cluster's `aws-auth` / access entries. |
| Discovery shows no contexts | `~/.kube/config` missing/unreadable or `$KUBECONFIG` points elsewhere for the daemon process (launchd environment ≠ your shell). Paste the kubeconfig or set `kubeconfig_path` explicitly. |
| Argo Rollouts / ArgoCD not shown in the rail | Capabilities cache is stale or the CRDs are not visible to your identity — hit **Refresh capabilities** (`?refresh=true`) and check `kubectl api-resources --api-group=argoproj.io`. |
| `rollout_promote` returns `the server could not find the requested resource` / unknown flag `--subresource` | kubectl older than 1.24 — reinstall via the module (latest stable), or brew upgrade. |
| Follow logs stop after ~20 s on some clusters | An intermediary (proxy / LB) idle timeout; the daemon itself imposes none on streams. Toggle follow again or use `since`. |
| CPU/MEM columns missing | No metrics-server, or your identity cannot read `metrics.k8s.io`; `GET …/metrics` returns `available:false`. |
| Exec opens then exits immediately | The image has neither `bash` nor `sh` (distroless). Pass an explicit `command` (e.g. a binary that exists) or use an ephemeral debug container from a terminal session. |
| `409`/`400` on import: `context 'x' not found in the pasted kubeconfig` | The YAML's `contexts[].name` values are listed in the error — pick one of them or omit `context_name` to use `current-context`. |

---

## 10. Related docs

- [`aws-console.md`](./aws-console.md) — AWS accounts, credentials, EKS import.
- [`connections-ssh-sftp.md`](./connections-ssh-sftp.md) — the session
  machinery `exec` / k9s reuse.
- [`rbac-multiuser-sharing.md`](./rbac-multiuser-sharing.md) — feature grants.
- [`mcp-control-plane.md`](./mcp-control-plane.md) — the `k8s_*` agent tools.
- [`../design/aws-k8s-consoles.md`](../design/aws-k8s-consoles.md) — the build
  contract (routes, normalisation rules, action semantics).
