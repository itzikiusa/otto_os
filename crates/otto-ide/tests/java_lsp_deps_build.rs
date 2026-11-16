//! Integration tests for stateless Java & polyglot diagnostics (`POST /ide/lsp/diagnostics`),
//! single-symbol hover (`POST /ide/lsp/hover`), concrete quick-fix edits,
//! build tools model (`POST /ide/build-tools/model`), manifest dependency inspector & actions
//! (`POST /ide/deps/inspect`, `POST /ide/deps/action`), and multi-repo Java workspace
//! precedence over `~/.m2` JARs.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::{build_test_app, send_json};
use serde_json::{json, Value};
use tokio::process::Command;

#[tokio::test]
async fn java_and_polyglot_diagnostics_quickfixes_and_clean_hover_work_end_to_end() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("otto-data");
    let db_path = tmp.path().join("state.db");
    let ws = tmp.path().join("java-svc");
    let pkg_dir = ws.join("src/main/java/com/acme/order");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(
        ws.join("pom.xml"),
        "<project><modelVersion>4.0.0</modelVersion><groupId>com.acme</groupId><artifactId>java-svc</artifactId><version>1.0.0</version></project>\n",
    )
    .unwrap();

    let order_file = pkg_dir.join("OrderProcessor.java");
    let java_src = r#"package com.acme.order;

import lombok.Data;
import lombok.extern.slf4j.Slf4j;
import foo.bar.DoesNotExist;
import java.util.Optional;

@Slf4j
@Data
public class OrderProcessor {
    private final String tenantId;

