//! Command builders per connection kind (spec §7.2).
//!
//! Secrets are injected via env vars or placeholder substitution — never in
//! argv except `clickhouse-client`, which is flagged with `warn_argv=true`.

use otto_core::domain::{Connection, ConnectionKind, Environment};
use otto_core::{Error, Result};
use otto_pty::CommandSpec;
use serde_json::Value;

fn opt_str<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// A plain login shell — the fallback when a connection omits its host /
/// required params. We deliberately DON'T validate those: a user may want to
/// save a connection whose whole invocation lives in `first_command` (which is
/// sent to the PTY after connect). With no host we just open a login shell and
/// the first command runs there.
fn login_shell() -> CommandSpec {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "bash".to_string());
    CommandSpec {
        program: shell,
        args: vec!["-l".to_string()],
        cwd: None,
        env: vec![],
    }
}

/// Optional port: accepts a JSON number or numeric string.
fn opt_port(params: &Value, key: &str, kind: &str) -> Result<Option<u16>> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| Error::Invalid(format!("{kind}: param '{key}' must be a port number"))),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) => s
            .parse::<u16>()
            .map(Some)
            .map_err(|_| Error::Invalid(format!("{kind}: param '{key}' must be a port number"))),
        Some(_) => Err(Error::Invalid(format!(
            "{kind}: param '{key}' must be a port number"
        ))),
    }
}

/// If `jump` is set for a non-SSH kind, wrap the given local command spec to
/// run via `ssh -t [-i identity] <jump> -- <program> <args…>`.
fn maybe_wrap_ssh_tunnel(p: &Value, spec: CommandSpec, kind_name: &str) -> Result<CommandSpec> {
    let jump = match opt_str(p, "jump") {
        Some(j) => j,
        None => return Ok(spec),
    };
    // Build: ssh -t [-i identity] <jump> -- <original_program> <original_args…>
    let mut ssh_args: Vec<String> = vec!["-t".into()];
    if let Some(identity) = opt_str(p, "identity_file") {
        ssh_args.push("-i".into());
        ssh_args.push(identity.into());
    }
    ssh_args.push(jump.into());
    ssh_args.push("--".into());
    ssh_args.push(spec.program.clone());
    ssh_args.extend(spec.args);
    let _ = kind_name; // used for docs only
    Ok(CommandSpec {
        program: "ssh".into(),
        args: ssh_args,
        cwd: None,
        env: spec.env,
    })
}

