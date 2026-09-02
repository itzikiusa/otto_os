# Design contract — AWS console & Kubernetes console

Status: **build contract** for branch `feat/aws-k8s-consoles`. Every implementer
(backend, UI, MCP) builds against THIS document. If you must deviate, edit this
file in the same change so the other halves see it. `docs/contracts/api.md`
remains the authoritative API reference once routes land — mirror it there.

## 0. Product decisions (fixed)

| Decision | Choice | Why |
|---|---|---|
| Engines | Shell out to `aws` CLI v2 and `kubectl` with `--output json` / `-o json`. **No** aws-sdk-rust, **no** kube-rs, **no** `argocd` CLI, **no** `kubectl-argo-rollouts` plugin. | User constraint: everything through kubectl; CLIs already handle SSO/MFA/assume-role/exec-credential plugins. |
| ArgoCD / Argo Rollouts | Via `kubectl` against the CRDs (`applications.argoproj.io`, `rollouts.argoproj.io`) — patches/annotations only. | See §4.6. |
| Install | On first enable (module opened, or `POST /{aws,k8s}/install`) the daemon checks for the binary and installs it if missing: prefer `brew` when present, else direct download into `<data_dir>/bin` (already on the daemon PATH — see `crates/ottod/src/main.rs` `augment_path`). Progress is polled via `/status`. | User: "if it does not exist, install". |
| Configuration | 100% in the Otto UI. Accounts / clusters are Otto DB rows (+ Keychain). Otto **never writes** `~/.aws/*` or the user's `~/.kube/config`. Otto-owned kubeconfigs live in `<data_dir>/kube/<id>.yaml` (0600). | Simple for the user, non-destructive. |
| Reuse | Optional one-click **k9s** PTY tab (installed on demand like kubectl). Headlamp rejected (Electron cask only, no server formula). | Survey result. |
| RBAC | New `Feature` keys: `aws`, `aws_s3`, `aws_sqs`, `aws_ec2`, `aws_athena`, `aws_eks`, `kubernetes`. Policy table already landed in `crates/otto-server/src/policy.rs` (search "AWS console"). | Per-service permissions requested. |
| S3 | Read-only: list + preview + download. No upload/delete. | User decision. |
| Athena | Execute is `Edit`; browsing/history/results/cancel are `View`. | User decision. |

Capability grid (server-enforced by the policy table; UI must mirror by
hiding/disabling controls with `auth.can(feature, cap)`):

| Feature | View | Edit | Admin |
|---|---|---|---|
| `aws` | list accounts, status, discover, regions, test, permissions | `login` (spawn `aws sso login` PTY) | create/update/delete accounts, install CLI |
| `aws_s3` | everything (buckets, objects, preview, download) | — | — |
| `aws_sqs` | list, attributes, peek | send, delete-message, purge, redrive | — |
| `aws_ec2` | list/describe | start/stop/reboot | — |
| `aws_athena` | workgroups/databases/tables/history/results/cancel | execute query | — |
| `aws_eks` | list/describe clusters + nodegroups | import kubeconfig (creates a k8s cluster row) | — |
| `kubernetes` | list clusters, status, discover, test, namespaces, nodes, resources, describe, events, logs, top, capabilities | exec, k9s, actions | create/import/update/delete clusters, install kubectl/k9s |

Already landed on the branch (do not redo): `Feature` variants + parse/as_str,
`ALL_FEATURES` in `routes/grants.rs`, policy branches, TS `Feature` union,
`Users.svelte` labels, `sidebar.ts` entries (`aws` with `featureAny`,
`kubernetes`), `App.svelte` imports/routes/palette commands, placeholder pages
`ui/src/modules/aws/AwsPage.svelte` + `ui/src/modules/kubernetes/KubernetesPage.svelte`,
icons `cloud` + `helm`, migrations `0113_aws_accounts.sql` + `0114_k8s_clusters.sql`,
shell crates `crates/otto-aws` + `crates/otto-k8s` (ctx trait + empty
`api_router`, wired into the workspace and `otto-server/src/modules.rs`), and
`Spawner::spawn_command` (ad-hoc PTY session; `crates/otto-connections/src/service.rs`).

## 1. Shared backend conventions

- **CLI runner** (`cli.rs` in each crate): `run(program, args, env, timeout, stdin)` →
  `CliOutput { status, stdout, stderr, duration_ms }`. `tokio::process::Command`,
  never a shell string, `kill_on_drop(true)`, `tokio::time::timeout` (default
  30 s; logs/exec streams excepted). Errors: non-zero exit ⇒ `Error::Invalid`
  with **redacted** stderr (reuse `otto_core::redact` + strip AWS key shapes);
  binary missing ⇒ `Error::Invalid("aws CLI not installed — open the AWS module to install it")`
  with code `not_installed` semantics (UI keys off the message prefix
  `not installed`).
