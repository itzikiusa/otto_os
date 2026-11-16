//! Run / Debug matrix driven the way the IDE UI drives it: `POST /ide/gutter`
//! finds the test or `main`, then the gutter's `run_spec` becomes either an ad-hoc
//! `POST /ide/runs` (Run) or a `POST /ide/debug/sessions` launch (Debug), built
//! exactly like `launchGutterAction` in `ui/src/modules/ide/store/ideDebugAndEnvController.ts`.
//!
//! Every Debug cell sets two breakpoints inside the test / `main`, must pause on
//! each exact line with the expected local NAMES, and must then run to a clean
//! exit. A cell skips when its toolchain is missing (see `ensure_bins_or_skip`).
//!
//! Skips, with the reason:
//! - pytest: no offline install; its cells run only when `python3` already
//!   imports pytest.
//! - Jest / Vitest: no offline `npm install`; a fixture links the runner's tree
//!   from the npx cache (`~/.npm/_npx`) and skips when it isn't there.
//! - Kotlin: its gutter comes from the runtime plugin pack
//!   (`examples/ide-plugins/kotlin-ide`), not from the daemon.
//! - Bats (shell tests): only when `bats` is installed; the shell script cells
//!   cover the bash bridge.

// Fixture setup probes toolchains and caches synchronously, once per cell.
#![allow(clippy::disallowed_methods)]

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use axum::Router;
use common::{build_test_app, ensure_bins_or_skip, extended_path, find_bin, send_json};
use serde_json::{json, Value};

/// Budget for the first pause: it includes the build (go, javac + mvn, cargo).
const FIRST_PAUSE: Duration = Duration::from_secs(180);

/// Variable names only: a failure message must never print values.
fn var_names<'a>(vars: &[&'a Value]) -> Vec<&'a str> {
    vars.iter().filter_map(|v| v["name"].as_str()).collect()
}

/// A session snapshot without its variables (see [`var_names`]).
fn brief(sess: &Value) -> String {
    format!(
        "state={} paused_file={} paused_line={} title={} error={}",
        sess["state"], sess["paused_file"], sess["paused_line"], sess["title"], sess["error"]
    )
}

fn all_vars(snap: &Value) -> Vec<&Value> {
    snap["scopes"]
        .as_array()
        .map(|scopes| {
            scopes
                .iter()
                .filter_map(|sc| sc["variables"].as_array())
                .flat_map(|v| v.iter())
                .collect()
        })
        .unwrap_or_default()
}

struct Fixture {
    _tmp: tempfile::TempDir,
    app: Router,
    root: String,
    /// `proj2`, the multi-root project's second folder, when a file is written
    /// under `../proj2/`.
    second_root: Option<String>,
}

