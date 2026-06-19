//! Turn a stored [`Connection`] profile (+ its keychain secret) into a
//! [`ResolvedConfig`] for a driver, plus any SSH tunnel config.
//!
//! Profile `params` (JSON) shape (all optional unless noted):
//! ```json
//! {
//!   "host": "db.internal", "port": 3306, "user": "otto", "db": "shopdb",
//!   "conn_string": "mongodb+srv://…/{secret}",        // mongo: full URI wins
//!   "tls":  { "mode": "required", "verify": true, "ca_cert": "-----BEGIN…" },
//!   "ssh":  { "host": "bastion", "port": 22, "user": "ec2-user",
//!             "identity_file": "/Users/me/.ssh/id_ed25519" },
//!   "secure": true                                     // clickhouse https / redis tls shorthand
//! }
//! ```

use otto_core::domain::Connection;
use otto_core::{Error, Result};
use serde_json::Value;

use crate::types::{Engine, ResolvedConfig, SshTunnelConfig, TlsConfig};

/// A parsed profile: the real DB endpoint config plus optional SSH tunnel.
pub struct ParsedProfile {
    pub config: ResolvedConfig,
    pub ssh: Option<SshTunnelConfig>,
}

fn opt_str(params: &Value, key: &str) -> Option<String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn opt_port(params: &Value, key: &str) -> Result<Option<u16>> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| Error::Invalid(format!("param '{key}' must be a port number"))),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) => s
            .parse::<u16>()
            .map(Some)
            .map_err(|_| Error::Invalid(format!("param '{key}' must be a port number"))),
        Some(_) => Err(Error::Invalid(format!(
            "param '{key}' must be a port number"
        ))),
    }
}

/// Parse a stored connection + its secret into a [`ParsedProfile`].
pub fn parse(conn: &Connection, secret: Option<String>) -> Result<ParsedProfile> {
    let engine = Engine::from_kind(conn.kind).ok_or_else(|| {
        Error::Invalid(format!(
            "connection kind '{}' is not a browsable database",
            conn.kind.as_str()
        ))
    })?;
    let p = &conn.params;

    let host = opt_str(p, "host").unwrap_or_default();
    let port = opt_port(p, "port")?.unwrap_or_else(|| engine.default_port());
    let user = opt_str(p, "user");
    let database = opt_str(p, "db").or_else(|| opt_str(p, "database"));

    // TLS: explicit `tls` object, else a `secure: true` shorthand → required TLS.
    let tls: TlsConfig = match p.get("tls") {
        Some(v) if !v.is_null() => serde_json::from_value(v.clone())
            .map_err(|e| Error::Invalid(format!("invalid tls config: {e}")))?,
        _ => {
            if p.get("secure").and_then(Value::as_bool).unwrap_or(false) {
                TlsConfig {
                    mode: crate::types::TlsMode::Required,
                    ..Default::default()
                }
            } else {
                TlsConfig::default()
            }
        }
    };

    let ssh: Option<SshTunnelConfig> = match p.get("ssh") {
        Some(v) if !v.is_null() => Some(
            serde_json::from_value(v.clone())
                .map_err(|e| Error::Invalid(format!("invalid ssh config: {e}")))?,
        ),
        _ => None,
    };

    let config = ResolvedConfig {
        engine,
        host,
        port,
        user,
        password: secret,
        database,
        tls,
        params: conn.params.clone(),
    };

    Ok(ParsedProfile { config, ssh })
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::{ConnectionKind, Environment};
    use serde_json::json;

    fn conn(kind: ConnectionKind, params: Value) -> Connection {
        Connection {
            id: "c1".into(),
            workspace_id: Some("w1".into()),
            name: "t".into(),
            kind,
            params,
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            created_by: "u1".into(),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn mysql_basic() {
        let c = conn(
            ConnectionKind::Mysql,
            json!({"host":"127.0.0.1","port":13306,"user":"otto","db":"shopdb"}),
        );
        let parsed = parse(&c, Some("pw".into())).unwrap();
        assert_eq!(parsed.config.engine, Engine::Mysql);
        assert_eq!(parsed.config.host, "127.0.0.1");
        assert_eq!(parsed.config.port, 13306);
        assert_eq!(parsed.config.user.as_deref(), Some("otto"));
        assert_eq!(parsed.config.password.as_deref(), Some("pw"));
        assert_eq!(parsed.config.database.as_deref(), Some("shopdb"));
        assert!(!parsed.config.tls.enabled());
        assert!(parsed.ssh.is_none());
    }

    #[test]
    fn default_port_applied() {
        let c = conn(ConnectionKind::Redis, json!({"host":"r1"}));
        let parsed = parse(&c, None).unwrap();
        assert_eq!(parsed.config.port, 6379);
    }

    #[test]
    fn port_as_string() {
        let c = conn(ConnectionKind::Clickhouse, json!({"host":"h","port":"18123"}));
        let parsed = parse(&c, None).unwrap();
        assert_eq!(parsed.config.port, 18123);
    }

    #[test]
    fn secure_shorthand_enables_tls() {
        let c = conn(ConnectionKind::Clickhouse, json!({"host":"h","secure":true}));
        let parsed = parse(&c, None).unwrap();
        assert!(parsed.config.tls.required());
    }

    #[test]
    fn tls_and_ssh_parsed() {
        let c = conn(
            ConnectionKind::Mysql,
            json!({
                "host":"db.internal","port":3306,"user":"app",
                "tls":{"mode":"required","verify":true,"ca_cert":"-----BEGIN CERT-----"},
                "ssh":{"host":"bastion","port":2222,"user":"ec2-user","identity_file":"/k"}
            }),
        );
        let parsed = parse(&c, None).unwrap();
        assert!(parsed.config.tls.required());
        assert!(parsed.config.tls.verify);
        let ssh = parsed.ssh.unwrap();
        assert_eq!(ssh.host, "bastion");
        assert_eq!(ssh.port, 2222);
        assert_eq!(ssh.user, "ec2-user");
        assert_eq!(ssh.identity_file.as_deref(), Some("/k"));
    }

    #[test]
    fn ssh_kind_rejected() {
        let c = conn(ConnectionKind::Ssh, json!({"host":"h"}));
        assert!(parse(&c, None).is_err());
    }
}