/// Build the terminal command for a connection. Returns the spec plus
/// `warn_argv`: true when the secret unavoidably appears in argv
/// (clickhouse-client only).
pub fn build_command(conn: &Connection, secret: Option<&str>) -> Result<(CommandSpec, bool)> {
    let p = &conn.params;
    match conn.kind {
        ConnectionKind::Ssh => {
            let host = match opt_str(p, "host") {
                Some(h) => h,
                None => return Ok((login_shell(), false)),
            };
            let mut args = Vec::new();
            if let Some(identity) = opt_str(p, "identity_file") {
                args.push("-i".into());
                args.push(identity.into());
            }
            if let Some(port) = opt_port(p, "port", "ssh")? {
                args.push("-p".into());
                args.push(port.to_string());
            }
            if let Some(jump) = opt_str(p, "jump") {
                args.push("-J".into());
                args.push(jump.into());
            }
            let target = match opt_str(p, "user") {
                Some(user) => format!("{user}@{host}"),
                None => host.to_string(),
            };
            args.push(target);
            Ok((
                CommandSpec {
                    program: "ssh".into(),
                    args,
                    cwd: None,
                    env: vec![],
                },
                false,
            ))
        }
        ConnectionKind::Mysql => {
            let host = match opt_str(p, "host") {
                Some(h) => h,
                None => return Ok((login_shell(), false)),
            };
            let mut args = vec!["-h".to_string(), host.to_string()];
            if let Some(port) = opt_port(p, "port", "mysql")? {
                args.push("-P".into());
                args.push(port.to_string());
            }
            if let Some(user) = opt_str(p, "user") {
                args.push("-u".into());
                args.push(user.into());
            }
            if let Some(db) = opt_str(p, "db") {
                args.push(db.into());
            }
            let mut env = Vec::new();
            if let Some(pw) = secret {
                env.push(("MYSQL_PWD".to_string(), pw.to_string()));
            }
            let spec = maybe_wrap_ssh_tunnel(
                p,
                CommandSpec {
                    program: "mysql".into(),
                    args,
                    cwd: None,
                    env,
                },
                "mysql",
            )?;
            Ok((spec, false))
        }
        ConnectionKind::Redis => {
            let host = match opt_str(p, "host") {
                Some(h) => h,
                None => return Ok((login_shell(), false)),
            };
            let mut args = vec!["-h".to_string(), host.to_string()];
            if let Some(port) = opt_port(p, "port", "redis")? {
                args.push("-p".into());
                args.push(port.to_string());
            }
            if let Some(db) = opt_str(p, "db") {
                args.push("-n".into());
                args.push(db.into());
            }
            let mut env = Vec::new();
            if let Some(pw) = secret {
                env.push(("REDISCLI_AUTH".to_string(), pw.to_string()));
            }
            let spec = maybe_wrap_ssh_tunnel(
                p,
                CommandSpec {
                    program: "redis-cli".into(),
                    args,
                    cwd: None,
                    env,
                },
                "redis",
            )?;
            Ok((spec, false))
        }
        ConnectionKind::Mongodb => {
            let template = match opt_str(p, "conn_string") {
                Some(t) => t,
                None => return Ok((login_shell(), false)),
            };
            let conn_string = if template.contains("{secret}") {
                let secret = secret.ok_or_else(|| {
                    Error::Invalid(
                        "mongodb conn_string references {secret} but no secret is stored".into(),
                    )
                })?;
                template.replace("{secret}", secret)
            } else {
                template.to_string()
            };
            Ok((
                CommandSpec {
                    program: "mongosh".into(),
                    args: vec![conn_string],
                    cwd: None,
                    env: vec![],
                },
                false,
            ))
        }
        ConnectionKind::Clickhouse => {
            let host = match opt_str(p, "host") {
                Some(h) => h,
                None => return Ok((login_shell(), false)),
            };
            let mut args = vec!["-h".to_string(), host.to_string()];
            if let Some(port) = opt_port(p, "port", "clickhouse")? {
                args.push("--port".into());
                args.push(port.to_string());
            }
            if let Some(user) = opt_str(p, "user") {
                args.push("-u".into());
                args.push(user.into());
            }
            let mut warn_argv = false;
            if let Some(pw) = secret {
                // clickhouse-client has no env/stdin password channel — argv
                // is the only option; flagged so the UI shows a warning.
                args.push("--password".into());
                args.push(pw.to_string());
                warn_argv = true;
            }
            if let Some(db) = opt_str(p, "db") {
                args.push("-d".into());
                args.push(db.into());
            }
            let spec = maybe_wrap_ssh_tunnel(
                p,
                CommandSpec {
                    program: "clickhouse-client".into(),
                    args,
                    cwd: None,
                    env: vec![],
                },
                "clickhouse",
            )?;
            Ok((spec, warn_argv))
        }
        ConnectionKind::Custom => {
            let template = match opt_str(p, "command_template") {
                Some(t) => t,
                None => return Ok((login_shell(), false)),
            };
            let mut rendered = template.to_string();
            if let Some(obj) = p.as_object() {
                for (key, value) in obj {
                    if key == "command_template" {
                        continue;
                    }
                    let placeholder = format!("{{{key}}}");
                    if !rendered.contains(&placeholder) {
                        continue;
                    }
                    let replacement = match value {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => b.to_string(),
                        _ => continue,
                    };
                    rendered = rendered.replace(&placeholder, &replacement);
                }
            }
            if rendered.contains("{secret}") {
                let secret = secret.ok_or_else(|| {
                    Error::Invalid(
                        "custom command references {secret} but no secret is stored".into(),
                    )
                })?;
                rendered = rendered.replace("{secret}", secret);
            }
            let words = shell_words::split(&rendered)
                .map_err(|e| Error::Invalid(format!("custom command parse error: {e}")))?;
            let mut iter = words.into_iter();
            let program = iter
                .next()
                .ok_or_else(|| Error::Invalid("custom command is empty".into()))?;
            Ok((
                CommandSpec {
                    program,
                    args: iter.collect(),
                    cwd: None,
                    env: vec![],
                },
                false,
            ))
        }
    }
}

