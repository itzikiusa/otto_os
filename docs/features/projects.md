# Workspaces as shared projects

A workspace is Otto's shared project boundary. Every session already belongs to
its workspace, so no additional project selection or membership is needed.

Right-click a workspace and choose **Workspace context**, or open Settings →
Workspace context. The editor shows the selected workspace and its folder.
Save its goal, shared instructions, curated memory, decisions, document/Vault
references and artifact links alongside the existing skill and soul choices.
References are supplied as text; listing a path does not read its contents.

Saved context applies across agent providers when a session starts or restarts,
including review sessions. Running conversations retain the context with which
they were launched. Shell and connection sessions remain workspace members but
do not receive agent instructions. Preview and Materialize now use the same
workspace context as agent launch. Materializing files does not send a new turn
to an already running conversation.

Workspace Viewers can read and preview; Admins can edit. Editor/Admin roles may
materialize saved context. Saves use a context version: a stale save returns a
conflict while preserving the draft, and **Reload saved context** explicitly
replaces it with the current saved version. Switching workspaces ignores late
load/save responses from the previous workspace.

The standalone Projects navigation and New Session project picker have been
removed. Old Projects bookmarks lead to the existing workspace context editor.
Swarm projects remain execution/task groupings inside a workspace. Existing
Swarm/common-project records and explicit session memberships are preserved;
their content is not automatically copied into workspace-wide instructions.
Existing scoped project context continues as an additional session-specific
section. Compatibility project endpoints remain available for existing clients.

Context uses the existing workspace settings record, with additive JSON fields
and no new database migration. Ordinary settings edits preserve context;
machine-generated review rules update their own field without overwriting user
knowledge. Default provider homes stay stable, and named subscription accounts
retain their isolated homes.

See [Workspace context contracts](../contracts/api.md) for request schemas,
field limits and optimistic concurrency behavior.
