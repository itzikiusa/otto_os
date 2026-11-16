//! Ephemeral local Testcontainers & in-process `miniredis` discovery (`DbWatchRegistry`).
//!
//! Discovers random-port Docker Testcontainers (`docker ps` + `docker inspect`)
//! across MySQL/MariaDB, PostgreSQL, MongoDB, Redis, and ClickHouse, plus active
//! child `127.0.0.1` Redis/`miniredis` TCP listeners of IDE test/debug runs.
//! Credentials remain strictly server-side inside [`DbWatchRegistry`] and are
//! never serialized over HTTP.

use std::collections::{BTreeSet, HashMap};
use std::process::Stdio;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use otto_core::{Error, Result};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::process::Command;

use super::{DbWatchQueryTarget, IdeDbEngine, IdeEphemeralDbTarget, IdeEphemeralSource};

const EPHEMERAL_SECRET_TTL: Duration = Duration::from_secs(600);
const DOCKER_CMD_TIMEOUT: Duration = Duration::from_millis(2_500);
const LSOF_CMD_TIMEOUT: Duration = Duration::from_millis(1_500);
const REDIS_PING_TIMEOUT: Duration = Duration::from_millis(120);

#[derive(Clone)]
struct VerifiedEphemeralEntry {
    public: IdeEphemeralDbTarget,
    user: Option<String>,
    password: Option<String>,
    recorded_at: Instant,
    pinned_for_test: bool,
}

/// Server-side registry of verified local loopback ephemeral DB targets.
#[derive(Default)]
pub struct DbWatchRegistry {
    entries: DashMap<String, VerifiedEphemeralEntry>,
}

impl DbWatchRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Discover live Docker Testcontainers and child `miniredis` listeners on `127.0.0.1`.
    pub async fn discover_local(&self, seed_pids: &[u32]) -> Vec<IdeEphemeralDbTarget> {
        self.evict_expired();
        let mut out: Vec<IdeEphemeralDbTarget> = self
            .entries
            .iter()
            .filter(|e| e.value().pinned_for_test)
            .map(|e| e.value().public.clone())
            .collect();
        let mut seen_ports: BTreeSet<u16> = out.iter().map(|t| t.port).collect();

        for found in discover_docker_testcontainers().await {
            seen_ports.insert(found.public.port);
            let pub_item = found.public.clone();
            self.entries.insert(pub_item.ephemeral_id.clone(), found);
            out.push(pub_item);
        }

        if !seed_pids.is_empty() {
            for found in discover_child_miniredis(seed_pids, &seen_ports).await {
                let pub_item = found.public.clone();
                self.entries.insert(pub_item.ephemeral_id.clone(), found);
                out.push(pub_item);
            }
        }

        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// Resolve a previously discovered `ephemeral_id` into a loopback query target.
    pub fn resolve_target(
        &self,
        ephemeral_id: &str,
        expected_engine: IdeDbEngine,
    ) -> Result<(String, DbWatchQueryTarget)> {
        self.evict_expired();
        let entry = self.entries.get(ephemeral_id).ok_or_else(|| {
            Error::NotFound(format!(
                "ephemeral target '{ephemeral_id}' is no longer running"
            ))
        })?;
        if entry.public.host != "127.0.0.1" {
            return Err(Error::Forbidden("ephemeral target must be loopback".into()));
        }
        if entry.public.engine != expected_engine {
            return Err(Error::Invalid(format!(
                "ephemeral target '{}' uses engine '{}', not '{}'",
                entry.public.name,
                entry.public.engine.as_str(),
                expected_engine.as_str()
            )));
        }
        Ok((
            entry.public.name.clone(),
            DbWatchQueryTarget::EphemeralLoopback {
                port: entry.public.port,
                user: entry.user.clone(),
                password: entry.password.clone(),
                database: entry.public.database.clone(),
            },
        ))
    }

    /// Register a verified `127.0.0.1` loopback target (used by integration tests).
    #[allow(clippy::too_many_arguments)]
    pub fn register_loopback_target(
        &self,
        ephemeral_id: impl Into<String>,
        name: impl Into<String>,
        engine: IdeDbEngine,
        port: u16,
        user: Option<String>,
        password: Option<String>,
        database: Option<String>,
    ) {
        let id = ephemeral_id.into();
        let public = IdeEphemeralDbTarget {
            target_kind: "ephemeral".into(),
            ephemeral_id: id.clone(),
            name: name.into(),
            engine,
            host: "127.0.0.1".into(),
            port,
            database,
            source: IdeEphemeralSource::Process,
            container_image: None,
        };
        self.entries.insert(
            id,
            VerifiedEphemeralEntry {
                public,
                user,
                password,
                recorded_at: Instant::now(),
                pinned_for_test: true,
            },
        );
    }

    fn evict_expired(&self) {
        let now = Instant::now();
        self.entries.retain(|_, v| {
            v.pinned_for_test || now.duration_since(v.recorded_at) <= EPHEMERAL_SECRET_TTL
        });
    }
}