impl Fixture {
    /// A temp project written from `files` (relative path → content), with the
    /// project environment carrying PATH plus `env`.
    async fn new(files: &[(&str, &str)], env: &[(&str, &str)]) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root_path = tmp.path().join("proj");
        for (rel, content) in files {
            let p = root_path.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, content).unwrap();
        }
        let (_ctx, app) =
            build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
        let root = root_path.to_string_lossy().into_owned();
        let second = tmp.path().join("proj2");
        let second_root = second
            .is_dir()
            .then(|| second.to_string_lossy().into_owned());
        let mut env_vars =
            vec![json!({ "key": "PATH", "value": path_with_extras(), "secret": false })];
        for (k, v) in env {
            env_vars.push(json!({ "key": k, "value": v, "secret": false }));
        }
        let (st, body) = send_json(
            &app,
            Method::PUT,
            "/ide/environments",
            Some(json!({
                "target": { "kind": "local", "root": root },
                "env_vars": env_vars,
                "additional_roots": second_root.iter().collect::<Vec<_>>(),
            })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "PUT /ide/environments failed: {body}");
        Self {
            _tmp: tmp,
            app,
            root,
            second_root,
        }
    }

    /// `rel` under the project root, or under the second root as `../proj2/…`.
    fn path(&self, rel: &str) -> String {
        match (rel.strip_prefix("../proj2/"), &self.second_root) {
            (Some(r), Some(second)) => format!("{second}/{r}"),
            _ => format!("{}/{rel}", self.root),
        }
    }

    fn folders(&self) -> Vec<&String> {
        std::iter::once(&self.root)
            .chain(&self.second_root)
            .collect()
    }

    fn target(&self) -> Value {
        json!({ "kind": "local", "root": self.root, "additional_roots": self.second_root.iter().collect::<Vec<_>>() })
    }

    /// The gutter item of `rel` whose kind is `kind` and label contains `label`.
    async fn gutter(&self, rel: &str, kind: &str, label: &str) -> Value {
        let path = self.path(rel);
        let content = std::fs::read_to_string(&path).unwrap();
        let (st, body) = send_json(
            &self.app,
            Method::POST,
            "/ide/gutter",
            Some(json!({ "target": self.target(), "path": path, "content": content })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "gutter failed: {body}");
        let items = body["items"].as_array().cloned().unwrap_or_default();
        items
            .iter()
            .find(|i| i["kind"] == kind && i["label"].as_str().unwrap_or("").contains(label))
            .cloned()
            .unwrap_or_else(|| {
                let seen: Vec<String> = items
                    .iter()
                    .map(|i| format!("{}:{}", i["kind"], i["label"]))
                    .collect();
                panic!("no {kind} gutter item '{label}' in {rel}; got {seen:?}")
            })
    }

    /// A JS/TS script has no gutter entry: the toolbar's "Current file" target
    /// runs it with `runner` and debugs the file itself.
    fn current_file(&self, rel: &str, runner: &str, language: &str) -> Value {
        let path = self.path(rel);
        json!({
            "line": 1, "kind": "main", "label": "Current file", "language": language,
            "run_spec": { "runner": runner, "command": null, "args": [], "cwd": self.root,
                          "target_path": path, "debug_program": path },
        })
    }

    /// The gutter Run action: an ad-hoc run from `run_spec`. Asserts it passes
    /// and prints `marker`.
    async fn run(&self, item: &Value, marker: &str) {
        let spec = &item["run_spec"];
        let test_filter = spec["test_filter"].clone();
        let (st, body) = send_json(
            &self.app,
            Method::POST,
            "/ide/runs",
            Some(json!({
                "target": self.target(),
                "project_folders": self.folders(),
                "adhoc": {
                    "id": format!("gutter-{}", item["line"]),
                    "name": item["label"],
                    "runner": spec["runner"],
                    "command": spec["command"],
                    "args": spec["args"],
                    "cwd": spec["cwd"],
                    "env_overrides": {},
                    "inherit_shell_env": true,
                    "target_path": spec["target_path"],
                    "test_filter": test_filter,
                },
                "offline": true,
            })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "start run failed: {body}");
        let id = body["run"]["id"].as_str().unwrap().to_string();
        let deadline = Instant::now() + FIRST_PAUSE;
        let run = loop {
            let (_, poll) =
                send_json(&self.app, Method::GET, &format!("/ide/runs/{id}"), None).await;
            if poll["run"]["status"] != "running" {
                break poll["run"].clone();
            }
            assert!(Instant::now() < deadline, "run {id} timed out");
            tokio::time::sleep(Duration::from_millis(150)).await;
        };
        let out: Vec<&str> = run["output"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let out = out.join("\n");
        assert_eq!(
            run["status"], "passed",
            "run {} did not pass:\n{out}",
            item["label"]
        );
        assert!(out.contains(marker), "run output lacks {marker}:\n{out}");
    }

    /// The gutter Debug action: pauses on each `(line, local names)` stop in
    /// `rel`, then runs to a clean exit.
    async fn debug(&self, item: &Value, rel: &str, stops: &[(u32, &[&str])]) {
        let spec = &item["run_spec"];
        let program = spec["debug_program"]
            .as_str()
            .or(spec["target_path"].as_str())
            .unwrap();
        let test_name = spec["debug_test_name"]
            .as_str()
            .or(spec["test_filter"].as_str());
        let bp_path = self.path(rel);
        let bps: Vec<Value> = stops
            .iter()
            .map(|(l, _)| json!({ "path": bp_path, "line": l }))
            .collect();
        let (st, body) = send_json(
            &self.app,
            Method::POST,
            "/ide/debug/sessions",
            Some(json!({
                "target": self.target(),
                "project_folders": self.folders(),
                "language": item["language"],
                "launch": { "program": program, "args": [], "cwd": spec["cwd"], "test_name": test_name },
                "breakpoints": bps,
                "offline": true,
            })),
        )
        .await;
        assert_eq!(
            st,
            StatusCode::OK,
            "start debug failed: {}",
            brief(&body["session"])
        );
        let sid = body["session"]["id"].as_str().unwrap().to_string();
        let mut budget = FIRST_PAUSE;
        let mut left = Value::Null;
        for (line, names) in stops {
            // A NEW pause: not the line we just continued from.
            let snap = wait_for(&self.app, &sid, budget, |s| {
                s["state"] == "paused" && s["paused_line"] != left
            })
            .await;
            left = snap["paused_line"].clone();
            budget = Duration::from_secs(30);
            assert_eq!(snap["paused_line"], *line, "wrong pause: {}", brief(&snap));
            assert!(
                snap["paused_file"]
                    .as_str()
                    .unwrap_or("")
                    .ends_with(rel.trim_start_matches("../")),
                "paused in the wrong file: {}",
                brief(&snap)
            );
            let vars = all_vars(&snap);
            let have = var_names(&vars);
            for n in *names {
                assert!(
                    have.contains(n),
                    "local {n} missing at line {line}; locals {have:?}"
                );
            }
            let (st, b) = send_json(
                &self.app,
                Method::POST,
                &format!("/ide/debug/sessions/{sid}/command"),
                Some(json!({ "action": "continue" })),
            )
            .await;
            assert_eq!(
                st,
                StatusCode::OK,
                "continue failed: {}",
                brief(&b["session"])
            );
        }
        let end = wait_for(&self.app, &sid, Duration::from_secs(60), |s| {
            s["state"] == "terminated" || s["state"] == "error"
        })
        .await;
        assert_eq!(
            end["state"],
            "terminated",
            "debug ended badly: {}",
            brief(&end)
        );
    }
}

async fn wait_for(
    app: &Router,
    sid: &str,
    budget: Duration,
    done: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + budget;
    loop {
        let (st, body) = send_json(
            app,
            Method::GET,
            &format!("/ide/debug/sessions/{sid}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let sess = &body["session"];
        if done(sess) {
            return sess.clone();
        }
        assert!(
            !matches!(sess["state"].as_str(), Some("terminated" | "error")),
            "session ended early: {}",
            brief(sess)
        );
        assert!(Instant::now() < deadline, "timed out: {}", brief(sess));
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
}

/// PATH plus the SDKMAN Maven bin and cargo's bin, where this Mac keeps them.
fn path_with_extras() -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    format!(
        "{home}/.sdkman/candidates/maven/current/bin:{home}/.cargo/bin:{}",
        extended_path()
    )
}

fn have_mvn() -> bool {
    Path::new(&format!(
        "{}/.sdkman/candidates/maven/current/bin/mvn",
        std::env::var("HOME").unwrap_or_default()
    ))
    .is_file()
        || find_bin("mvn").is_some()
}

// ---------------------------------------------------------------- Go

const GO_MOD: &str =
    "module example.com/gm\n\ngo 1.21\n\nrequire github.com/stretchr/testify v1.11.1\n";
const GO_CALC: &str = "package calc\n\nfunc Add(a, b int) int {\n\treturn a + b\n}\n";
const GO_TEST: &str = r#"package calc

import (
	"testing"

	"github.com/stretchr/testify/assert"
)

func TestAddWorks(t *testing.T) {
	left := 20
	right := 22
	sum := Add(left, right)
	assert.Equal(t, 42, sum)
	t.Log("GO_TEST_DONE")
}
"#;
const GO_MAIN: &str = r#"package main

import (
	"fmt"

	"example.com/gm/calc"
)

func main() {
	base := 40
	total := calc.Add(base, 2)
	fmt.Println("GO_MAIN_DONE", total)
}
"#;

/// A test two packages deep (the user's `go_utilities/orm/sql` shape); with
/// `tag` it is a `//go:build <tag>` suite like the component tests.
fn go_nested_test(tag: Option<&str>, pkg: &str, name: &str) -> String {
    let header = tag.map_or(String::new(), |t| format!("//go:build {t}\n\n"));
    format!(
        "{header}package {pkg}\n\nimport \"testing\"\n\nfunc {name}(t *testing.T) {{\n\ttable := \"players\"\n\tid := 7\n\tquery := table + \"/\"\n\tif id == 0 {{\n\t\tt.Fatal(query)\n\t}}\n\tt.Log(\"GO_NESTED_DONE\", query)\n}}\n"
    )
}

async fn go_fixture() -> Fixture {
    let nested = go_nested_test(None, "sql", "TestDeleteSuccess");
    let tagged = go_nested_test(Some("component"), "flow", "TestFlowComponent");
    let second = go_nested_test(None, "deep", "TestSecondRoot");
    let at_root = go_nested_test(None, "gm", "TestRootPkg");
    Fixture::new(
        &[
            ("go.mod", GO_MOD),
            ("calc/calc.go", GO_CALC),
            ("calc/calc_test.go", GO_TEST),
            ("cmd/app/main.go", GO_MAIN),
            ("root_test.go", &at_root),
            ("orm/sql/query_builder_test.go", &nested),
            ("test/flow/flow_component_test.go", &tagged),
            ("../proj2/go.mod", "module example.com/second\n\ngo 1.21\n"),
            ("../proj2/svc/deep/deep_test.go", &second),
        ],
        // testify comes from the module cache: never reach the network.
        &[
            ("GOFLAGS", "-mod=mod"),
            ("GOPROXY", "off"),
            ("GOSUMDB", "off"),
        ],
    )
    .await
}

#[tokio::test]
async fn matrix_go_run_test() {
    if !ensure_bins_or_skip(&["go"]) {
        return;
    }
    let f = go_fixture().await;
    let item = f.gutter("calc/calc_test.go", "test", "TestAddWorks").await;
    f.run(&item, "GO_TEST_DONE").await;
}

#[tokio::test]
async fn matrix_go_debug_test() {
    if !ensure_bins_or_skip(&["go", "dlv"]) {
        return;
    }
    let f = go_fixture().await;
    let item = f.gutter("calc/calc_test.go", "test", "TestAddWorks").await;
    f.debug(
        &item,
        "calc/calc_test.go",
        &[(12, &["left", "right"]), (14, &["sum"])],
    )
    .await;
}

/// The gutter test items of the nested, tagged and second-root Go tests.
/// Their bodies share one shape: stop on `id := 7` (line `l`) and the `t.Log` line.
const GO_NESTED_CELLS: &[(&str, &str, u32)] = &[
    ("root_test.go", "TestRootPkg", 7),
    ("orm/sql/query_builder_test.go", "TestDeleteSuccess", 7),
    ("test/flow/flow_component_test.go", "TestFlowComponent", 9),
    ("../proj2/svc/deep/deep_test.go", "TestSecondRoot", 7),
];

#[tokio::test]
async fn matrix_go_run_nested_tagged_and_second_root_tests() {
    if !ensure_bins_or_skip(&["go"]) {
        return;
    }
    let f = go_fixture().await;
    for (rel, name, _) in GO_NESTED_CELLS {
        let item = f.gutter(rel, "test", name).await;
        f.run(&item, "GO_NESTED_DONE").await;
    }
}

#[tokio::test]
async fn matrix_go_debug_nested_tagged_and_second_root_tests() {
    if !ensure_bins_or_skip(&["go", "dlv"]) {
        return;
    }
    let f = go_fixture().await;
    for (rel, name, l) in GO_NESTED_CELLS {
        let item = f.gutter(rel, "test", name).await;
        f.debug(&item, rel, &[(*l, &["table"]), (*l + 5, &["query", "id"])])
            .await;
    }
}

#[tokio::test]
async fn matrix_go_run_main() {
    if !ensure_bins_or_skip(&["go"]) {
        return;
    }
    let f = go_fixture().await;
    let item = f.gutter("cmd/app/main.go", "main", "main").await;
    f.run(&item, "GO_MAIN_DONE 42").await;
}

#[tokio::test]
async fn matrix_go_debug_main() {
    if !ensure_bins_or_skip(&["go", "dlv"]) {
        return;
    }
    let f = go_fixture().await;
    let item = f.gutter("cmd/app/main.go", "main", "main").await;
    f.debug(
        &item,
        "cmd/app/main.go",
        &[(11, &["base"]), (12, &["total"])],
    )
    .await;
}

// ---------------------------------------------------------------- Java (multi-module Maven)

const POM_PARENT: &str = r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.acme</groupId><artifactId>parent</artifactId><version>1.0.0</version><packaging>pom</packaging>
  <modules><module>core</module><module>legacy</module></modules>
  <properties><maven.compiler.release>17</maven.compiler.release><project.build.sourceEncoding>UTF-8</project.build.sourceEncoding></properties>
  <build><pluginManagement><plugins>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-compiler-plugin</artifactId><version>3.13.0</version></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-resources-plugin</artifactId><version>3.3.1</version></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-surefire-plugin</artifactId><version>3.2.5</version></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-jar-plugin</artifactId><version>3.4.1</version></plugin>
    <plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-install-plugin</artifactId><version>3.1.2</version></plugin>
  </plugins></pluginManagement></build>
</project>
"#;
const POM_CORE: &str = r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <parent><groupId>com.acme</groupId><artifactId>parent</artifactId><version>1.0.0</version></parent>
  <artifactId>core</artifactId>
  <dependencies><dependency><groupId>org.junit.jupiter</groupId><artifactId>junit-jupiter</artifactId><version>5.10.5</version><scope>test</scope></dependency></dependencies>
</project>
"#;
// JUnit 4 runs on surefire 2.12.4's junit4 provider (the one cached offline).
const POM_LEGACY: &str = r#"<project xmlns="http://maven.apache.org/POM/4.0.0">
  <modelVersion>4.0.0</modelVersion>
  <parent><groupId>com.acme</groupId><artifactId>parent</artifactId><version>1.0.0</version></parent>
  <artifactId>legacy</artifactId>
  <dependencies><dependency><groupId>junit</groupId><artifactId>junit</artifactId><version>4.13.2</version><scope>test</scope></dependency></dependencies>
  <build><plugins><plugin><groupId>org.apache.maven.plugins</groupId><artifactId>maven-surefire-plugin</artifactId><version>2.12.4</version></plugin></plugins></build>
</project>
"#;
const JAVA_CALC: &str = r#"package com.acme.core;

public class Calc {
    public static int add(int a, int b) {
        return a + b;
    }

    public static void main(String[] args) {
        int base = 40;
        int total = add(base, 2);
        System.out.println("JAVA_MAIN_DONE " + total);
    }
}
"#;
const JAVA_CALC_TEST: &str = r#"package com.acme.core;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

class CalcTest {
    @Test
    void addsNumbers() {
        int left = 20;
        int sum = Calc.add(left, 22);
        assertEquals(42, sum);
        System.out.println("JUNIT5_DONE");
    }
}
"#;
const JAVA_OLD: &str = "package com.acme.legacy;\n\npublic class Old {\n    public static int twice(int n) {\n        return n * 2;\n    }\n}\n";
const JAVA_OLD_TEST: &str = r#"package com.acme.legacy;

import static org.junit.Assert.assertEquals;

import org.junit.Test;

public class OldTest {
    @Test
    public void doublesValue() {
        int seed = 21;
        int doubled = Old.twice(seed);
        assertEquals(42, doubled);
        System.out.println("JUNIT4_DONE");
    }
}
"#;
const CALC_TEST_REL: &str = "core/src/test/java/com/acme/core/CalcTest.java";
const OLD_TEST_REL: &str = "legacy/src/test/java/com/acme/legacy/OldTest.java";
const CALC_REL: &str = "core/src/main/java/com/acme/core/Calc.java";

async fn java_fixture() -> Fixture {
    Fixture::new(
        &[
            ("pom.xml", POM_PARENT),
            ("core/pom.xml", POM_CORE),
            ("legacy/pom.xml", POM_LEGACY),
            (CALC_REL, JAVA_CALC),
            (CALC_TEST_REL, JAVA_CALC_TEST),
            ("legacy/src/main/java/com/acme/legacy/Old.java", JAVA_OLD),
            (OLD_TEST_REL, JAVA_OLD_TEST),
        ],
        &[],
    )
    .await
}

fn java_ready() -> bool {
    ensure_bins_or_skip(&["java", "javac"]) && have_mvn()
}

#[tokio::test]
async fn matrix_java_junit5_run_test() {
    if !java_ready() {
        return;
    }
    let f = java_fixture().await;
    let item = f.gutter(CALC_TEST_REL, "test", "addsNumbers").await;
    f.run(&item, "JUNIT5_DONE").await;
}

#[tokio::test]
async fn matrix_java_junit5_debug_test() {
    if !java_ready() {
        return;
    }
    let f = java_fixture().await;
    let item = f.gutter(CALC_TEST_REL, "test", "addsNumbers").await;
    f.debug(&item, CALC_TEST_REL, &[(11, &["left"]), (12, &["sum"])])
        .await;
}

#[tokio::test]
async fn matrix_java_junit4_run_test() {
    if !java_ready() {
        return;
    }
    let f = java_fixture().await;
    let item = f.gutter(OLD_TEST_REL, "test", "doublesValue").await;
    f.run(&item, "JUNIT4_DONE").await;
}

#[tokio::test]
async fn matrix_java_junit4_debug_test() {
    if !java_ready() {
        return;
    }
    let f = java_fixture().await;
    let item = f.gutter(OLD_TEST_REL, "test", "doublesValue").await;
    f.debug(&item, OLD_TEST_REL, &[(11, &["seed"]), (12, &["doubled"])])
        .await;
}

#[tokio::test]
async fn matrix_java_run_main() {
    if !java_ready() {
        return;
    }
    let f = java_fixture().await;
    let item = f.gutter(CALC_REL, "main", "main").await;
    f.run(&item, "JAVA_MAIN_DONE 42").await;
}

#[tokio::test]
async fn matrix_java_debug_main() {
    if !java_ready() {
        return;
    }
    let f = java_fixture().await;
    let item = f.gutter(CALC_REL, "main", "main").await;
    f.debug(&item, CALC_REL, &[(10, &["base"]), (11, &["total"])])
        .await;
}

// ---------------------------------------------------------------- Rust

const CARGO_TOML: &str =
    "[package]\nname = \"rs-matrix\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";
const RS_MAIN: &str = r#"fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn main() {
    let base = 40;
    let total = add(base, 2);
    println!("RS_MAIN_DONE {total}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_numbers() {
        let left = 20;
        let sum = add(left, 22);
        assert_eq!(sum, 42);
        println!("RS_TEST_DONE");
    }
}
"#;

async fn rust_fixture() -> Fixture {
    Fixture::new(&[("Cargo.toml", CARGO_TOML), ("src/main.rs", RS_MAIN)], &[]).await
}

#[tokio::test]
async fn matrix_rust_run_test() {
    if !ensure_bins_or_skip(&["cargo"]) {
        return;
    }
    let f = rust_fixture().await;
    let item = f.gutter("src/main.rs", "test", "adds_numbers").await;
    f.run(&item, "RS_TEST_DONE").await;
}

#[tokio::test]
async fn matrix_rust_debug_test() {
    if !ensure_bins_or_skip(&["cargo", "lldb"]) {
        return;
    }
    let f = rust_fixture().await;
    let item = f.gutter("src/main.rs", "test", "adds_numbers").await;
    f.debug(&item, "src/main.rs", &[(18, &["left"]), (19, &["sum"])])
        .await;
}

#[tokio::test]
async fn matrix_rust_run_main() {
    if !ensure_bins_or_skip(&["cargo"]) {
        return;
    }
    let f = rust_fixture().await;
    let item = f.gutter("src/main.rs", "main", "main").await;
    f.run(&item, "RS_MAIN_DONE 42").await;
}

#[tokio::test]
async fn matrix_rust_debug_main() {
    if !ensure_bins_or_skip(&["cargo", "lldb"]) {
        return;
    }
    let f = rust_fixture().await;
    let item = f.gutter("src/main.rs", "main", "main").await;
    f.debug(&item, "src/main.rs", &[(7, &["base"]), (8, &["total"])])
        .await;
}

// ---------------------------------------------------------------- Python

const PY_MAIN: &str = r#"def add(a, b):
    return a + b


def main():
    base = 40
    total = add(base, 2)
    print("PY_MAIN_DONE", total)


if __name__ == "__main__":
    main()
"#;
const PY_TEST: &str = r#"from app import add


def test_adds_numbers():
    left = 20
    total = add(left, 22)
    assert total == 42
    print("PY_TEST_DONE")
"#;

async fn py_fixture() -> Fixture {
    Fixture::new(&[("app.py", PY_MAIN), ("test_app.py", PY_TEST)], &[]).await
}

/// pytest only when it is already importable (no offline install exists).
fn pytest_ready() -> bool {
    ensure_bins_or_skip(&["python3"])
        && std::process::Command::new("python3")
            .args(["-c", "import pytest"])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
}

#[tokio::test]
async fn matrix_python_run_pytest() {
    if !pytest_ready() {
        return;
    }
    let f = py_fixture().await;
    let item = f.gutter("test_app.py", "test", "test_adds_numbers").await;
    f.run(&item, "passed").await;
}

#[tokio::test]
async fn matrix_python_debug_pytest() {
    if !pytest_ready() {
        return;
    }
    let f = py_fixture().await;
    let item = f.gutter("test_app.py", "test", "test_adds_numbers").await;
    f.debug(&item, "test_app.py", &[(6, &["left"]), (7, &["total"])])
        .await;
}

#[tokio::test]
async fn matrix_python_run_script() {
    if !ensure_bins_or_skip(&["python3"]) {
        return;
    }
    let f = py_fixture().await;
    let item = f.gutter("app.py", "main", "__main__").await;
    f.run(&item, "PY_MAIN_DONE 42").await;
}

#[tokio::test]
async fn matrix_python_debug_script() {
    if !ensure_bins_or_skip(&["python3"]) {
        return;
    }
    let f = py_fixture().await;
    let item = f.gutter("app.py", "main", "__main__").await;
    f.debug(&item, "app.py", &[(7, &["base"]), (8, &["total"])])
        .await;
}

// ---------------------------------------------------------------- Shell

const SH_MAIN: &str = r#"#!/usr/bin/env bash
add() {
  echo $(( $1 + $2 ))
}
base=40
total=$(add "$base" 2)
echo "SH_MAIN_DONE $total"
"#;

#[tokio::test]
async fn matrix_shell_run_script() {
    if !ensure_bins_or_skip(&["bash"]) {
        return;
    }
    let f = Fixture::new(&[("run.sh", SH_MAIN)], &[]).await;
    let item = f.gutter("run.sh", "main", "run.sh").await;
    f.run(&item, "SH_MAIN_DONE 42").await;
}

#[tokio::test]
async fn matrix_shell_debug_script() {
    if !ensure_bins_or_skip(&["bash"]) {
        return;
    }
    let f = Fixture::new(&[("run.sh", SH_MAIN)], &[]).await;
    let item = f.gutter("run.sh", "main", "run.sh").await;
    f.debug(&item, "run.sh", &[(6, &["base"]), (7, &["total"])])
        .await;
}

// ---------------------------------------------------------------- JS / TS

const JS_MAIN: &str = r#"function add(a, b) {
  return a + b;
}

function main() {
  const base = 40;
  const total = add(base, 2);
  console.log('JS_MAIN_DONE', total);
}

main();
"#;
const TS_MAIN: &str = r#"function add(a: number, b: number): number {
  return a + b;
}

function main(): void {
  const base: number = 40;
  const total: number = add(base, 2);
  console.log('TS_MAIN_DONE', total);
}

main();
"#;
const VITEST_TEST: &str = r#"import { expect, test } from 'vitest';

function add(a: number, b: number): number {
  return a + b;
}

test('adds numbers', () => {
  const left = 20;
  const sum = add(left, 22);
  expect(sum).toBe(42);
  console.log('VITEST_DONE');
});
"#;
const JEST_TEST: &str = r#"function add(a, b) {
  return a + b;
}

