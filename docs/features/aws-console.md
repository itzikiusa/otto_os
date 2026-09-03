# AWS console — S3, SQS, EC2, Athena & EKS through the `aws` CLI

The **AWS console** lets you browse and operate AWS from inside Otto, per saved
**account**, without leaving the app: read-only **S3** (buckets → prefixes →
preview / download), **SQS** (peek, send, purge, redrive), **EC2** (list, start /
stop / reboot), **Athena** (catalog tree, SQL editor, results in the DB
Explorer grid, history) and **EKS** (clusters + nodegroups, one-click import
into the [Kubernetes console](./kubernetes-console.md)).

Everything is executed by the **`aws` CLI v2** as a subprocess with
`--output json` — there is no AWS SDK in the daemon. That is deliberate: the
CLI already handles IAM Identity Center (SSO), assume-role chains, MFA prompts
and `credential_process`, so whatever works for you in a terminal works here.
If the CLI is missing, Otto installs it for you (Homebrew when present, else
the official `.pkg` into your home — never `sudo`).

An **account** is one of two things:

| `auth_mode` | What Otto stores | How the CLI is fed |
|---|---|---|
| `profile` | The profile **name** from `~/.aws/config` (+ region, environment, color). No secret. | `AWS_PROFILE=<name>` — SSO / role / MFA handled by the CLI. **Sign in** spawns `aws sso login` in a PTY tab when the token expires. |
| `access_keys` | The **access-key id** in the DB row; the **secret key + optional session token** in the macOS Keychain under `aws-<id>`. | `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` / `AWS_SESSION_TOKEN` env vars on each subprocess. Optional `role_arn` ⇒ `sts assume-role`, cached ~55 min in memory. |

Otto **never writes** `~/.aws/config` or `~/.aws/credentials` (it only reads
profile *names* from them), and never puts a credential on a command line.

---

## 1. Overview & where it lives

| Concern | Location |
|---|---|
| CLI runner (timeouts, stderr classification, redaction) | `crates/otto-aws/src/cli.rs` |
| Binary discovery + on-demand installer job | `crates/otto-aws/src/install.rs` |
| Account service (CRUD, Keychain, env builder, test, permission probe, `sso login`) | `crates/otto-aws/src/accounts.rs` |
| `~/.aws` profile discovery (names/metadata only) | `crates/otto-aws/src/discover.rs` |
| Per-service modules (CLI JSON → DTO normalizers) | `crates/otto-aws/src/{s3,sqs,ec2,athena,eks}.rs` |
| REST router (`/aws/*`) + audit | `crates/otto-aws/src/http.rs` |
| Account rows (`aws_accounts`, migration `0113`) | `crates/otto-state/src/aws_accounts.rs` |
| RBAC policy branches ("AWS console") | `crates/otto-server/src/policy.rs` |
| Server wiring (`AwsCtx` impl, router merge) | `crates/otto-server/src/modules.rs` |
| WS events (`aws_account_updated`, `aws_install_updated`) | `crates/otto-core/src/event.rs`, `docs/contracts/ws.md` |
| UI module | `ui/src/modules/aws/`, store `ui/src/lib/stores/aws.svelte.ts`, API `ui/src/lib/api/aws.ts` |
| Build contract (all halves) | `docs/design/aws-k8s-consoles.md` |
| Authoritative API reference | `docs/contracts/api.md` § "AWS console (`/aws/*`)" |

---

## 2. Setup

### 2.1 The CLI

Open **AWS** in the sidebar. `GET /aws/status` reports whether an `aws` binary
was found and its version. The lookup ladder is: `OTTO_AWS_BIN` env override →
`$PATH` → `/opt/homebrew/bin` → `/usr/local/bin` → `~/.local/bin` →
`<data_dir>/bin` → `~/aws-cli` (where the pkg installer lands).

If nothing is found the module shows a first-run panel: **Install now**
(`POST /aws/install`, `aws:Admin`) starts a background job and the panel polls
`/status` every 1.5 s showing `install.log_tail`:

