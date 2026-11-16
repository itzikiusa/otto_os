//! Debugging a JUnit test: the JVM can't start a test class (it has no `main`), so
//! a tiny launcher compiled next to the module's classes runs the one selected test
//! (`com.acme.CalcTest#adds` or a whole class) through JUnit 5's Launcher API or
//! JUnit 4's `JUnitCore`, whichever the module's test classpath carries.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use otto_core::{Error, Result};

use super::builder::{run_javac, JavaExecutionPlan};
use super::resolve_m2_repo;

const JUNIT5: &str = r##"import static org.junit.platform.engine.discovery.DiscoverySelectors.*;

import java.io.PrintWriter;
import org.junit.platform.launcher.LauncherDiscoveryRequest;
import org.junit.platform.launcher.core.LauncherDiscoveryRequestBuilder;
import org.junit.platform.launcher.core.LauncherFactory;
import org.junit.platform.launcher.listeners.SummaryGeneratingListener;
import org.junit.platform.launcher.listeners.TestExecutionSummary;

public class OttoJUnitLauncher {
    public static void main(String[] args) {
        String[] sel = args[0].split("#", 2);
        LauncherDiscoveryRequest req = LauncherDiscoveryRequestBuilder.request()
            .selectors(sel.length > 1 ? selectMethod(sel[0], sel[1]) : selectClass(sel[0]))
            .build();
        SummaryGeneratingListener listener = new SummaryGeneratingListener();
        LauncherFactory.create().execute(req, listener);
        TestExecutionSummary s = listener.getSummary();
        s.printFailuresTo(new PrintWriter(System.err, true), 30);
        System.out.println("Tests run: " + s.getTestsFoundCount() + ", Failures: " + s.getTotalFailureCount());
        System.exit(s.getTotalFailureCount() == 0 && s.getTestsFoundCount() > 0 ? 0 : 1);
    }
}
"##;

const JUNIT4: &str = r##"import org.junit.runner.JUnitCore;
import org.junit.runner.Request;
import org.junit.runner.Result;
import org.junit.runner.notification.Failure;

public class OttoJUnitLauncher {
    public static void main(String[] args) throws Exception {
        String[] sel = args[0].split("#", 2);
        Class<?> cls = Class.forName(sel[0]);
        Request req = sel.length > 1 ? Request.method(cls, sel[1]) : Request.aClass(cls);
        Result r = new JUnitCore().run(req);
        for (Failure f : r.getFailures()) System.err.println(f.getTrace());
        System.out.println("Tests run: " + r.getRunCount() + ", Failures: " + r.getFailureCount());
        System.exit(r.wasSuccessful() && r.getRunCount() > 0 ? 0 : 1);
    }
}
"##;

/// Point `plan` at the JUnit launcher for `selector`; returns the launcher's
/// argv. A classpath with neither JUnit 5 nor JUnit 4 is left as it is.
pub fn use_junit_launcher(
    plan: &mut JavaExecutionPlan,
    selector: &str,
    javac_bin: &str,
    out_base_dir: &Path,
    env_map: &BTreeMap<String, String>,
) -> Result<Vec<String>> {
    let sep = if cfg!(windows) { ";" } else { ":" };
    let mut cp: Vec<String> = plan.classpath.split(sep).map(str::to_string).collect();
    let source = if let Some(engine) = cp.iter().find(|j| jar_name(j).starts_with("junit-platform-engine-")) {
        // Surefire supplies the Launcher API at test time, so a module rarely
        // declares it: take the one matching its platform version from ~/.m2.
        if !cp.iter().any(|j| jar_name(j).starts_with("junit-platform-launcher-")) {
            let version = jar_name(engine)
                .trim_start_matches("junit-platform-engine-")
                .trim_end_matches(".jar")
                .to_string();
            let jar = launcher_jar(env_map, &version).ok_or_else(|| {
                Error::NotFound(format!(
                    "junit-platform-launcher {version} is not in the local Maven repository; \
                     add it as a test dependency to debug JUnit 5 tests"
                ))
            })?;
            cp.push(jar.to_string_lossy().into_owned());
        }
        JUNIT5
    } else if cp.iter().any(|j| jar_name(j).starts_with("junit-4")) {
        JUNIT4
    } else {
        return Ok(Vec::new());
    };
    let dir = out_base_dir.join("otto-junit-launcher");
    fs::create_dir_all(&dir).map_err(|e| Error::Internal(format!("create launcher dir: {e}")))?;
    let src = dir.join("OttoJUnitLauncher.java");
    fs::write(&src, source).map_err(|e| Error::Internal(format!("write launcher: {e}")))?;
    let classpath = cp.join(sep);
    run_javac(javac_bin, &dir, &classpath, &[src], env_map, &mut Vec::new())?;
    plan.classpath = format!("{}{sep}{classpath}", dir.display());
    plan.main_class = "OttoJUnitLauncher".into();
    Ok(vec![selector.to_string()])
}

fn jar_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn launcher_jar(env_map: &BTreeMap<String, String>, version: &str) -> Option<PathBuf> {
    let jar = resolve_m2_repo(env_map)?
        .join("org/junit/platform/junit-platform-launcher")
        .join(version)
        .join(format!("junit-platform-launcher-{version}.jar"));
    jar.is_file().then_some(jar)
}
