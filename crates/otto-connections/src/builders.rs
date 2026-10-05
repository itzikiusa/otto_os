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

/// Refuse a profile value that `ssh` could read as an OPTION rather than a
/// host / user: a leading `-`, or whitespace / control characters that would
/// split or smuggle an argument. Profiles can be imported from third-party
/// connection files, so these values are not trusted just for being saved.
pub(crate) fn reject_option_like(kind: &str, key: &str, value: &str) -> Result<()> {
    if value.starts_with('-') || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(Error::Invalid(format!(
            "{kind}: param '{key}' must not start with '-' or contain whitespace"
        )));
    }
    Ok(())
}

/// True when a DB-kind profile runs its client ON the jump host
/// (`params.jump`, see [`maybe_wrap_ssh_tunnel`]).
fn runs_on_jump_host(p: &Value) -> bool {
    opt_str(p, "jump").is_some()
}

/// If `jump` is set for a non-SSH kind, wrap the given local command spec to
/// run via `ssh -t [-i identity] <jump> -- <program> <args…>`.
///
/// ssh hands the remote command to the jump host's login shell as ONE string
/// (its argv joined by spaces), so every word is shell-quoted here: a database
/// name, user or (ClickHouse) password containing a space, `;`, `$` or a quote
/// would otherwise be split or interpreted by the remote shell.
///
/// The spec's environment is dropped: ssh never forwards it, so a password in
/// `MYSQL_PWD` / `PGPASSWORD` / `REDISCLI_AUTH` would only sit in the local ssh
/// process without ever reaching the client. Builders ask the remote client
/// to prompt instead (see the MySQL arm).
fn maybe_wrap_ssh_tunnel(p: &Value, spec: CommandSpec, kind_name: &str) -> Result<CommandSpec> {
    let jump = match opt_str(p, "jump") {
        Some(j) => j,
        None => return Ok(spec),
    };
    reject_option_like(kind_name, "jump", jump)?;
    // Build: ssh -t -o StrictHostKeyChecking=accept-new [-i identity] <jump>
    //        -- <original_program> <original_args…>
    // `accept-new` trusts a first-time bastion on first connect (recording its
    // key in known_hosts) while still rejecting a changed key — so a brand-new
    // jump host doesn't block the DB terminal on host-key verification.
    let mut ssh_args: Vec<String> = vec![
        "-t".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
    ];
    ssh_args.extend(keepalive_opts(user_ssh_config().as_deref()));
    if let Some(identity) = opt_str(p, "identity_file") {
        ssh_args.push("-i".into());
        ssh_args.push(identity.into());
    }
    ssh_args.push(jump.into());
    ssh_args.push("--".into());
    ssh_args.push(shell_words::quote(&spec.program).into_owned());
    ssh_args.extend(spec.args.iter().map(|a| shell_words::quote(a).into_owned()));
    let _ = kind_name; // used for docs only
    Ok(CommandSpec {
        program: "ssh".into(),
        args: ssh_args,
        cwd: None,
        env: vec![],
    })
}

/// Keep-alive for interactive ssh terminals: probe every 15 s and give up
/// after 2 missed replies, so a dead link (sleep, Wi-Fi change, a silently
/// dropped NAT entry) ends the session in ~45 s instead of leaving a hung
/// PTY and ssh process behind until TCP gives up hours later. Command-line
/// `-o` wins over ssh_config, so each option is added only when the user's
/// `~/.ssh/config` doesn't mention it at all (any value there is theirs).
fn keepalive_opts(user_config: Option<&str>) -> Vec<String> {
    let mentioned = |key: &str| {
        user_config.is_some_and(|cfg| {
            cfg.lines().any(|l| {
                l.trim_start()
                    .split(|c: char| c.is_whitespace() || c == '=')
                    .next()
                    .is_some_and(|k| k.eq_ignore_ascii_case(key))
            })
        })
    };
    let mut out = Vec::new();
    for (key, value) in [("ServerAliveInterval", "15"), ("ServerAliveCountMax", "2")] {
        if !mentioned(key) {
            out.push("-o".into());
            out.push(format!("{key}={value}"));
        }
    }
    out
}