- **Binary discovery** (`install.rs`): `locate()` ladder = `which` → `/opt/homebrew/bin`,
  `/usr/local/bin`, `~/.local/bin`, `<data_dir>/bin`. **Installer** = background
  task with state `{state: idle|running|done|failed, tool, log_tail, started_at, finished_at}`
  kept in an `Arc<Mutex<..>>` inside the service (one per tool).
  - `aws`: `brew install awscli` if `brew` on PATH; else download
    `https://awscli.amazonaws.com/AWSCLIV2.pkg` to a temp dir and run
    `installer -pkg AWSCLIV2.pkg -target CurrentUserHomeDirectory` (installs
    to `~/aws-cli/`), then symlink `~/aws-cli/aws` + `aws_completer` into
    `<data_dir>/bin`.
  - `kubectl`: brew if present; else download
    `https://dl.k8s.io/release/$(curl -L -s https://dl.k8s.io/release/stable.txt)/bin/darwin/{arm64|amd64}/kubectl`
    into `<data_dir>/bin/kubectl`, chmod 755, verify `kubectl version --client -o json`.
  - `k9s`: brew if present; else GitHub release tarball
    `https://github.com/derailed/k9s/releases/latest/download/k9s_Darwin_{arm64|amd64}.tar.gz`
    → extract `k9s` into `<data_dir>/bin`.
  - Never `sudo`. Downloads use `curl -fsSL` (system curl) to avoid adding an
    HTTP client dependency. Verify the binary runs before reporting `done`.
- **Auth injection**:
  - AWS `profile` accounts: env `AWS_PROFILE=<profile>`, `AWS_REGION=<region>`,
    `AWS_PAGER=""`, `AWS_CLI_AUTO_PROMPT=off`. Also pass `--output json`.
  - AWS `access_keys` accounts: env `AWS_ACCESS_KEY_ID` (from `params.access_key_id`),
    `AWS_SECRET_ACCESS_KEY` + optional `AWS_SESSION_TOKEN` (Keychain JSON under
    `secret_ref = "aws-<id>"`: `{"secret_access_key": "...", "session_token": "..."}`),
    `AWS_REGION`. Optional `params.role_arn` ⇒ we run `sts assume-role` once
    per hour and cache temp creds in memory.
  - Region override: every service endpoint accepts `?region=` (falls back to
    the account's region).
  - K8s: always `kubectl --kubeconfig <path or user default> --context <context_name>`;
    plus `-n <ns>` when given. Env `KUBECONFIG` is NOT relied upon. For `eks`
    source clusters also inject the linked AWS account's env (the kubeconfig's
    exec plugin is `aws eks get-token`).
- **Expired / missing credentials detection** (AWS): stderr containing
  `ExpiredToken`, `ExpiredTokenException`, `UnauthorizedSSOTokenError`,
  `Error loading SSO Token`, `The SSO session associated with this profile has expired`,
  `Unable to locate credentials` ⇒ return `Error::Invalid` whose message
  starts with `login required:`; the UI shows a "Sign in" button that calls
  `POST /aws/accounts/{id}/login`.
- **Permission probe** (`GET /aws/accounts/{id}/permissions`, cached 10 min in
  `permissions_json`, `?refresh=true` bypasses): run in parallel, 8 s each:
  `s3 ls` (s3:ListAllMyBuckets), `sqs list-queues --max-results 1`,
  `ec2 describe-instances --max-results 5`, `athena list-work-groups --max-results 1`,
  `eks list-clusters --max-results 1`, plus `sts get-caller-identity`. Result
  per service: `allowed | denied | unknown` (denied = `AccessDenied` /
  `UnauthorizedOperation` / `AccessDeniedException`; unknown = other error).
  Edit-level probes are NOT attempted (optimistic; the action surfaces the
  AccessDenied if it happens).
- **K8s capability probe** (`GET /k8s/clusters/{id}/capabilities`, cached in
  `capabilities_json`, `?refresh=true`): `kubectl version -o json` (server
  version), `kubectl get --raw /apis/metrics.k8s.io/v1beta1` (metrics-server),
  `kubectl api-resources --api-group=argoproj.io -o name` (contains `rollouts` ⇒
  argo_rollouts, `applications` ⇒ argocd), and `kubectl auth can-i --list -n <ns>`
  is NOT run (too slow); instead each mutating action maps kubectl's
  `forbidden` stderr to `Error::Forbidden`.
- **IDs** = ULIDs via `otto_core::Id` helpers (see `crates/otto-state/src/connections.rs`
  for the row mapping style). Repos live in `crates/otto-state/src/aws_accounts.rs`
  and `crates/otto-state/src/k8s_clusters.rs` (register in `otto-state/src/lib.rs`).
