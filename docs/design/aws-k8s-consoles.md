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
| `POST /aws/accounts` | `UpsertAwsAccountReq { name, auth_mode: 'profile'\|'access_keys', profile?, region, access_key_id?, secret_access_key?, session_token?, role_arn?, endpoint_url?, environment?, color? }` → `AwsAccount` (201). Validates: profile mode needs `profile`; keys mode needs id+secret; `endpoint_url` must be `http(s)://` (plain `http` only for loopback hosts). Runs `sts get-caller-identity` best-effort and stores `identity`. |
| `GET /aws/accounts/{id}` | → `AwsAccount` |
| `PATCH /aws/accounts/{id}` | partial `UpsertAwsAccountReq` (secret fields optional = keep) → `AwsAccount` |
| `DELETE /aws/accounts/{id}` | → 204 (deletes Keychain secret; k8s clusters linked via `aws_account_id` are kept, FK sets NULL) |
| `POST /aws/accounts/{id}/test` | → `AwsTestResp { ok, latency_ms, message, identity?: AwsIdentity, login_required: bool }` |
| `GET /aws/accounts/{id}/permissions?refresh=` | → `AwsPermissions { checked_at, identity?, services: { s3, sqs, ec2, athena, eks }: 'allowed'\|'denied'\|'unknown', login_required: bool }` |
| `POST /aws/accounts/{id}/login` | `{ workspace_id }` → `Session` — spawns `aws sso login --profile <p>` (profile mode) in a PTY via `Spawner::spawn_command(provider="aws")`; for `access_keys` returns 400. |

`AwsAccount { id, name, auth_mode, profile, region, access_key_id?, role_arn?, endpoint_url?, environment, color?, identity?: AwsIdentity { account, arn, user_id }, permissions?: AwsPermissions, created_by, created_at, updated_at, last_used_at }` — **never** includes secrets.
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

### 2.7 Implementation notes / deviations (AWS backend, as built)

- **Probe commands**: the S3 probe is `s3api list-buckets --max-items 1` (same
  IAM action `s3:ListAllMyBuckets` as `s3 ls`, but JSON). A probe snapshot with
  `login_required: true` is **not** cached, so the first call after `sso login`
  re-probes.
- **S3 object paging** uses the CLI paginator (`--max-items` / `--starting-token`);
  `next_token` is the CLI's `NextToken` (a raw `NextContinuationToken` is
  accepted too). The prefix marker object (`key == prefix`) is dropped.
- **Installer state** lives in a process-wide `OnceLock` in `install.rs` (the
  `AwsCtx` trait carries no service handle; the router is stateless), not in a
  service struct. `locate()` additionally honours an `OTTO_AWS_BIN` env
  override at the top of the ladder (tests / power users).
- **`POST /aws/accounts`** defaults `region` to `us-east-1` when omitted;
  `created_by` is nullable in the DTO (FK `ON DELETE SET NULL`).
- **`AWS_PROFILE` handling (as built)**: the daemon's own `AWS_PROFILE` is
  removed from every child env (`Command::env_remove` in `cli::run_raw` and
  the S3 download spawn); profile-mode accounts add it back explicitly. It is
  never set to `""` — CLI v2 reads that as a profile literally named `""` and
  fails every call with `The config profile () could not be found` (found by
  the LocalStack e2e; it broke all access-keys accounts).
- **Custom endpoint (as built, §2.1)**: `params_json.endpoint_url` /
  `UpsertAwsAccountReq.endpoint_url` / `AwsAccount.endpoint_url` (optional,
  PATCH-able — `""` clears, omitted keeps). Validated by
  `accounts::validate_endpoint_url`: `http://` or `https://` only, no
  whitespace, and plain `http` only for loopback hosts
  (`otto_netguard::require_tls_or_loopback`). When set, `build_env` injects
  `AWS_ENDPOINT_URL=<url>` + `AWS_EC2_METADATA_DISABLED=true` into **every**
  `aws` subprocess in both auth modes (assume-role, probes, `s3 cp` download
  stream, `sso login` included) — no per-command `--endpoint-url` flags. This
  is what the LocalStack e2e (`ui/e2e/desktop-aws-localstack.spec.ts`) and
  VPC-endpoint / S3-compatible setups rely on.
- **Audit** adds `aws.sqs.delete_message`, `aws.sqs.redrive` and
  `aws.eks.import_kubeconfig` to the listed actions.
- **`import-kubeconfig`** returns **201**, emits `k8s_cluster_updated`, and
  inserts the `k8s_clusters` row with a local sqlx statement (TODO: switch to
  `K8sClustersRepo`). Region is passed both as env and `--region`.
