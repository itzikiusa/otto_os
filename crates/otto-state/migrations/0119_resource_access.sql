-- Group/user authorization per connection, MCP server, AWS account, and K8s
-- cluster. Existing resources have no current-policy row and therefore remain
-- explicitly in the synthetic Legacy revision 0 state.

CREATE TABLE access_groups (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    description TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);

CREATE TABLE access_group_members (
    group_id  TEXT NOT NULL REFERENCES access_groups(id) ON DELETE CASCADE,
    user_id   TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (group_id, user_id)
);
CREATE INDEX idx_access_group_members_user ON access_group_members(user_id, group_id);

-- Roles are named presets. Policy rules copy their operation arrays at save
-- time, so editing a preset never mutates an already-assigned policy.
CREATE TABLE access_roles (
    id                        TEXT PRIMARY KEY,
    name                      TEXT NOT NULL,
    description               TEXT,
    resource_kind             TEXT NOT NULL CHECK (resource_kind IN ('connection','mcp_server','aws_account','k8s_cluster')),
    operations_json           TEXT NOT NULL,
    grantable_operations_json TEXT NOT NULL DEFAULT '[]',
    created_at                TEXT NOT NULL,
    updated_at                TEXT NOT NULL,
    UNIQUE(resource_kind, name)
);

-- One mutable head points at immutable JSON versions below. The composite key
-- prevents a resource id collision across families.
CREATE TABLE resource_access_policies (
    resource_kind TEXT NOT NULL CHECK (resource_kind IN ('connection','mcp_server','aws_account','k8s_cluster')),
    resource_id   TEXT NOT NULL,
    mode          TEXT NOT NULL CHECK (mode IN ('legacy','enforced')),
    revision      INTEGER NOT NULL CHECK (revision > 0),
    updated_at    TEXT NOT NULL,
    PRIMARY KEY (resource_kind, resource_id)
);

CREATE TABLE resource_access_policy_versions (
    resource_kind    TEXT NOT NULL,
    resource_id      TEXT NOT NULL,
    revision         INTEGER NOT NULL CHECK (revision > 0),
    policy_json      TEXT NOT NULL,
    actor_user_id    TEXT NOT NULL,
    effective_user_id TEXT,
    created_at       TEXT NOT NULL,
    PRIMARY KEY (resource_kind, resource_id, revision),
    FOREIGN KEY (resource_kind, resource_id)
        REFERENCES resource_access_policies(resource_kind, resource_id)
        ON DELETE CASCADE
);
CREATE INDEX idx_resource_access_versions_resource
    ON resource_access_policy_versions(resource_kind, resource_id, revision DESC);

-- New resources fail closed immediately, including raw/import insert paths.
-- The creator policy is filled by ResourceAccessRepo after the caller computes
-- its feature/token/delegation ceiling; until then the empty enforced policy
-- grants nothing. Existing rows predate these triggers and remain Legacy.
CREATE TRIGGER resource_access_connection_insert
AFTER INSERT ON connections
BEGIN
    INSERT INTO resource_access_policies
        (resource_kind, resource_id, mode, revision, updated_at)
    VALUES ('connection', NEW.id, 'enforced', 1, NEW.created_at);
    INSERT INTO resource_access_policy_versions
        (resource_kind, resource_id, revision, policy_json, actor_user_id,
         effective_user_id, created_at)
    VALUES (
        'connection', NEW.id, 1,
        json_object('kind', 'connection', 'resource_id', NEW.id, 'mode', 'enforced',
                    'revision', 1, 'rules', json('[]')),
        NEW.created_by, NULL, NEW.created_at
    );
END;

CREATE TRIGGER resource_access_mcp_server_insert
AFTER INSERT ON mcp_servers
BEGIN
    INSERT INTO resource_access_policies
        (resource_kind, resource_id, mode, revision, updated_at)
    VALUES ('mcp_server', NEW.id, 'enforced', 1, NEW.created_at);
    INSERT INTO resource_access_policy_versions
        (resource_kind, resource_id, revision, policy_json, actor_user_id,
         effective_user_id, created_at)
    VALUES (
        'mcp_server', NEW.id, 1,
        json_object('kind', 'mcp_server', 'resource_id', NEW.id, 'mode', 'enforced',
                    'revision', 1, 'rules', json('[]')),
        NEW.created_by, NULL, NEW.created_at
    );
END;

CREATE TRIGGER resource_access_aws_account_insert
AFTER INSERT ON aws_accounts
BEGIN
    INSERT INTO resource_access_policies
        (resource_kind, resource_id, mode, revision, updated_at)
    VALUES ('aws_account', NEW.id, 'enforced', 1, NEW.created_at);
    INSERT INTO resource_access_policy_versions
        (resource_kind, resource_id, revision, policy_json, actor_user_id,
         effective_user_id, created_at)
    VALUES (
        'aws_account', NEW.id, 1,
        json_object('kind', 'aws_account', 'resource_id', NEW.id, 'mode', 'enforced',
                    'revision', 1, 'rules', json('[]')),
        COALESCE(NEW.created_by, 'system'), NULL, NEW.created_at
    );
END;

CREATE TRIGGER resource_access_k8s_cluster_insert
AFTER INSERT ON k8s_clusters
BEGIN
    INSERT INTO resource_access_policies
        (resource_kind, resource_id, mode, revision, updated_at)
    VALUES ('k8s_cluster', NEW.id, 'enforced', 1, NEW.created_at);
    INSERT INTO resource_access_policy_versions
        (resource_kind, resource_id, revision, policy_json, actor_user_id,
         effective_user_id, created_at)
    VALUES (
        'k8s_cluster', NEW.id, 1,
        json_object('kind', 'k8s_cluster', 'resource_id', NEW.id, 'mode', 'enforced',
                    'revision', 1, 'rules', json('[]')),
        COALESCE(NEW.created_by, 'system'), NULL, NEW.created_at
    );
END;

-- Resource ids are polymorphic, so SQLite cannot express these cascades as a
-- foreign key. Explicit delete triggers remove the head and its FK-cascaded
-- immutable versions before a same-id recreation is allowed.
CREATE TRIGGER resource_access_connection_delete AFTER DELETE ON connections
BEGIN
    DELETE FROM resource_access_policies
    WHERE resource_kind = 'connection' AND resource_id = OLD.id;
END;

CREATE TRIGGER resource_access_mcp_server_delete AFTER DELETE ON mcp_servers
BEGIN
    DELETE FROM resource_access_policies
    WHERE resource_kind = 'mcp_server' AND resource_id = OLD.id;
END;

CREATE TRIGGER resource_access_aws_account_delete AFTER DELETE ON aws_accounts
BEGIN
    DELETE FROM resource_access_policies
    WHERE resource_kind = 'aws_account' AND resource_id = OLD.id;
END;

CREATE TRIGGER resource_access_k8s_cluster_delete AFTER DELETE ON k8s_clusters
BEGIN
    DELETE FROM resource_access_policies
    WHERE resource_kind = 'k8s_cluster' AND resource_id = OLD.id;
END;