async fn discover_docker_testcontainers() -> Vec<VerifiedEphemeralEntry> {
    let Ok(Ok(ps_out)) = tokio::time::timeout(
        DOCKER_CMD_TIMEOUT,
        Command::new("docker")
            .args(["ps", "--format", "{{.ID}}"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output(),
    )
    .await
    else {
        return Vec::new();
    };
    if !ps_out.status.success() {
        return Vec::new();
    }
    let ids: Vec<String> = String::from_utf8_lossy(&ps_out.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .take(24)
        .map(str::to_string)
        .collect();
    if ids.is_empty() {
        return Vec::new();
    }

    let Ok(Ok(insp_out)) = tokio::time::timeout(
        DOCKER_CMD_TIMEOUT,
        Command::new("docker")
            .arg("inspect")
            .args(&ids)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output(),
    )
    .await
    else {
        return Vec::new();
    };
    if !insp_out.status.success() {
        return Vec::new();
    }
    parse_docker_inspect_json(&String::from_utf8_lossy(&insp_out.stdout))
}

fn parse_docker_inspect_json(raw: &str) -> Vec<VerifiedEphemeralEntry> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(raw) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let cid = item.get("Id").and_then(Value::as_str).unwrap_or_default();
        let short_id: String = cid.chars().take(12).collect();
        if short_id.is_empty() {
            continue;
        }
        let image = item
            .pointer("/Config/Image")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let is_tc = item
            .pointer("/Config/Labels/org.testcontainers")
            .and_then(Value::as_str)
            == Some("true")
            || image.to_ascii_lowercase().contains("testcontainers");
        let env_map = parse_container_env(item.pointer("/Config/Env"));
        let Some(ports_obj) = item
            .pointer("/NetworkSettings/Ports")
            .and_then(Value::as_object)
        else {
            continue;
        };

        for (container_port, bindings) in ports_obj {
            let Some((engine, creds)) = classify_container_port(&image, container_port, &env_map)
            else {
                continue;
            };
            let Some(host_port) = extract_loopback_host_port(bindings) else {
                continue;
            };
            let prefix = if is_tc { "Testcontainer" } else { "Container" };
            let ephem_id = format!("tc:{short_id}:{host_port}");
            let label_img = image.split('/').next_back().unwrap_or(&image);
            out.push(VerifiedEphemeralEntry {
                public: IdeEphemeralDbTarget {
                    target_kind: "ephemeral".into(),
                    ephemeral_id: ephem_id,
                    name: format!("{prefix} {label_img} (:{host_port})"),
                    engine,
                    host: "127.0.0.1".into(),
                    port: host_port,
                    database: creds.2,
                    source: IdeEphemeralSource::Testcontainers,
                    container_image: Some(image.clone()),
                },
                user: creds.0,
                password: creds.1,
                recorded_at: Instant::now(),
                pinned_for_test: false,
            });
        }
    }
    out
}

fn parse_container_env(val: Option<&Value>) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let Some(arr) = val.and_then(Value::as_array) else {
        return map;
    };
    for item in arr {
        if let Some((k, v)) = item.as_str().and_then(|s| s.split_once('=')) {
            map.insert(k.to_string(), v.to_string());
        }
    }
    map
}

fn extract_loopback_host_port(bindings: &Value) -> Option<u16> {
    let arr = bindings.as_array()?;
    for b in arr {
        let ip = b.get("HostIp").and_then(Value::as_str).unwrap_or("0.0.0.0");
        if matches!(ip, "127.0.0.1" | "0.0.0.0" | "::1" | "::" | "") {
            if let Some(port) = b
                .get("HostPort")
                .and_then(Value::as_str)
                .and_then(|p| p.parse::<u16>().ok())
            {
                return Some(port);
            }
        }
    }
    None
}