- **Athena `QueryResult`** is a local struct with the identical serde shape
  (`crates/otto-aws/src/athena.rs`) rather than a dependency on
  `otto-dbviewer` (keeps the DB drivers out of this crate). Partition keys
  are appended to `tables[].columns`.
- **`sso login` PTY** meta is `{ aws: { account_id, profile } }`, title
  `aws sso login · <account name>`; the assume-role layer is skipped for the
  login itself (it signs in the base profile).

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

**As built (UI half — deviations / decisions the spec left open):**

- **Wizard step 2 is "Save & test", not test → save.** `POST …/test` needs an
  account row (it runs `sts get-caller-identity` with the stored creds), so the
  wizard saves first and immediately tests; a third panel shows the result
  (identity, or `login_required` with a "Sign in now" shortcut) plus "Test again"
  / "Back" (edit + retest) / "Done". Edit mode reuses the same sheet (secrets
  blank = keep, per the PATCH contract).
- **Sign-in sheet is global**: `aws.beginLogin(accountId, workspaceId)` stores
  `{accountId, sessionId}` in the store and `AwsPage` mounts one
  `LoginSheet` (`<Terminal preferDom>` on the PTY session, `/test` polled every
  3 s, auto-closes on `ok`, then `permissions?refresh=true`). Any view whose
  error message starts with `login required` shows a "Sign in" action that
  routes here. `login` for `access_keys` accounts is never offered (400 on the
  server) — the card says "update the keys via Edit" instead.
- **Deep link encoding**: the S3 prefix rides in the LAST hash segment as
  `#/aws/<id>/s3/<encodeURIComponent(bucket)>?prefix=<encodeURIComponent(prefix)>`.
  The hash router splits on `/` BEFORE decoding, so an un-encoded prefix with
  slashes would be split into extra segments; the browser parses the segment
  with `splitBucketSegment()` (`util.ts`). Bucket rows / breadcrumbs navigate
  with `router.go` so back/forward walk the prefix history.
- **S3 download** streams the daemon's response into a Blob
  (`awsDownloadBlob`, with a byte-progress bar + Cancel via AbortSignal) and
  hands it to the browser with `<a download>` in both the web build and the
  Tauri shell — the SFTP "pick a local dir" pattern doesn't apply because the
  bytes already transit the daemon→webview connection (no server-side
  `local_path`). Preview drawer is inline on desktop and a Modal sheet on mobile.
- **Toolbar conventions** (`ViewToolbar.svelte`): every service view has a sticky
  toolbar with the client-side filter box (`/` focuses it), a Refresh button and
  an "Auto" checkbox (10 s interval, per view, off by default). EC2/EKS add a
  region `<select>` fed by `/aws/regions`; EC2 adds a state filter with counts.
- **Rail vs. mobile**: desktop shows the account → service rail
  (`AccountRail.svelte`; services hidden when the user lacks `aws_<svc>:View`,
  greyed + "denied" when the account's probe says `denied`, still clickable so
  the AccessDenied is visible). On phone/tablet the rail is replaced by the
  accounts overview (cards) and a service view takes the full width behind a
  "‹ Accounts" bar; SQS/EKS/EC2 details open as full-width panes / Modal sheets.
- **Typed confirmations**: SQS purge (queue name), EC2 stop/reboot (instance
  id), all via `confirmer.promptText`; `confirmer.ask` for delete-message,
  start, account delete, redrive and EKS import. Never a native dialog.
- **Athena**: the SQL text + chosen workgroup/database are remembered per
  account in `localStorage`. "Run" executes the editor SELECTION when one
  exists (label flips to "Run selection"). Completion is fully client-side from
  the loaded tree (`db.` → tables, `table.` / `db.table.` → columns, bare →
  databases + current-db tables/columns + keywords); tables are loaded lazily
  when a database node is expanded. History rows re-open by loading the SQL
  into the editor AND fetching `GET …/query/{qid}` for that execution's result.
  Cost estimate = `max(bytes, 10 MiB) / TiB × $5`.
- **Types**: `EksImportResp` in the AWS section is the minimal
  `{ id, name, …rest }` projection of `K8sCluster` (the full shape lives in the
  Kubernetes section) so the two halves' type sections don't depend on each
  other. Event variants `aws_account_updated` / `aws_install_updated` are in the
  `OttoEvent` union; `events.svelte.ts` routes both to `aws.applyEvent`.
- **Mock layer**: `ui/src/lib/api/mock.ts` serves every §2 route (two accounts,
  S3/SQS/EC2/Athena/EKS fixtures, a 2-poll RUNNING→SUCCEEDED Athena query, a
  4 s fake installer); set `localStorage.otto_mock_aws_missing=1` to see the
  first-run install panel.
