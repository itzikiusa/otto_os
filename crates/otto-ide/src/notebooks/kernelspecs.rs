//! Notebook kernel discovery: Jupyter kernelspecs plus the built-in fallback.
//!
//! Static file reads only (`kernel.json` via `read_kernelspec_jsons`, Python
//! env layouts): discovery never runs `jupyter` or any other process.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::IdeNotebookKernelInfo;
use crate::environments::db_store::discover_local_python_venvs;
use crate::types::IdeProjectEnvironment;

const IPYKERNEL_TAIL: [&str; 4] = ["-m", "ipykernel_launcher", "-f", "{connection_file}"];
const TRUST_FILE: &str = "jupyter-kernel-trust.json";

/// How a Jupyter kernel wants to be interrupted (`interrupt_mode` in kernel.json).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptMode {
    Signal,
    Message,
}

/// A launchable Jupyter kernel: a kernelspec read from disk, or ipykernel's own
/// spec shape synthesized for a Python env that has ipykernel and no spec.
#[derive(Debug, Clone)]
pub struct JupyterSpec {
    pub spec_name: String,
    pub display_name: String,
    pub language: String,
    pub argv: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub interrupt_mode: InterruptMode,
}

#[derive(Debug, Clone)]
pub enum KernelLaunch {
    /// Otto's zero-install Python kernel (frozen fallback for envs without ipykernel).
    Builtin {
        interpreter: String,
    },
    Jupyter(JupyterSpec),
}

#[derive(Debug, Clone)]
pub struct DiscoveredKernel {
    pub info: IdeNotebookKernelInfo,
    pub launch: KernelLaunch,
    /// Trust-store key for a workspace kernelspec (`origin == "workspace"`).
    pub trust_key: Option<String>,
}

struct PyEnv {
    python: String,
    kind: String,
    prefix: PathBuf,
    label: String,
    version: Option<String>,
    ipykernel: bool,
    registered: bool,
}

fn canon(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// The env prefix of `<prefix>/bin/python`. Never canonicalize the binary
/// itself: every venv's `bin/python` links to the same base interpreter.
fn prefix_of(python: &str) -> PathBuf {
    let p = Path::new(python);
    canon(p.parent().and_then(Path::parent).unwrap_or(p))
}

#[allow(clippy::disallowed_methods)] // bounded env-layout reads, run inside spawn_blocking
fn has_ipykernel(prefix: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(prefix.join("lib")) else {
        return false;
    };
    rd.flatten().any(|e| {
        e.file_name().to_string_lossy().starts_with("python3")
            && e.path()
                .join("site-packages/ipykernel/__init__.py")
                .is_file()
    })
}

/// Python envs (project venvs, uv / conda / poetry, system python3), no process spawned.
fn python_envs(root: &Path, data_dir: &Path) -> Vec<PyEnv> {
    let mut out: Vec<PyEnv> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut push = |python: PathBuf, kind: &str, label: String, version: Option<String>| {
        let Some(prefix) = python
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
        else {
            return;
        };
        let key = canon(&prefix);
        if seen.contains(&key) {
            return;
        }
        seen.push(key);
        out.push(PyEnv {
            python: python.to_string_lossy().into_owned(),
            kind: kind.to_string(),
            ipykernel: has_ipykernel(&prefix),
            prefix,
            label,
            version,
            registered: false,
        });
    };
    for venv in discover_local_python_venvs(root, data_dir) {
        let dir = Path::new(&venv.path);
        let Some(python) = ["bin/python", "bin/python3"]
            .iter()
            .map(|b| dir.join(b))
            .find(|p| p.is_file())
        else {
            continue;
        };
        let folder = dir.file_name().and_then(|s| s.to_str()).unwrap_or("python");
        let label = format!("Python {} ({folder})", venv.version);
        push(python, &venv.kind, label, Some(venv.version));
    }
    let search_dirs = crate::toolchains::system_search_dirs();
    if let Some(py3) = crate::toolchains::find_executable_in(&search_dirs, &["python3", "python"]) {
        // `python3` usually links to `python3.13`: the version without running it.
        let version = canon(&py3)
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|n| n.strip_prefix("python"))
            .filter(|v| v.contains('.'))
            .map(str::to_string);
        let label = match &version {
            Some(v) => format!("Python {v} (system)"),
            None => "Python 3 (system)".into(),
        };
        push(py3, "system", label, version);
    }
    out
}

