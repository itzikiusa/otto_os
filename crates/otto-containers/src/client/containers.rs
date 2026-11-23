//! Container lifecycle, Compose stack actions, logs, stats, and create validation.

use std::net::IpAddr;
use std::str::FromStr;

use futures_util::stream::{self, StreamExt};
use hyper::Method;
use otto_core::{Error, Result};
use serde_json::{json, Value};

use crate::types::{
    ContainerActionResp, ContainerDetail, ContainerLogsResp, ContainerStatsResp, ContainerSummary,
    ContainersListResp, PullImageReq, RunContainerReq, RunContainerResp, StackActionFailure,
    StackActionResp,
};

use super::logs::decode_docker_log_stream;
use super::parse::{
    build_compose_stacks, parse_container_detail, parse_container_stats, parse_container_summary,
    short_id, str_field,
};
use super::transport::{
    check_status, encode_query_component, DEFAULT_TIMEOUT, MAX_JSON_BODY, MAX_LOGS_BODY,
};
use super::{DockerClient, API_PREFIX};

const MAX_PORTS: usize = 64;
const MAX_VOLUMES: usize = 64;
const MAX_VOLUME_LEN: usize = 4096;
const MAX_ENV: usize = 256;
const MAX_ENV_LEN: usize = 8192;
const MAX_CMD_ARGS: usize = 128;
const MAX_CMD_ARG_LEN: usize = 8192;
const MAX_WORKDIR_LEN: usize = 4096;
const STACK_ACTION_CONCURRENCY: usize = 8;

impl DockerClient {
    /// List containers + compose project summaries (`GET /containers`).
    pub async fn list_containers(&self, all: bool) -> Result<ContainersListResp> {
        let path = format!(
            "{API_PREFIX}/containers/json?all={}",
            if all { "true" } else { "false" }
        );
        let raw: Vec<Value> = self.get_json(&path, true, DEFAULT_TIMEOUT).await?;
        let mut containers = Vec::with_capacity(raw.len());
        for item in &raw {
            containers.push(parse_container_summary(item));
        }
        // Running first, then newest created first.
        containers.sort_by(|a, b| {
            let a_run = a.state == "running";
            let b_run = b.state == "running";
            b_run
                .cmp(&a_run)
                .then_with(|| b.created.cmp(&a.created))
                .then_with(|| a.name.cmp(&b.name))
        });

        let stacks = build_compose_stacks(&containers);
        Ok(ContainersListResp { containers, stacks })
    }

    /// Inspect one container (`GET /containers/{id}`).
    pub async fn inspect_container(
        &self,
        id: &str,
        reveal_secrets: bool,
    ) -> Result<ContainerDetail> {
        validate_id("container id", id)?;
        let path = format!(
            "{API_PREFIX}/containers/{}/json",
            encode_query_component(id)
        );
        let v: Value = self.get_json(&path, true, DEFAULT_TIMEOUT).await?;
        Ok(parse_container_detail(&v, reveal_secrets))
    }

    /// Create and start a container (`POST /containers/run`).
    pub async fn run_container(&self, req: &RunContainerReq) -> Result<RunContainerResp> {
        let image = req.image.trim();
        validate_image_ref(image)?;
        if let Some(name) = req.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            validate_container_name(name)?;
        }