    public int process(List<String> items) {
        log.info("processing items");
        DoesNotExist missing = null;
        return items.size();
    }
}
"#;
    std::fs::write(&order_file, java_src).unwrap();

    let (_ctx, app) = build_test_app(data_dir, &db_path).await;
    let ws_str = ws.to_string_lossy().into_owned();
    let order_path = order_file.to_string_lossy().into_owned();

    // 1. Java diagnostics: unresolved import + unresolved body type + missing List import candidate
    //    + unused import warning + zero false positives on Lombok `@Slf4j` / `log.info(...)`:
    let (st_diag, diag_body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/diagnostics",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": order_path,
            "content": java_src,
            "language": "java",
        })),
    )
    .await;
    assert_eq!(
        st_diag,
        StatusCode::OK,
        "java diagnostics failed: {diag_body}"
    );
    let diags = diag_body["diagnostics"].as_array().unwrap();

    let unres_imp = diags
        .iter()
        .find(|d| {
            d["code"] == "unresolved_import"
                && d["message"].as_str().unwrap().contains("DoesNotExist")
        })
        .expect("expected unresolved_import on foo.bar.DoesNotExist");
    assert_eq!(unres_imp["severity"], "error");
    assert_eq!(unres_imp["source"], "otto-java");
    assert_eq!(unres_imp["quick_fixes"][0]["delete_line"], true);
    assert!(
        !unres_imp["quick_fixes"][0]["edits"]
            .as_array()
            .unwrap()
            .is_empty(),
        "quick_fix must carry concrete 1-based IdeTextEdit"
    );

    assert!(
        diags.iter().any(|d| d["code"] == "unresolved_type"
            && d["message"].as_str().unwrap().contains("DoesNotExist")),
        "expected unresolved_type on body usage of DoesNotExist: {diags:?}"
    );
    let list_diag = diags
        .iter()
        .find(|d| {
            d["code"] == "unresolved_type" && d["message"].as_str().unwrap().contains("'List'")
        })
        .expect("expected unresolved_type on unimported List");
    assert!(
        list_diag["quick_fixes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|qf| qf["title"] == "Import 'java.util.List'"
                && !qf["edits"].as_array().unwrap().is_empty()),
        "expected Import 'java.util.List' quick fix with concrete edits: {list_diag}"
    );
    assert!(
        diags.iter().any(|d| d["code"] == "unused_import"
            && d["severity"] == "warning"
            && d["message"]
                .as_str()
                .unwrap()
                .contains("java.util.Optional")),
        "expected unused_import warning on Optional: {diags:?}"
    );
    assert!(
        !diags.iter().any(|d| {
            let m = d["message"].as_str().unwrap_or("");
            m.contains("Slf4j") || m.contains("lombok") || m.contains("'log'")
        }),
        "Lombok @Slf4j/@Data and synthesized log must produce zero errors: {diags:?}"
    );

    // 2. Polyglot diagnostics: JSON exact line/col, Python SyntaxError, Shell syntax error:
    let (st_json, json_body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/diagnostics",
        Some(json!({
            "path": "broken.json",
            "content": "{\n  \"ok\": true,\n  \"bad\": \n}\n",
            "language": "json",
        })),
    )
    .await;
    assert_eq!(st_json, StatusCode::OK);
    assert_eq!(json_body["diagnostics"][0]["severity"], "error");

    let (st_py, py_body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/diagnostics",
        Some(json!({
            "path": "broken.py",
            "content": "def calc(x):\n    if x > 0\n        return x\n",
            "language": "python",
        })),
    )
    .await;
    assert_eq!(st_py, StatusCode::OK);
    assert_eq!(py_body["diagnostics"][0]["line"], 2);

    let (st_sh, sh_body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/diagnostics",
        Some(json!({
            "path": "broken.sh",
            "content": "#!/usr/bin/env bash\nif [ 1 -eq 1 ]; then\n  echo hi\n",
            "language": "shell",
        })),
    )
    .await;
    assert_eq!(st_sh, StatusCode::OK);
    assert!(!sh_body["diagnostics"].as_array().unwrap().is_empty());

    // 3. `POST /ide/lsp/code-actions` returns concrete 1-based `edits`:
    let go_src = "package main\n\nfunc run() error {\n\tres, err := call()\n\treturn err\n}\n";
    let (st_ca, ca_body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/code-actions",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": "main.go",
            "content": go_src,
            "line": 4,
            "character": 5,
        })),
    )
    .await;
    assert_eq!(st_ca, StatusCode::OK);
    let actions = ca_body["actions"].as_array().unwrap();
    assert!(actions
        .iter()
        .any(|a| !a["edits"].as_array().unwrap().is_empty()));

    // 4. `POST /ide/lsp/hover` isolates grouped Go var/const docs without neighbor bleed:
    let decimal_go = ws.join("decimal.go");
    let decimal_src = r#"package decimal

var (
	// DivisionPrecision is the number of decimal places in the result when it
	// doesn't divide exactly.
	DivisionPrecision = 16

	// PowPrecisionNegativeExponent specifies the maximum precision when computing
	// negative exponents.
	PowPrecisionNegativeExponent = 16
)

func UsePrecision() int {
	return DivisionPrecision
}
"#;
    std::fs::write(&decimal_go, decimal_src).unwrap();
    let (st_hov, hov_body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/hover",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": "decimal.go",
            "content": decimal_src,
            "line": 14,
            "character": 12,
        })),
    )
    .await;
    assert_eq!(st_hov, StatusCode::OK);
    assert_eq!(hov_body["found"], true);
    assert_eq!(hov_body["symbol"], "DivisionPrecision");
    assert_eq!(hov_body["package_name"], "decimal");
    let full_doc = hov_body["full_doc"].as_str().unwrap_or("");
    assert!(full_doc.contains("DivisionPrecision is the number of decimal places"));
    assert!(
        !full_doc.contains("PowPrecisionNegativeExponent"),
        "hover leaked neighbor doc: {hov_body}"
    );
}