test('adds numbers', () => {
  const left = 20;
  const sum = add(left, 22);
  expect(sum).toBe(42);
  console.log('JEST_DONE');
});
"#;

/// A `node_modules` holding `pkg` from the npx cache (`~/.npm/_npx/*`): there
/// is no offline `npm install`, so a fixture links to an already-fetched tree.
fn npx_cached_node_modules(pkg: &str) -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    std::fs::read_dir(Path::new(&home).join(".npm/_npx"))
        .ok()?
        .flatten()
        .map(|e| e.path().join("node_modules"))
        .find(|nm| nm.join(pkg).join("package.json").is_file())
}

async fn node_test_fixture(pkg: &str, file: &str, src: &str) -> Option<Fixture> {
    if !ensure_bins_or_skip(&["node"]) {
        return None;
    }
    let nm = npx_cached_node_modules(pkg)?;
    let manifest = format!(r#"{{"name":"m","type":"module","devDependencies":{{"{pkg}":"*"}}}}"#);
    let f = Fixture::new(&[("package.json", &manifest), (file, src)], &[]).await;
    std::os::unix::fs::symlink(nm, f.path("node_modules")).unwrap();
    Some(f)
}

#[tokio::test]
async fn matrix_ts_vitest_run_test() {
    let Some(f) = node_test_fixture("vitest", "src/calc.test.ts", VITEST_TEST).await else {
        return;
    };
    let item = f.gutter("src/calc.test.ts", "test", "adds numbers").await;
    f.run(&item, "VITEST_DONE").await;
}

#[tokio::test]
async fn matrix_ts_vitest_debug_test() {
    let Some(f) = node_test_fixture("vitest", "src/calc.test.ts", VITEST_TEST).await else {
        return;
    };
    let item = f.gutter("src/calc.test.ts", "test", "adds numbers").await;
    f.debug(&item, "src/calc.test.ts", &[(9, &["left"]), (10, &["sum"])])
        .await;
}

#[tokio::test]
async fn matrix_js_jest_run_test() {
    let Some(f) = node_test_fixture("jest", "calc.test.js", JEST_TEST).await else {
        return;
    };
    let item = f.gutter("calc.test.js", "test", "adds numbers").await;
    f.run(&item, "JEST_DONE").await;
}

#[tokio::test]
async fn matrix_js_jest_debug_test() {
    let Some(f) = node_test_fixture("jest", "calc.test.js", JEST_TEST).await else {
        return;
    };
    let item = f.gutter("calc.test.js", "test", "adds numbers").await;
    f.debug(&item, "calc.test.js", &[(7, &["left"]), (8, &["sum"])])
        .await;
}

#[tokio::test]
async fn matrix_js_run_main() {
    if !ensure_bins_or_skip(&["node"]) {
        return;
    }
    let f = Fixture::new(&[("app.js", JS_MAIN)], &[]).await;
    let item = f.current_file("app.js", "node", "javascript");
    f.run(&item, "JS_MAIN_DONE 42").await;
}

#[tokio::test]
async fn matrix_js_debug_main() {
    if !ensure_bins_or_skip(&["node"]) {
        return;
    }
    let f = Fixture::new(&[("app.js", JS_MAIN)], &[]).await;
    let item = f.current_file("app.js", "node", "javascript");
    f.debug(&item, "app.js", &[(7, &["base"]), (8, &["total"])])
        .await;
}

#[tokio::test]
async fn matrix_ts_run_main() {
    if !ensure_bins_or_skip(&["node"]) {
        return;
    }
    let f = Fixture::new(&[("app.ts", TS_MAIN)], &[]).await;
    let item = f.current_file("app.ts", "node", "typescript");
    f.run(&item, "TS_MAIN_DONE 42").await;
}

#[tokio::test]
async fn matrix_ts_debug_main() {
    if !ensure_bins_or_skip(&["node"]) {
        return;
    }
    let f = Fixture::new(&[("app.ts", TS_MAIN)], &[]).await;
    let item = f.current_file("app.ts", "node", "typescript");
    f.debug(&item, "app.ts", &[(7, &["base"]), (8, &["total"])])
        .await;
}

// ------------------------------------- nested folders and a second project root
// Debug must not depend on the file sitting at the project root: each language
// again with the file several folders deep (and Python / Go in a second root).

const RS_DEEP: &str = r#"pub fn mul(a: i32, b: i32) -> i32 {
    a * b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multiplies_deep() {
        let left = 6;
        let product = mul(left, 7);
        assert_eq!(product, 42);
        println!("RS_DEEP_DONE");
    }
}
"#;