- **WS events** (add to `otto_core::event::Event`, `ws_events.rs::scope_of`
  as GLOBAL scope, and `docs/contracts/ws.md`):
  `aws_account_updated { account_id, deleted: bool }`,
  `aws_install_updated { tool, state }`,
  `k8s_cluster_updated { cluster_id, deleted: bool }`,
  `k8s_install_updated { tool, state }`.
- **Audit**: mutating AWS/K8s actions call the existing audit repo with actions
  `aws.sqs.send`, `aws.sqs.purge`, `aws.ec2.start|stop|reboot`,
  `aws.athena.execute`, `k8s.action.<action>`, `k8s.exec` (see how
  `routes/grants.rs` records `grant.changed`).
- **Every new route** must (a) appear verbatim in `docs/contracts/api.md`
  (`tests/route_inventory.rs`), (b) be covered by the policy table
  (`tests/policy_coverage.rs`) — the branches below already exist; if you add
  a route outside these templates, add a policy branch + test.

## 2. AWS — routes (`crates/otto-aws`, handler-relative, nested under `/api/v1`)

### 2.1 Accounts & plumbing
| Method & path | Body → Response |
|---|---|
| `GET /aws/status` | → `AwsStatus { installed: bool, version: string\|null, path: string\|null, install: InstallJob }` |
| `POST /aws/install` | → `InstallJob` (202-style; idempotent while running) |
| `GET /aws/discover` | → `{ profiles: DiscoveredProfile[] }` parsed from `~/.aws/config` + `~/.aws/credentials`: `{ name, region?, sso_start_url?, sso_session?, role_arn?, source: 'config'\|'credentials' }`. **Never** returns key values. |
| `GET /aws/regions` | → `{ regions: { code, name }[] }` (static list, 30+ regions) |
| `GET /aws/accounts` | → `AwsAccount[]` |
| `POST /aws/accounts` | `UpsertAwsAccountReq { name, auth_mode: 'profile'\|'access_keys', profile?, region, access_key_id?, secret_access_key?, session_token?, role_arn?, environment?, color? }` → `AwsAccount` (201). Validates: profile mode needs `profile`; keys mode needs id+secret. Runs `sts get-caller-identity` best-effort and stores `identity`. |
| `GET /aws/accounts/{id}` | → `AwsAccount` |
| `PATCH /aws/accounts/{id}` | partial `UpsertAwsAccountReq` (secret fields optional = keep) → `AwsAccount` |
| `DELETE /aws/accounts/{id}` | → 204 (deletes Keychain secret; k8s clusters linked via `aws_account_id` are kept, FK sets NULL) |
| `POST /aws/accounts/{id}/test` | → `AwsTestResp { ok, latency_ms, message, identity?: AwsIdentity, login_required: bool }` |
| `GET /aws/accounts/{id}/permissions?refresh=` | → `AwsPermissions { checked_at, identity?, services: { s3, sqs, ec2, athena, eks }: 'allowed'\|'denied'\|'unknown', login_required: bool }` |
| `POST /aws/accounts/{id}/login` | `{ workspace_id }` → `Session` — spawns `aws sso login --profile <p>` (profile mode) in a PTY via `Spawner::spawn_command(provider="aws")`; for `access_keys` returns 400. |

`AwsAccount { id, name, auth_mode, profile, region, access_key_id?, role_arn?, environment, color?, identity?: AwsIdentity { account, arn, user_id }, permissions?: AwsPermissions, created_by, created_at, updated_at, last_used_at }` — **never** includes secrets.
`InstallJob { tool: 'aws'|'kubectl'|'k9s', state: 'idle'|'running'|'done'|'failed', log_tail: string, started_at?, finished_at?, error? }`.

### 2.2 S3 (all View)
| Method & path | → |
|---|---|
| `GET /aws/accounts/{id}/s3/buckets` | → `{ buckets: { name, creation_date, region? }[] }` (`s3api list-buckets`) |
| `GET /aws/accounts/{id}/s3/buckets/{bucket}/objects?prefix=&token=&max=` | → `{ prefixes: string[], objects: { key, size, last_modified, storage_class, etag }[], next_token?, is_truncated }` (`s3api list-objects-v2 --delimiter /`) |
| `GET /aws/accounts/{id}/s3/buckets/{bucket}/object?key=` | → head: `{ key, size, content_type, last_modified, etag, metadata, storage_class }` |
| `GET /aws/accounts/{id}/s3/buckets/{bucket}/preview?key=&max_bytes=` | → `{ text, truncated, content_type }` — `s3api get-object --range bytes=0-<max-1>`; max default 64 KiB, cap 1 MiB; refuse non-text content types except JSON/CSV/YAML/log (`{ binary: true }`) |
| `GET /aws/accounts/{id}/s3/buckets/{bucket}/download?key=` | → streamed body with `Content-Disposition: attachment` (`s3 cp s3://.. -` piped stdout; cap 2 GiB) |

