---
id: aws
title: AWS
group: Infrastructure
route: aws
summary: Browse and operate S3, SQS, EC2, Athena, EKS and RDS for each saved AWS account, using the aws CLI and the credentials you already have.
---
## What it's for

The AWS page lets you work with AWS from inside Otto without switching to the web console. You save one entry per **account**. Otto then runs the official `aws` CLI v2 for you, so anything that works in your terminal also works here: SSO (IAM Identity Center), assume-role chains, MFA and `credential_process`.

Each account can open six services: **S3** (read-only browsing and downloads), **SQS**, **EC2**, **Athena**, **EKS** and **RDS** (read-only). EC2, SQS and RDS also show CloudWatch metrics.

## Getting started

1. Open **AWS** in the sidebar (Infrastructure group).
2. If the `aws` CLI isn't installed, the page shows an install panel. Choose **Install now** to let Otto install it: it uses Homebrew when available, otherwise it installs the official package into your home folder. Otto never uses `sudo`. This needs the AWS Admin grant. Choose **Check again** if you installed the CLI yourself.
3. Choose **Add account**. The wizard has 3 steps:
   - **1 · Credentials** — pick **Use an existing AWS profile** (the list comes from `~/.aws/config` and `~/.aws/credentials`; Otto reads profile names only) or **Enter access keys** (access key ID, secret, optional session token, optional assume-role ARN). Choose a region.
   - **2 · Details** — a name, an environment (`dev`, `staging` or `prod`) and a colour.
   - **3 · Test** — Otto runs `sts get-caller-identity` and shows the account and ARN. If an SSO profile needs a fresh login, choose **Sign in now**.
4. Back on the overview, each account card shows its identity, auth mode, region and a permission chip per service. Select a chip, or **Open**, to go to that service.
5. Use the left rail to switch between accounts and services.

## Everything it can do

**Accounts**
- Two credential sources:
  - **Profile**: Otto stores only the profile name. SSO, role and MFA are handled by the CLI.
  - **Access keys**: the secret key and session token go to the macOS Keychain. The database row stores only the key ID. With an assume-role ARN, Otto calls `sts assume-role` and caches the temporary credentials in memory.
- Optional **Endpoint URL** (under **Advanced**) for LocalStack (`http://localhost:4566`), VPC interface endpoints or S3-compatible stores. Plain `http` is accepted for localhost only.
- Environment pill on every card and header. `prod` accounts get the red treatment, and destructive actions ask you to type a confirmation.
- Per-service **permission chips**: allowed, denied (IAM `AccessDenied`) or unknown. Select the refresh icon, or choose **Re-check permissions** in the ⋯ menu, to probe again. Results are cached for 10 minutes.
- **Sign in (aws sso login)** for profile accounts whose token expired. It opens a terminal sheet that runs `aws sso login`, and the card turns green once the login works. Access-key accounts have nothing to sign in to: update the keys with **Edit…** instead.
- The account ⋯ menu also has **Manage access…** (per-account access rules), **Edit…** and **Delete**. Deleting removes the Keychain secret. Kubernetes clusters imported from the account keep working.
- **Filter accounts…** appears in the header once you have more than 3 accounts.

**Every service view**
- A region switcher, a filter box, **Refresh** and a 10-second auto-refresh toggle.
- Right-click (or the ⋯ button) on a row for copy actions and row operations.
- A details drawer with tabs. On phones it opens as a sheet.

**S3 (read-only)**
- Browse buckets, then prefixes as folders (folders first), with breadcrumbs. The current bucket and prefix are in the URL, so you can bookmark or share the link.
- **Preview** text objects (up to the first 64 KiB): text, pretty-printed JSON, or CSV as a table.
- **Download** streams the object with a progress bar and **Cancel**. One download runs at a time, and objects over 2 GiB are refused.
- **Copy key** and **Copy S3 URI**.
- There is no upload, delete or presigned URL.

**SQS**
- Queue list with approximate available, in-flight and delayed counts, plus FIFO and dead-letter-queue badges.
- **Messages**: **Peek** N messages without hiding them from consumers (visibility timeout 0). Bodies show as pretty JSON. You can copy a body or delete a single message.
- **Send**: body, delay, string message attributes, and group ID / dedup ID for `.fifo` queues.
- **Attributes**: every queue attribute and the DLQ target.
- **Metrics**: CloudWatch charts.
- **Redrive**: move messages from a DLQ back to its source queue, or to a destination ARN you choose.
- **Purge queue…** (⋯ menu): you must type the queue name. SQS then empties the queue over about 60 seconds.

**EC2**
- Instances table with state, name, instance ID, type, private and public IP, and launch time. Filter by state or by text.
- **Start**, **Reboot** and **Stop**. Start asks for confirmation; stop and reboot ask you to type the instance ID.
- Drawer tabs: **Overview** (fields and tags), **Metrics** and **Raw JSON**.

