---
id: kubernetes
title: Kubernetes
group: Infrastructure
route: kubernetes
summary: A k9s-style console for any cluster kubectl can reach — browse workloads, read logs, open shells, run rollout and Argo actions, and monitor pod health.
---
## What it's for

The Kubernetes page is a k9s-style console for any cluster you can already reach with `kubectl`. You can browse namespaces and resources, read describe output, manifests and events, follow logs, and open a shell in a pod or a full k9s session. You can also run the everyday actions — restart, scale, rollout undo, Argo Rollouts and Argo CD operations, CronJob triggers — without typing kubectl commands.

Each action is a plain `kubectl` call under your kubeconfig's identity. Otto never switches your current context and never edits `~/.kube/config`. The **Monitor** dashboards add pod-level health on top: restart classification, memory against limits, request rate, 5xx rate and latency.

## Getting started

1. Open **Kubernetes** in the sidebar (Infrastructure group).
2. If `kubectl` is missing, choose **Install kubectl**. Otto uses Homebrew when available, otherwise it downloads kubectl into its own data folder. Otto never uses `sudo`, and installing needs the Kubernetes Admin grant. If kubectl is already somewhere else on your Mac, choose **Continue without installing**.
3. Choose **Add cluster** and pick a source:
   - **From kubeconfig** — pick one or more contexts found in `~/.kube/config` and `$KUBECONFIG`. Otto reads the file in place.
   - **Paste kubeconfig** — Otto saves the YAML under its own data folder (mode 0600). The context defaults to the file's current-context.
   - **From EKS** — opens [AWS](#/walkthroughs/aws). Pick an account, go to EKS and choose **Open in Kubernetes…**.
4. Name the cluster. Optionally set a default namespace and a comma-separated list of known namespaces, which helps when you can't list namespaces. Then pick an environment (`dev`, `staging` or `prod`).
5. Open the cluster card. Pick a resource kind in the left rail, then select a row to open its details drawer.

## Everything it can do

**Clusters overview**
- Cluster cards show the environment, the context and default namespace, the server version, and capability chips: **metrics** (metrics-server), **rollouts** (Argo Rollouts CRD) and **argocd** (Argo CD Application CRD).
- Card menu: **Open**, **Test connection**, **Refresh capabilities**, **Manage access…**, **Edit…** and **Delete**. Deleting a pasted or EKS cluster also removes Otto's copy of its kubeconfig. Your own kubeconfig files are never touched.
- **Install k9s** in the header when k9s is missing (admins only).
- **Monitor** opens the monitoring dashboards.

**Cluster workspace**
- Top bar:
  - an environment badge (`prod` gets the red treatment) and a cluster switcher
  - a namespace picker with **All namespaces**, remembered per cluster. You can type a namespace that isn't listed.
  - a filter box, **Refresh** and a 10-second auto-refresh toggle
  - buttons for **Monitor**, **k9s** and the shortcut sheet (`?`)
- Resource kinds: Pods, Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs, CronJobs, Services, Ingresses, ConfigMaps, Secrets, PVCs, HPAs, Nodes and Events. Argo Rollouts and ArgoCD Apps appear only when the cluster has those CRDs.
- Status colours work like k9s. For example, `CrashLoopBackOff`, `ImagePullBackOff` and `OOMKilled` show red, and pods that are still starting show as progressing.
- CPU and MEM columns appear when metrics-server is available.
- The table stays fast on large clusters. You can drag the divider to resize the drawer.

**Details drawer**
- Tabs for every kind: **Overview**, **Manifest** (managed fields removed, Secret values redacted), **Describe** and **Events**.
- Pods add **Logs**, **Terminal** (a shell via `kubectl exec`, bash when the image has it) and **Metrics** (per-container CPU and memory, refreshed every 10 seconds).
- Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs and Rollouts add **Pods** (the matching pods, with readiness, restarts, CPU/MEM and age, refreshed every 10 seconds) and **Logs** across all their pods.

**Logs**
- Choose a container (or all containers), a tail length (100, 500, 1000 or 5000 lines) and a since window (all, 5m, 15m, 1h, 6h or 24h).
- Toggles: **Follow** (streams `kubectl logs -f`), **Timestamps** and **Previous** (the crashed instance).
- **Search…**, **Reload**, **Download** (saves the filtered lines) and **Jump to bottom**.
- Workload logs tag each line with its pod. Select a tag to keep only that pod's lines, ⌥-click it or use **Open pod** to open that pod, or use the pod dropdown.

**Shells and k9s**
- **Shell (exec)** and the k9s button open real Otto terminal sessions: they appear in your session list, can be shared, and are audited.
- k9s opens inside the workspace. Choose **Close k9s** to return to the table.

**Actions** (row right-click menu or the drawer toolbar)
- Deployments, StatefulSets and DaemonSets:
  - **Restart (rollout restart)**, **Scale…** (not for DaemonSets), **Rollout status** and **Rollout undo**
  - Deployments also have **Pause rollout** and **Resume rollout**
- Pods: **Delete pod**.
- Argo Rollouts: **Restart**, **Promote**, **Promote (full)**, **Abort**, **Retry**, **Pause**/**Resume** and **Scale…**.
- ArgoCD Apps: **Sync…** (revision and prune), **Refresh**, **Hard refresh**, **Restart workloads (redeploy)** and **Terminate operation**.
- CronJobs: **Trigger now** and **Suspend**/**Resume**.
- Destructive actions ask you to type the resource name: delete pod, rollout undo, scale to 0 and sync with prune.

**Monitor** (`Kubernetes → Monitor`)
- **Overview**: one card per cluster. Each card shows a health badge (healthy, degraded or incident), pods, unplanned restarts by class, memory against limits, req/s and 5xx %, mixed versions, and the collector status. Choose **Enable monitoring** on clusters that aren't monitored yet.
- Per-cluster tabs:
  - **Workloads**: a sortable table with sparklines. Select a row to see every pod.
  - **Events**: classified restarts and churn.
  - **Insights**: the watchdog agent's latest report, with **Run now**.
  - **Settings**: probe presets, ports, namespaces, exclusions, transport, interval, concurrency, series cap, retention, **Keep request path labels** and **Test probes**.
- **Fleet dashboard**: one view across every cluster, with **Overview**, **Table**, **Events** and **Requests** tabs, read from stored history only.
- Windows: 1h, 6h, 24h and 7d.
- Restart classes: OOM, crash, probe failure, unknown, planned churn (rollout, scale, drain, Otto action), completed and new version.

**Agents (Otto MCP tools)**
- Read-only tools: `k8s_list_clusters`, `k8s_get_resources`, `k8s_describe`, `k8s_logs` (a log tail, never follow), `k8s_top` and `k8s_health`. `k8s_health` returns the Monitor digest.
- `k8s_action` runs one operational action and needs approval.
- The **Kubernetes watchdog** template in [Personal Agents](#/walkthroughs/personal-agents) checks every monitored cluster every 15 minutes and ends each report with a verdict.

## Keyboard shortcuts

These work in the cluster workspace when you aren't typing, no dialog or menu is open, and k9s isn't showing.

| Keys | Action |
|---|---|
| `/` | Focus the filter |
| `n` | Focus the namespace picker |
| `j` / `k` | Next / previous row |
| ↓ / ↑ | Next / previous row (when a row has focus) |
| ↵ | Open details for the selected row |
| Space | Select the focused row |
| `d` | Describe |
| `y` | Manifest (YAML) |
| `l` | Logs (pods) |
| `s` | Shell into the pod (needs exec access) |
| `r` | Refresh |
| Esc | Close details, or leave the filter |
| `?` | Show this shortcut list |
| ⇧F10 | Open the row menu |
| ← / → | Switch drawer tabs while a tab is focused |
| ⌥-click | On a pod tag in workload logs: open that pod |

App-wide shortcuts are listed in [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts).

## Tips and limits

- **Two layers of access.**
  - Otto's `kubernetes` feature grant sets what you may ask for. View covers browsing, logs and metrics. Edit adds exec, k9s and actions. Admin adds installing kubectl and k9s.
  - The cluster's own RBAC sets what kubectl may do. A denial shows as `cluster RBAC: …` with kubectl's reason.
- **Per-cluster rules.** **Manage access…** can limit you to named namespaces and to specific operations: workloads, other resources, Secrets, logs, metrics, exec, scale, restart, delete and apply. Namespace-limited users can't pick **All namespaces** or view Nodes. k9s needs unrestricted cluster access.
- **Adding, editing and deleting clusters is for the Otto owner (root).** Delegated administrators can rename a cluster.
- **Buttons follow Otto grants, not cluster RBAC.** Otto doesn't run `kubectl auth can-i` in advance, so an action the cluster forbids fails when you run it.
- **Monitoring needs the Usage engine** (embedded ClickHouse), because samples are stored there. Enable it on a staging cluster first. When the API-server proxy is denied, the collector opens one `kubectl port-forward` per pod per cycle.
- **Limits:**
  - One-shot logs are capped at 5 MiB and 60 seconds. Use **Follow** or a shorter **since** for more.
  - CPU and memory need metrics-server.
  - Promote, abort and retry for Argo Rollouts need kubectl 1.24 or later.
  - **Restart workloads (redeploy)** only acts on the cluster that hosts the Argo CD app.
- **Not supported:** port-forwarding, applying or editing arbitrary manifests, and Helm. Use the pod shell or a terminal session instead.
- Every exec, k9s launch, install and action is written to the audit log with its parameters.

## Related

- [AWS](#/walkthroughs/aws)
- [Personal Agents](#/walkthroughs/personal-agents)
- [Usage](#/walkthroughs/usage)
- [MCP Control Plane](#/walkthroughs/mcp)
- [Agents](#/walkthroughs/agents)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