        let payload = build_create_payload(req)?;
        let create_path = match req.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(name) => format!(
                "{API_PREFIX}/containers/create?name={}",
                encode_query_component(name)
            ),
            None => format!("{API_PREFIX}/containers/create"),
        };

        let body_bytes = serde_json::to_vec(&payload)?;
        let mut resp = self
            .request(
                Method::POST,
                &create_path,
                Some(body_bytes.clone()),
                false,
                DEFAULT_TIMEOUT,
                MAX_JSON_BODY,
            )
            .await?;

        if resp.status == 404 && req.pull_if_missing {
            let _ = self
                .pull_image(&PullImageReq {
                    reference: image.to_string(),
                    platform: None,
                })
                .await?;
            resp = self
                .request(
                    Method::POST,
                    &create_path,
                    Some(body_bytes),
                    false,
                    DEFAULT_TIMEOUT,
                    MAX_JSON_BODY,
                )
                .await?;
        }
        check_status(&resp)?;

        let created: Value = serde_json::from_slice(&resp.body)
            .map_err(|e| Error::Upstream(format!("parse create response: {e}")))?;
        let id = str_field(&created, "Id")
            .ok_or_else(|| Error::Upstream("Docker create response missing Id".into()))?;
        let warnings = created
            .get("Warnings")
            .and_then(|w| w.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        let start_path = format!(
            "{API_PREFIX}/containers/{}/start",
            encode_query_component(&id)
        );
        let start_resp = self
            .request(
                Method::POST,
                &start_path,
                None,
                false,
                DEFAULT_TIMEOUT,
                MAX_JSON_BODY,
            )
            .await?;
        check_status(&start_resp)?;

        Ok(RunContainerResp {
            short_id: short_id(&id),
            id,
            warnings,
        })
    }

    /// Execute a lifecycle POST action (`start`, `stop`, `restart`, `pause`, `unpause`, `kill`).
    pub async fn container_action(
        &self,
        id: &str,
        action: &str,
        query: Option<&str>,
    ) -> Result<ContainerActionResp> {
        validate_id("container id", id)?;
        let mut path = format!(
            "{API_PREFIX}/containers/{}/{action}",
            encode_query_component(id)
        );
        if let Some(q) = query.filter(|q| !q.is_empty()) {
            path.push('?');
            path.push_str(q);
        }
        let resp = self
            .request(
                Method::POST,
                &path,
                None,
                false,
                DEFAULT_TIMEOUT,
                MAX_JSON_BODY,
            )
            .await?;
        // Docker returns 304 Not Modified when starting an already-running or
        // stopping an already-stopped container; treat that as idempotent OK.
        if resp.status == 304 {
            return Ok(ContainerActionResp {
                ok: true,
                id: id.to_string(),
                action: action.to_string(),
                message: Some("no state change needed".into()),
            });
        }
        check_status(&resp)?;
        Ok(ContainerActionResp {
            ok: true,
            id: id.to_string(),
            action: action.to_string(),
            message: None,
        })
    }

    /// Remove a container (`DELETE /containers/{id}`).
    pub async fn remove_container(
        &self,
        id: &str,
        force: bool,
        remove_volumes: bool,
    ) -> Result<ContainerActionResp> {
        validate_id("container id", id)?;
        let path = format!(
            "{API_PREFIX}/containers/{}?force={}&v={}",
            encode_query_component(id),
            force,
            remove_volumes
        );
        let resp = self
            .request(
                Method::DELETE,
                &path,
                None,
                false,
                DEFAULT_TIMEOUT,
                MAX_JSON_BODY,
            )
            .await?;
        check_status(&resp)?;
        Ok(ContainerActionResp {
            ok: true,
            id: id.to_string(),
            action: "remove".into(),
            message: None,
        })
    }

    /// Batch start/stop/restart all containers belonging to a Compose project
    /// concurrently with bounded parallelism.
    pub async fn stack_action(&self, project: &str, action: &str) -> Result<StackActionResp> {
        let project_trimmed = project.trim();
        if project_trimmed.is_empty() {
            return Err(Error::Invalid("project name cannot be empty".into()));
        }
        let list = self.list_containers(true).await?;
        let targets: Vec<ContainerSummary> = list
            .containers
            .into_iter()
            .filter(|c| c.compose_project.as_deref() == Some(project_trimmed))
            .collect();
        if targets.is_empty() {
            return Err(Error::NotFound(format!(
                "no containers found for compose stack `{project_trimmed}`"
            )));
        }
        let query = if action == "stop" || action == "restart" {
            Some("t=10")
        } else {
            None
        };
        let outcomes: Vec<(String, String, Result<ContainerActionResp>)> = stream::iter(targets)
            .map(|c| async move {
                let res = self.container_action(&c.id, action, query).await;
                (c.id, c.name, res)
            })
            .buffer_unordered(STACK_ACTION_CONCURRENCY)
            .collect()
            .await;

        let mut succeeded = Vec::new();
        let mut failed = Vec::new();
        for (id, name, res) in outcomes {
            match res {
                Ok(_) => succeeded.push(id),
                Err(err) => failed.push(StackActionFailure {
                    id,
                    name,
                    error: err.to_string(),
                }),
            }
        }
        Ok(StackActionResp {
            project: project_trimmed.to_string(),
            action: action.to_string(),
            succeeded,
            failed,
        })
    }

    /// Fetch bounded container logs (`GET /containers/{id}/logs`).
    pub async fn container_logs(
        &self,
        id: &str,
        tail: u32,
        since: Option<i64>,
        timestamps: bool,
    ) -> Result<ContainerLogsResp> {
        validate_id("container id", id)?;
        let capped_tail = tail.clamp(1, 5000);
        let mut path = format!(
            "{API_PREFIX}/containers/{}/logs?stdout=true&stderr=true&tail={capped_tail}&timestamps={timestamps}",
            encode_query_component(id)
        );
        if let Some(ts) = since {
            if ts < 0 {
                return Err(Error::Invalid(format!(
                    "since timestamp must be non-negative, got {ts}"
                )));
            }
            path.push_str(&format!("&since={ts}"));
        }
        let resp = self
            .request(
                Method::GET,
                &path,
                None,
                true,
                DEFAULT_TIMEOUT,
                MAX_LOGS_BODY,
            )
            .await?;
        check_status(&resp)?;
        let lines = decode_docker_log_stream(&resp.body, timestamps);
        Ok(ContainerLogsResp {
            container_id: id.to_string(),
            lines,
            truncated: resp.truncated,
        })
    }

    /// Fetch one-shot container CPU/mem/network/block I/O stats.
    pub async fn container_stats(&self, id: &str) -> Result<ContainerStatsResp> {
        validate_id("container id", id)?;
        let path = format!(
            "{API_PREFIX}/containers/{}/stats?stream=false",
            encode_query_component(id)
        );
        let v: Value = self.get_json(&path, true, DEFAULT_TIMEOUT).await?;
        Ok(parse_container_stats(id, &v))
    }
}

