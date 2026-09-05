# Resource access and reviewed database changes

Otto intersects three checks for a governed operation: the user's feature/page grant, access to the requested resource and child, and the operation on that child. Workspace membership remains an additional check for workspace resources. Granting a page does not grant its databases, MCP servers, AWS accounts or Kubernetes clusters.

## Set up access

1. As the owner, open Settings → Access groups. Create groups, add members, and optionally create named operation presets. Presets are copied into rules; editing a preset does not silently change existing grants.
2. Open the Access editor for a connection, MCP server, AWS account or Kubernetes cluster. Add group rules and individual user rules. Include `discover` for each visible scope, then select the permitted operations.
3. Specify exact child names when access is restricted. Blank/all children includes future children. An explicit deny wins over grants from any group or user, including direct user grants. Denying a user an operation therefore reduces inherited group rights.
4. Preview the selected user's effective access at the parent and child levels. For an existing resource in Legacy mode, review the before/after preview before enabling enforcement. Save uses the policy revision and rejects stale edits.

New resources start enforced and private to their creator. Existing resources retain legacy behavior until their owner explicitly activates enforcement; this avoids changing access silently during the SQLite upgrade. Root bypasses resource rules but still follows reviewed production-change requirements. A limited administrator is an ordinary user with delegated operations, not root.

Only the owner manages groups and attaches or changes native credentials. A delegated resource administrator can manage permitted operations within an explicit grantable ceiling. They cannot expand their own rights, remove existing denies, change enforcement mode or assign execution credentials. Native profile, context, command, credential and environment edits are owner-only; cosmetic configuration can be delegated.

## Resource scopes and operations

| Resource | Exact child scope | Independently controlled operations |
|---|---|---|
| Database connection | Database/schema name | Browse, query, export, data changes, schema changes, submit/approve/execute reviewed changes |
| SSH connection | Connection itself | Shell, SFTP read, SFTP write |
| MCP server | Tool name | Discover, invoke, approve, configure |
| AWS account | `bucket:bucket-name` for S3 | Bucket listing, object read/write/delete, bucket administration; EC2, SQS, Athena, EKS, RDS and metrics operations at account scope |
| Kubernetes cluster | `namespace:namespace-name` | Workloads, other resources, secrets, logs, metrics, exec, apply, scale, restart and delete |

For example, give the administrators group discovery and workloads access on clusters X and Y, and the operators group the same rights only on Y. Grant logs on Y separately. An individual deny for `exec` prevents that user executing even if another group allows it. A metrics-only user needs discovery plus metrics, not deployment access.

Namespace-restricted callers cannot request all namespaces or cluster-scoped resources. Secrets have their own operation. k9s exposes a broad interactive client, so it requires unrestricted cluster scope and the full relevant operation set; use the separate namespace-scoped views for limited users. S3 bucket-scoped rules do not grant access to other AWS services.

Lists, direct URLs, HTTP calls and Otto's outward MCP self-calls enforce the same resource checks. Hidden resources return 404. Non-configurers receive redacted connection/credential metadata. Attached resource terminals and protected streams recheck access; cached UI results clear when the effective permissions or signed-in identity changes.

## Database credentials and reviewed changes

For nonroot governed MySQL/PostgreSQL execution, provision a restricted native database credential and select its connection profile in the access rule. Otto checks the actual native privilege ceiling; an application rule cannot make an overly privileged database account safe. Ambiguous native roles, global/delegation privileges, routines and other authority that cannot be proven are refused with `native_scope_required`. The owner must fix the profile rather than broaden application rules just to dismiss the error. Engines without native scope verification reject restricted execution instead of falling back to a privileged shared account.

Governed direct SQL uses a deliberately restricted syntax. Read execution uses native read-only transactions, including owner reads, to prevent functions, operators or views from hiding writes. Production data/schema changes require the reviewed change workflow; `confirm_write` does not bypass it. Read-only connections cannot execute changes.

In Database Explorer → Changes:

1. Create a draft with a title, SQL script and target connection/database.
2. Select an eligible executor and validate. Validation checks syntax, targets, permissions and credentials without running the script.
3. Submit the immutable artifact for review. A different person approves or rejects it. Impersonation cannot turn self-approval into independent approval.
4. The selected executor executes the approved hash. Script, targets, credentials, policy and approval are checked again before dispatch. Changed inputs require validation and approval again.
5. Inspect per-target attempts and the audit history. Cancellation requests stop further dispatch. An uncertain attempt retains a target lock and requires explicit reconciliation; Otto never retries an uncertain migration automatically.

Reviewed execution currently supports MySQL and PostgreSQL. It is not an automatic rollback engine: DDL may commit before a later statement fails. A failed later statement is recorded as an unknown outcome with completed-statement progress, not success. Restart recovery likewise marks interrupted execution unknown and preserves locks. A reconciled partial outcome releases the lock; inspect it and prepare a separate compensating change when needed.

## Legacy MCP activation

Enforced MCP servers run through the governed gateway and are excluded from direct agent configuration. Enable Otto MCP for the workspace and discover the registered server’s tools in the MCP control plane before invoking them from an agent. Stop active workspace sessions before switching a legacy direct server to enforcement. Otto retires its tracked direct launcher entries while preserving unrelated configuration. Untracked direct entries must be removed explicitly before activation. Previously exported credentials or clients outside Otto must be retired or rotated at their source; changing an Otto policy cannot revoke an external copy of a secret.

## Boundaries and troubleshooting

DB Assistant launches a host coding agent and therefore also requires Agents Edit. Its query helper still enforces connection/database permissions; reading a database alone does not grant access to run an agent. Assist sessions cannot be resumed by a different user or switched to a different child scope.

Revocation prevents subsequent mediated actions and closes denied attached terminals; it cannot undo an already completed action or guarantee immediate termination of an unattached native process.

These controls govern mediated Otto operations. AWS IAM, Kubernetes RBAC and database privileges remain additional native restrictions. They do not isolate an unrestricted host shell or an independently authorized coding agent from the daemon host's ambient credentials.

- Missing resource: check the feature grant, workspace membership and discovery rule for the exact child.
- Visible resource but unavailable action: inspect effective operations and applicable denies.
- `native_scope_required`: provision or select an appropriately restricted native profile; do not use owner credentials for limited users.
- Stale preview/approval: reload, validate and preview the current policy or change again.
- Unknown migration outcome: inspect the target database, then reconcile the recorded attempt. Do not submit the same script as a new change to evade the lock.

The authoritative HTTP shapes are in [API contracts](../contracts/api.md), in the resource-access and database-change sections. Persistence upgrades are append-only migrations 0119 and 0120.