/// Validate that `params` are sufficient for `kind` (used at create/update).
pub fn validate_params(kind: ConnectionKind, params: &Value, has_secret: bool) -> Result<()> {
    let conn = Connection {
        id: String::new(),
        workspace_id: None,
        name: String::new(),
        kind,
        params: params.clone(),
        secret_ref: None,
        first_command: None,
        section_id: None,
        environment: Environment::Dev,
        read_only: false,
        created_by: String::new(),
        created_at: chrono::Utc::now(),
    };
    let secret = has_secret.then_some("x");
    build_command(&conn, secret).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn conn(kind: ConnectionKind, params: Value) -> Connection {
        Connection {
            id: "c1".into(),
            workspace_id: Some("w1".into()),
            name: "test".into(),
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
    fn ssh_full() {
        let c = conn(
            ConnectionKind::Ssh,
            json!({"host":"db.example.com","port":2222,"user":"deploy",
                   "identity_file":"/home/me/.ssh/id_ed25519","jump":"bastion.example.com"}),
        );
        let (spec, warn) = build_command(&c, None).unwrap();
        assert_eq!(spec.program, "ssh");
        assert_eq!(
            spec.args,
            vec![
                "-i",
                "/home/me/.ssh/id_ed25519",
                "-p",
                "2222",
                "-J",
                "bastion.example.com",
                "deploy@db.example.com"
            ]
        );
        assert!(!warn);
        assert!(spec.env.is_empty());
    }

    #[test]
    fn ssh_minimal_and_missing_host() {
        let c = conn(ConnectionKind::Ssh, json!({"host":"h1"}));
        let (spec, _) = build_command(&c, None).unwrap();
        assert_eq!(spec.args, vec!["h1"]);

        // No host: we don't validate — fall back to a login shell so a
        // user-supplied first_command can run there.
        let shell = conn(ConnectionKind::Ssh, json!({"user":"me"}));
        let (spec, _) = build_command(&shell, None).unwrap();
        assert_eq!(spec.args, vec!["-l"], "missing host → login shell");
    }

    #[test]
    fn mysql_password_via_env() {
        let c = conn(
            ConnectionKind::Mysql,
            json!({"host":"127.0.0.1","port":"3306","user":"root","db":"app_db"}),
        );
        let (spec, warn) = build_command(&c, Some("s3cret")).unwrap();
        assert_eq!(spec.program, "mysql");
        assert_eq!(
            spec.args,
            vec!["-h", "127.0.0.1", "-P", "3306", "-u", "root", "app_db"]
        );
        assert_eq!(
            spec.env,
            vec![("MYSQL_PWD".to_string(), "s3cret".to_string())]
        );
        assert!(!warn);
        assert!(!spec.args.iter().any(|a| a.contains("s3cret")));
    }

    #[test]
    fn mysql_missing_host() {
        let c = conn(ConnectionKind::Mysql, json!({"user":"root"}));
        let (spec, _) = build_command(&c, None).unwrap();
        assert_eq!(spec.args, vec!["-l"], "missing host → login shell");
    }

    #[test]
    fn redis_password_via_env() {
        let c = conn(
            ConnectionKind::Redis,
            json!({"host":"r1","port":6380,"db":"2"}),
        );
        let (spec, warn) = build_command(&c, Some("pw")).unwrap();
        assert_eq!(spec.program, "redis-cli");
        assert_eq!(spec.args, vec!["-h", "r1", "-p", "6380", "-n", "2"]);
        assert_eq!(
            spec.env,
            vec![("REDISCLI_AUTH".to_string(), "pw".to_string())]
        );
        assert!(!warn);
    }

    #[test]
    fn redis_missing_host() {
        let c = conn(ConnectionKind::Redis, json!({}));
        let (spec, _) = build_command(&c, None).unwrap();
        assert_eq!(spec.args, vec!["-l"], "missing host → login shell");
    }

    #[test]
    fn mongodb_secret_substitution() {
        let c = conn(
            ConnectionKind::Mongodb,
            json!({"conn_string":"mongodb://app:{secret}@m1:27017/db"}),
        );
        let (spec, warn) = build_command(&c, Some("pw")).unwrap();
        assert_eq!(spec.program, "mongosh");
        assert_eq!(spec.args, vec!["mongodb://app:pw@m1:27017/db"]);
        assert!(!warn);
    }

    #[test]
    fn mongodb_secret_placeholder_without_secret_fails() {
        let c = conn(
            ConnectionKind::Mongodb,
            json!({"conn_string":"mongodb://app:{secret}@m1/db"}),
        );
        assert!(matches!(build_command(&c, None), Err(Error::Invalid(_))));
    }

    #[test]
    fn mongodb_missing_conn_string() {
        let c = conn(ConnectionKind::Mongodb, json!({"host":"m1"}));
        let (spec, _) = build_command(&c, None).unwrap();
        assert_eq!(spec.args, vec!["-l"], "missing conn_string → login shell");
    }

    #[test]
    fn clickhouse_password_in_argv_warns() {
        let c = conn(
            ConnectionKind::Clickhouse,
            json!({"host":"ch1","port":9000,"user":"default","db":"analytics"}),
        );
        let (spec, warn) = build_command(&c, Some("pw")).unwrap();
        assert_eq!(spec.program, "clickhouse-client");
        assert_eq!(
            spec.args,
            vec![
                "-h",
                "ch1",
                "--port",
                "9000",
                "-u",
                "default",
                "--password",
                "pw",
                "-d",
                "analytics"
            ]
        );
        assert!(warn);
    }

    #[test]
    fn clickhouse_without_secret_does_not_warn() {
        let c = conn(ConnectionKind::Clickhouse, json!({"host":"ch1"}));
        let (spec, warn) = build_command(&c, None).unwrap();
        assert_eq!(spec.args, vec!["-h", "ch1"]);
        assert!(!warn);
    }

    #[test]
    fn custom_placeholders_and_secret() {
        let c = conn(
            ConnectionKind::Custom,
            json!({"command_template":"psql -h {host} -p {port} -U {user} --password={secret}",
                   "host":"pg1","port":5432,"user":"admin"}),
        );
        let (spec, warn) = build_command(&c, Some("pw")).unwrap();
        assert_eq!(spec.program, "psql");
        assert_eq!(
            spec.args,
            vec!["-h", "pg1", "-p", "5432", "-U", "admin", "--password=pw"]
        );
        assert!(!warn);
    }

    #[test]
    fn custom_quoted_words() {
        let c = conn(
            ConnectionKind::Custom,
            json!({"command_template":"kubectl exec -it pod -- sh -c 'tail -f /var/log/app.log'"}),
        );
        let (spec, _) = build_command(&c, None).unwrap();
        assert_eq!(spec.program, "kubectl");
        assert_eq!(spec.args.last().unwrap(), "tail -f /var/log/app.log");
    }

    #[test]
    fn custom_missing_template_or_secret() {
        // No template at all → login shell (no validation).
        let c = conn(ConnectionKind::Custom, json!({}));
        let (spec, _) = build_command(&c, None).unwrap();
        assert_eq!(spec.args, vec!["-l"]);

        // A {secret} reference with no stored secret is still a real error.
        let c = conn(
            ConnectionKind::Custom,
            json!({"command_template":"x {secret}"}),
        );
        assert!(matches!(build_command(&c, None), Err(Error::Invalid(_))));

        let c = conn(ConnectionKind::Custom, json!({"command_template":"   "}));
        assert!(matches!(build_command(&c, None), Err(Error::Invalid(_))));
    }

    #[test]
    fn mysql_tunneled_via_jump() {
        // When `jump` is present on a mysql connection the command should be
        // wrapped: `ssh -t [-i identity] <jump> -- mysql <mysql-args…>`
        let c = conn(
            ConnectionKind::Mysql,
            json!({
                "host": "db.internal",
                "port": 3306,
                "user": "root",
                "db": "mydb",
                "jump": "bastion.example.com",
                "identity_file": "/home/me/.ssh/id_rsa"
            }),
        );
        let (spec, warn) = build_command(&c, Some("s3cret")).unwrap();
        assert_eq!(spec.program, "ssh");
        assert_eq!(
            spec.args,
            vec![
                "-t",
                "-i",
                "/home/me/.ssh/id_rsa",
                "bastion.example.com",
                "--",
                "mysql",
                "-h",
                "db.internal",
                "-P",
                "3306",
                "-u",
                "root",
                "mydb",
            ]
        );
        // Password is still passed via env, not argv
        assert_eq!(
            spec.env,
            vec![("MYSQL_PWD".to_string(), "s3cret".to_string())]
        );
        assert!(!warn);
    }

    #[test]
    fn mysql_no_jump_unchanged() {
        // Without jump, mysql stays as a local client call (no ssh wrapping).
        let c = conn(
            ConnectionKind::Mysql,
            json!({"host": "127.0.0.1", "user": "root"}),
        );
        let (spec, _) = build_command(&c, None).unwrap();
        assert_eq!(spec.program, "mysql");
    }

    #[test]
    fn redis_tunneled_no_identity() {
        // Tunnel without identity file — ssh args must not include -i.
        let c = conn(
            ConnectionKind::Redis,
            json!({"host": "cache.internal", "port": 6379, "jump": "bastion.example.com"}),
        );
        let (spec, warn) = build_command(&c, None).unwrap();
        assert_eq!(spec.program, "ssh");
        assert_eq!(
            spec.args,
            vec![
                "-t",
                "bastion.example.com",
                "--",
                "redis-cli",
                "-h",
                "cache.internal",
                "-p",
                "6379"
            ]
        );
        assert!(!warn);
    }
}