fn build_create_payload(req: &RunContainerReq) -> Result<Value> {
    if req.ports.len() > MAX_PORTS {
        return Err(Error::Invalid(format!(
            "too many port mappings (max {MAX_PORTS})"
        )));
    }
    if req.volumes.len() > MAX_VOLUMES {
        return Err(Error::Invalid(format!(
            "too many volume bindings (max {MAX_VOLUMES})"
        )));
    }
    if req.env.len() > MAX_ENV {
        return Err(Error::Invalid(format!(
            "too many environment variables (max {MAX_ENV})"
        )));
    }
    if req.cmd.len() > MAX_CMD_ARGS {
        return Err(Error::Invalid(format!(
            "too many command arguments (max {MAX_CMD_ARGS})"
        )));
    }

    let mut exposed_ports = serde_json::Map::new();
    let mut port_bindings = serde_json::Map::new();

    for p in &req.ports {
        if p.container_port == 0 {
            return Err(Error::Invalid("container_port must be >= 1".into()));
        }
        if p.host_port == Some(0) {
            return Err(Error::Invalid("host_port must be >= 1".into()));
        }
        let proto = p
            .protocol
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("tcp");
        if !matches!(proto, "tcp" | "udp" | "sctp") {
            return Err(Error::Invalid(format!("invalid port protocol: {proto}")));
        }
        let host_ip = match p
            .host_ip
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(raw_ip) => {
                IpAddr::from_str(raw_ip)
                    .map_err(|_| Error::Invalid(format!("invalid host_ip `{raw_ip}`")))?;
                raw_ip.to_string()
            }
            None => String::new(),
        };
        let key = format!("{}/{proto}", p.container_port);
        exposed_ports.insert(key.clone(), json!({}));
        let host_port = p.host_port.map(|n| n.to_string()).unwrap_or_default();
        port_bindings.insert(key, json!([{ "HostIp": host_ip, "HostPort": host_port }]));
    }

    for vol in &req.volumes {
        if vol.trim().is_empty()
            || vol.len() > MAX_VOLUME_LEN
            || vol.chars().any(|c| c.is_control())
        {
            return Err(Error::Invalid(
                "invalid volume binding (empty, too long, or contains control characters)".into(),
            ));
        }
    }
    for entry in &req.env {
        if entry.len() > MAX_ENV_LEN || entry.contains('\0') {
            return Err(Error::Invalid(
                "invalid env entry (too long or contains NUL byte)".into(),
            ));
        }
    }
    for arg in &req.cmd {
        if arg.len() > MAX_CMD_ARG_LEN || arg.contains('\0') {
            return Err(Error::Invalid(
                "invalid cmd argument (too long or contains NUL byte)".into(),
            ));
        }
    }

    let mut host_config = serde_json::Map::new();
    if !port_bindings.is_empty() {
        host_config.insert("PortBindings".into(), Value::Object(port_bindings));
    }
    if !req.volumes.is_empty() {
        host_config.insert("Binds".into(), json!(req.volumes));
    }
    if req.auto_remove {
        host_config.insert("AutoRemove".into(), json!(true));
    }
    if let Some(policy) = req
        .restart_policy
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "no")
    {
        if !matches!(policy, "always" | "unless-stopped" | "on-failure") {
            return Err(Error::Invalid(format!("invalid restart_policy `{policy}`")));
        }
        host_config.insert("RestartPolicy".into(), json!({ "Name": policy }));
    }

    let mut body = serde_json::Map::new();
    body.insert("Image".into(), json!(req.image.trim()));
    if !req.cmd.is_empty() {
        body.insert("Cmd".into(), json!(req.cmd));
    }
    if !req.env.is_empty() {
        body.insert("Env".into(), json!(req.env));
    }
    if let Some(wd) = req
        .working_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if !wd.starts_with('/') || wd.len() > MAX_WORKDIR_LEN || wd.chars().any(|c| c.is_control())
        {
            return Err(Error::Invalid(format!(
                "working_dir must be a control-character-free absolute path: `{wd}`"
            )));
        }
        body.insert("WorkingDir".into(), json!(wd));
    }
    if !exposed_ports.is_empty() {
        body.insert("ExposedPorts".into(), Value::Object(exposed_ports));
    }
    body.insert("HostConfig".into(), Value::Object(host_config));
    Ok(Value::Object(body))
}

