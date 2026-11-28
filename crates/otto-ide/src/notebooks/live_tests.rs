//! Live Jupyter scenarios against real kernels (design A3, J3–J10, J14).
//! `#[ignore]`d: run by hand with throwaway envs, e.g.
//!   OTTO_JUPYTER_LIVE_PYTHON=<venv>/bin/python   (ipykernel, matplotlib, pandas)
//!   OTTO_JUPYTER_LIVE_BASH_SPEC=<dir with the bash kernel.json>
//!   cargo test -p otto-ide --lib notebooks::live_tests -- --ignored --nocapture

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::Value;
use tokio::sync::broadcast;

use super::kernelspecs::{InterruptMode, JupyterSpec, KernelLaunch};
use super::{IdeNotebookExecuteResp, NotebookKernelCtx, NotebookManager};

fn live_python() -> Option<String> {
    std::env::var("OTTO_JUPYTER_LIVE_PYTHON")
        .ok()
        .filter(|p| !p.is_empty())
}

fn base_env() -> BTreeMap<String, String> {
    ["PATH", "HOME", "TMPDIR"]
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
        .collect()
}

fn ipykernel(python: &str, mode: InterruptMode, env: BTreeMap<String, String>) -> KernelLaunch {
    KernelLaunch::Jupyter(JupyterSpec {
        spec_name: "python3".into(),
        display_name: "Python (live test)".into(),
        language: "python".into(),
        argv: [
            python,
            "-m",
            "ipykernel_launcher",
            "-f",
            "{connection_file}",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect(),
        env,
        interrupt_mode: mode,
    })
}

fn text_of(resp: &IdeNotebookExecuteResp) -> String {
    resp.outputs
        .iter()
        .map(|o| match o["output_type"].as_str() {
            Some("stream") => o["text"].as_str().unwrap_or("").to_string(),
            Some("error") => format!(
                "{}: {}",
                o["ename"].as_str().unwrap_or(""),
                o["evalue"].as_str().unwrap_or("")
            ),
            _ => o["data"]["text/plain"].as_str().unwrap_or("").to_string(),
        })
        .collect::<Vec<_>>()
        .join("")
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn group_alive(pid: u32) -> bool {
    extern "C" {
        fn kill(pid: std::ffi::c_int, sig: std::ffi::c_int) -> std::ffi::c_int;
    }
    // SAFETY: signal 0 only probes the process group.
    unsafe { kill(-(pid as std::ffi::c_int), 0) == 0 }
}

struct Live {
    mgr: NotebookManager,
    env: BTreeMap<String, String>,
    cwd: String,
    runtime: std::path::PathBuf,
    path: String,
    kernel_id: String,
    launch: KernelLaunch,
    _tmp: tempfile::TempDir,
}

impl Live {
    fn new(launch: KernelLaunch, mut env: BTreeMap<String, String>) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        // The kernel's HOME is throwaway: IPython writes ~/.ipython and
        // bash_kernel's interactive bash appends ~/.bash_history.
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        env.insert("HOME".into(), home.to_string_lossy().into_owned());
        Live {
            mgr: NotebookManager::new(),
            env,
            cwd: tmp.path().to_string_lossy().into_owned(),
            runtime: tmp.path().join("jupyter-runtime"),
            path: tmp.path().join("live.ipynb").to_string_lossy().into_owned(),
            kernel_id: "jupyter:live".into(),
            launch,
            _tmp: tmp,
        }
    }

    fn ctx(&self) -> NotebookKernelCtx<'_> {
        NotebookKernelCtx {
            owner: "u1",
            project_key: "local:/live",
            notebook_path: &self.path,
            cwd: &self.cwd,
            env: &self.env,
            runtime_dir: &self.runtime,
        }
    }

    async fn run(&self, code: &str) -> IdeNotebookExecuteResp {
        let t = Instant::now();
        let r = self
            .mgr
            .execute(
                &self.ctx(),
                &self.kernel_id,
                &self.launch,
                code,
                "run",
                Some(Duration::from_secs(120)),
            )
            .await
            .unwrap_or_else(|e| panic!("{code:?}: {e}"));
        println!(
            "[{:>5} ms] {code:?} -> {} {:?}",
            t.elapsed().as_millis(),
            r.status,
            text_of(&r)
        );
        r
    }

    fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.mgr.subscribe("u1", "local:/live", &self.path).1
    }

    fn pid(&self) -> u32 {
        self.mgr
            .current_jupyter("u1", "local:/live", &self.path)
            .unwrap()
            .pid()
    }
}