/// `kernel.json` locations, user first: `JUPYTER_PATH` from the project env, the
/// user data dir, every Python env's `share/jupyter`, then the system dirs.
fn data_dirs(
    project_env: Option<&IdeProjectEnvironment>,
    envs: &[PyEnv],
) -> Vec<(PathBuf, Option<usize>)> {
    let mut dirs: Vec<(PathBuf, Option<usize>)> = Vec::new();
    let jupyter_path = project_env.and_then(|e| {
        e.env_vars
            .iter()
            .find(|v| !v.secret && v.key == "JUPYTER_PATH")
            .and_then(|v| v.value.clone())
    });
    for p in jupyter_path.iter().flat_map(|s| s.split(':')) {
        if !p.trim().is_empty() {
            dirs.push((PathBuf::from(p.trim()), None));
        }
    }
    if let Ok(user) = jupyter_zmq_client::user_data_dir() {
        dirs.push((user, None));
    }
    for (i, env) in envs.iter().enumerate() {
        dirs.push((env.prefix.join("share/jupyter"), Some(i)));
    }
    for sys in jupyter_zmq_client::system_data_dirs() {
        dirs.push((sys, None));
    }
    dirs
}

fn ipykernel_argv(python: &str) -> Vec<String> {
    std::iter::once(python)
        .chain(IPYKERNEL_TAIL)
        .map(str::to_string)
        .collect()
}

fn argv_preview(argv: &[String]) -> String {
    shell_words::join(argv)
}

/// sha256 over (project, spec dir, kernel.json bytes): a changed spec asks again.
fn trust_key(project_key: &str, spec_dir: &Path, bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(project_key.as_bytes());
    h.update([0]);
    h.update(spec_dir.to_string_lossy().as_bytes());
    h.update([0]);
    h.update(bytes);
    hex::encode(h.finalize())
}