**Athena**
- Catalog tree of databases, tables and columns, with **Filter catalog…**. It also feeds SQL autocompletion. The table menu offers `SELECT * … LIMIT 100`, **Insert name**, **Copy qualified name** and `DESCRIBE`.
- Editor with **Workgroup** and **Database** selectors. **Run** runs the selection when you have one. Workgroups without a result location are marked "(no output location)".
- A status bar with the query state, bytes scanned, a $5/TB cost estimate, **Cancel**, and a button to copy the execution ID.
- **Results** open in the same grid as the Database Explorer. **History** lists recent executions; use **Load into editor**, **Open result**, **Copy SQL** or **Copy execution id**.

**EKS**
- Clusters table (status, version, endpoint, created) and a detail view with managed node groups (desired/min/max, instance types, AMI) and the raw JSON.
- **Open in Kubernetes…** imports the cluster into the [Kubernetes](#/walkthroughs/kubernetes) console. Otto runs `aws eks update-kubeconfig` into its own kubeconfig file (never `~/.kube/config`) and links the cluster to this account.
- **Copy ARN** and **Copy endpoint**.

**RDS (read-only)**
- DB instances with status, engine, class, Multi-AZ, storage, endpoint and created date.
- Drawer tabs: **Overview** (including DB name, master user, public flag and tags), **Metrics** and **Raw JSON**. Start, stop and reboot aren't available.

**CloudWatch metrics** (SQS, EC2, RDS)
- Ranges: 1h, 6h, 24h, 7d and 30d. The chart refreshes automatically every 60 seconds while the tab is open.
- Grouped cards, such as queue depth, age of oldest message, CPU, network, disk, IOPS, latency, connections and free storage. Each card shows current, min, max and sum or average.

**Agents (Otto MCP tools)**
- Agents can call `aws_list_accounts`, `aws_s3_list_buckets`, `aws_s3_list_objects`, `aws_s3_preview`, `aws_sqs_list_queues`, `aws_sqs_peek`, `aws_ec2_list_instances`, `aws_athena_list_tables`, `aws_athena_get_query` and `aws_eks_list_clusters`. All of these are read-only.
- `aws_sqs_send` and `aws_athena_query` change state or cost money, so they need approval. Agents can't start, stop or reboot EC2 instances.

## Keyboard shortcuts

| Keys | Action |
|---|---|
| `/` | Focus the filter box of the current service view |
| ⌘↵ | Athena: run the query, or only the selection when there is one |
| ↵ | Open the focused row (bucket, folder, object, queue, instance, cluster, history entry) |
| Esc | Close the details drawer (when you aren't typing in a field) |
| ← / → | Switch drawer tabs while a tab is focused |

App-wide shortcuts are listed in [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts).

## Tips and limits

- **Access is layered.** The page appears when you hold any of the AWS feature grants: `aws` (accounts), `aws_s3`, `aws_sqs`, `aws_ec2`, `aws_athena`, `aws_eks` or `aws_rds`. Each service needs its own View grant.
  - Sending, deleting messages, purging and redriving need SQS Edit.
  - Starting, stopping and rebooting instances need EC2 Edit.
  - Running a query needs Athena Edit. Cancelling a query needs only View, because it stops spending.
  - Per-account access rules (**Manage access…**) can narrow this further, down to specific S3 buckets.
- **Adding accounts is for the Otto owner (root).** Only the owner changes native credentials. Delegated administrators can edit the name and colour.
- **Importing an EKS cluster** needs EKS Edit **and** Kubernetes Admin, because it creates a Kubernetes cluster entry.
- **Chips cover list permissions only.** A green chip doesn't guarantee that `sqs:SendMessage` or `ec2:StopInstances` is allowed. An IAM denial appears when you act.
- **"Credentials expired or missing"** on a card usually means the SSO token expired (typically after 8–12 hours). Choose **Sign in**.
- **Athena needs a result location.** Set an output location on the workgroup in AWS. The editor doesn't let you enter one.
- **Every call is a CLI process**, so expect about 0.2–0.6 seconds per request. Requests time out after 30 seconds, and permission probes after 8 seconds. EKS and Athena workgroup details are fetched for the first 20 items only.
- **Metrics need `cloudwatch:GetMetricData`** in IAM.
- **Otto never writes `~/.aws`** and never puts a credential on a command line. Credentials are passed through the environment, and error text is redacted before you see it.
- Every change is recorded in the audit log: SQS send, delete, purge and redrive; EC2 start, stop and reboot; Athena query runs; and EKS imports.

## Related

- [Kubernetes](#/walkthroughs/kubernetes)
- [Database Explorer](#/walkthroughs/database)
- [MCP Control Plane](#/walkthroughs/mcp)
- [Settings](#/walkthroughs/settings)
- [Keyboard shortcuts](#/walkthroughs/keyboard-shortcuts)