type ContainerCreds = (Option<String>, Option<String>, Option<String>);

fn classify_container_port(
    image: &str,
    container_port: &str,
    env: &HashMap<String, String>,
) -> Option<(IdeDbEngine, ContainerCreds)> {
    let img = image.to_ascii_lowercase();
    if img.contains("ryuk") {
        return None;
    }
    match container_port {
        "3306/tcp"
            if img.contains("mysql") || img.contains("mariadb") || img.contains("percona") =>
        {
            let root_pw = env
                .get("MARIADB_ROOT_PASSWORD")
                .or_else(|| env.get("MYSQL_ROOT_PASSWORD"))
                .cloned();
            let (user, pw) = if let Some(rpw) = root_pw {
                (Some("root".into()), Some(rpw))
            } else {
                (
                    env.get("MYSQL_USER")
                        .or_else(|| env.get("MARIADB_USER"))
                        .cloned()
                        .or_else(|| Some("root".into())),
                    env.get("MYSQL_PASSWORD")
                        .or_else(|| env.get("MARIADB_PASSWORD"))
                        .cloned(),
                )
            };
            let db = env
                .get("MARIADB_DATABASE")
                .or_else(|| env.get("MYSQL_DATABASE"))
                .cloned();
            Some((IdeDbEngine::Mysql, (user, pw, db)))
        }
        "5432/tcp"
            if img.contains("postgres") || img.contains("postgis") || img.contains("timescale") =>
        {
            let user = env
                .get("POSTGRES_USER")
                .cloned()
                .or_else(|| Some("postgres".into()));
            let pw = env.get("POSTGRES_PASSWORD").cloned();
            let db = env.get("POSTGRES_DB").cloned().or_else(|| user.clone());
            Some((IdeDbEngine::Postgres, (user, pw, db)))
        }
        "27017/tcp" if img.contains("mongo") => {
            let user = env.get("MONGO_INITDB_ROOT_USERNAME").cloned();
            let pw = env.get("MONGO_INITDB_ROOT_PASSWORD").cloned();
            let db = env
                .get("MONGODB_DATABASE")
                .or_else(|| env.get("MONGO_INITDB_DATABASE"))
                .cloned();
            Some((IdeDbEngine::Mongodb, (user, pw, db)))
        }
        "8123/tcp" if img.contains("clickhouse") => {
            let user = env
                .get("CLICKHOUSE_USER")
                .cloned()
                .or_else(|| Some("default".into()));
            let pw = env.get("CLICKHOUSE_PASSWORD").cloned();
            let db = env.get("CLICKHOUSE_DB").cloned();
            Some((IdeDbEngine::Clickhouse, (user, pw, db)))
        }
        "6379/tcp"
            if img.contains("redis") || img.contains("valkey") || img.contains("dragonfly") =>
        {
            let pw = env.get("REDIS_PASSWORD").cloned();
            Some((IdeDbEngine::Redis, (None, pw, None)))
        }
        _ => None,
    }
}