async fn rust_deep_fixture() -> Fixture {
    Fixture::new(
        &[
            ("Cargo.toml", CARGO_TOML),
            ("src/lib.rs", "pub mod a;\n"),
            ("src/a/mod.rs", "pub mod b;\n"),
            ("src/a/b.rs", RS_DEEP),
        ],
        &[],
    )
    .await
}

#[tokio::test]
async fn matrix_rust_run_nested_test() {
    if !ensure_bins_or_skip(&["cargo"]) {
        return;
    }
    let f = rust_deep_fixture().await;
    let item = f.gutter("src/a/b.rs", "test", "multiplies_deep").await;
    f.run(&item, "RS_DEEP_DONE").await;
}

#[tokio::test]
async fn matrix_rust_debug_nested_test() {
    if !ensure_bins_or_skip(&["cargo", "lldb"]) {
        return;
    }
    let f = rust_deep_fixture().await;
    let item = f.gutter("src/a/b.rs", "test", "multiplies_deep").await;
    f.debug(&item, "src/a/b.rs", &[(12, &["left"]), (13, &["product"])])
        .await;
}

const PY_DEEP: &str = r#"def main():
    base = 40
    total = base + 2
    print("PY_DEEP_DONE", total)


if __name__ == "__main__":
    main()
"#;
const PY_DEEP_CELLS: &[&str] = &["scripts/sub/tool.py", "../proj2/pkg/deep/tool.py"];