- **E2E**: `ui/e2e/desktop-aws.spec.ts` — renders `#/aws` (install panel,
  overview, or the explicit "unavailable" state) with no console errors,
  checks the wizard Modal with `expectFullyInViewport`, and seeds an
  `access_keys` account over the API asserting the card + `prod` pill. The two
  API-dependent tests `test.skip` (with the reason logged) when
  `GET /api/v1/aws/status` is 404 or the test daemon has no `aws` binary.

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

**As built (UI, `ui/src/modules/kubernetes/`) — deviations & additions:**
- Route encoding: a cluster-scoped row (Nodes) has no namespace, so the
  selected-row segment is `#/kubernetes/<id>/nodes/-/<name>` (`-` = empty ns).
  Cluster / kind / selected row are URL state; namespace, filter, drawer tab
  and the resource cache live in `ui/src/lib/stores/k8s.svelte.ts`.
- `InstallJob` is typed as `K8sInstallJob` in `types.ts` (same fields) so the
  AWS and Kubernetes type sections never collide on one exported name.
- The first-run panel has a session-only **"Continue without installing"**
  link (a Viewer can't install; kubectl on a non-standard path would otherwise
  lock the whole module) — clusters remain reachable, resource loads then
  surface the daemon's `not installed` error in the table.
- The wizard's "test" happens **after** save (`/test` needs a cluster id):
  Save & test → create → best-effort `POST /test` + capability refresh, each
  toasting its outcome; a single new cluster opens its workspace directly.
  Multi-select creates one row per context, named after the context.
- Nodes (own endpoint) are folded into the shared `K8sRow` shape client-side
  so the table/drawer render every kind identically; `extra` keys the UI
  doesn't know (newer daemon) are appended as generic columns, never dropped.
- Extra shortcuts beyond the contract: `n` namespace, `j`/`k` move selection,
  `Enter` open, `y` manifest, `r` refresh. Logs is View-level (only Shell and
  §4.6 actions are Edit-gated). Exec/k9s PTY sessions are killed when their
  view closes (`DELETE /sessions/{id}`); they are not listed in Agents.
- `GET /k8s/status` returning 404 renders a "console isn't available on this
  daemon" state (older daemon) instead of the install panel.

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

**As built (MCP half — decisions the list above left open; backend/UI need not
change, but these are the argument names and bindings agents see):**

- **Optional extras beyond the minimal signatures above**, all mapped 1:1 onto
  the routes' existing query/body fields — every AWS service tool accepts
  `region?` (§1 "every service endpoint accepts `?region=`"); `aws_s3_list_objects`
  also takes `token?`/`max?` (paging), `aws_sqs_list_queues` `prefix?`,
  `aws_ec2_list_instances` `q?`, `aws_athena_list_tables` `catalog?`,
  `aws_athena_get_query` `token?`/`max?` (result paging), `aws_athena_query`
  `output_location?`, `aws_sqs_send` `delay_seconds?`/`group_id?`/`dedup_id?`/
  `message_attributes?`, `k8s_get_resources` `q?`, `k8s_logs`
  `since?`/`previous?`/`timestamps?`. Nothing new is required of the routes.
- **Argument → query naming**: the tools say `namespace`; the routes say `ns`
  (§3.2). The tools translate. `namespace` omitted on `k8s_get_resources` /
  `k8s_top` ⇒ the `ns` param is **not sent** (the route's all-namespaces
  default), not sent as empty.
- **`aws_sqs_peek` is a POST but a read**: the tool pins `visibility_timeout: 0`
  and clamps `max` into 1..10 client-side; it lives with the reads on both
  surfaces (policy already grades `/peek` as `aws_sqs:View`).
- **`k8s_logs`** returns `{ text, truncated }` (not raw text). `follow` is never
  forwarded (a streaming response would hang a tool call). The text is capped
  at **256 KiB keeping the tail** (newest lines) on both surfaces; the inward
  server uses a 65 s call timeout for this one route because the non-follow
  logs route itself has a 60 s kubectl budget (the generic tool timeout is 20 s).
- **`k8s_describe` / `k8s_action` require `namespace`** as written. Every kind in
  the §3.2 list is namespaced, so no cluster-scoped escape hatch was added;
  if `nodes` (or another cluster-scoped kind) is ever added to `?kind=`, relax
  the tool to `namespace?` at the same time.
- **`k8s_action.params`** is forwarded verbatim, defaulting to `{}`; the route
  owns the `confirm_name` / `replicas` / `prune` / `revision` semantics. The
  tool descriptions tell the agent to set `params.confirm_name` only after an
  explicit user confirmation.
- **Not exposed to agents (by design, not omission)**: S3 download (binary
  stream), SQS delete-message / purge / redrive, EC2 start/stop/reboot, Athena
  cancel, EKS kubeconfig import, `exec` / `k9s` PTYs, account/cluster CRUD and
  CLI installs. Add any of these only as `DANGEROUS` + module-doc exceptions.