#[tokio::test]
async fn build_tools_model_and_manifest_deps_inspect_and_action_record_history() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("otto-data");
    let db_path = tmp.path().join("state.db");
    let root = tmp.path().join("platform-parent");
    let child = root.join("orders-service");
    std::fs::create_dir_all(&child).unwrap();

    std::fs::write(
        root.join("pom.xml"),
        r#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.acme</groupId>
  <artifactId>platform-parent</artifactId>
  <version>1.0.0</version>
  <packaging>pom</packaging>
  <modules>
    <module>orders-service</module>
  </modules>
  <profiles>
    <profile><id>prod</id></profile>
  </profiles>
  <dependencyManagement>
    <dependencies>
      <dependency>
        <groupId>com.google.guava</groupId>
        <artifactId>guava</artifactId>
        <version>32.1.2-jre</version>
      </dependency>
    </dependencies>
  </dependencyManagement>
</project>
"#,
    )
    .unwrap();

    let child_pom = child.join("pom.xml");
    std::fs::write(
        &child_pom,
        r#"<project>
  <parent>
    <groupId>com.acme</groupId>
    <artifactId>platform-parent</artifactId>
    <version>1.0.0</version>
    <relativePath>../pom.xml</relativePath>
  </parent>
  <artifactId>orders-service</artifactId>
  <properties>
    <slf4j.version>2.0.7</slf4j.version>
  </properties>
  <dependencies>
    <dependency>
      <groupId>org.slf4j</groupId>
      <artifactId>slf4j-api</artifactId>
      <version>${slf4j.version}</version>
    </dependency>
    <dependency>
      <groupId>com.google.guava</groupId>
      <artifactId>guava</artifactId>
    </dependency>
  </dependencies>
</project>
"#,
    )
    .unwrap();
    std::fs::write(
        root.join("build.gradle.kts"),
        "plugins { java }\ndependencies {\n    implementation(\"io.netty:netty-all:4.1.100.Final\")\n}\n",
    )
    .unwrap();

    let (_ctx, app) = build_test_app(data_dir, &db_path).await;
    let root_str = root.to_string_lossy().into_owned();

    // 1. `POST /ide/build-tools/model`:
    let (st_bt, bt_body) = send_json(
        &app,
        Method::POST,
        "/ide/build-tools/model",
        Some(json!({
            "target": { "kind": "local", "root": root_str },
            "roots": [root_str],
        })),
    )
    .await;
    assert_eq!(st_bt, StatusCode::OK, "build-tools failed: {bt_body}");
    assert_eq!(bt_body["has_maven"], true);
    assert_eq!(bt_body["has_gradle"], true);
    assert!(bt_body["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["id"] == "prod"));

    // 2. `POST /ide/deps/inspect` on child `pom.xml`:
    let (st_ins, ins_body) = send_json(
        &app,
        Method::POST,
        "/ide/deps/inspect",
        Some(json!({
            "target": { "kind": "local", "root": root_str },
            "path": "orders-service/pom.xml",
        })),
    )
    .await;
    assert_eq!(st_ins, StatusCode::OK, "deps inspect failed: {ins_body}");
    let items = ins_body["items"].as_array().unwrap();
    let slf4j = items
        .iter()
        .find(|i| i["name"] == "org.slf4j:slf4j-api")
        .expect("slf4j-api item");
    assert_eq!(slf4j["current_version"], "2.0.7");
    assert_eq!(slf4j["property_name"], "slf4j.version");

    let guava = items
        .iter()
        .find(|i| i["name"] == "com.google.guava:guava")
        .expect("guava item");
    assert_eq!(guava["current_version"], "32.1.2-jre");
    assert!(guava["managed_in"]["path"]
        .as_str()
        .unwrap()
        .ends_with("platform-parent/pom.xml"));

    // 3. `POST /ide/deps/action` (`update_single` -> updates `<slf4j.version>` & snapshots Local History):
    let (st_act, act_body) = send_json(
        &app,
        Method::POST,
        "/ide/deps/action",
        Some(json!({
            "target": { "kind": "local", "root": root_str },
            "path": "orders-service/pom.xml",
            "action": "update_single",
            "module": "org.slf4j:slf4j-api",
            "version": "2.0.16",
        })),
    )
    .await;
    assert_eq!(st_act, StatusCode::OK, "deps action failed: {act_body}");
    assert!(act_body["updated_content"]
        .as_str()
        .unwrap()
        .contains("<slf4j.version>2.0.16</slf4j.version>"));

    let (st_hist, hist_body) = send_json(
        &app,
        Method::GET,
        &format!(
            "/ide/history/list?root={}&path=orders-service/pom.xml",
            common::urlencode(&root_str)
        ),
        None,
    )
    .await;
    assert_eq!(st_hist, StatusCode::OK);
    assert!(hist_body["entries"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["event_kind"] == "before_deps_action"));
}

#[tokio::test]
async fn multi_repo_java_prefers_attached_local_module_over_m2_jar_and_falls_back_when_detached() {
    if Command::new("javac")
        .arg("-version")
        .output()
        .await
        .is_err()
        || Command::new("jar").arg("--version").output().await.is_err()
    {
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("otto-data");
    let db_path = tmp.path().join("state.db");
    let m2_repo = tmp.path().join("m2-repo");

    // 1. Build v1 JAR (`"M2_JAR_V1"`) into `<m2_repo>/com/acme/toolkit/1.0.0/toolkit-1.0.0.jar`:
    let v1_src_dir = tmp.path().join("v1-build/src/com/acme/tk");
    let v1_cls_dir = tmp.path().join("v1-build/classes");
    std::fs::create_dir_all(&v1_src_dir).unwrap();
    std::fs::create_dir_all(&v1_cls_dir).unwrap();
    let v1_java = v1_src_dir.join("Greeting.java");
    std::fs::write(
        &v1_java,
        "package com.acme.tk;\npublic class Greeting {\n    public static String banner() { return \"M2_JAR_V1\"; }\n}\n",
    )
    .unwrap();
    let st_javac = Command::new("javac")
        .arg("-g")
        .arg("-d")
        .arg(&v1_cls_dir)
        .arg(&v1_java)
        .status()
        .await
        .unwrap();
    assert!(st_javac.success());
    let jar_dest = m2_repo.join("com/acme/toolkit/1.0.0/toolkit-1.0.0.jar");
    std::fs::create_dir_all(jar_dest.parent().unwrap()).unwrap();
    let st_jar = Command::new("jar")
        .arg("cf")
        .arg(&jar_dest)
        .arg("-C")
        .arg(&v1_cls_dir)
        .arg(".")
        .status()
        .await
        .unwrap();
    assert!(st_jar.success());

    // 2. Create local `toolkit` v2 (`"LOCAL_TOOLKIT_V2"`) and `service-app` depending on `com.acme:toolkit:1.0.0`:
    let toolkit = tmp.path().join("toolkit");
    std::fs::create_dir_all(toolkit.join("src/main/java/com/acme/tk")).unwrap();
    std::fs::write(
        toolkit.join("pom.xml"),
        "<project><modelVersion>4.0.0</modelVersion><groupId>com.acme</groupId><artifactId>toolkit</artifactId><version>1.0.0</version></project>\n",
    )
    .unwrap();
    std::fs::write(
        toolkit.join("src/main/java/com/acme/tk/Greeting.java"),
        "package com.acme.tk;\npublic class Greeting {\n    public static String banner() { return \"LOCAL_TOOLKIT_V2\"; }\n}\n",
    )
    .unwrap();

    let service_app = tmp.path().join("service-app");
    std::fs::create_dir_all(service_app.join("src/main/java/com/acme/app")).unwrap();
    std::fs::write(
        service_app.join("pom.xml"),
        r#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.acme</groupId>
  <artifactId>service-app</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency>
      <groupId>com.acme</groupId>
      <artifactId>toolkit</artifactId>
      <version>1.0.0</version>
    </dependency>
  </dependencies>
</project>
"#,
    )
    .unwrap();
    let app_main_src = r#"package com.acme.app;

import com.acme.tk.Greeting;

public class AppMain {
    public static void main(String[] args) {
        System.out.println(Greeting.banner());
    }
}
"#;
    let app_main = service_app.join("src/main/java/com/acme/app/AppMain.java");
    std::fs::write(&app_main, app_main_src).unwrap();

    let (_ctx, app) = build_test_app(data_dir, &db_path).await;
    let svc_str = service_app.to_string_lossy().into_owned();
    let tk_str = toolkit.to_string_lossy().into_owned();

    // Configure `MAVEN_REPO_LOCAL` on `service-app` environment:
    let (st_env, _) = send_json(
        &app,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": svc_str },
            "vars": [{ "key": "MAVEN_REPO_LOCAL", "value": m2_repo.to_string_lossy(), "secret": false }],
        })),
    )
    .await;
    assert_eq!(st_env, StatusCode::OK);

    // 3A. With `toolkit` attached in `project_folders`: Go-to-Definition jumps to editable local `toolkit`:
    let (st_def, def_body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/definition",
        Some(json!({
            "target": { "kind": "local", "root": svc_str },
            "project_folders": [tk_str],
            "path": app_main.to_string_lossy(),
            "content": app_main_src,
            "line": 7,
            "character": 29,
        })),
    )
    .await;
    assert_eq!(st_def, StatusCode::OK);
    let locs = def_body["locations"].as_array().unwrap();
    assert!(
        !locs.is_empty(),
        "expected definition location, got {def_body}"
    );
    assert!(
        locs[0]["path"]
            .as_str()
            .unwrap()
            .contains("toolkit/src/main/java/com/acme/tk/Greeting.java"),
        "expected jump to local toolkit Greeting.java, got {locs:?}"
    );
    assert!(
        !locs[0]["path"].as_str().unwrap().contains(".m2"),
        "local attached module must win over .m2 JAR: {locs:?}"
    );

    // 3B. Running `AppMain.java` with `project_folders: [toolkit]` prints `"LOCAL_TOOLKIT_V2"`:
    let out_attached =
        run_java_main_and_collect_output(&app, &svc_str, &app_main.to_string_lossy(), vec![tk_str])
            .await;
    assert!(
        out_attached.contains("LOCAL_TOOLKIT_V2"),
        "expected LOCAL_TOOLKIT_V2 when toolkit attached, got: {out_attached}"
    );

    // 3C. Running `AppMain.java` without `toolkit` (`project_folders: []`) falls back to `~/.m2` JAR (`"M2_JAR_V1"`):
    let out_detached =
        run_java_main_and_collect_output(&app, &svc_str, &app_main.to_string_lossy(), Vec::new())
            .await;
    assert!(
        out_detached.contains("M2_JAR_V1"),
        "expected M2_JAR_V1 when toolkit detached, got: {out_detached}"
    );
}

async fn run_java_main_and_collect_output(
    app: &axum::Router,
    root: &str,
    main_path: &str,
    project_folders: Vec<String>,
) -> String {
    let (st, start_body) = send_json(
        app,
        Method::POST,
        "/ide/runs",
        Some(json!({
            "target": { "kind": "local", "root": root },
            "project_folders": project_folders,
            "adhoc": {
                "name": "Run AppMain",
                "runner": "java",
                "working_dir": root,
                "target_path": main_path,
                "symbol": "main",
            }
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "start_run failed: {start_body}");
    let run_id = start_body["run"]["id"].as_str().unwrap().to_string();
    for _ in 0..80 {
        let (_, poll_body): (_, Value) =
            send_json(app, Method::GET, &format!("/ide/runs/{run_id}"), None).await;
        let status = poll_body["run"]["status"].as_str().unwrap_or("");
        if status != "running" {
            let lines: Vec<&str> = poll_body["run"]["output"]
                .as_array()
                .map(|arr| arr.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            return lines.join("\n");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("timed out waiting for Java run {run_id}");
}
