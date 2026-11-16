use super::*;
use crate::types::{IdeGutterKind, IdeRunnerKind};

#[test]
fn go_detects_main_tests_and_benchmarks() {
    let main_src = "\
package main

import \"fmt\"

func main() {
    fmt.Println(\"hi\")
}
";
    let main_items = detect_gutter_items("/ws/cmd/app/main.go", main_src, &[]);
    assert_eq!(main_items.len(), 1);
    assert_eq!(main_items[0].line, 5);
    assert_eq!(main_items[0].kind, IdeGutterKind::Main);
    assert_eq!(main_items[0].run_spec.runner, IdeRunnerKind::GoRun);
    assert_eq!(main_items[0].run_spec.cwd.as_deref(), Some("/ws/cmd/app"));
    assert_eq!(
        main_items[0].run_spec.debug_program.as_deref(),
        Some("/ws/cmd/app/main.go"),
        "GoRun debug_program must target the main.go file path so sibling *_test.go files never force dlv test mode"
    );
    assert_eq!(main_items[0].run_spec.debug_test_name, None);

    let test_src = "\
package calc

import \"testing\"

func helper() {}

func TestAdd(t *testing.T) {
    if 1+1 != 2 { t.Fail() }
}

func BenchmarkAdd(b *testing.B) {
    for i := 0; i < b.N; i++ {}
}
";
    let items = detect_gutter_items("/ws/pkg/calc/calc_test.go", test_src, &[]);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].line, 7);
    assert_eq!(items[0].kind, IdeGutterKind::Test);
    assert_eq!(items[0].run_spec.test_filter.as_deref(), Some("^TestAdd$"));
    assert_eq!(
        items[0].run_spec.debug_program.as_deref(),
        Some("/ws/pkg/calc")
    );
    assert_eq!(items[1].line, 11);
    assert_eq!(items[1].kind, IdeGutterKind::Bench);
    assert_eq!(
        items[1].run_spec.test_filter.as_deref(),
        Some("^BenchmarkAdd$")
    );
}

#[test]
fn java_detects_package_main_junit_test_and_parameterized_test() {
    let src = "\
package com.acme.billing;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;

public class InvoiceServiceTest {
    public static void main(String[] args) {
        System.out.println(\"boot\");
    }

    @Test
    void computesTax() {
    }

    @ParameterizedTest
    @ValueSource(ints = {1, 2})
    public void validatesQuantities(int qty) {
    }
}
";
    let items = detect_gutter_items(
        "/ws/src/test/java/com/acme/billing/InvoiceServiceTest.java",
        src,
        &[],
    );
    assert_eq!(items.len(), 4);
    assert_eq!(items[0].line, 6);
    assert_eq!(items[0].kind, IdeGutterKind::Suite);
    assert_eq!(
        items[0].run_spec.test_filter.as_deref(),
        Some("com.acme.billing.InvoiceServiceTest")
    );

    assert_eq!(items[1].line, 7);
    assert_eq!(items[1].kind, IdeGutterKind::Main);
    assert_eq!(items[1].run_spec.runner, IdeRunnerKind::JavaMain);
    assert_eq!(
        items[1].run_spec.test_filter.as_deref(),
        Some("com.acme.billing.InvoiceServiceTest")
    );

    assert_eq!(items[2].line, 12);
    assert_eq!(items[2].kind, IdeGutterKind::Test);
    assert_eq!(
        items[2].run_spec.test_filter.as_deref(),
        Some("com.acme.billing.InvoiceServiceTest#computesTax")
    );

    assert_eq!(items[3].line, 17);
    assert_eq!(items[3].kind, IdeGutterKind::Test);
    assert_eq!(
        items[3].run_spec.test_filter.as_deref(),
        Some("com.acme.billing.InvoiceServiceTest#validatesQuantities")
    );
}

#[test]
fn rust_detects_main_unit_tests_tokio_tests_and_module_path() {
    let main_rs = "\
fn main() {
    println!(\"hello\");
}
";
    let main_items = detect_gutter_items("/ws/src/main.rs", main_rs, &[]);
    assert_eq!(main_items.len(), 1);
    assert_eq!(main_items[0].kind, IdeGutterKind::Main);
    assert_eq!(main_items[0].run_spec.runner, IdeRunnerKind::CargoRun);

    let mod_rs = "\
pub fn add(a: i32, b: i32) -> i32 { a + b }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_positive_numbers() {
        assert_eq!(add(2, 3), 5);
    }

    #[tokio::test]
    async fn async_roundtrip() {
        assert_eq!(add(1, 1), 2);
    }
}
";
    let items = detect_gutter_items("/ws/src/math/ops.rs", mod_rs, &[]);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].line, 8);
    assert_eq!(items[0].kind, IdeGutterKind::Test);
    assert_eq!(
        items[0].run_spec.test_filter.as_deref(),
        Some("math::ops::tests::adds_positive_numbers")
    );
    assert_eq!(items[1].line, 13);
    assert_eq!(
        items[1].run_spec.test_filter.as_deref(),
        Some("math::ops::tests::async_roundtrip")
    );
}