#[tokio::test]
async fn matrix_python_run_and_debug_nested_and_second_root_scripts() {
    if !ensure_bins_or_skip(&["python3"]) {
        return;
    }
    let f = Fixture::new(
        &[(PY_DEEP_CELLS[0], PY_DEEP), (PY_DEEP_CELLS[1], PY_DEEP)],
        &[],
    )
    .await;
    for rel in PY_DEEP_CELLS {
        let item = f.gutter(rel, "main", "__main__").await;
        f.run(&item, "PY_DEEP_DONE 42").await;
        f.debug(&item, rel, &[(3, &["base"]), (4, &["total"])])
            .await;
    }
}

const JAVA_DEEP_TEST_REL: &str = "core/src/test/java/com/acme/core/deep/inner/DeepTest.java";
const JAVA_DEEP_TEST: &str = r#"package com.acme.core.deep.inner;

import static org.junit.jupiter.api.Assertions.assertEquals;

import com.acme.core.Calc;
import org.junit.jupiter.api.Test;

class DeepTest {
    @Test
    void addsDeep() {
        int left = 30;
        int sum = Calc.add(left, 12);
        assertEquals(42, sum);
        System.out.println("JUNIT5_DEEP_DONE");
    }
}
"#;

#[tokio::test]
async fn matrix_java_run_and_debug_test_in_a_deeper_package() {
    if !java_ready() {
        return;
    }
    let f = Fixture::new(
        &[
            ("pom.xml", POM_PARENT),
            ("core/pom.xml", POM_CORE),
            ("legacy/pom.xml", POM_LEGACY),
            (CALC_REL, JAVA_CALC),
            (JAVA_DEEP_TEST_REL, JAVA_DEEP_TEST),
            ("legacy/src/main/java/com/acme/legacy/Old.java", JAVA_OLD),
        ],
        &[],
    )
    .await;
    let item = f.gutter(JAVA_DEEP_TEST_REL, "test", "addsDeep").await;
    f.run(&item, "JUNIT5_DEEP_DONE").await;
    f.debug(
        &item,
        JAVA_DEEP_TEST_REL,
        &[(12, &["left"]), (13, &["sum"])],
    )
    .await;
}

#[tokio::test]
async fn matrix_ts_run_and_debug_nested_main() {
    if !ensure_bins_or_skip(&["node"]) {
        return;
    }
    let rel = "src/deep/inner/app.ts";
    let f = Fixture::new(&[(rel, TS_MAIN)], &[]).await;
    let item = f.current_file(rel, "node", "typescript");
    f.run(&item, "TS_MAIN_DONE 42").await;
    f.debug(&item, rel, &[(7, &["base"]), (8, &["total"])])
        .await;
}