1. `brew install awscli` when `brew` is on the ladder; else
2. `curl -fsSL https://awscli.amazonaws.com/AWSCLIV2.pkg` into
   `<data_dir>/tmp/aws-install/`, then
   `installer -pkg AWSCLIV2.pkg -target CurrentUserHomeDirectory` (lands in
   `~/aws-cli/`, no admin rights), then symlinks `aws` + `aws_completer` into
   `<data_dir>/bin` (already on the daemon's `PATH`).
3. Verifies `aws --version` runs before reporting `done`.

The job is idempotent (`POST` while `running` just returns the current
snapshot) and broadcasts `aws_install_updated` on every state change.

### 2.2 Adding an account

**Setup → Add account** is a two-step wizard:

1. **Use an existing AWS profile** — the list comes from `GET /aws/discover`,
   which parses `~/.aws/config` (`[profile x]`, `[default]`, and resolves
   `sso_session` blocks to their `sso_start_url`) and `~/.aws/credentials`
   (names only). Rows show SSO / role hints so you can tell them apart. Or
   **Enter access keys** — id, secret, optional session token, region.
2. Name, environment (`dev` / `staging` / `prod` — prod gets the red treatment
   everywhere), color → **Test** → **Save**.

`POST /aws/accounts` validates the mode (`profile` needs `profile`; keys need
id + secret), stores the secret in the Keychain, and runs
`sts get-caller-identity` best-effort — a wrong key or an expired SSO token
does **not** block saving; the card simply shows **Sign in** / the error.

`PATCH` is partial: omitted secret fields keep the stored secret;
`session_token: ""` clears the token. `DELETE` removes the Keychain entry;
Kubernetes clusters imported from this account keep working but lose the link
(`aws_account_id` → NULL).

**Advanced: custom endpoint (LocalStack / VPC endpoints / S3-compatible).**
Step 1 of the wizard (access-keys mode) has an **Advanced** disclosure with an
**Endpoint URL (optional)** field — e.g. `http://localhost:4566` for
[LocalStack](https://localstack.cloud), a VPC interface endpoint
(`https://vpce-….s3.eu-west-1.vpce.amazonaws.com`), or an S3-compatible store
(MinIO, Ceph RGW). It is stored as `params_json.endpoint_url`
(`endpoint_url` on `AwsAccount` / `UpsertAwsAccountReq`, PATCH-able —
`""` clears it) and shown on the account card. When set, the daemon injects
`AWS_ENDPOINT_URL=<url>` **and** `AWS_EC2_METADATA_DISABLED=true` into the
environment of **every** `aws` subprocess for that account, in both auth
modes — the permission probes, assume-role, the streamed `s3 cp` download and
`sso login` included (CLI v2 ≥ 2.13 honours the variable for every command,
so no per-call `--endpoint-url` flags are needed). Validation: the value must
be an `http://` or `https://` URL without whitespace, and plain `http` is
accepted for **loopback hosts only** (`localhost`, `127.0.0.1`, `[::1]`) —
the same rule as `otto_netguard::require_tls_or_loopback`, because static
keys would otherwise travel unencrypted; anything else is a `400`. Profile
accounts can set it too (the wizard exposes it under **Credentials & region**
when editing).

### 2.3 Sign in (SSO expiry)

Every CLI failure is classified. stderr containing `ExpiredToken`,
`ExpiredTokenException`, `UnauthorizedSSOTokenError`, `Error loading SSO Token`,
`The SSO session associated with this profile has expired`,
`Unable to locate credentials` (and the newer `Token has expired and refresh
failed`) becomes a `400` whose message starts with **`login required:`**.
`POST /aws/accounts/{id}/test` reports the same as `{ ok: false,
login_required: true }`. The UI then offers **Sign in**, which calls
`POST /aws/accounts/{id}/login` (`aws:Edit`) → `aws sso login --profile <p>`
in a real PTY session (`provider: "aws"`, title `aws sso login · <name>`) so
the browser-based device flow works exactly as in a terminal. The UI polls
`/test` every 3 s until `ok` and closes the tab. Access-keys accounts have
nothing to sign in to — `/login` returns 400; re-enter the keys instead.

### 2.4 Permission chips

Each account card shows five chips from
`GET /aws/accounts/{id}/permissions` — six probes run in parallel (8 s each):
`sts get-caller-identity`, `s3api list-buckets`, `sqs list-queues`,
`ec2 describe-instances`, `athena list-work-groups`, `eks list-clusters`.
Per service: **allowed** (exit 0), **denied** (`AccessDenied` /
`AccessDeniedException` / `UnauthorizedOperation`), **unknown** (any other
failure, e.g. a region with no endpoint). The result is cached for **10 min**
in `permissions_json` (`?refresh=true` bypasses); a snapshot with
`login_required` is never cached. Edit-level actions are *not* probed — the
action itself surfaces the IAM denial as a 403 if it happens.

---

## 3. Walkthrough per service

Every service endpoint accepts `?region=` and falls back to the account's
region; the toolbar region switcher just sets that parameter.

### 3.1 S3 (read-only by design)

Buckets (`s3api list-buckets`) → object browser with breadcrumb prefixes
(`list-objects-v2 --delimiter /`, folders first, `token` for the next page,
the `prefix` "directory marker" object is hidden) → per-object **head**,
**preview** and **download**.

- **Preview** does a ranged `get-object` (`bytes=0-<max-1>`, default 64 KiB,
  cap 1 MiB) into a temp file under `<data_dir>/tmp` and returns `{ text,
  truncated, content_type }`. Only text-like objects are previewed: `text/*`,
  JSON / NDJSON / XML / YAML / CSV / JS / SQL types, or an `octet-stream` whose
  key has a text-looking extension (`.log`, `.json`, `.csv`, `.yaml`, …); a
  NUL byte in the sample or any other type yields `{ binary: true }`.
- **Download** streams `aws s3 cp s3://bucket/key -` straight into the HTTP
  response (`Content-Disposition: attachment`, `Content-Length` from the head,
  `Cache-Control: no-store`). The child process is killed the moment the
  client disconnects. Objects over **2 GiB** are refused (413) — use the CLI.

There is no upload, delete, or presign. Everything here is `aws_s3:View`.

### 3.2 SQS

Queue list with approximate counts → queue tabs:

- **Messages** — *Peek N* runs `receive-message --visibility-timeout 0
  --wait-time-seconds 1 --attribute-names All --message-attribute-names All`,
  so peeking does not hide messages from consumers (`aws_sqs:View`). Per-row
  **Delete message** uses the receipt handle (`aws_sqs:Edit`).
- **Send** — body + attributes; FIFO fields (`group_id`, `dedup_id`) only for
  `.fifo` queues. Bodies are capped at 256 KiB. Audited as `aws.sqs.send`.
- **Attributes** — `get-queue-attributes --attribute-names All`, plus parsed
  `approx_messages / approx_not_visible / approx_delayed` and the DLQ target
  from `RedrivePolicy`.
- **Redrive** — `start-message-move-task` from a DLQ ARN back to its source
  (or an explicit destination). Audited `aws.sqs.redrive`.
- **Purge** (⋯ menu) — typed confirmation: `confirm_name` must equal the queue
  name or the daemon refuses with 400. Audited `aws.sqs.purge`.

### 3.3 EC2

Instances table (state pill, Name tag, id, type, AZ, private/public IP,
launch time) from `describe-instances`, with a server-side `state=` filter
(`Name=instance-state-name`) and a client-side `q=` text filter over
id/name/ips/type. Row detail shows tags and the raw JSON.

**Start / Stop / Reboot** (`aws_ec2:Edit`) call `start-instances` /
`stop-instances` / `reboot-instances`. Stop and reboot require `confirm_id ==
instance_id` in the body (typed confirm in the UI). The response is
`{ previous_state, current_state }` from the CLI; reboot returns none, so both
fields carry the observed state. Audited `aws.ec2.start|stop|reboot`.

### 3.4 Athena

A three-pane workbench like the DB Explorer:

- **Catalog tree** — workgroups (`list-work-groups` + `get-work-group` for the
  output location, first 20), databases (`list-databases`), tables and columns
  (`list-table-metadata`; partition keys are appended to the columns).
- **Editor** — workgroup / database selectors, ⌘↵ **Run** (`aws_athena:Edit`)
  → `start-query-execution`. If neither the chosen workgroup nor the request
  carries an `output_location`, the daemon answers 400 with a hint *before*
  Athena's `InvalidRequestException` would. Audited `aws.athena.execute`
  (SQL clipped to 2000 chars in the audit detail).
- **Results** — the UI polls `GET …/athena/query/{qid}` every second while
  `QUEUED` / `RUNNING`; on `SUCCEEDED` the same call returns `result` in the
  **DB Explorer `QueryResult` shape** (`columns[{name, type_hint}]`, `rows`,
  `stats{duration_ms,row_count,bytes_read}`, `truncated`) so `ResultsGrid`
  renders it unchanged. Athena's header row is dropped on the first page;
  `next_token` pages further. The status bar shows scanned bytes and the
  `$5/TB` estimate. **Cancel** → `stop-query-execution` (View-level: it stops
  spending money, so it is deliberately not Edit-gated).
- **History** — `list-query-executions` (per workgroup) →
  `batch-get-query-execution`; click to reload the SQL.

### 3.5 EKS

Clusters table (`list-clusters` + `describe-cluster` fan-out, first 20) →
detail with nodegroups (`list-nodegroups` + `describe-nodegroup`: desired /
min / max, instance types, AMI type).

**Open in Kubernetes** → `POST …/eks/clusters/{name}/import-kubeconfig`
(requires `aws_eks:Edit` **and** `kubernetes:Admin`, the latter checked in the
handler because a cluster row is created). The daemon runs
`aws eks update-kubeconfig --name <c> --kubeconfig <data_dir>/kube/<new_id>.yaml
--alias <name> --region <r>` with the account's env, `chmod 600`s the file,
inserts a `k8s_clusters` row (`source: "eks"`, `aws_account_id`, params
`{eks_region, eks_cluster}`) and returns the `K8sCluster`. The kubeconfig's
exec plugin is `aws eks get-token`, so the Kubernetes console injects the same
account env when it drives that cluster. Your own `~/.kube/config` is never
touched. Audited `aws.eks.import_kubeconfig`.

---

## 4. API / contract reference

The full tables live in `docs/contracts/api.md` § "AWS console (`/aws/*`)".
Summary:

| Area | Routes |
|---|---|
| Plumbing | `GET /aws/status`, `POST /aws/install`, `GET /aws/discover`, `GET /aws/regions` |
| Accounts | `GET/POST /aws/accounts`, `GET/PATCH/DELETE /aws/accounts/{id}`, `POST …/test`, `GET …/permissions?refresh=`, `POST …/login` |
| S3 | `GET …/s3/buckets`, `GET …/s3/buckets/{bucket}/objects`, `…/object`, `…/preview`, `…/download` |
| SQS | `GET …/sqs/queues`, `GET …/sqs/queues/attributes`, `POST …/sqs/queues/peek\|send\|delete-message\|purge\|redrive` |
| EC2 | `GET …/ec2/instances`, `GET …/ec2/instances/{instance_id}`, `POST …/ec2/instances/{instance_id}/start\|stop\|reboot` |
| Athena | `GET …/athena/workgroups\|databases\|tables\|history`, `POST …/athena/query`, `GET …/athena/query/{qid}`, `POST …/athena/query/{qid}/cancel` |
| EKS | `GET …/eks/clusters`, `GET …/eks/clusters/{name}`, `POST …/eks/clusters/{name}/import-kubeconfig` |
| RDS | `GET …/rds/instances`, `GET …/rds/instances/{identifier}` (read-only) |
| CloudWatch | `GET …/metrics?namespace=&dim_name=&dim_value=&range=` — one `cloudwatch get-metric-data` per call, cached 30 s; catalog + period rules in `docs/contracts/api.md` |

Error mapping (`crates/otto-aws/src/cli.rs`):

| Situation | HTTP | Message |
|---|---|---|
| No `aws` binary | 400 `invalid` | `aws CLI not installed — open the AWS module to install it` (UI keys off `not installed`) |
| Expired / missing credentials | 400 `invalid` | starts with `login required:` (UI shows **Sign in**) |
| IAM denial | 403 `forbidden` | first stderr line, redacted |
| Any other CLI failure | 400 `invalid` | full stderr, **redacted** (`otto_core::redact` + AWS secret-key / session-token shapes), clipped to 2000 chars |
| CLI timeout (30 s default, 8 s probes) | 502 `upstream` | `aws timed out after Ns` |
| Object > 2 GiB on download | 413 `payload_too_large` | — |

WS events: `aws_account_updated { account_id, deleted }` (create / update /
delete / probe refresh) and `aws_install_updated { tool: "aws", state }` —
both global scope.

---

## 5. RBAC

Seven `Feature` keys; the server enforces them in the policy table and the UI
mirrors them by hiding / disabling controls with `auth.can(feature, cap)`.
Root always passes. Grant them in **Settings → Users → Feature grants**.

| Feature | View | Edit | Admin |
|---|---|---|---|
| `aws` | list accounts, `/status`, `/discover`, `/regions`, `/test`, `/permissions` | `/login` (spawn `aws sso login` PTY) | create / update / delete accounts, `/install` |
| `aws_s3` | everything (buckets, objects, preview, download) | — | — |
| `aws_sqs` | list, attributes, peek | send, delete-message, purge, redrive | — |
| `aws_ec2` | list / describe | start / stop / reboot | — |
| `aws_athena` | workgroups / databases / tables / history / results / cancel | execute query | — |
| `aws_eks` | list / describe clusters + nodegroups | import kubeconfig (**also** needs `kubernetes:Admin`) | — |
| `aws_rds` | list / describe DB instances, CloudWatch metrics | — (read-only by design) | — |

CloudWatch metrics (`GET …/metrics?namespace=AWS/SQS|AWS/EC2|AWS/RDS`) are
gated by `aws:View` in the policy table **plus** View on the namespace's own
key (`aws_sqs` / `aws_ec2` / `aws_rds`), checked in the handler. The IAM side
needs `cloudwatch:GetMetricData`.

`peek` and `cancel` are POSTs that mutate nothing, so they are graded back
down to View in `policy.rs` (same trick as `/db/query-plan`). Every mutation
writes an `audit_log` row: `aws.sqs.send`, `aws.sqs.delete_message`,
`aws.sqs.purge`, `aws.sqs.redrive`, `aws.ec2.start|stop|reboot`,
`aws.athena.execute`, `aws.eks.import_kubeconfig`.

---

## 6. Capabilities & limitations

- ✅ Any auth the CLI supports — SSO (IAM Identity Center), assume-role,
  MFA, `credential_process`, static keys — with zero AWS config written by Otto.
- ✅ Per-service feature grants; a role can browse S3 without seeing EC2.
- ✅ Streamed S3 downloads with disconnect-kill; text preview for logs / JSON /
  CSV / YAML.
- ✅ Athena results drop straight into the DB Explorer grid.
- ✅ One-click EKS → Kubernetes console import.
- ✅ Custom endpoints per account (`endpoint_url`) — LocalStack, VPC interface
  endpoints, S3-compatible stores — via `AWS_ENDPOINT_URL` on every call.
- ⚠️ **S3 is read-only** (no upload / delete / presign) — a product decision.
- ⚠️ Every call is a subprocess: expect ~200–600 ms per request (the CLI's
  Python start-up), and 30 s hard timeouts (8 s per permission probe).
- ⚠️ The permission probe checks *read* actions only; Edit-level denials
  surface when you act.
- ⚠️ The EKS/Athena list views fan out `describe` calls (first 20 items) —
  accounts with hundreds of clusters / workgroups show the first 20 in detail.
- ⚠️ Athena returns every cell as a string (`VarCharValue`); `type_hint`
  carries the declared type for the grid.
- ⚠️ macOS only (the installer uses `installer -pkg` / Homebrew; Keychain for
  secrets).

---

## 7. Security model

- **Secrets in the Keychain only.** `aws_accounts.secret_ref` is the opaque
  `aws-<id>` key; the JSON payload `{secret_access_key, session_token}` never
  leaves the daemon and is never serialized into `AwsAccount`. Assume-role
  temp credentials live in daemon memory only.
- **Credentials via env, never argv.** Every subprocess gets
  `AWS_ACCESS_KEY_ID/…` or `AWS_PROFILE` in its environment; `AWS_PAGER=""` and
  `AWS_CLI_AUTO_PROMPT=off` keep the CLI non-interactive. The daemon's own
  `AWS_PROFILE` is **stripped** from every child (`env_remove`, never blanked —
  `AWS_PROFILE=""` makes CLI v2 fail with `The config profile () could not be
  found`); profile-mode accounts set it explicitly.
- **No writes to `~/.aws`.** Discovery is read-only and returns names /
  regions / SSO metadata only.
- **stderr redaction** before anything reaches the client (AKIA ids,
  40-char secret keys, session tokens, PEM blocks, bearer tokens).
- **Argument validation** (bucket / key / queue URL / ARN / instance id /
  cluster name / query id shapes) happens before any subprocess spawns; every
  call is a `Command` with an argv array — no shell.
- **Typed confirmations** for purge, stop, reboot are enforced server-side.
- **Never `sudo`.** The installer only touches `$HOME` and `<data_dir>`.

---

## 8. Troubleshooting

| Symptom | Likely cause / fix |
|---|---|
| **"aws CLI not installed — open the AWS module to install it"** | No binary on the ladder. Click **Install now** (`aws:Admin`), or `brew install awscli` yourself and reopen the module. If you installed to a custom prefix, symlink it into `~/.local/bin` or set `OTTO_AWS_BIN` in the daemon environment. |
| **Install job `failed` with `installer exited with 1`** | The pkg fallback needs a writable home and network access to `awscli.amazonaws.com`; check `install.log_tail` in `/status`. Installing Homebrew first is the smoother path. |
| **Card shows "Sign in" / `login required: Error loading SSO Token`** | The SSO token for that profile expired (typically after 8–12 h). Click **Sign in** — a terminal tab runs `aws sso login --profile <p>`; finish the browser flow, the card turns green within a few seconds. |
| **`login required: Unable to locate credentials`** | Profile mode with a profile that has no credentials source, or the profile name is misspelt (compare with `GET /aws/discover`). For keys mode: the Keychain entry is missing — PATCH the account with the secret again. |
| **`login` returns 400 for an access-keys account** | Expected — there is no SSO session to refresh. Re-enter the keys (PATCH). |
| **Permission chip is `unknown`, not `denied`** | The probe failed for a non-IAM reason: region without the service, network, throttling, or timeout (8 s). Try `?refresh=true`, or switch the account region. |
| **403 on a service route while the chip says allowed** | The chip covers the *list* action only; the specific action (e.g. `sqs:SendMessage`, `ec2:StopInstances`) is denied by IAM. The 403 body carries AWS's first stderr line. |
| **403 "importing an EKS kubeconfig … requires kubernetes:Admin"** | `aws_eks:Edit` is not enough — importing creates a Kubernetes cluster entry. Ask for the `kubernetes:Admin` grant. |
| **Athena `POST /query` → 400 "workgroup 'primary' has no query result location"** | Set an output location on the workgroup in the AWS console, or pass `output_location: s3://bucket/prefix/` in the request (the editor has a field for it). |
| **Athena results `truncated: true`** | More than `max` rows (≤ 1000 per page); pass `next_token` to page, or narrow the query. |
| **S3 preview says `binary: true` for a text file** | The object's `Content-Type` is something binary (e.g. `application/zip`) or the sample contains a NUL byte. Download it instead. |
| **S3 download stops mid-way** | The client disconnected (the daemon kills `aws s3 cp` on disconnect) or the object exceeds 2 GiB (refused up front with 413). |
| **EC2 stop/reboot → 400 "confirm_id must equal the instance id"** | The typed confirmation didn't match. This is enforced server-side on purpose. |
| **Calls are slow (~0.5 s each)** | Each request is a fresh `aws` process (Python start-up). Auto-refresh lists at 10 s, not 1 s. |
| **400 "endpoint_url: … is reached over plain http"** | A custom endpoint on a non-loopback host must be `https://`. Plain `http` is only accepted for `localhost` / `127.0.0.1` / `[::1]` (LocalStack). |
| **Custom endpoint: chips `unknown`, `Could not connect to the endpoint URL`** | The endpoint is down or the port is wrong (`curl <url>/_localstack/health` for LocalStack). Athena / EKS chips stay non-green against LocalStack Community — those APIs are not emulated. |

---

## 9. Testing

Unit tests live next to the code (`cargo test -p otto-aws`): stderr
classification / redaction (`cli.rs`), env building incl. `endpoint_url`
injection and validation (`accounts.rs`), and every CLI-JSON normalizer.

**Real end-to-end with LocalStack** — `ui/e2e/desktop-aws-localstack.spec.ts`
drives the actual UI against the actual `aws` CLI against a LocalStack
container. It self-skips (with the reason) when Docker is not running, when
`aws` is not on `PATH`, or when the test daemon has no `/aws/*` routes.

```bash
# prerequisites: Docker running, `brew install awscli`, a worktree build
cargo build -p ottod
cd ui
OTTO_E2E_BIN=$PWD/../target/debug/ottod \
  npx playwright test desktop-aws-localstack desktop-aws --project=desktop-browser --reporter=line
# parallel-safe: add OTTO_E2E_SLOT=4 OTTO_E2E_PORT=7824 OTTO_E2E_PW_PORT=5184
```

What it does: starts `localstack/localstack:4.14.0` (the last Community
release that runs without a licence — `latest` exits 55 with "License
activation failed" unless `LOCALSTACK_AUTH_TOKEN` is set; override the image
with `OTTO_E2E_LOCALSTACK_IMAGE`, the token is passed through when present) as
`otto-e2e-localstack-<slot>` on a free host port (reusing a container of that
name if one is already up — then it is left running), waits for
`/_localstack/health`, seeds S3 (a folder prefix with JSON / CSV / binary
objects), SQS (a standard queue with 3 messages + a `.fifo` queue) and EC2
(one `otto-e2e` instance) with the CLI, then creates an `access_keys` account
(`test`/`test`, `endpoint_url: http://127.0.0.1:<port>`) through the API and
asserts: the card shows account `000000000000`, endpoint, green S3/SQS/EC2
chips (Athena/EKS not green); S3 browse → folder first → JSON preview →
downloads whose byte size equals the seeded objects; SQS count 3 → Peek 3 →
Send → 4 → typed Purge → 0; EC2 `running` → typed Stop → `stopped`. The
container it started is removed in `afterAll`. Note `OTTO_E2E_BIN` — without
it the harness drives the *installed* daemon, which may predate the routes.

LocalStack Community limits you will notice: Athena / EKS are not emulated
(`InternalFailure … not included within your LocalStack license` ⇒ chips
`unknown`, never green), `stop-instances` completes instantly (no `stopping`
phase), services lazy-load (`available` until first use), and the container is
started with `SQS_ENDPOINT_STRATEGY=path` + `LOCALSTACK_HOST=127.0.0.1:<port>`
so the queue URLs it returns are reachable from the host.

---

## 10. Related docs

- [`./kubernetes-console.md`](./kubernetes-console.md) — where imported EKS
  clusters land; `kubectl` / k9s over the same account credentials.
- [`./database-explorer.md`](./database-explorer.md) — the `QueryResult`
  shape and `ResultsGrid` Athena results render into.
- [`./rbac-multiuser-sharing.md`](./rbac-multiuser-sharing.md) — the full
  feature-grant table.
- [`../design/aws-k8s-consoles.md`](../design/aws-k8s-consoles.md) — the build
  contract shared by backend, UI and MCP.
- [`../contracts/api.md`](../contracts/api.md) · [`../contracts/ws.md`](../contracts/ws.md)
  — authoritative routes and events.