fn read_trusted(data_dir: &Path) -> Vec<String> {
    std::fs::read(data_dir.join(TRUST_FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn kernel_trusted(data_dir: &Path, key: &str) -> bool {
    read_trusted(data_dir).iter().any(|k| k == key)
}

/// Remember that the person accepted a workspace kernelspec (only accepts are stored).
pub fn record_kernel_trust(data_dir: &Path, key: &str) -> otto_core::Result<()> {
    let mut keys = read_trusted(data_dir);
    if keys.iter().any(|k| k == key) {
        return Ok(());
    }
    keys.push(key.to_string());
    let body = serde_json::to_vec(&keys).map_err(|e| otto_core::Error::Internal(e.to_string()))?;
    std::fs::write(data_dir.join(TRUST_FILE), body)
        .map_err(|e| otto_core::Error::Internal(format!("save kernel trust: {e}")))
}

/// Every kernel the notebook can run on: Jupyter kernelspecs (registered or
/// synthesized ipykernel) first, then the built-in kernel for Python envs
/// without ipykernel. Exactly one is `active`: the project's interpreter, else the first.
pub async fn discover_kernels(
    root: &Path,
    data_dir: &Path,
    project_key: &str,
    project_env: Option<&IdeProjectEnvironment>,
) -> Vec<DiscoveredKernel> {
    let (r, d) = (root.to_path_buf(), data_dir.to_path_buf());
    let mut envs = tokio::task::spawn_blocking(move || python_envs(&r, &d))
        .await
        .unwrap_or_default();
    let root_canon = canon(root);
    let mut jupyter: Vec<DiscoveredKernel> = Vec::new();
    let mut seen_dirs: Vec<PathBuf> = Vec::new();

    for (dir, env_idx) in data_dirs(project_env, &envs) {
        for spec in jupyter_zmq_client::read_kernelspec_jsons(&dir).await {
            let spec_dir = canon(&spec.path);
            if seen_dirs.contains(&spec_dir) {
                continue;
            }
            seen_dirs.push(spec_dir.clone());
            let ks = spec.kernelspec;
            let mut argv = ks.argv.clone();
            if argv.is_empty() {
                continue;
            }
            // ipykernel's wheel ships `share/jupyter/kernels/python3` with a bare
            // `python`: in an env prefix that means the env's own interpreter.
            if let Some(i) = env_idx {
                if matches!(argv[0].as_str(), "python" | "python3") {
                    argv[0] = envs[i].python.clone();
                }
            }
            let env_map: BTreeMap<String, String> =
                ks.env.clone().unwrap_or_default().into_iter().collect();
            // The env's own ipykernel runs the env's python itself (the same
            // file in the same prefix), not just any binary under its prefix.
            let owner_env = envs.iter().position(|e| {
                argv.len() == 5
                    && argv[1..] == IPYKERNEL_TAIL
                    && prefix_of(&argv[0]) == prefix_of(&e.python)
                    && canon(Path::new(&argv[0])) == canon(Path::new(&e.python))
            });
            let standard = owner_env.is_some() && env_map.is_empty();
            if let Some(i) = owner_env {
                envs[i].registered = true;
            }
            let in_project = spec_dir.starts_with(&root_canon);
            let origin = if in_project && !standard {
                "workspace"
            } else if env_idx.is_some() {
                "env"
            } else if jupyter_zmq_client::system_data_dirs().contains(&dir) {
                "system"
            } else {
                "user"
            };
            let display_name = match (standard, owner_env) {
                (true, Some(i)) => format!("{} · ipykernel", envs[i].label),
                _ => ks.display_name.clone(),
            };
            let debugger = ks
                .metadata
                .as_ref()
                .and_then(|m| m.get("debugger"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let trust_key = if origin == "workspace" {
                let bytes = tokio::fs::read(spec.path.join("kernel.json"))
                    .await
                    .unwrap_or_default();
                Some(trust_key(project_key, &spec_dir, &bytes))
            } else {
                None
            };
            let needs_trust = trust_key
                .as_deref()
                .is_some_and(|k| !kernel_trusted(data_dir, k));
            jupyter.push(DiscoveredKernel {
                info: IdeNotebookKernelInfo {
                    id: format!("jupyter:{}", spec_dir.to_string_lossy()),
                    display_name: display_name.clone(),
                    language: ks.language.clone(),
                    interpreter: argv[0].clone(),
                    kind: "jupyter".into(),
                    version: owner_env.and_then(|i| envs[i].version.clone()),
                    active: false,
                    spec_name: Some(spec.kernel_name.clone()),
                    origin: Some(origin.into()),
                    argv_preview: Some(argv_preview(&argv)),
                    needs_trust,
                    debugger,
                    env_kind: owner_env.map(|i| envs[i].kind.clone()),
                },
                launch: KernelLaunch::Jupyter(JupyterSpec {
                    spec_name: spec.kernel_name,
                    display_name,
                    language: ks.language,
                    argv,
                    env: env_map,
                    interrupt_mode: if ks.interrupt_mode.as_deref() == Some("message") {
                        InterruptMode::Message
                    } else {
                        InterruptMode::Signal
                    },
                }),
                trust_key,
            });
        }
    }

    let mut env_kernels: Vec<DiscoveredKernel> = Vec::new();
    let mut builtin: Vec<DiscoveredKernel> = Vec::new();
    for env in &envs {
        if env.ipykernel && !env.registered {
            let argv = ipykernel_argv(&env.python);
            let display_name = format!("{} · ipykernel", env.label);
            env_kernels.push(DiscoveredKernel {
                info: IdeNotebookKernelInfo {
                    id: format!("jupyter:ipykernel:{}", env.python),
                    display_name: display_name.clone(),
                    language: "python".into(),
                    interpreter: env.python.clone(),
                    kind: "jupyter".into(),
                    version: env.version.clone(),
                    active: false,
                    spec_name: Some("python3".into()),
                    origin: Some("env".into()),
                    argv_preview: Some(argv_preview(&argv)),
                    needs_trust: false,
                    debugger: true,
                    env_kind: Some(env.kind.clone()),
                },
                launch: KernelLaunch::Jupyter(JupyterSpec {
                    spec_name: "python3".into(),
                    display_name,
                    language: "python".into(),
                    argv,
                    env: BTreeMap::new(),
                    interrupt_mode: InterruptMode::Signal,
                }),
                trust_key: None,
            });
        } else if !env.ipykernel {
            builtin.push(DiscoveredKernel {
                info: IdeNotebookKernelInfo {
                    id: format!("builtin:{}", env.python),
                    display_name: format!("{} · built-in, no IPython", env.label),
                    language: "python".into(),
                    interpreter: env.python.clone(),
                    kind: "builtin".into(),
                    version: env.version.clone(),
                    active: false,
                    spec_name: None,
                    origin: None,
                    argv_preview: None,
                    needs_trust: false,
                    debugger: false,
                    env_kind: Some(env.kind.clone()),
                },
                launch: KernelLaunch::Builtin {
                    interpreter: env.python.clone(),
                },
                trust_key: None,
            });
        }
    }

    // Env ipykernels (registered in the env or synthesized) lead, in env order.
    let env_rank = |k: &DiscoveredKernel| {
        envs.iter()
            .position(|e| prefix_of(&e.python) == prefix_of(&k.info.interpreter))
            .filter(|_| k.info.origin.as_deref() == Some("env"))
            .unwrap_or(usize::MAX)
    };
    let mut all: Vec<DiscoveredKernel> = jupyter.into_iter().chain(env_kernels).collect();
    all.sort_by_key(env_rank);
    all.extend(builtin);

    let wanted = project_env
        .and_then(|e| e.python_interpreter.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(prefix_of);
    let active = wanted.and_then(|prefix| {
        all.iter()
            .position(|k| k.info.language == "python" && prefix_of(&k.info.interpreter) == prefix)
    });
    if let Some(k) = all.get_mut(active.unwrap_or(0)) {
        k.info.active = true;
    }
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(p: &Path, body: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    #[tokio::test]
    async fn discovery_reads_specs_synthesizes_ipykernel_and_marks_workspace_specs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        let data = tmp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        // Both venvs' bin/python link to one interpreter (BUG-38): still two envs.
        let real_py = tmp.path().join("base/bin/python3.12");
        write(&real_py, "");
        for v in [".venv", "venv"] {
            write(&root.join(v).join("pyvenv.cfg"), "version = 3.12.1\n");
            std::fs::create_dir_all(root.join(v).join("bin")).unwrap();
            #[cfg(unix)]
            std::os::unix::fs::symlink(&real_py, root.join(v).join("bin/python")).unwrap();
        }
        write(
            &root.join(".venv/lib/python3.12/site-packages/ipykernel/__init__.py"),
            "",
        );
        let std_spec = root.join(".venv/share/jupyter/kernels/python3/kernel.json");
        // ipykernel's wheel ships this spec with a bare `python`.
        write(
            &std_spec,
            r#"{"argv":["python","-m","ipykernel_launcher","-f","{connection_file}"],"display_name":"Python 3 (ipykernel)","language":"python","metadata":{"debugger":true}}"#,
        );
        write(
            &root.join(".venv/share/jupyter/kernels/proj-k/kernel.json"),
            r#"{"argv":["/bin/echo","{connection_file}"],"display_name":"Project K","language":"python","interrupt_mode":"message"}"#,
        );

        let found = discover_kernels(&root, &data, "local:proj", None).await;
        let venv_py = format!("{}/bin/python", root.join(".venv").to_string_lossy());
        let std_k = found
            .iter()
            .find(|k| k.info.display_name == "Python 3.12.1 (.venv) · ipykernel")
            .unwrap_or_else(|| panic!("env ipykernel: {found:#?}"));
        assert_eq!(std_k.info.kind, "jupyter");
        assert_eq!(std_k.info.origin.as_deref(), Some("env"));
        assert!(
            !std_k.info.needs_trust,
            "the env's own ipykernel spec doesn't ask"
        );
        assert!(std_k.info.debugger);
        let KernelLaunch::Jupyter(spec) = &std_k.launch else {
            panic!("jupyter launch")
        };
        assert_eq!(
            spec.argv[0], venv_py,
            "bare python means the env's interpreter"
        );
        assert!(std_k.info.active, "the first env kernel is active");
        assert!(
            !found
                .iter()
                .any(|k| k.info.id.starts_with("jupyter:ipykernel:")),
            "a registered spec means no synthesized one"
        );

        let ws = found
            .iter()
            .find(|k| k.info.display_name == "Project K")
            .expect("workspace spec");
        assert_eq!(ws.info.origin.as_deref(), Some("workspace"));
        assert!(ws.info.needs_trust);
        assert_eq!(
            ws.info.argv_preview.as_deref(),
            Some("/bin/echo {connection_file}")
        );
        let KernelLaunch::Jupyter(ws_spec) = &ws.launch else {
            panic!()
        };
        assert_eq!(ws_spec.interrupt_mode, InterruptMode::Message);

        let builtin: Vec<_> = found.iter().filter(|k| k.info.kind == "builtin").collect();
        assert!(
            builtin
                .iter()
                .any(|k| k.info.display_name == "Python 3.12.1 (venv) · built-in, no IPython"),
            "the env without ipykernel falls back to the built-in kernel: {found:#?}"
        );
        assert!(
            !builtin.iter().any(|k| k.info.interpreter == venv_py),
            "an env with ipykernel never offers the built-in kernel"
        );

        // Accepting the workspace spec is remembered for this project and contents.
        record_kernel_trust(&data, ws.trust_key.as_deref().unwrap()).unwrap();
        let again = discover_kernels(&root, &data, "local:proj", None).await;
        assert!(
            !again
                .iter()
                .find(|k| k.info.display_name == "Project K")
                .unwrap()
                .info
                .needs_trust
        );
        let other_project = discover_kernels(&root, &data, "local:other", None).await;
        assert!(
            other_project
                .iter()
                .find(|k| k.info.display_name == "Project K")
                .unwrap()
                .info
                .needs_trust
        );

        // Without a registered spec, ipykernel's own spec shape is synthesized.
        std::fs::remove_file(&std_spec).unwrap();
        let synth = discover_kernels(&root, &data, "local:proj", None).await;
        let k = synth
            .iter()
            .find(|k| k.info.id == format!("jupyter:ipykernel:{venv_py}"))
            .expect("synthesized ipykernel");
        let KernelLaunch::Jupyter(spec) = &k.launch else {
            panic!()
        };
        assert_eq!(spec.argv, ipykernel_argv(&venv_py));
    }

    #[tokio::test]
    async fn a_project_spec_that_runs_another_binary_of_the_env_asks_first() {
        // SEC6-6: same env prefix, ipykernel argv tail, no env, but not the env's python.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("proj");
        let data = tmp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        let real_py = tmp.path().join("base/bin/python3.12");
        write(&real_py, "");
        write(&root.join(".venv/pyvenv.cfg"), "version = 3.12.1\n");
        write(&root.join(".venv/bin/bash"), "");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real_py, root.join(".venv/bin/python")).unwrap();
        write(
            &root.join(".venv/lib/python3.12/site-packages/ipykernel/__init__.py"),
            "",
        );
        let bash = root.join(".venv/bin/bash").to_string_lossy().into_owned();
        write(
            &root.join(".venv/share/jupyter/kernels/python3/kernel.json"),
            &format!(
                r#"{{"argv":["{bash}","-m","ipykernel_launcher","-f","{{connection_file}}"],"display_name":"Python 3 (ipykernel)","language":"python"}}"#
            ),
        );
        let found = discover_kernels(&root, &data, "local:proj", None).await;
        let k = found
            .iter()
            .find(|k| k.info.interpreter == bash)
            .unwrap_or_else(|| panic!("the spec is listed: {found:#?}"));
        assert_eq!(k.info.origin.as_deref(), Some("workspace"));
        assert!(
            k.info.needs_trust,
            "a spec that is not the env's python asks first"
        );
    }
}
