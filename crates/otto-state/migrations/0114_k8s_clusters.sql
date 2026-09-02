-- Feature: Kubernetes console — the cluster (kubeconfig context) registry.
-- Conventions: TEXT ULID ids, RFC3339 TEXT timestamps, `*_json` TEXT blobs,
-- INTEGER booleans.
--
-- A "cluster" row pins one (kubeconfig file, context name) pair plus UI
-- defaults. Three sources:
--   kubeconfig — an existing context from ~/.kube/config / $KUBECONFIG.
--                `kubeconfig_path` may be NULL meaning "the user's default".
--   imported   — YAML pasted/uploaded in the UI; the daemon writes it 0600 to
--                <data_dir>/kube/<id>.yaml and stores that path here.
--   eks        — created by the AWS module (`aws eks update-kubeconfig` into
--                <data_dir>/kube/<id>.yaml); `aws_account_id` links back so
--                the token exec-plugin can be refreshed with that account's
--                credentials.
-- Every kubectl invocation is `kubectl --kubeconfig <path> --context <ctx>`,
-- so Otto never mutates the user's current-context.
CREATE TABLE IF NOT EXISTS k8s_clusters (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL,
    source            TEXT NOT NULL CHECK (source IN ('kubeconfig', 'imported', 'eks')),
    kubeconfig_path   TEXT,
    context_name      TEXT NOT NULL,
    default_namespace TEXT,
    aws_account_id    TEXT REFERENCES aws_accounts(id) ON DELETE SET NULL,
    -- {"eks_region": "...", "eks_cluster": "...", "color": "..."}
    params_json       TEXT NOT NULL DEFAULT '{}',
    -- Cached capability probe: {"metrics_server": true, "argo_rollouts": false,
    -- "argocd": true, "server_version": "v1.30.2", "checked_at": "..."}
    capabilities_json TEXT,
    environment       TEXT NOT NULL DEFAULT 'dev' CHECK (environment IN ('dev', 'staging', 'prod')),
    created_by        TEXT REFERENCES users(id) ON DELETE SET NULL,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    last_used_at      TEXT
);
CREATE INDEX IF NOT EXISTS idx_k8s_clusters_name ON k8s_clusters(name);