async fn discover_child_miniredis(
    seed_pids: &[u32],
    already_seen_ports: &BTreeSet<u16>,
) -> Vec<VerifiedEphemeralEntry> {
    let all_pids = collect_process_tree_pids(seed_pids).await;
    if all_pids.is_empty() {
        return Vec::new();
    }
    let pid_csv = all_pids
        .iter()
        .take(32)
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let Ok(Ok(lsof_out)) = tokio::time::timeout(
        LSOF_CMD_TIMEOUT,
        Command::new("lsof")
            .args(["-Pan", "-p", &pid_csv, "-iTCP", "-sTCP:LISTEN"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output(),
    )
    .await
    else {
        return Vec::new();
    };
    let mut candidates = Vec::new();
    for line in String::from_utf8_lossy(&lsof_out.stdout).lines().skip(1) {
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 9 {
            continue;
        }
        let Ok(pid) = cols[1].parse::<u32>() else {
            continue;
        };
        let addr = cols[8];
        if let Some((_, port_str)) = addr.rsplit_once(':') {
            if let Ok(port) = port_str.parse::<u16>() {
                if !already_seen_ports.contains(&port)
                    && !candidates.iter().any(|(_, p)| *p == port)
                {
                    candidates.push((pid, port));
                }
            }
        }
    }

    let mut out = Vec::new();
    for (pid, port) in candidates.into_iter().take(8) {
        if probe_redis_ping_loopback(port).await {
            out.push(VerifiedEphemeralEntry {
                public: IdeEphemeralDbTarget {
                    target_kind: "ephemeral".into(),
                    ephemeral_id: format!("proc:{pid}:{port}"),
                    name: format!("In-process Redis / miniredis (:{port})"),
                    engine: IdeDbEngine::Redis,
                    host: "127.0.0.1".into(),
                    port,
                    database: None,
                    source: IdeEphemeralSource::Process,
                    container_image: None,
                },
                user: None,
                password: None,
                recorded_at: Instant::now(),
                pinned_for_test: false,
            });
        }
    }
    out
}

async fn collect_process_tree_pids(seeds: &[u32]) -> Vec<u32> {
    let mut all: BTreeSet<u32> = seeds.iter().copied().filter(|&p| p > 1).collect();
    let mut frontier: Vec<u32> = all.iter().copied().collect();
    for _ in 0..3 {
        if frontier.is_empty() {
            break;
        }
        let parent_csv = frontier
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let Ok(Ok(out)) = tokio::time::timeout(
            Duration::from_millis(500),
            Command::new("pgrep")
                .args(["-P", &parent_csv])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .output(),
        )
        .await
        else {
            break;
        };
        let mut next = Vec::new();
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Ok(child_pid) = line.trim().parse::<u32>() {
                if all.insert(child_pid) {
                    next.push(child_pid);
                }
            }
        }
        frontier = next;
    }
    all.into_iter().collect()
}

pub(crate) async fn probe_redis_ping_loopback(port: u16) -> bool {
    let Ok(Ok(mut stream)) =
        tokio::time::timeout(REDIS_PING_TIMEOUT, TcpStream::connect(("127.0.0.1", port))).await
    else {
        return false;
    };
    if tokio::time::timeout(
        REDIS_PING_TIMEOUT,
        stream.write_all(b"*1\r\n$4\r\nPING\r\n"),
    )
    .await
    .is_err()
    {
        return false;
    }
    let mut buf = [0u8; 16];
    let Ok(Ok(n)) = tokio::time::timeout(REDIS_PING_TIMEOUT, stream.read(&mut buf)).await else {
        return false;
    };
    n >= 5 && buf[..n].starts_with(b"+PONG")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_testcontainers_docker_inspect_across_mariadb_mongo_and_clickhouse() {
        let sample = r#"[
          {
            "Id": "abcdef1234567890",
            "Config": {
              "Image": "mariadb:11.8.9",
              "Labels": { "org.testcontainers": "true" },
              "Env": ["MARIADB_ROOT_PASSWORD=123123", "MARIADB_DATABASE=pr_bo_1"]
            },
            "NetworkSettings": {
              "Ports": {
                "3306/tcp": [{ "HostIp": "0.0.0.0", "HostPort": "55123" }]
              }
            }
          },
          {
            "Id": "9988776655443322",
            "Config": {
              "Image": "mongo:9.0",
              "Labels": { "org.testcontainers": "true" },
              "Env": ["MONGO_INITDB_ROOT_USERNAME=component", "MONGO_INITDB_ROOT_PASSWORD=123123", "MONGODB_DATABASE=promo_db"]
            },
            "NetworkSettings": {
              "Ports": {
                "27017/tcp": [{ "HostIp": "127.0.0.1", "HostPort": "55124" }]
              }
            }
          }
        ]"#;
        let parsed = parse_docker_inspect_json(sample);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].public.engine, IdeDbEngine::Mysql);
        assert_eq!(parsed[0].public.port, 55123);
        assert_eq!(parsed[0].user.as_deref(), Some("root"));
        assert_eq!(parsed[0].password.as_deref(), Some("123123"));
        assert_eq!(parsed[0].public.database.as_deref(), Some("pr_bo_1"));
        assert_eq!(parsed[1].public.engine, IdeDbEngine::Mongodb);
        assert_eq!(parsed[1].public.port, 55124);
        assert_eq!(parsed[1].user.as_deref(), Some("component"));
    }
}
