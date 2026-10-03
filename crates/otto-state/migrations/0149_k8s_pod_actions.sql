-- Kubernetes pod HTTP actions (K-3): saved per-workload HTTP requests the
-- console runs against a workload's pods through the API-server pod proxy
-- (e.g. Spring Boot actuator `POST /actuator/loggers/{{logger}}`). Template
-- variables in `path` / `body_template` are filled client-side at run time;
-- the row stores them raw. `headers_json` is a JSON object of non-secret
-- request headers. Rows go with their cluster.
CREATE TABLE k8s_pod_actions (
    id            TEXT PRIMARY KEY,
    cluster_id    TEXT NOT NULL REFERENCES k8s_clusters(id) ON DELETE CASCADE,
    namespace     TEXT NOT NULL,
    workload_kind TEXT NOT NULL,
    workload      TEXT NOT NULL,
    name          TEXT NOT NULL,
    method        TEXT NOT NULL,
    port          INTEGER NOT NULL,
    path          TEXT NOT NULL,
    headers_json  TEXT NOT NULL DEFAULT '{}',
    body_template TEXT,
    created_by    TEXT,
    updated_at    TEXT NOT NULL
);

CREATE INDEX idx_k8s_pod_actions_workload
    ON k8s_pod_actions (cluster_id, namespace, workload_kind, workload);