### 2.3 SQS
| Method & path | cap | → |
|---|---|---|
| `GET /aws/accounts/{id}/sqs/queues?prefix=` | View | `{ queues: { url, name, fifo: bool }[] }` |
| `GET /aws/accounts/{id}/sqs/queues/attributes?url=` | View | `{ attributes: Record<string,string> }` (All) + parsed `{ approx_messages, approx_not_visible, approx_delayed, dlq_target_arn? }` |
| `POST /aws/accounts/{id}/sqs/queues/peek` | View | `{ url, max?: 1..10, visibility_timeout?: 0 }` → `{ messages: { message_id, receipt_handle, body, attributes, message_attributes, md5 }[] }` (`receive-message --visibility-timeout 0 --wait-time-seconds 1 --attribute-names All --message-attribute-names All`) |
| `POST /aws/accounts/{id}/sqs/queues/send` | Edit | `{ url, body, delay_seconds?, group_id?, dedup_id?, message_attributes? }` → `{ message_id }` |
| `POST /aws/accounts/{id}/sqs/queues/delete-message` | Edit | `{ url, receipt_handle }` → 204 |
| `POST /aws/accounts/{id}/sqs/queues/purge` | Edit | `{ url, confirm_name }` (must equal queue name) → 204 |
| `POST /aws/accounts/{id}/sqs/queues/redrive` | Edit | `{ source_arn, destination_arn? }` → `{ task_handle }` (`start-message-move-task`) |

### 2.4 EC2
| Method & path | cap | → |
|---|---|---|
| `GET /aws/accounts/{id}/ec2/instances?region=&state=&q=` | View | `{ instances: Ec2Instance[] }` — `Ec2Instance { instance_id, name (Name tag), state, type, az, private_ip, public_ip, launch_time, platform, vpc_id, subnet_id, tags: Record }` |
| `GET /aws/accounts/{id}/ec2/instances/{instance_id}?region=` | View | `Ec2Instance & { raw: Value }` |
| `POST /aws/accounts/{id}/ec2/instances/{instance_id}/start\|stop\|reboot?region=` | Edit | `{ confirm_id }` (must equal instance id for stop/reboot) → `{ previous_state, current_state }` |

### 2.5 Athena
| Method & path | cap | → |
|---|---|---|
| `GET /aws/accounts/{id}/athena/workgroups` | View | `{ workgroups: { name, state, output_location? }[] }` |
| `GET /aws/accounts/{id}/athena/databases?catalog=AwsDataCatalog` | View | `{ databases: string[] }` |
| `GET /aws/accounts/{id}/athena/tables?database=&catalog=` | View | `{ tables: { name, type, columns: { name, type }[] }[] }` (`list-table-metadata`) |
| `GET /aws/accounts/{id}/athena/history?workgroup=&max=` | View | `{ executions: { id, query, state, submitted_at, completed_at?, data_scanned_bytes?, execution_ms? }[] }` |
| `POST /aws/accounts/{id}/athena/query` | Edit | `{ sql, database?, workgroup?, output_location? }` → `{ query_execution_id }`. If neither workgroup has an output location nor `output_location` given ⇒ 400 with hint. |
| `GET /aws/accounts/{id}/athena/query/{qid}?token=&max=` | View | `AthenaQueryStatus { state: QUEUED\|RUNNING\|SUCCEEDED\|FAILED\|CANCELLED, reason?, stats: { data_scanned_bytes, execution_ms }, result?: QueryResult, next_token? }` — `result` is the DB Explorer `QueryResult` shape (`columns: {name,type_hint}`, `rows: unknown[][]`, `stats: {duration_ms,row_count,bytes_read}`, `truncated`) so `ResultsGrid` renders it as-is; first row (header) dropped. |
| `POST /aws/accounts/{id}/athena/query/{qid}/cancel` | View | → 204 |

### 2.6 EKS
| Method & path | cap | → |
|---|---|---|
| `GET /aws/accounts/{id}/eks/clusters?region=` | View | `{ clusters: { name, status, version, endpoint, arn, created_at }[] }` (list + describe fan-out, max 20) |
| `GET /aws/accounts/{id}/eks/clusters/{name}?region=` | View | `{ cluster: Value, nodegroups: { name, status, desired, min, max, instance_types, ami_type }[] }` |
| `POST /aws/accounts/{id}/eks/clusters/{name}/import-kubeconfig?region=` | Edit | `{ cluster_name_override?, default_namespace? }` → `K8sCluster` — runs `aws eks update-kubeconfig --name .. --kubeconfig <data_dir>/kube/<new_id>.yaml --alias <name>` with the account env, then inserts a `k8s_clusters` row `source='eks', aws_account_id=id`. Requires the caller to also hold `kubernetes:Admin` (check in handler via `GrantsRepo::check_global`). |

