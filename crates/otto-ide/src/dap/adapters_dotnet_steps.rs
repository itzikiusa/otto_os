//! Debug All Tests for .NET: one step per (test project, target framework).
//!
//! One `dotnet test` of a solution or a multi-targeted project starts a test
//! host per project and framework at once, and `VSTEST_HOST_DEBUG` pauses every
//! one of them (BUG-25). The plan builds once, reads each project, and gives
//! every (project, framework) its own step: VSTest steps run `dotnet test -f`
//! and attach to its one host; Microsoft.Testing.Platform steps launch the
//! built test app.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use otto_core::{Error, Result};

use super::super::adapters::locate_binary;
use super::super::run_target_command;
use super::super::session::{DapStartContext, DebugPlan, PlannedStep, StepKind};
use super::{
    detect_dotnet_test_debug_intent, dotnet_bin_candidates, resolve_dotnet_program_path,
    select_local_dotnet_test_projects, select_ssh_dotnet_test_projects, DotnetTestSelection,
};
use crate::types::{IdeStartDebugReq, IdeTargetKind};

/// The steps of an unfiltered .NET "Debug Tests" that covers more than one
/// (project, framework), or one Microsoft.Testing.Platform project; `None`
/// keeps today's single session.
pub(crate) async fn plan_dotnet_test_steps(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
) -> Result<Option<DebugPlan>> {
    let filtered = req
        .launch
        .test_name
        .as_deref()
        .is_some_and(|t| !t.trim().is_empty());
    let raw = req.launch.program.trim();
    if filtered
        || req.launch.test_framework.is_some()
        || req.launch.attach_pid.is_some()
        || raw.starts_with("unity://")
    {
        return Ok(None);
    }
    let norm = resolve_dotnet_program_path(&req.target, raw, cwd);
    if norm.ends_with(".dll") || detect_dotnet_test_debug_intent(req, ctx, &norm, cwd) != Some(None)
    {
        return Ok(None);
    }
    let sel = match req.target.kind {
        IdeTargetKind::Local => select_local_dotnet_test_projects(&norm, cwd),
        IdeTargetKind::Ssh => select_ssh_dotnet_test_projects(req, ctx, &norm, cwd, &env).await?,
    };
    if sel.tests.is_empty()
        || (sel.tests.len() == 1
            && req.target.kind == IdeTargetKind::Local
            && plainly_single_target(&sel.tests[0]))
    {
        return Ok(None);
    }
    let cands = dotnet_bin_candidates(data_dir, req.target.kind, &env);
    let Some(dotnet) = locate_binary(&req.target, ctx, &env, &cands).await else {
        return Ok(None);
    };
    let tool = DotnetTool {
        req,
        ctx,
        dotnet: &dotnet,
        cwd,
        env: &env,
    };
    let mut output = vec!["Reading test projects…".to_string()];
    build_once(&tool, &sel, &mut output).await?;
    let n = sel.tests.len();
    let mut steps = Vec::new();
    for (i, proj) in sel.tests.iter().enumerate() {
        output.push(format!("Reading test projects {}/{n}…", i + 1));
        let program = proj.to_string_lossy().into_owned();
        let name = proj
            .file_stem()
            .map_or_else(|| program.clone(), |s| s.to_string_lossy().into_owned());
        let read = tool
            .msbuild(
                proj,
                &[
                    "-getProperty:TargetFramework",
                    "-getProperty:TargetFrameworks",
                ],
            )
            .await
            .and_then(|out| parse_frameworks(&msbuild_props(&out)?));
        let frameworks = match read {
            Ok(f) => f,
            Err(err) => {
                let file = proj
                    .file_name()
                    .map_or(name.clone(), |f| f.to_string_lossy().into_owned());
                steps.push(PlannedStep {
                    label: name,
                    req: req.clone(),
                    kind: StepKind::VsTest,
                    error: Some(format!("Couldn't read {file}: {}", human(&err))),
                    program,
                });
                continue;
            }
        };
        for tfm in &frameworks {
            let label = format!("{name} ({tfm})");
            let mut step_req = req.clone();
            step_req.launch.debug_step = true;
            step_req.launch.test_framework = Some(tfm.clone());
            // Per framework: an outer (multi-target) evaluation can claim
            // `IsTestingPlatformApplication` for a VSTest project.
            let framework = format!("-p:TargetFramework={tfm}");
            let props = tool
                .msbuild(
                    proj,
                    &[
                        &framework,
                        "-getProperty:IsTestingPlatformApplication",
                        "-getProperty:TargetPath",
                    ],
                )
                .await
                .and_then(|out| msbuild_props(&out));
            let props = match props {
                Ok(p) => p,
                Err(err) => {
                    steps.push(PlannedStep {
                        label,
                        req: step_req,
                        kind: StepKind::VsTest,
                        error: Some(format!("Couldn't read {name} ({tfm}): {}", human(&err))),
                        program: program.clone(),
                    });
                    continue;
                }
            };
            let mtp = prop(&props, "IsTestingPlatformApplication").eq_ignore_ascii_case("true");
            if mtp {
                let target_path = prop(&props, "TargetPath");
                step_req.launch.program = target_path.clone();
                step_req.launch.is_test = false;
                steps.push(PlannedStep {
                    label,
                    req: step_req,
                    kind: StepKind::Mtp,
                    error: target_path
                        .is_empty()
                        .then(|| format!("Couldn't find the built test app of {name} ({tfm})")),
                    program: program.clone(),
                });
            } else {
                step_req.launch.program = program.clone();
                step_req.launch.is_test = true;
                steps.push(PlannedStep {
                    label,
                    req: step_req,
                    kind: StepKind::VsTest,
                    error: None,
                    program: program.clone(),
                });
            }
        }
    }
    if steps.len() == 1 && steps[0].kind == StepKind::VsTest && steps[0].error.is_none() {
        // One VSTest host: today's single session, which builds and attaches itself.
        return Ok(None);
    }
    let title = format!(
        "Debug all tests · {n} test project{}",
        if n == 1 { "" } else { "s" }
    );
    Ok(Some(DebugPlan {
        title,
        steps,
        output,
    }))
}