async fn next_event(rx: &mut broadcast::Receiver<Value>, kind: &str, secs: u64) -> Value {
    tokio::time::timeout(Duration::from_secs(secs), async {
        loop {
            let v = rx.recv().await.unwrap();
            if v["kind"] == kind {
                return v;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no {kind} event within {secs} s"))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: needs OTTO_JUPYTER_LIVE_PYTHON (a venv with ipykernel, matplotlib, pandas)"]
#[allow(clippy::disallowed_methods)] // one small read_dir of the test's own runtime dir
async fn live_ipykernel_scenarios() {
    let Some(py) = live_python() else { return };
    let live = std::sync::Arc::new(Live::new(
        ipykernel(&py, InterruptMode::Signal, BTreeMap::new()),
        base_env(),
    ));

    // J3 magics and plots
    let r = live
        .run("%matplotlib inline\nimport matplotlib.pyplot as plt\nplt.plot([1, 2])")
        .await;
    assert!(
        r.outputs.iter().any(|o| o["data"]["image/png"].is_string()),
        "inline png: {:?}",
        r.outputs
    );
    let r = live.run("%timeit -n 10 -r 2 sum(range(100))").await;
    assert!(text_of(&r).contains("per loop"), "timeit: {}", text_of(&r));
    let r = live.run("!echo hi").await;
    assert_eq!(text_of(&r).trim(), "hi");

    // J4 rich outputs
    let r = live
        .run("import pandas as pd\npd.DataFrame({'a': [1, 2]})")
        .await;
    assert!(r.outputs.iter().any(|o| {
        o["data"]["text/html"]
            .as_str()
            .is_some_and(|h| h.contains("<table"))
    }));
    let r = live
        .run("h = display('first', display_id=True)\nh.update('second')")
        .await;
    assert_eq!(r.outputs.len(), 1, "{:?}", r.outputs);
    assert_eq!(r.outputs[0]["data"]["text/plain"], "'second'");
    let r = live.run("from IPython.display import clear_output\nfor i in range(3):\n    clear_output(wait=True)\n    print(i)").await;
    assert_eq!(r.outputs.len(), 1);
    assert_eq!(text_of(&r), "2\n");

    // J5 streaming: line 0 arrives within 1.5 s, before the cell ends
    let mut rx = live.subscribe();
    let l2 = live.clone();
    let started = Instant::now();
    let cell = tokio::spawn(async move {
        l2.run("import time\nfor i in range(5):\n    print(i)\n    time.sleep(1)")
            .await
    });
    let first = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let v = rx.recv().await.unwrap();
            if v["kind"] == "output"
                && v["output"]["text"]
                    .as_str()
                    .is_some_and(|t| t.starts_with('0'))
            {
                return started.elapsed();
            }
        }
    })
    .await
    .expect("streamed line 0");
    assert!(
        first < Duration::from_millis(1500),
        "line 0 after {first:?}"
    );
    assert!(!cell.is_finished(), "streamed before execute_reply");
    println!("J5: line 0 streamed after {} ms", first.as_millis());
    assert_eq!(text_of(&cell.await.unwrap()), "0\n1\n2\n3\n4\n");

    // J6 stdin
    let mut rx = live.subscribe();
    let l2 = live.clone();
    let cell = tokio::spawn(async move { l2.run("x = input('name? ')").await });
    let req = next_event(&mut rx, "input_request", 10).await;
    assert_eq!(req["prompt"], "name? ");
    assert_eq!(req["password"], false);
    assert!(live
        .mgr
        .send_input("u1", "local:/live", &live.path, "ada")
        .await
        .unwrap());
    cell.await.unwrap();
    assert_eq!(text_of(&live.run("x").await), "'ada'");

    // Variables come from the debugger's inspectVariables
    let vars = live
        .mgr
        .list_variables("u1", "local:/live", &live.path)
        .await
        .unwrap();
    assert!(
        vars.variables
            .iter()
            .any(|v| v.name == "x" && v.preview.contains("ada")),
        "{:?}",
        vars.variables
    );

    // J7 interrupt (signal): KeyboardInterrupt within 2 s, kernel alive
    let l2 = live.clone();
    let cell = tokio::spawn(async move { l2.run("import time\ntime.sleep(60)").await });
    tokio::time::sleep(Duration::from_millis(800)).await;
    let t = Instant::now();
    let (ok, msg) = live
        .mgr
        .interrupt_kernel("u1", "local:/live", &live.path)
        .await;
    let r = cell.await.unwrap();
    assert!(ok, "{msg}");
    assert!(
        t.elapsed() < Duration::from_secs(2),
        "interrupt took {:?}",
        t.elapsed()
    );
    assert!(text_of(&r).contains("KeyboardInterrupt"), "{}", text_of(&r));
    assert_eq!(text_of(&live.run("1 + 1").await), "2");

    // J9 restart, then a kernel that dies
    live.run("y = 5").await;
    let sid = live
        .mgr
        .restart_kernel(&live.ctx(), &live.kernel_id, &live.launch)
        .await
        .unwrap();
    println!("J9: restarted as {sid}");
    assert!(text_of(&live.run("y").await).contains("NameError"));
    let mut rx = live.subscribe();
    let r = live.run("import os\nos._exit(3)").await;
    assert_eq!(r.status, "dead");
    assert!(text_of(&r).contains("exit code 3"), "{}", text_of(&r));
    let dead = next_event(&mut rx, "status", 5).await;
    println!("J9: status event {dead}");
    let st = live
        .mgr
        .kernel_status("u1", "local:/live", &live.path)
        .unwrap();
    assert_eq!((st.state.as_str(), st.exit_code), ("dead", Some(3)));
    let err = live
        .mgr
        .execute(&live.ctx(), &live.kernel_id, &live.launch, "1", "run", None)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("Restart to continue"), "{err}");
    live.mgr
        .restart_kernel(&live.ctx(), &live.kernel_id, &live.launch)
        .await
        .unwrap();
    assert_eq!(text_of(&live.run("40 + 2").await), "42");

    // J14 lifecycle: the process group is gone within 5 s of closing the notebook
    let pid = live.pid();
    assert!(group_alive(pid));
    live.mgr
        .shutdown_session("u1", "local:/live", &live.path)
        .await;
    let t = Instant::now();
    while group_alive(pid) && t.elapsed() < Duration::from_secs(5) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!group_alive(pid), "kernel group {pid} still alive");
    println!(
        "J14: group {pid} gone {} ms after shutdown",
        t.elapsed().as_millis()
    );
    assert_eq!(
        std::fs::read_dir(&live.runtime).unwrap().count(),
        0,
        "connection file deleted"
    );
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: needs OTTO_JUPYTER_LIVE_PYTHON"]
async fn live_message_interrupt_and_spec_env() {
    let Some(py) = live_python() else { return };
    let spec_env = BTreeMap::from([("OTTO_SPEC_PROBE".to_string(), "${HOME}/x".to_string())]);
    let live = std::sync::Arc::new(Live::new(
        ipykernel(&py, InterruptMode::Message, spec_env),
        base_env(),
    ));
    let probe = format!("{}/x", live.env["HOME"]);
    // J8: ${VAR} expands from the kernel env only; the daemon's own env stays out.
    let r = live
        .run("import os\n(os.environ['OTTO_SPEC_PROBE'], 'CARGO' in os.environ)")
        .await;
    assert_eq!(text_of(&r), format!("('{probe}', False)"));
    let l2 = live.clone();
    let cell = tokio::spawn(async move { l2.run("import time\ntime.sleep(60)").await });
    tokio::time::sleep(Duration::from_millis(800)).await;
    let (ok, msg) = live
        .mgr
        .interrupt_kernel("u1", "local:/live", &live.path)
        .await;
    assert!(ok, "{msg}");
    assert!(text_of(&cell.await.unwrap()).contains("KeyboardInterrupt"));
    live.mgr
        .shutdown_session("u1", "local:/live", &live.path)
        .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: needs OTTO_JUPYTER_LIVE_BASH_SPEC (a bash_kernel kernelspec dir)"]
async fn live_bash_kernel() {
    let Some(dir) = std::env::var("OTTO_JUPYTER_LIVE_BASH_SPEC").ok() else {
        return;
    };
    let raw = std::fs::read(std::path::Path::new(&dir).join("kernel.json")).unwrap();
    let ks: jupyter_protocol::JupyterKernelspec = serde_json::from_slice(&raw).unwrap();
    let launch = KernelLaunch::Jupyter(JupyterSpec {
        spec_name: "bash".into(),
        display_name: ks.display_name,
        language: ks.language,
        argv: ks.argv,
        env: ks.env.unwrap_or_default().into_iter().collect(),
        interrupt_mode: InterruptMode::Signal,
    });
    let live = std::sync::Arc::new(Live::new(launch, base_env()));
    std::fs::write(std::path::Path::new(&live.cwd).join("marker.txt"), "x").unwrap();
    // J10
    let r = live.run("echo $SHELL; ls").await;
    assert!(text_of(&r).contains("marker.txt"), "{}", text_of(&r));
    let l2 = live.clone();
    let cell = tokio::spawn(async move { l2.run("sleep 60").await });
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let t = Instant::now();
    let (ok, msg) = live
        .mgr
        .interrupt_kernel("u1", "local:/live", &live.path)
        .await;
    assert!(ok, "{msg}");
    cell.await.unwrap();
    println!(
        "J10: sleep 60 interrupted in {} ms",
        t.elapsed().as_millis()
    );
    assert_eq!(text_of(&live.run("echo after").await).trim(), "after");
    live.mgr
        .shutdown_session("u1", "local:/live", &live.path)
        .await;
}