## 3. Kubernetes — routes (`crates/otto-k8s`)

### 3.1 Clusters & plumbing
| Method & path | → |
|---|---|
| `GET /k8s/status` | → `K8sStatus { kubectl: ToolStatus, k9s: ToolStatus, install: { kubectl: InstallJob, k9s: InstallJob } }`, `ToolStatus { installed, version?, path? }` |
| `POST /k8s/install` | `{ tool: 'kubectl'\|'k9s' }` → `InstallJob` |
| `GET /k8s/discover` | → `{ contexts: { name, cluster, user, namespace?, kubeconfig_path, server? }[] }` from `~/.kube/config` and each `$KUBECONFIG` entry (`kubectl config view -o json --kubeconfig ..`, never prints secrets — use `--minify`? No: use `config get-contexts`-equivalent by parsing `config view -o json` and only reading `contexts[]`/`clusters[].cluster.server`). |
| `GET /k8s/clusters` | → `K8sCluster[]` |
| `POST /k8s/clusters` | `UpsertK8sClusterReq { name, source: 'kubeconfig', kubeconfig_path?, context_name, default_namespace?, environment?, color? }` → `K8sCluster` |
| `POST /k8s/clusters/import` | `{ name, kubeconfig_yaml, context_name?, default_namespace?, environment? }` → `K8sCluster` — writes `<data_dir>/kube/<id>.yaml` 0600; if `context_name` omitted uses the file's current-context; validates with `kubectl config get-contexts`. |
| `GET /k8s/clusters/{id}` · `PATCH` · `DELETE` | as usual; DELETE removes the Otto-owned kubeconfig file for `imported`/`eks` sources only. |
| `POST /k8s/clusters/{id}/test` | → `{ ok, latency_ms, message, server_version? }` (`kubectl version -o json --request-timeout=8s`) |
| `GET /k8s/clusters/{id}/capabilities?refresh=` | → `K8sCapabilities { server_version?, metrics_server: bool, argo_rollouts: bool, argocd: bool, checked_at }` |

`K8sCluster { id, name, source, kubeconfig_path?, context_name, default_namespace?, aws_account_id?, environment, color?, capabilities?, created_by, created_at, updated_at, last_used_at }`.

### 3.2 Reads (View)
| Method & path | → |
|---|---|
| `GET /k8s/clusters/{id}/namespaces` | `{ namespaces: { name, status, age_seconds }[] }` |
| `GET /k8s/clusters/{id}/nodes` | `{ nodes: { name, status, roles, version, cpu_capacity, mem_capacity, cpu_usage?, mem_usage?, age_seconds }[] }` (merge `get nodes` + `top nodes` when metrics available) |
| `GET /k8s/clusters/{id}/resources?kind=&ns=&label=&q=` | `{ kind, items: K8sRow[], has_metrics }` — `kind ∈ pods, deployments, statefulsets, daemonsets, replicasets, jobs, cronjobs, services, ingresses, configmaps, secrets, pvcs, hpas, rollouts, applications, events`. `ns=` empty ⇒ all namespaces (`-A`). `K8sRow { name, namespace, kind, status, ready?, restarts?, age_seconds, node?, ip?, cpu?, mem?, images?, labels, extra: Record<string,string>, health?: 'ok'\|'warn'\|'bad'\|'progressing' }` — `extra` carries kind-specific columns (e.g. deployments: `desired/updated/available`; rollouts: `strategy, step, weight, phase`; applications: `sync, health, revision`; secrets: `type, keys` (NEVER values)). For pods when metrics_server is present, merge `top pods`. |
| `GET /k8s/clusters/{id}/resource?kind=&ns=&name=` | `{ manifest: Value (managedFields stripped, secrets' data values replaced by "<redacted>"), describe: string, events: { type, reason, message, count, last_seen }[] }` |
| `GET /k8s/clusters/{id}/pods/{ns}/{name}/containers` | `{ containers: { name, image, ready, state, restarts, init: bool }[] }` |
| `GET /k8s/clusters/{id}/pods/{ns}/{name}/logs?container=&tail=500&since=&previous=&follow=&timestamps=` | `text/plain` body. `follow=true` ⇒ chunked streaming response that stays open (`kubectl logs -f`); the child is killed when the client disconnects (drop guard). Non-follow has a 60 s timeout and 5 MiB cap. |
| `GET /k8s/clusters/{id}/metrics?ns=` | `{ pods: { name, namespace, cpu_millicores, mem_bytes, containers: {..}[] }[], available: bool }` |