/// A project file naming one `<TargetFramework>` and no Microsoft.Testing.Platform
/// runner needs no plan (and no MSBuild query).
#[allow(clippy::disallowed_methods)] // one small project file read at debug session start
fn plainly_single_target(proj: &Path) -> bool {
    let xml = std::fs::read_to_string(proj).unwrap_or_default();
    !xml.contains("<TargetFrameworks>")
        && ![
            "EnableMSTestRunner",
            "UseMicrosoftTestingPlatformRunner",
            "TestingPlatformDotnetTestSupport",
            "IsTestingPlatformApplication",
            "EnableNUnitRunner",
            "UseMicrosoftTestingPlatform",
        ]
        .iter()
        .any(|m| xml.contains(m))
}

struct DotnetTool<'a> {
    req: &'a IdeStartDebugReq,
    ctx: &'a DapStartContext,
    dotnet: &'a str,
    cwd: &'a str,
    env: &'a BTreeMap<String, String>,
}

impl DotnetTool<'_> {
    async fn run(&self, args: Vec<String>, timeout: Duration) -> Result<(bool, String)> {
        let out = run_target_command(
            &self.req.target,
            self.ctx.ssh.as_ref(),
            self.dotnet,
            &args,
            self.cwd,
            self.env,
            timeout,
        )
        .await?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        Ok((out.status.success(), text))
    }

    /// `dotnet msbuild <proj> -getProperty:…` with the Debug configuration.
    async fn msbuild(&self, proj: &Path, props: &[&str]) -> Result<String> {
        let mut args: Vec<String> = ["msbuild", "-nologo", "-v:q", "-p:Configuration=Debug"]
            .map(String::from)
            .into();
        args.extend(props.iter().map(|p| p.to_string()));
        args.push(proj.to_string_lossy().into_owned());
        let (ok, text) = self.run(args, Duration::from_secs(60)).await?;
        if !ok {
            let first = text
                .lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("dotnet msbuild failed");
            return Err(Error::Invalid(first.to_string()));
        }
        Ok(text)
    }
}