#[test]
fn python_detects_main_guard_test_classes_sync_and_async_tests() {
    let src = "\
def test_top_level():
    assert 1 == 1

class TestOrderFlow:
    def test_checkout(self):
        assert True

    async def test_webhook_delivery(self):
        assert True

if __name__ == \"__main__\":
    print('run')
";
    let items = detect_gutter_items("/ws/tests/test_orders.py", src, &[]);
    assert_eq!(items.len(), 5);
    assert_eq!(
        items[0].run_spec.test_filter.as_deref(),
        Some("/ws/tests/test_orders.py::test_top_level")
    );
    assert_eq!(items[1].kind, IdeGutterKind::Suite);
    assert_eq!(
        items[1].run_spec.test_filter.as_deref(),
        Some("/ws/tests/test_orders.py::TestOrderFlow")
    );
    assert_eq!(
        items[2].run_spec.test_filter.as_deref(),
        Some("/ws/tests/test_orders.py::TestOrderFlow::test_checkout")
    );
    assert_eq!(
        items[3].run_spec.test_filter.as_deref(),
        Some("/ws/tests/test_orders.py::TestOrderFlow::test_webhook_delivery")
    );
    assert_eq!(items[4].kind, IdeGutterKind::Main);
    assert_eq!(items[4].run_spec.runner, IdeRunnerKind::Python);
}

#[test]
fn js_ts_detects_describe_it_test_and_package_json_scripts() {
    let ts_src = "\
import { describe, it, test } from 'vitest';

describe('CartEngine', () => {
  it('computes subtotal', () => {});
  test('applies coupon code', async () => {});
});
";
    let items = detect_gutter_items("/ws/src/cart.test.ts", ts_src, &[]);
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].kind, IdeGutterKind::Suite);
    assert_eq!(items[0].run_spec.runner, IdeRunnerKind::Vitest);
    assert_eq!(items[1].kind, IdeGutterKind::Test);
    assert_eq!(
        items[1].run_spec.test_filter.as_deref(),
        Some("computes subtotal")
    );
    assert_eq!(
        items[2].run_spec.test_filter.as_deref(),
        Some("applies coupon code")
    );

    let pkg_json = r#"{
  "name": "demo",
  "scripts": {
    "dev": "vite",
    "test:unit": "vitest run"
  }
}"#;
    let pkg_items = detect_gutter_items("/ws/package.json", pkg_json, &[]);
    assert_eq!(pkg_items.len(), 2);
    assert_eq!(pkg_items[0].line, 4);
    assert_eq!(pkg_items[0].kind, IdeGutterKind::Main);
    assert_eq!(pkg_items[0].run_spec.runner, IdeRunnerKind::Npm);
    assert_eq!(pkg_items[1].line, 5);
    assert_eq!(pkg_items[1].kind, IdeGutterKind::Test);
}

#[test]
fn plugin_detectors_extend_gutter_for_kotlin_and_makefile() {
    let kotlin_det = PluginGutterDetector {
        language: "kotlin".into(),
        extensions: vec![".kt".into()],
        pattern: r"^\s*fun\s+(main)\s*\(".into(),
        kind: IdeGutterKind::Main,
        runner: IdeRunnerKind::Custom,
        command_template: "kotlinc".into(),
        args_template: vec!["{file}".into()],
        label_prefix: Some("Run Kotlin".into()),
    };
    let kt_items = detect_gutter_items(
        "/ws/src/App.kt",
        "package demo\nfun main() {\n  println(\"ok\")\n}\n",
        &[kotlin_det],
    );
    assert_eq!(kt_items.len(), 1);
    assert_eq!(kt_items[0].line, 2);
    assert_eq!(kt_items[0].label, "Run Kotlin main");
    assert_eq!(kt_items[0].run_spec.args, vec!["/ws/src/App.kt"]);
}