/// Validate a container/image/network/volume identifier before embedding it in a
/// Docker API path or CLI argv.
pub fn validate_id(label: &str, value: &str) -> Result<()> {
    let s = value.trim();
    if s.is_empty() || s.len() > 256 {
        return Err(Error::Invalid(format!("{label} is empty or too long")));
    }
    if s.starts_with('-') || s.contains('/') || s.contains('\\') || s.contains("..") {
        return Err(Error::Invalid(format!("invalid {label}: `{s}`")));
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | ':'))
    {
        return Err(Error::Invalid(format!(
            "{label} contains disallowed characters"
        )));
    }
    Ok(())
}

/// Validate a container name (`[a-zA-Z0-9][a-zA-Z0-9_.-]*`).
pub fn validate_container_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 128 {
        return Err(Error::Invalid("container name is empty or too long".into()));
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphanumeric() {
        return Err(Error::Invalid(
            "container name must start with an alphanumeric character".into(),
        ));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) {
        return Err(Error::Invalid(
            "container name may only contain letters, digits, `_`, `.`, or `-`".into(),
        ));
    }
    Ok(())
}

/// Validate an image reference (`registry/repo:tag` or `@sha256:...`).
pub fn validate_image_ref(reference: &str) -> Result<()> {
    let s = reference.trim();
    if s.is_empty() || s.len() > 512 {
        return Err(Error::Invalid(
            "image reference is empty or too long".into(),
        ));
    }
    if s.starts_with('-') || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::Invalid(format!("invalid image reference: `{s}`")));
    }
    Ok(())
}