### 3.3 Writes (Edit)
| Method & path | body → |
|---|---|
| `POST /k8s/clusters/{id}/exec` | `{ workspace_id, ns, pod, container?, command?: string[] }` → `Session` — PTY via `Spawner::spawn_command(provider="k8s")`, program `kubectl`, args `--kubeconfig .. --context .. -n ns exec -it pod [-c c] -- sh -c 'command -v bash >/dev/null && exec bash || exec sh'` (or the given command). Title `"<pod> · <ns>"`, meta `{ k8s: { cluster_id, ns, pod, container } }`. |
| `POST /k8s/clusters/{id}/k9s` | `{ workspace_id, ns? }` → `Session` — `k9s --kubeconfig .. --context .. [-n ns]`; 400 `not installed` if k9s missing. |
| `POST /k8s/clusters/{id}/actions` | `K8sActionReq { action, kind, ns, name, params? }` → `{ ok, message, output? }`. Actions table in §4.6. Destructive ones (`delete_pod`, `scale` to 0, `rollout_undo`, `argocd_sync` with prune) require `params.confirm_name == name`. |

## 4. Behaviour details

### 4.1 kubectl arg building
`base_args(cluster) = ["--kubeconfig", path?, "--context", ctx, "--request-timeout", "20s"]`
(`--kubeconfig` omitted when `kubeconfig_path` is NULL ⇒ kubectl's own default resolution). Never pass `-o wide`; always `-o json` and normalize in Rust.

### 4.2 Row normalization (pods example)
`status` = `.status.phase` unless a container is in `CrashLoopBackOff`/`ImagePullBackOff`/`Error` (use that reason), `Terminating` when `metadata.deletionTimestamp` set; `ready` = `"n/m"`; `restarts` = sum of `restartCount`; `health` = `ok` if Running+all ready or Succeeded; `bad` for CrashLoop/Error/Failed/ImagePull; `progressing` for Pending/ContainerCreating/Init; `warn` otherwise.

### 4.3 Deployments / StatefulSets / DaemonSets
`extra = { desired, updated, ready, available }`, health from readiness math; `rollout restart` supported; `scale` via `kubectl scale --replicas`.

### 4.4 Argo Rollouts (`rollouts.argoproj.io/v1alpha1`)
`extra = { strategy: canary|blueGreen, phase: .status.phase, step: "<currentStepIndex>/<len(steps)>", weight: .status.canary.weights.canary.weight?, paused: .spec.paused, message: .status.message }`.

### 4.5 ArgoCD Applications (`argoproj.io/v1alpha1` Application)
`extra = { sync: .status.sync.status, health: .status.health.status, revision: .status.sync.revision[0:8], repo: .spec.source.repoURL, path: .spec.source.path, dest_ns: .spec.destination.namespace, operation: .status.operationState.phase? }`.

### 4.6 Actions (all via `kubectl`)
| action | applies to | implementation |
|---|---|---|
| `restart` | deployments, statefulsets, daemonsets | `kubectl rollout restart <kind>/<name>` |
| `restart` | rollouts | `kubectl patch rollout <name> --type merge -p '{"spec":{"restartAt":"<RFC3339 now>"}}'` |
| `scale` | deployments, statefulsets, rollouts | `kubectl scale <kind>/<name> --replicas=<params.replicas>` |
| `delete_pod` | pods | `kubectl delete pod <name> [--grace-period=<params.grace>]` |
| `rollout_status` | deployments, statefulsets, daemonsets | `kubectl rollout status --timeout=5s` (returns output) |
| `rollout_undo` | deployments, statefulsets, daemonsets | `kubectl rollout undo <kind>/<name> [--to-revision]` |
| `rollout_pause` / `rollout_resume` | deployments | `kubectl rollout pause|resume` |
| `rollout_pause` / `rollout_resume` | rollouts | patch `{"spec":{"paused":true|false}}` |
| `rollout_promote` | rollouts | merge-patch status via `kubectl patch rollout <n> --subresource=status --type merge -p '{"status":{"pauseConditions":null}}'`; with `params.full=true` also `{"status":{"promoteFull":true}}` and `{"spec":{"paused":false}}`. (Mirrors what `kubectl argo rollouts promote` does.) |
| `rollout_abort` | rollouts | `--subresource=status` patch `{"status":{"abort":true}}` |
| `rollout_retry` | rollouts | `--subresource=status` patch `{"status":{"abort":false}}` |
| `argocd_sync` | applications | `kubectl patch application <n> -n <ns> --type merge -p '{"operation":{"initiatedBy":{"username":"otto"},"sync":{"revision":"<params.revision or .spec.source.targetRevision>","prune":<params.prune>,"syncStrategy":{"hook":{}}}}}'` (this is what `argocd --core app sync` writes) |
| `argocd_refresh` | applications | annotate `argocd.argoproj.io/refresh=normal` (or `hard` when `params.hard`) |
| `argocd_terminate_op` | applications | `kubectl patch application <n> --type json -p '[{"op":"remove","path":"/operation"}]'` |
| `argocd_app_restart` | applications | for each Deployment/Rollout/StatefulSet/DaemonSet in `.status.resources[]` matching `params.resource_kind?` do the `restart` action above (in the app's destination ns / context). This is "redeploy" for ArgoCD-managed workloads. |
| `cronjob_trigger` | cronjobs | `kubectl create job --from=cronjob/<name> <name>-manual-<ts>` |
| `cronjob_suspend` / `cronjob_resume` | cronjobs | patch `{"spec":{"suspend":true|false}}` |

kubectl `forbidden` in stderr ⇒ `Error::Forbidden("cluster RBAC: <first line>")`.

## 5. UI

Shared UX rules (both modules): a **first-run panel** when `status.installed == false`
("Otto needs the AWS CLI. Install now" → progress bar polling `/status` every 1.5 s,
log tail collapsible; success auto-continues). A **Setup / Accounts** sheet
reachable from a gear button. Cards for accounts/clusters with environment
pill (dev/staging/prod — prod gets the same red treatment as connections),
color dot, identity summary. All destructive confirmations go through
`confirmer.ask` / `confirmer.promptText` (typed-name confirm for purge, stop,
delete pod, prune sync). `toasts` for outcomes. Every table: sticky header,
client-side filter box, ⌘/Ctrl+K searchable, right-click `ctxMenu` with the
row's actions. Refresh button + auto-refresh toggle (10 s) on live lists.
Mobile: lists stack; details open as a sheet. Add `'aws'` and `'kubernetes'`
to `ui/e2e/helpers.ts` `PAGES`.

### 5.1 AWS (`ui/src/modules/aws/`, store `ui/src/lib/stores/aws.svelte.ts`, api `ui/src/lib/api/aws.ts`, types appended to `types.ts` under a `// AWS console` banner)
Routes: `#/aws` (accounts overview) · `#/aws/<accountId>/<service>` where
service ∈ `s3|sqs|ec2|athena|eks` · deep links `#/aws/<id>/s3/<bucket>?prefix=`.
- **Accounts overview**: cards; "Add account" wizard (2 steps): (1) pick
  "Use an existing AWS profile" (list from `/aws/discover`, shows SSO/role
  hints, one-click) OR "Enter access keys" (id/secret/session token, region
  select); (2) name, environment, color → test → save. Cards show identity
  (account id, role name from ARN), region, per-service permission chips
  (green allowed / grey denied / hollow unknown), "Sign in" button when
  `login_required` (opens a `<Terminal>` sheet on the returned session; poll
  `/test` every 3 s until ok, then close). Left rail inside the module lists
  accounts → services (greyed when denied or when the user lacks the
  feature's View grant).
- **S3**: bucket list (search) → object browser with breadcrumb prefixes,
  folder rows first, size/modified columns, preview drawer (text/JSON/CSV
  pretty), Download button (uses `authedBlobUrl`-style fetch + `<a download>`;
  in Tauri prefer the save-dialog pattern already used by SFTP download).
- **SQS**: queue list with approx counts; queue detail tabs: Messages (Peek
  N, message body viewer JSON-pretty, delete-message per row [Edit]),
  Send (body editor + attributes, FIFO fields shown only for `.fifo`),
  Attributes, Redrive (DLQ → source) [Edit]. Purge in the ⋯ menu [Edit, typed
  confirm].
- **EC2**: instances table (state pill, name, id, type, AZ, IPs, launch),
  filters by state, region switcher in toolbar, row actions start/stop/reboot
  [Edit], detail sheet with tags + raw JSON tree (`JsonTree`).
- **Athena**: three-pane like DB Explorer: catalog tree (databases → tables →
  columns, `CodeEditor` language sql with completion from the tree), editor
  with workgroup/database selectors + Run (⌘↵, Edit-gated), results via
  `ResultsGrid` (`connectionId={null}`), status bar with state, scanned
  bytes and cost estimate (`$5/TB`), Cancel; History tab (click to reload
  SQL). Poll status every 1 s while QUEUED/RUNNING.
- **EKS**: clusters table + detail (nodegroups); "Open in Kubernetes"
  button [aws_eks Edit + kubernetes Admin] → import → navigates to
  `#/kubernetes/<clusterId>`.

### 5.2 Kubernetes (`ui/src/modules/kubernetes/`, store `ui/src/lib/stores/k8s.svelte.ts`, api `ui/src/lib/api/k8s.ts`, types under `// Kubernetes console`)
Routes: `#/kubernetes` (clusters) · `#/kubernetes/<clusterId>` · `#/kubernetes/<clusterId>/<kind>` · `#/kubernetes/<clusterId>/<kind>/<ns>/<name>`.
- **Clusters overview**: cards (+ server version, capability chips
  metrics/rollouts/argocd, environment). "Add cluster" wizard: (1) "Pick a
  context from your kubeconfig" (list from `/k8s/discover`, multi-select)
  OR "Paste a kubeconfig" OR "From EKS" (jumps to AWS/EKS). (2) name,
  default namespace, environment → test → save.
- **Cluster workspace** (k9s-like): top bar = cluster switcher, **namespace
  filter** (combobox, "All namespaces", remembers last per cluster in
  localStorage), free-text filter, refresh/auto-refresh. Left = resource kinds
  list (Pods, Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs,
  CronJobs, Services, Ingresses, ConfigMaps, Secrets, PVCs, HPAs, Nodes,
  Events, + "Argo Rollouts" and "ArgoCD Apps" only when capabilities say so).
  Center = table (`VirtualList`) with health-colored status, CPU/MEM columns
  when metrics available. Right/bottom = detail drawer with tabs: Overview
  (normalized fields), Manifest (YAML via `CodeEditor` readonly, secrets
  redacted), Describe, Events, **Logs** (container selector, tail/since,
  follow toggle streaming via `fetch` ReadableStream, timestamps toggle,
  search, download), **Terminal** (Exec: opens `<Terminal sessionId>` inline
  — `preferDom`), Metrics (per-container cpu/mem bars).
- Row actions (ctxMenu + drawer buttons, Edit-gated): pods → Logs, Exec,
  Delete; deployments/sts/ds → Restart, Scale, Rollout status/undo/pause/
  resume; rollouts → Restart, Promote, Promote (full), Abort, Retry, Pause/
  Resume, Scale; applications → Sync (dialog: revision, prune checkbox,
  typed confirm when prune), Refresh / Hard refresh, Restart workloads
  ("Redeploy"), Terminate operation; cronjobs → Trigger now, Suspend/Resume.
- **k9s button** in the top bar (Edit): opens a full-height `<Terminal
  preferDom>` tab; if k9s missing shows "Install k9s" (Admin) using
  `/k8s/install`.
- Keyboard: `/` focuses filter, `Esc` closes drawer, `l` logs, `s` shell,
  `d` describe on the selected row (k9s muscle memory), shown in a `?` hint.

## 6. MCP tools (`crates/ottod/src/mcp_tools.rs`) — inward server
Add to `tool_catalog()` + `run_tool()`; all call the HTTP routes above with
the session token (so RBAC applies). Read tools:
`aws_list_accounts`, `aws_s3_list_buckets {account_id}`, `aws_s3_list_objects {account_id,bucket,prefix?}`,
`aws_s3_preview {account_id,bucket,key,max_bytes?}`, `aws_sqs_list_queues {account_id}`,
`aws_sqs_peek {account_id,url,max?}`, `aws_ec2_list_instances {account_id,region?,state?}`,
`aws_athena_list_tables {account_id,database}`, `aws_athena_get_query {account_id,query_execution_id}`,
`aws_eks_list_clusters {account_id,region?}`, `k8s_list_clusters`,
`k8s_get_resources {cluster_id,kind,namespace?,label?}`, `k8s_describe {cluster_id,kind,namespace,name}`,
`k8s_logs {cluster_id,namespace,pod,container?,tail?}`, `k8s_top {cluster_id,namespace?}`.
Write tools (documented as mutating in the module doc comment):
`aws_athena_query {account_id,sql,database?,workgroup?}` (returns id; agent
then polls `aws_athena_get_query`), `aws_sqs_send {account_id,url,body}`,
`k8s_action {cluster_id,action,kind,namespace,name,params?}`.
Also register the same set on the **outward** governed surface
(`crates/otto-server/src/mcp_outward.rs`): reads in `DEFAULT_ENABLED`, the
three writers in `DANGEROUS` (so they need human approval by default), with
`route_for` mappings + the classification tests updated. Update
`docs/features/mcp-control-plane.md` tool lists.

## 7. Docs to produce
`docs/features/aws-console.md`, `docs/features/kubernetes-console.md` (follow
the outline of `docs/features/connections-ssh-sftp.md`), rows in
`docs/features/README.md` under "Data & infrastructure", API sections in
`docs/contracts/api.md` (numbered rows are not required — a `## AWS console
(/aws/*)` and `## Kubernetes console (/k8s/*)` section with the un-numbered
table format is fine, but every route path must appear verbatim), WS events
in `docs/contracts/ws.md`, the RBAC feature table in
`docs/features/rbac-multiuser-sharing.md`.