- **Outward pipeline**: no per-tool feature mapping was needed — the governed
  self-call reuses the `/aws/*` / `/k8s/*` policy branches, and the token
  workspace pin is skipped because these tools carry no `workspace_id`
  (accounts/clusters are global rows, like connections).

**As built (Kubernetes backend half, `crates/otto-k8s` + `otto-state::k8s_clusters`)
— refinements to §1/§3/§4; the routes and DTO names are exactly as listed:**

- **Installer state is process-global**, not "inside the service": `K8sCtx` is
  fixed by otto-server and carries no service object, so the per-tool
  `InstallJob` slots live in a `OnceLock<Arc<Installer>>` (`install.rs`). Same
  contract on the wire (`/k8s/status` + `k8s_install_updated`).
- **Streams drop `--request-timeout`.** `base_args()` is exactly §4.1, but
  `exec`, `logs?follow=true` and k9s use `base_args_stream()` (same flags minus
  `--request-timeout 20s`) — kubectl applies that flag to its HTTP client and
  would cut a long-lived stream/PTY.
- **Metrics come from the metrics API, not `kubectl top` text**:
  `get --raw /apis/metrics.k8s.io/v1beta1/[namespaces/<ns>/]pods|nodes` (JSON,
  parsed `Quantity`s). `K8sRow.cpu` is **millicores**, `mem` **bytes**;
  `NodeRow.cpu_capacity/cpu_usage` millicores, `mem_*` bytes (numbers, UI formats).
- **Argo CRDs are fully qualified** in every argv (`rollouts.argoproj.io/<n>`,
  `applications.argoproj.io/<n>`; the `<plural>/<name>` form everywhere) so an
  unrelated CRD sharing a short name can never be targeted.
- **`rollout_promote` always unpauses the spec** (`{"spec":{"paused":false}}`)
  after clearing `pauseConditions`, not only with `full` — an indefinite
  `pause: {}` step sets both, exactly what `kubectl argo rollouts promote` does.
  With `full` the `promoteFull` status patch is sent in between.
- **`argocd_app_restart` runs in the hosting cluster's context** (the app's
  destination is assumed to be that cluster); the resource's own namespace from
  `.status.resources[]` wins over the app's destination namespace.
  `params.resource_name?` was added next to `resource_kind?`.
- **`scale` also accepts `replicasets`**; `delete_pod` adds `--force` automatically
  when `params.grace == 0` (kubectl refuses `--grace-period=0` without it).
- **`rollout_status`** returns `200 { ok:false, output }` (not an error) when the
  rollout has not converged within the 5 s timeout.
- **`/resource?kind=`** additionally accepts `nodes` / `namespaces`
  (cluster-scoped, `ns` ignored); kind short names (`po`, `deploy`, `svc`, …)
  are accepted on both list and detail. Events for the detail view are selected
  by `involvedObject.uid`, not name.
- **`K8sCluster` carries `params`** (non-secret extras such as `eks_region`) and
  `DiscoveredContext` carries `current: bool`; `K8sRow.extra` gets a few more
  per-kind keys than the examples (documented in `api.md`). `PATCH` refuses
  `kubeconfig_path` changes on `imported`/`eks` rows.
- **Import validates with `kubectl --kubeconfig <file> config view -o json`**
  (also yields `current-context` and the context's namespace) rather than
  `config get-contexts`; no YAML parser was added to the crate.
- **Redaction** = `otto_core::redact::redact_text` + an `AKIA…/ASIA…` scrubber
  (`cli::strip_aws_keys`) because the core redactor only classifies whole
  whitespace-delimited words.
- **Extra audit verbs**: `k8s.install` and `k8s.k9s` alongside the contract's
  `k8s.action.<action>` / `k8s.exec`; action audits are written for failures too.
- **EKS env injection** is a minimal local mirror (`clusters::build_aws_env`,
  raw `aws_accounts` query + Keychain read) with a TODO to share
  `otto_aws::accounts::build_env`; assume-role caching is NOT replicated here
  (profile-mode accounts cover it through the CLI itself).

## 7. Docs to produce
`docs/features/aws-console.md`, `docs/features/kubernetes-console.md` (follow
the outline of `docs/features/connections-ssh-sftp.md`), rows in
`docs/features/README.md` under "Data & infrastructure", API sections in
`docs/contracts/api.md` (numbered rows are not required — a `## AWS console
(/aws/*)` and `## Kubernetes console (/k8s/*)` section with the un-numbered
table format is fine, but every route path must appear verbatim), WS events
in `docs/contracts/ws.md`, the RBAC feature table in
`docs/features/rbac-multiuser-sharing.md`.
