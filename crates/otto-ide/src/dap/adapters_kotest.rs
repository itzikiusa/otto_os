//! Kotest Debug: the build tool runs the chosen spec, container or test (the same command as
//! Run) with its own debug switch, and the JDWP bridge attaches to the suspended test JVM.
//! Gradle and Maven compile Kotlin and resolve the test classpath; Otto's planner does neither.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Duration;

use otto_core::{Error, Result};
use serde_json::json;

use super::adapters::{normalize_target_path, DapAdapterTransport, ResolvedDapAdapter};
use super::adapters_jvm_js::infer_java_source_root;
use super::java_bridge::JavaJdwpBridgeSpec;
use super::{BridgeGuard, DapStartContext};
use crate::environments::runner_cmd::runner_kotest::{
    kotest_debug_command, kotest_env, GRADLE_DEBUG_PORT,
};
use crate::types::{
    IdeDebugOutputEntry, IdeRunConfig, IdeRunnerKind, IdeStartDebugReq, IdeTargetKind,
};

/// How long the build may take before its test JVM listens (a cold Gradle build downloads).
const LISTEN_TIMEOUT: Duration = Duration::from_secs(600);
/// Build output kept for the console and for the error when the build fails first.
const MAX_CAPTURED_LINES: usize = 200;

/// The port in a JVM's `Listening for transport dt_socket at address: 5005` line.
fn listening_port(line: &str) -> Option<u16> {
    let (_, rest) = line.split_once("Listening for transport dt_socket at address:")?;
    rest.trim().rsplit(':').next()?.trim().parse().ok()
}

pub(super) async fn resolve_kotest_adapter(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    mut env: BTreeMap<String, String>,
    test_path: &[String],
) -> Result<ResolvedDapAdapter> {
    let cfg = IdeRunConfig {
        runner: IdeRunnerKind::Kotest,
        test_filter: Some(req.launch.program.trim().to_string()),
        test_path: test_path.to_vec(),
        maven_profiles: req.launch.maven_profiles.clone(),
        offline: req.launch.offline,
        ..IdeRunConfig::default()
    };
    env.extend(kotest_env(&cfg));
    let local = req.target.kind == IdeTargetKind::Local;
    if !local {
        // The SSH Maven / Gradle launchers look for the build wrapper up to the target root.
        env.insert("OTTO_TARGET_ROOT".into(), req.target.root.clone());
    }
    let maven_port = if local {
        std::net::TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map(|a| a.port())
            .map_err(|e| Error::Internal(format!("no free debug port: {e}")))?
    } else {
        GRADLE_DEBUG_PORT
    };
    let cmd = kotest_debug_command(&cfg, req.target.kind, cwd, &env, maven_port);
    if local
        && cmd.tool == "Gradle"
        && tokio::net::TcpStream::connect(("127.0.0.1", GRADLE_DEBUG_PORT))
            .await
            .is_ok()
    {
        return Err(Error::Conflict(
            "Port 5005 is in use, so Gradle can't start the test JVM for debugging. Stop the other debug session and try again.".into(),
        ));
    }

    let mut child = super::spawn_target_process(
        &req.target,
        ctx.ssh.as_ref(),
        &cmd.program,
        &cmd.args,
        cwd,
        &env,
    )
    .await?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    for pipe in [
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        crate::line_reader::spawn_line_drain(
            pipe,
            crate::line_reader::MAX_OUTPUT_LINE_BYTES,
            move |l| {
                let _ = tx.send(l);
            },
        );
    }
    drop(tx);
    // Owned before the wait: a Stop that cancels this start kills the build.
    let companion = BridgeGuard::new();
    companion.push_child(child);

    let mut captured: Vec<String> = vec![format!(
        "$ {} {}",
        cmd.program,
        shell_words::join(&cmd.args)
    )];
    let deadline = tokio::time::Instant::now() + LISTEN_TIMEOUT;
    let port = loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(line)) => {
                let port = listening_port(&line);
                if captured.len() >= MAX_CAPTURED_LINES {
                    captured.remove(1);
                }
                captured.push(line);
                if let Some(p) = port {
                    break p;
                }
            }
            Ok(None) => {
                return Err(Error::Upstream(format!(
                    "The {} build failed before tests started.\n{}",
                    cmd.tool,
                    captured.join("\n")
                )));
            }
            Err(_) => {
                return Err(Error::Upstream(format!(
                    "The test JVM did not open its debug port within 10 minutes. See the build output above.\n{}",
                    captured.join("\n")
                )));
            }
        }
    };
    if port != cmd.port {
        return Err(Error::Upstream(format!(
            "The test JVM listens on port {port}, not {} (the build sets its own debug options).",
            cmd.port
        )));
    }
    // The rest of the build's output (test results, BUILD SUCCESSFUL) streams to the console.
    companion.set_output(rx);

    let mut source_roots: Vec<String> = Vec::new();
    let mut source_map = HashMap::new();
    for bp in &req.breakpoints {
        let p = normalize_target_path(&req.target, &bp.path);
        if let Some(name) = Path::new(&p).file_name().and_then(|n| n.to_str()) {
            source_map.insert(name.to_string(), p.clone());
        }
        let root = infer_java_source_root(&p, cwd);
        if !source_roots.contains(&root) {
            source_roots.push(root);
        }
    }
    for sub in [
        "src/test/kotlin",
        "src/main/kotlin",
        "src/test/java",
        "src/main/java",
    ] {
        let dir = format!("{}/{sub}", cwd.trim_end_matches('/'));
        if !source_roots.contains(&dir) && (!local || Path::new(&dir).is_dir()) {
            source_roots.push(dir);
        }
    }
    let initial_output = vec![
        IdeDebugOutputEntry {
            seq: 0,
            category: "stdout".into(),
            text: format!("{}\n", captured.join("\n")),
        },
        IdeDebugOutputEntry {
            seq: 0,
            category: "console".into(),
            text: format!("Attaching to test JVM on port {port}\n"),
        },
    ];
    Ok(ResolvedDapAdapter {
        adapter_name: "java-jdwp-dap".into(),
        language: "java".into(),
        transport: DapAdapterTransport::JavaJdwpBridge(JavaJdwpBridgeSpec {
            java_bin: "java".into(),
            jvm_flags: Vec::new(),
            classpath: cwd.to_string(),
            main_class: String::new(),
            args: Vec::new(),
            cwd: cwd.to_string(),
            env,
            attach_port: Some(port),
            source_roots,
            source_map,
            companion: Some(companion),
        }),
        initialize_args: json!({
            "adapterID": "java",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path"
        }),
        launch_command: "attach".into(),
        launch_args: json!({ "mainClass": req.launch.program, "cwd": cwd }),
        normalized_cwd: cwd.to_string(),
        initial_output,
    })
}

#[cfg(test)]
mod tests {
    use super::listening_port;

    #[test]
    fn reads_the_jdwp_listening_line_of_gradle_and_maven() {
        assert_eq!(
            listening_port("Listening for transport dt_socket at address: 5005"),
            Some(5005)
        );
        assert_eq!(
            listening_port("Listening for transport dt_socket at address: 127.0.0.1:61234"),
            Some(61234)
        );
        assert_eq!(listening_port("> Task :test"), None);
    }
}