/// One `dotnet build -c Debug` of the solution (or of each test project of a
/// folder) before step 1, instead of one build per step.
async fn build_once(
    tool: &DotnetTool<'_>,
    sel: &DotnetTestSelection,
    output: &mut Vec<String>,
) -> Result<()> {
    let targets: Vec<&PathBuf> = match &sel.solution {
        Some(sln) => vec![sln],
        None => sel.tests.iter().collect(),
    };
    for target in targets {
        let args = vec![
            "build".to_string(),
            "-c".to_string(),
            "Debug".to_string(),
            target.to_string_lossy().into_owned(),
        ];
        let (ok, text) = tool.run(args, Duration::from_secs(600)).await?;
        if !ok {
            return Err(Error::Invalid(format!(
                "dotnet test build failed ({}): {}",
                target.display(),
                text.trim()
            )));
        }
        output.extend(text.trim().lines().map(str::to_string));
    }
    Ok(())
}

/// The error's own text, without the `invalid:` prefix its `Display` adds.
fn human(err: &Error) -> String {
    match err {
        Error::Invalid(m) | Error::Upstream(m) | Error::Internal(m) | Error::NotFound(m) => {
            m.clone()
        }
        other => other.to_string(),
    }
}

/// Read `dotnet msbuild -getProperty` output (several properties print
/// `{"Properties": {…}}`).
pub(crate) fn msbuild_props(out: &str) -> Result<serde_json::Value> {
    let start = out.find('{').ok_or_else(|| {
        Error::Invalid(
            out.lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("no MSBuild properties")
                .to_string(),
        )
    })?;
    let json: serde_json::Value = serde_json::from_str(out[start..].trim())
        .map_err(|e| Error::Invalid(format!("unreadable MSBuild properties ({e})")))?;
    Ok(json["Properties"].clone())
}

fn prop(props: &serde_json::Value, key: &str) -> String {
    props[key].as_str().unwrap_or("").trim().to_string()
}

/// A project's target frameworks, in declared order.
pub(crate) fn parse_frameworks(props: &serde_json::Value) -> Result<Vec<String>> {
    let mut frameworks: Vec<String> = prop(props, "TargetFrameworks")
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    if frameworks.is_empty() {
        let single = prop(props, "TargetFramework");
        if single.is_empty() {
            return Err(Error::Invalid("no TargetFramework".into()));
        }
        frameworks.push(single);
    }
    Ok(frameworks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_frameworks_in_declared_order() {
        let multi = r#"{
  "Properties": {
    "TargetFramework": "",
    "TargetFrameworks": "net10.0;net11.0"
  }
}"#;
        let props = msbuild_props(multi).unwrap();
        assert_eq!(
            parse_frameworks(&props).unwrap(),
            vec!["net10.0".to_string(), "net11.0".to_string()]
        );
        let single = r#"{"Properties":{"TargetFramework":"net10.0","TargetFrameworks":"","IsTestingPlatformApplication":"true"}}"#;
        let props = msbuild_props(single).unwrap();
        assert_eq!(
            parse_frameworks(&props).unwrap(),
            vec!["net10.0".to_string()]
        );
        assert_eq!(prop(&props, "IsTestingPlatformApplication"), "true");
        let err = msbuild_props("MSBUILD : error MSB1009: Project file does not exist.\n")
            .unwrap_err()
            .to_string();
        assert!(err.contains("MSB1009"), "{err}");
    }
}