/// The user's `~/.ssh/config` (a small file, read when a terminal opens).
/// Unit tests never read the real one, so their argv is deterministic.
fn user_ssh_config() -> Option<String> {
    if cfg!(test) {
        return None;
    }
    let home = std::env::var_os("HOME")?;
    std::fs::read_to_string(std::path::Path::new(&home).join(".ssh/config")).ok()
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
            reject_option_like("ssh", "host", host)?;
            // Trust a first-time host on first connect (adds its key to
            // known_hosts), matching what a user does by answering "yes" to the
            // authenticity prompt; a *changed* known key is still refused.
            let mut args = vec![
                "-o".to_string(),
                "StrictHostKeyChecking=accept-new".to_string(),
            ];
            args.extend(keepalive_opts(user_ssh_config().as_deref()));
            if let Some(identity) = opt_str(p, "identity_file") {
                args.push("-i".into());
                args.push(identity.into());
            }
            if let Some(port) = opt_port(p, "port", "ssh")? {
                args.push("-p".into());
                args.push(port.to_string());
            }
            if let Some(jump) = opt_str(p, "jump") {
                reject_option_like("ssh", "jump", jump)?;
                args.push("-J".into());
                args.push(jump.into());
            }
            let target = match opt_str(p, "user") {
                Some(user) => {
                    reject_option_like("ssh", "user", user)?;
                    format!("{user}@{host}")
                }
                None => host.to_string(),
            };
            // `--` ends option parsing: the destination is never an option.
            args.push("--".into());
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
            let mut env = Vec::new();
            if secret.is_some() && runs_on_jump_host(p) {
                // The saved password can't travel to the jump host without
                // landing in an argv; have mysql ask for it on the terminal
                // instead of failing with "using password: NO".
                args.push("-p".into());
            } else if let Some(pw) = secret {
                env.push(("MYSQL_PWD".to_string(), pw.to_string()));
            }
            if let Some(db) = opt_str(p, "db") {
                // One `--database=` token, never a bare positional: profiles
                // are imported from third-party configs, and a positional
                // `db = "--execute=system …"` would be parsed as an option.
                args.push(format!("--database={db}"));
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
            // mongosh's only positional is the connection string; one that
            // starts with `-` (`--eval=require('child_process')…` from an
            // imported profile) would run as an option instead.
            if template.trim_start().starts_with('-') {
                return Err(Error::Invalid(
                    "mongodb: param 'conn_string' must not start with '-'".into(),
                ));
            }
            let conn_string = if template.contains("{secret}") {
                let secret = secret.ok_or_else(|| {
                    Error::Invalid(
                        "mongodb conn_string references {secret} but no secret is stored".into(),
                    )
                })?;
                template.replace(
                    "{secret}",
                    &otto_core::connection_credentials::encode_password(secret),
                )
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
        ConnectionKind::Postgres => {
            let host = match opt_str(p, "host") {
                Some(h) => h,
                None => return Ok((login_shell(), false)),
            };
            let mut args = vec!["-h".to_string(), host.to_string()];
            if let Some(port) = opt_port(p, "port", "postgres")? {
                args.push("-p".into());
                args.push(port.to_string());
            }
            if let Some(user) = opt_str(p, "user") {
                args.push("-U".into());
                args.push(user.into());
            }
            if let Some(db) = opt_str(p, "db").or_else(|| opt_str(p, "database")) {
                args.push("-d".into());
                args.push(db.into());
            }
            let mut env = Vec::new();
            if let Some(pw) = secret {
                // psql reads the password from PGPASSWORD (never argv — it would
                // leak in the process list).
                env.push(("PGPASSWORD".to_string(), pw.to_string()));
            }
            let spec = maybe_wrap_ssh_tunnel(
                p,
                CommandSpec {
                    program: "psql".into(),
                    args,
                    cwd: None,
                    env,
                },
                "postgres",
            )?;
            Ok((spec, false))
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
pub fn validate_params(kind: ConnectionKind, params: &Value, _has_secret: bool) -> Result<()> {
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
        last_opened_at: None,
        pinned: false,
    };
    // Validate only the params SHAPE here. We deliberately pretend a secret is
    // available (`Some`) regardless of `_has_secret`: a connection whose
    // conn_string / command template references `{secret}` may legitimately be
    // *created* before its password is stored — e.g. connection profiles
    // imported from another tool (which never carry the password), or any
    // "set it up now, add the credential later" flow. The authoritative check
    // stays at OPEN time, where `build_command` runs with the real Keychain
    // secret and returns the clear "references {secret} but no secret is stored"
    // error if it is still missing.
    let secret = Some("x");
    build_command(&conn, secret).map(|_| ())?;
    // A DB profile's SSH tunnel (`params.ssh`, opened by the Database Explorer
    // / Kafka viewer): refuse a host or user ssh could read as an option now,
    // not only when the tunnel is first opened.
    if let Some(ssh) = params.get("ssh").filter(|v| !v.is_null()) {
        let tunnel: otto_ssh::SshTunnelConfig = serde_json::from_value(ssh.clone())
            .map_err(|e| Error::Invalid(format!("invalid ssh config: {e}")))?;
        tunnel.validate()?;
    }
    Ok(())
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
            last_opened_at: None,
            pinned: false,
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
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=2",
                "-i",
                "/home/me/.ssh/id_ed25519",
                "-p",
                "2222",
                "-J",
                "bastion.example.com",
                "--",
                "deploy@db.example.com"
            ]
        );
        assert!(!warn);
        assert!(spec.env.is_empty());
    }

    /// A profile value (possibly imported) can never reach ssh's argv as an
    /// option — neither the terminal's host/user/jump nor a DB tunnel's.
    #[test]
    fn option_like_ssh_values_are_refused() {
        for params in [
            json!({"host":"-oProxyCommand=x"}),
            json!({"host":"h1","user":"-oProxyCommand=x"}),
            json!({"host":"h1","jump":"-oProxyCommand=x"}),
            json!({"host":"h 1"}),
        ] {
            assert!(
                build_command(&conn(ConnectionKind::Ssh, params.clone()), None).is_err(),
                "{params}"
            );
        }
        assert!(build_command(
            &conn(
                ConnectionKind::Mysql,
                json!({"host":"db","jump":"-oProxyCommand=x"})
            ),
            None
        )
        .is_err());
        assert!(validate_params(
            ConnectionKind::Mysql,
            &json!({"host":"db","ssh":{"host":"-oProxyCommand=x"}}),
            false
        )
        .is_err());
        assert!(validate_params(
            ConnectionKind::Mysql,
            &json!({"host":"db","ssh":{"host":"bastion","user":"-l"}}),
            false
        )
        .is_err());
        assert!(validate_params(
            ConnectionKind::Mysql,
            &json!({"host":"db","ssh":{"host":"bastion.internal","user":"ec2-user"}}),
            false
        )
        .is_ok());
    }

    /// F15: keep-alive options are added unless the user's ssh_config sets
    /// them (any value, any case, `Key value` or `Key=value`).
    #[test]
    fn keepalive_defers_to_the_users_ssh_config() {
        assert_eq!(
            keepalive_opts(None),
            vec![
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=2"
            ]
        );
        assert_eq!(
            keepalive_opts(Some("Host *\n  serveraliveinterval 60\n")),
            vec!["-o", "ServerAliveCountMax=2"]
        );
        assert!(keepalive_opts(Some(
            "ServerAliveInterval=0\nHost x\n\tServerAliveCountMax 9\n"
        ))
        .is_empty());
        // A comment or an unrelated key doesn't count.
        assert_eq!(
            keepalive_opts(Some("# ServerAliveInterval 5\nServerAliveIntervalX 1\n")).len(),
            4
        );
    }

    #[test]
    fn ssh_minimal_and_missing_host() {
        let c = conn(ConnectionKind::Ssh, json!({"host":"h1"}));
        let (spec, _) = build_command(&c, None).unwrap();
        assert_eq!(
            spec.args,
            vec![
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=2",
                "--",
                "h1"
            ]
        );

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
            vec![
                "-h",
                "127.0.0.1",
                "-P",
                "3306",
                "-u",
                "root",
                "--database=app_db"
            ]
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
    fn postgres_password_via_env_never_argv() {
        let c = conn(
            ConnectionKind::Postgres,
            json!({"host":"127.0.0.1","port":15432,"user":"otto","db":"shopdb"}),
        );
        let (spec, warn) = build_command(&c, Some("s3cret")).unwrap();
        assert_eq!(spec.program, "psql");
        assert_eq!(
            spec.args,
            vec![
                "-h",
                "127.0.0.1",
                "-p",
                "15432",
                "-U",
                "otto",
                "-d",
                "shopdb"
            ]
        );
        assert_eq!(
            spec.env,
            vec![("PGPASSWORD".to_string(), "s3cret".to_string())]
        );
        assert!(!warn);
        assert!(
            !spec.args.iter().any(|a| a.contains("s3cret")),
            "password must never appear in argv"
        );
    }

    #[test]
    fn postgres_missing_host() {
        let c = conn(ConnectionKind::Postgres, json!({"user":"otto"}));
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

    /// S6-13: an imported profile's positional values never become options.
    #[test]
    fn imported_positional_values_cannot_inject_client_options() {
        let c = conn(
            ConnectionKind::Mysql,
            json!({"host":"h","user":"u","db":"--execute=system id"}),
        );
        let (spec, _) = build_command(&c, None).unwrap();
        assert_eq!(spec.args.last().unwrap(), "--database=--execute=system id");
        assert!(!spec.args.iter().any(|a| a.starts_with("--execute")));
        let m = conn(
            ConnectionKind::Mongodb,
            json!({"conn_string":"--eval=require('child_process').execSync('id')"}),
        );
        assert!(matches!(build_command(&m, None), Err(Error::Invalid(_))));
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
    fn validate_allows_mongodb_secret_placeholder_without_secret() {
        // Create-time validation tolerates a deferred `{secret}` (imported
        // profiles never carry the password). The open-time guard
        // (`mongodb_secret_placeholder_without_secret_fails`) still protects use.
        let params = json!({"conn_string":"mongodb+srv://app:{secret}@m1/db?authSource=admin"});
        assert!(validate_params(ConnectionKind::Mongodb, &params, false).is_ok());
        // And a custom template referencing {secret} likewise validates pre-secret.
        let custom = json!({"command_template":"psql -h h -U u {secret}"});
        assert!(validate_params(ConnectionKind::Custom, &custom, false).is_ok());
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
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=2",
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
                "-p",
                "--database=mydb",
            ]
        );
        // The password never reaches argv, and isn't parked in the local ssh
        // process's env either (ssh would not forward it): mysql prompts.
        assert!(spec.env.is_empty());
        assert!(!spec.args.iter().any(|a| a.contains("s3cret")));
        assert!(!warn);

        // Without a stored secret there is nothing to prompt for.
        let (spec, _) = build_command(&c, None).unwrap();
        assert!(!spec.args.iter().any(|a| a == "-p"));
    }

    /// The remote command is one string parsed by the jump host's shell, so
    /// each word is quoted: spaces / metacharacters can't split or inject.
    #[test]
    fn jump_wrapped_remote_command_is_shell_quoted() {
        let c = conn(
            ConnectionKind::Clickhouse,
            json!({"host":"ch.internal","user":"a b","db":"x;touch /tmp/p","jump":"bastion"}),
        );
        let (spec, warn) = build_command(&c, Some("p$w'd")).unwrap();
        assert!(warn, "clickhouse password is still in argv");
        let dash = spec.args.iter().position(|a| a == "--").unwrap();
        let remote = &spec.args[dash + 1..];
        assert_eq!(remote[0], "clickhouse-client");
        // Failure messages never print `remote`: it carries the password.
        assert!(remote.contains(&"'a b'".to_string()), "user not quoted");
        assert!(
            remote.contains(&"'x;touch /tmp/p'".to_string()),
            "database not quoted"
        );
        // Re-parsing the joined command (what the remote shell does) yields
        // the original argv exactly.
        let reparsed = shell_words::split(&remote.join(" ")).unwrap();
        assert_eq!(
            reparsed,
            vec![
                "clickhouse-client",
                "-h",
                "ch.internal",
                "-u",
                "a b",
                "--password",
                "p$w'd",
                "-d",
                "x;touch /tmp/p"
            ]
        );
        assert!(spec.env.is_empty());
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
                "-o",
                "StrictHostKeyChecking=accept-new",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=2",
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