#[test]
fn go_subtests_shell_functions_bats_and_deno_json_tasks_detected_instantly() {
    let go_sub = "\
package admission

import \"testing\"

func TestAdmissionService(t *testing.T) {
    t.Run(\"allows vip player\", func(t *testing.T) {})
    t.Run(`rejects banned ip`, func(t *testing.T) {})
}
";
    let go_items = detect_gutter_items("/ws/service_test.go", go_sub, &[]);
    assert_eq!(go_items.len(), 3);
    assert_eq!(
        go_items[1].run_spec.test_filter.as_deref(),
        Some("^TestAdmissionService$/^allows_vip_player$")
    );
    assert_eq!(
        go_items[2].run_spec.test_filter.as_deref(),
        Some("^TestAdmissionService$/^rejects_banned_ip$")
    );

    let sh_src = "\
#!/usr/bin/env bash
set -euo pipefail

deploy_staging() {
  echo deploying
}

function rollback_release {
  echo rolling back
}
";
    let sh_items = detect_gutter_items("/ws/scripts/deploy.sh", sh_src, &[]);
    assert_eq!(sh_items.len(), 3);
    assert_eq!(sh_items[0].line, 1);
    assert_eq!(sh_items[0].run_spec.runner, IdeRunnerKind::Shell);
    assert_eq!(sh_items[1].label, "Run deploy_staging()");
    assert_eq!(sh_items[2].label, "Run rollback_release()");

    let bats_src = "\
#!/usr/bin/env bats
@test \"health endpoint returns 200\" {
  [ 1 -eq 1 ]
}
";
    let bats_items = detect_gutter_items("/ws/tests/smoke.bats", bats_src, &[]);
    assert_eq!(bats_items.len(), 2);
    assert_eq!(bats_items[0].kind, IdeGutterKind::Suite);
    assert_eq!(bats_items[1].kind, IdeGutterKind::Test);

    let deno_jsonc =
        "// deno config\n{\n  \"tasks\": {\n    \"dev\": \"deno run -A main.ts\"\n  }\n}";
    let deno_items = detect_gutter_items("/ws/deno.jsonc", deno_jsonc, &[]);
    assert_eq!(deno_items.len(), 1);
    assert_eq!(deno_items[0].label, "deno task dev");
}

#[test]
fn go_gutter_extracts_file_build_tags_into_args() {
    let go_comp = "\
//go:build component && !windows

package tests

import \"testing\"

func TestAssignFrbToPlayers(t *testing.T) {}
";
    let items = detect_gutter_items(
        "/ws/tests/assign_frb_to_players_component_test.go",
        go_comp,
        &[],
    );
    assert_eq!(items.len(), 1);
    assert!(
        items[0]
            .run_spec
            .args
            .contains(&"-tags=component".to_string()),
        "args={:?}",
        items[0].run_spec.args
    );
}

#[test]
fn plugin_registered_language_resolves_gutter_detector_without_hardcoded_match() {
    let tmp = tempfile::tempdir().unwrap();
    let plugin_dir = tmp.path().join("ide-plugins/elixir-pack");
    std::fs::create_dir_all(&plugin_dir).unwrap();
    std::fs::write(
        plugin_dir.join("otto-ide-plugin.json"),
        r#"{
            "slug": "elixir-pack",
            "name": "Elixir Pack",
            "version": "1.0.0",
            "contributions": {
                "languages": [{
                    "id": "elixir",
                    "display_name": "Elixir",
                    "extensions": [".exs"]
                }],
                "gutter_detectors": [{
                    "language": "elixir",
                    "extensions": [],
                    "pattern": "^\\s*test\\s+\"([^\"]+)\"",
                    "kind": "test",
                    "runner": "custom",
                    "command_template": "mix",
                    "args_template": ["test", "{file}"],
                    "label_prefix": "Mix"
                }]
            }
        }"#,
    )
    .unwrap();
    let reg = crate::plugins::IdePluginRegistry::with_dirs(tmp.path().to_path_buf(), None);
    reg.toggle("elixir-pack", true).unwrap();

    let src = "defmodule MathTest do\n  test \"adds two numbers\" do\n    assert 1 + 1 == 2\n  end\nend\n";
    let items = detect_gutter_items_with_plugins("/ws/test/math_test.exs", src, &reg);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].line, 2);
    assert_eq!(items[0].language, "elixir");
    assert_eq!(items[0].label, "Mix adds two numbers");
}
