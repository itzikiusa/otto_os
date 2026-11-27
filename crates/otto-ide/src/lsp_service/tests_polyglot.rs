//! Regression unit tests for RFC 4180 multiline CSV/TSV formatting, UTF-16 JSON
//! diagnostic columns, Go import sorting on format/code-actions, flexible hover
//! request shapes, and workspace-confined definition navigation.

use super::*;
use crate::types::{IdeHoverPosition, IdeHoverReq};

#[test]
fn native_csv_and_tsv_formatting_preserves_rfc4180_multiline_cells_and_quoted_spaces() {
    let csv_src = "id,notes,padded\n1,\"first line\nsecond line, with comma\",\"  keep  \"\n2,plain,trim_me  \n";
    let formatted = format_builtin_fallback("csv", csv_src);

    // Must remain 3 logical RFC 4180 records (header + 2 rows), never splitting the embedded newline!
    assert!(
        formatted.contains("\"first line\nsecond line, with comma\""),
        "multiline quoted CSV cell must stay intact: {formatted:?}"
    );
    assert!(
        formatted.contains("\"  keep  \""),
        "quoted whitespace inside CSV cell must not be trimmed: {formatted:?}"
    );
    assert!(
        formatted.contains("2,plain,trim_me\n"),
        "unquoted trailing spaces should still normalize cleanly: {formatted:?}"
    );

    let tsv_src = "col_a\tcol_b\n\"multi\nline\tcell\"\t\"   \"\n";
    let formatted_tsv = format_builtin_fallback("tsv", tsv_src);
    assert!(
        formatted_tsv.contains("\"multi\nline\tcell\"\t\"   \""),
        "multiline and whitespace-quoted TSV cells must stay intact: {formatted_tsv:?}"
    );
}

#[test]
fn json_diagnostics_report_utf16_character_columns_after_multibyte_utf8() {
    // Prefix before invalid token `bad` contains Hebrew (2-byte UTF-8, 1 UTF-16 unit)
    // and rocket emoji (4-byte UTF-8, 2 UTF-16 units).
    let prefix = "{\"title\": \"שלום 🚀\", ";
    let broken_json = format!("{prefix}bad}}\n");
    let diags = diagnostics::analyze_json_diagnostics("cfg.json", "json", &broken_json);
    assert_eq!(diags.len(), 1);
    let expected_utf16_col = (prefix.encode_utf16().count() + 1) as u32;
    assert_eq!(
        diags[0].col, expected_utf16_col,
        "JSON diagnostic column must use UTF-16 editor units rather than raw UTF-8 byte offset"
    );

    let jsonl = format!("{{\"ok\": true}}\n{prefix}bad}}\n");
    let jsonl_diags = diagnostics::analyze_json_diagnostics("events.jsonl", "jsonl", &jsonl);
    assert_eq!(jsonl_diags.len(), 1);
    assert_eq!(jsonl_diags[0].line, 2);
    assert_eq!(jsonl_diags[0].col, expected_utf16_col);
}

#[test]
fn go_organize_imports_groups_stdlib_before_external_in_format_and_code_actions() {
    let go_src = "package main\n\nimport (\n\t\"github.com/acme/service/pkg\"\n\t\"os\"\n\t\"fmt\"\n\t\"golang.org/x/sync/errgroup\"\n)\n\nfunc main() {}\n";
    let organized = format_builtin_fallback_opts("go", go_src, true);
    let expected_block = "import (\n\t\"fmt\"\n\t\"os\"\n\n\t\"github.com/acme/service/pkg\"\n\t\"golang.org/x/sync/errgroup\"\n)";
    assert!(
        organized.contains(expected_block),
        "expected grouped & sorted Go imports, got:\n{organized}"
    );

    let actions = generate_code_actions("main.go", "go", go_src, 4, 2, &[]);
    let act = actions
        .iter()
        .find(|a| a.id == "refactor.organize_imports")
        .expect("expected organize_imports action for Go");
    assert!(!act.edits.is_empty());
    assert!(act
        .replacement_content
        .as_deref()
        .unwrap_or("")
        .contains(expected_block));
}

#[test]
fn hover_req_supports_nested_lsp_position_and_column_alias() {
    let by_pos: IdeHoverReq = serde_json::from_value(serde_json::json!({
        "uri": "main.go",
        "position": { "line": 4, "character": 9 }
    }))
    .expect("deserialize nested position");
    assert_eq!(by_pos.path, "main.go");
    assert_eq!(by_pos.effective_line(), 5);
    assert_eq!(by_pos.effective_character(), 9);

    let by_col: IdeHoverReq = serde_json::from_value(serde_json::json!({
        "file_path": "lib.rs",
        "line": 12,
        "column": 7
    }))
    .expect("deserialize column alias");
    assert_eq!(by_col.path, "lib.rs");
    assert_eq!(by_col.effective_line(), 12);
    assert_eq!(by_col.effective_character(), 7);
    let _ = IdeHoverPosition {
        line: 0,
        character: 0,
    };
}

#[test]
fn navigation_read_confinement_blocks_outside_workspace_and_secret_files() {
    let outer = tempfile::tempdir().unwrap();
    let ws = outer.path().join("ws");
    let outside = outer.path().join("outside");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    let canon_ws = ws.canonicalize().unwrap();

    let ok_file = canon_ws.join("helper.sh");
    let env_file = canon_ws.join(".env");
    let escaped_file = outside.join("secret.sh");
    std::fs::write(&ok_file, "export SAFE_VAL=1\n").unwrap();
    std::fs::write(&env_file, "SECRET_TOKEN=topsecret\n").unwrap();
    std::fs::write(&escaped_file, "OUTSIDE_SECRET=1\n").unwrap();

    let roots = std::slice::from_ref(&canon_ws);
    assert!(
        navigation::read_confined_nav_file(&ok_file, roots, &[], false).is_some(),
        "in-workspace regular source file should be readable"
    );
    assert!(
        navigation::read_confined_nav_file(&env_file, roots, &[], false).is_none(),
        "secret .env file must be blocked for non-root navigation"
    );
    assert!(
        navigation::read_confined_nav_file(&escaped_file, roots, &[], false).is_none(),
        "file outside authorized workspace roots must be blocked"
    );
}

#[test]
fn go_sibling_package_var_resolves_first_and_ignores_unrelated_short_var_decls() {
    let dir = tempfile::tempdir().unwrap();
    let tests_pkg = dir.path().join("tests");
    let other_pkg = dir.path().join("internal/providerconfig");
    std::fs::create_dir_all(&tests_pkg).unwrap();
    std::fs::create_dir_all(&other_pkg).unwrap();

    let main_test = tests_pkg.join("main_test.go");
    let comp_test = tests_pkg.join("assign_component_test.go");
    let unrelated = other_pkg.join("service.go");

    std::fs::write(
        &main_test,
        "//go:build component\npackage tests\n\nvar (\n\tclient *HttpClient\n)\n",
    )
    .unwrap();
    std::fs::write(
        &comp_test,
        "//go:build component\npackage tests\n\nfunc TestAssign() {\n\t_ = client.Do()\n}\n",
    )
    .unwrap();
    let unrelated_src = "package providerconfig\n\nfunc Load() {\n\tclient := newProviderClient()\n\t_ = client\n}\n";
    std::fs::write(&unrelated, unrelated_src).unwrap();

    let sib_hits =
        go_mod_resolve::search_same_dir_go_siblings(&tests_pkg, &comp_test, "client", false);
    assert_eq!(
        sib_hits.len(),
        1,
        "expected sibling main_test.go var client"
    );
    assert!(sib_hits[0].path.ends_with("main_test.go"));
    assert_eq!(sib_hits[0].line, 5);

    let cross_pkg_hits = navigation_matchers::find_go_package_level_definitions(
        "internal/providerconfig/service.go",
        unrelated_src,
        "client",
        false,
    );
    assert!(
        cross_pkg_hits.is_empty(),
        "function-local `:=` in unrelated package must never match cross-file Go definitions: {cross_pkg_hits:?}"
    );
}

#[test]
fn ts_import_type_and_type_prefixed_names_are_not_misread_as_unused() {
    let src = "import type { CityInput } from './types';\n\
               import typeorm from 'typeorm';\n\
               import { gone } from './gone';\n\
               export const f = (c: CityInput) => typeorm(c);\n";
    let diags = diagnostics_polyglot::analyze_js_ts_diagnostics("a.ts", src, &[]);
    let unused: Vec<_> = diags.iter().filter(|d| d.code == "unused_import").collect();
    assert_eq!(unused.len(), 1, "{diags:?}");
    assert_eq!(unused[0].message, "Unused import 'gone'");
    assert!(
        !unused[0].quick_fixes[0].is_preferred,
        "a heuristic fix is not the default"
    );
}

#[test]
fn jsonc_files_allow_comments_and_trailing_commas() {
    let src = "{\n  // Compiler options\n  \"compilerOptions\": {\n    \"strict\": true, /* on */\n    \"paths\": {\"@/*\": [\"src/*\"]},\n  },\n  \"include\": [\"src\", \"a//b\",],\n}\n";
    for path in [
        "tsconfig.json",
        "app/tsconfig.base.json",
        ".vscode/settings.json",
        "x.jsonc",
    ] {
        let diags = diagnostics::analyze_json_diagnostics(path, "json", src);
        assert!(diags.is_empty(), "{path}: {diags:?}");
    }
    // Plain JSON stays strict, and a real JSONC error keeps its line.
    assert_eq!(
        diagnostics::analyze_json_diagnostics("data.json", "json", src).len(),
        1
    );
    let broken = "{\n  // c\n  \"a\": 1\n  \"b\": 2\n}\n";
    let diags = diagnostics::analyze_json_diagnostics("tsconfig.json", "json", broken);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].line, 4);
}

#[test]
fn kotlin_lsp_gutter_kotest_deps_symbols_and_mixed_jvm_workspace_work() {
    use crate::deps_inspector::actions::bump_manifest_version_in_place_scoped;
    use crate::deps_inspector::{parse_gradle_dependencies, parse_gradle_version_catalog};
    use crate::gutter::detect_gutter_items;
    use crate::java_workspace::{infer_java_fqcn_from_file, scan_java_workspace};
    use crate::lsp_host::servers::{resolve_lsp_module_root, REGISTRY};
    use crate::toolchains_catalog::{
        curated_catalog, kotlin_compiler_zip_url, kotlin_lsp_zip_url, resolve_sdk_download,
        EcosystemKind, TargetPlatform,
    };
    use crate::types::{IdeEcosystemKind, IdeGutterKind, IdeSymbolNode};

    // 1. Ecosystem + catalog + LSP server
    let eco_from_alias: IdeEcosystemKind = serde_json::from_str("\"kt\"").unwrap();
    assert_eq!(eco_from_alias, IdeEcosystemKind::Kotlin);
    assert_eq!(EcosystemKind::parse("kts"), Some(EcosystemKind::Kotlin));
    let plat = TargetPlatform::host();
    let cat = curated_catalog(EcosystemKind::Kotlin, plat);
    assert!(!cat.is_empty());
    let resolved = resolve_sdk_download(EcosystemKind::Kotlin, Some("2.1.20"), plat, &cat).unwrap();
    assert_eq!(resolved.url, kotlin_compiler_zip_url("2.1.20"));
    assert!(kotlin_lsp_zip_url().contains("kotlin-language-server"));

    let srv = REGISTRY
        .iter()
        .find(|s| s.lang == "kotlin")
        .expect("built-in kotlin-language-server");
    assert_eq!(srv.cmd, "kotlin-language-server");

    // 2. Mixed Java + Kotlin Gradle workspace discovery + module root resolution
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("kt-mixed");
    std::fs::create_dir_all(ws.join("src/main/kotlin/com/acme")).unwrap();
    std::fs::create_dir_all(ws.join("src/main/java/com/acme")).unwrap();
    std::fs::create_dir_all(ws.join("src/test/kotlin/com/acme")).unwrap();
    std::fs::write(
        ws.join("settings.gradle.kts"),
        "rootProject.name = \"kt-mixed\"\n",
    )
    .unwrap();
    std::fs::write(
        ws.join("build.gradle.kts"),
        "plugins { kotlin(\"jvm\") version \"2.1.20\" }\n",
    )
    .unwrap();
    let app_kt = ws.join("src/main/kotlin/com/acme/App.kt");
    std::fs::write(
        &app_kt,
        "package com.acme\n\nfun main() {\n    println(\"hi\")\n}\n",
    )
    .unwrap();
    let canon_ws = ws.canonicalize().unwrap();
    let canon_app = app_kt.canonicalize().unwrap();

    let mod_root = resolve_lsp_module_root(
        "kotlin",
        &canon_ws.to_string_lossy(),
        Some(&canon_app.to_string_lossy()),
        std::slice::from_ref(&canon_ws),
    )
    .unwrap();
    assert_eq!(mod_root, canon_ws.to_string_lossy());

    let jws = scan_java_workspace(std::slice::from_ref(&canon_ws));
    assert_eq!(jws.modules.len(), 1);
    assert!(jws.modules[0]
        .main_src_dirs
        .iter()
        .any(|d| d.ends_with("src/main/kotlin")));
    assert!(jws.modules[0]
        .test_src_dirs
        .iter()
        .any(|d| d.ends_with("src/test/kotlin")));
    assert_eq!(
        infer_java_fqcn_from_file(&canon_app).as_deref(),
        Some("com.acme.AppKt")
    );

    // 3. Gutter targets: top-level main, @file:JvmName, JUnit backtick tests, Kotest specs
    let kt_main = "@file:JvmName(\"Launcher\")\npackage com.acme\n\nfun main(args: Array<String>) {\n    println(args.size)\n}\n";
    let main_targets = detect_gutter_items("src/main/kotlin/com/acme/App.kt", kt_main, &[]);
    assert!(
        main_targets.iter().any(|t| {
            t.kind == IdeGutterKind::Main
                && t.run_spec.debug_program.as_deref() == Some("com.acme.Launcher")
                && t.line == 4
        }),
        "expected @file:JvmName main gutter target, got: {main_targets:?}"
    );

    let kt_junit = "package com.acme\nimport org.junit.jupiter.api.Test\nclass MathTest {\n    @Test\n    fun `adds positive numbers`() {\n        assert(1 + 1 == 2)\n    }\n}\n";
    let junit_targets = detect_gutter_items("src/test/kotlin/com/acme/MathTest.kt", kt_junit, &[]);
    assert!(
        junit_targets
            .iter()
            .any(|t| t.kind == IdeGutterKind::Test && t.label.contains("adds positive numbers")),
        "expected JUnit backtick test gutter target, got: {junit_targets:?}"
    );

    let kt_kotest = "package com.acme\nimport io.kotest.core.spec.style.StringSpec\nclass CalcSpec : StringSpec({\n    \"multiplies two ints\" {\n        assert(2 * 3 == 6)\n    }\n})\n";
    let kotest_targets =
        detect_gutter_items("src/test/kotlin/com/acme/CalcSpec.kt", kt_kotest, &[]);
    assert!(
        kotest_targets
            .iter()
            .any(|t| t.kind == IdeGutterKind::Suite && t.label.contains("CalcSpec")),
        "expected Kotest suite gutter target, got: {kotest_targets:?}"
    );
    assert!(
        kotest_targets
            .iter()
            .any(|t| t.kind == IdeGutterKind::Test && t.label.contains("multiplies two ints")),
        "expected Kotest test gutter target, got: {kotest_targets:?}"
    );

    // 4. Gradle Kotlin DSL + Version Catalog (`gradle/libs.versions.toml`) parsing + in-place bump
    let gradle_kts = r#"dependencies {
    implementation(kotlin("stdlib"))
    implementation("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.9.0")
    ksp("com.google.dagger:hilt-compiler:2.52")
    testImplementation("io.kotest:kotest-runner-junit5:5.9.1")
}
"#;
    let gdeps = parse_gradle_dependencies(gradle_kts);
    assert!(gdeps
        .iter()
        .any(|d| d.name == "org.jetbrains.kotlin:kotlin-stdlib"));
    assert!(gdeps.iter().any(
        |d| d.name == "org.jetbrains.kotlinx:kotlinx-coroutines-core"
            && d.current_version == "1.9.0"
    ));
    assert!(gdeps
        .iter()
        .any(|d| d.name == "com.google.dagger:hilt-compiler"));
    assert!(gdeps
        .iter()
        .any(|d| d.name == "io.kotest:kotest-runner-junit5" && d.indirect));

    let catalog_toml = r#"[versions]
coroutines = "1.9.0"
kotest = "5.9.1"

[libraries]
kotlinx-coroutines = { module = "org.jetbrains.kotlinx:kotlinx-coroutines-core", version.ref = "coroutines" }
kotest-runner = { group = "io.kotest", name = "kotest-runner-junit5", version.ref = "kotest" }
junit-jupiter = "org.junit.jupiter:junit-jupiter:5.11.0"
"#;
    let cdeps = parse_gradle_version_catalog(catalog_toml);
    assert_eq!(cdeps.len(), 3, "catalog deps={cdeps:?}");
    assert!(cdeps.iter().any(
        |d| d.name == "org.jetbrains.kotlinx:kotlinx-coroutines-core"
            && d.current_version == "1.9.0"
            && d.property_name.as_deref() == Some("coroutines")
    ));
    let bumped = bump_manifest_version_in_place_scoped(
        &canon_ws.join("gradle/libs.versions.toml"),
        "libs.versions.toml",
        catalog_toml,
        "org.jetbrains.kotlinx:kotlinx-coroutines-core",
        "1.10.1",
        std::slice::from_ref(&canon_ws),
        false,
    )
    .expect("bump version.ref in libs.versions.toml");
    assert!(
        bumped.contains("coroutines = \"1.10.1\""),
        "expected [versions] bump, got:\n{bumped}"
    );

    // 5. Document symbols + kotlinc stderr parser + unused import diagnostics
    let kt_sym_src = r#"package com.acme
import java.util.UUID
import java.time.Instant

data class Order(val id: UUID)
sealed interface ResultState
object Registry {
    companion object {
        fun String.shout(): String = uppercase()
    }
}
"#;
    let syms = parse_document_symbols("Order.kt", "kotlin", kt_sym_src);
    fn collect_names(nodes: &[IdeSymbolNode], out: &mut Vec<String>) {
        for s in nodes {
            out.push(s.name.clone());
            collect_names(&s.children, out);
        }
    }
    let mut names = Vec::new();
    collect_names(&syms, &mut names);
    for expected in ["Order", "ResultState", "Registry", "shout"] {
        assert!(
            names.iter().any(|n| n.contains(expected)),
            "missing Kotlin symbol {expected} in {names:?}"
        );
    }

    let kt_diags = parse_kotlinc_stderr(
        "src/main/kotlin/App.kt:12:5: error: unresolved reference: foo\nsrc/main/kotlin/App.kt:14:9: warning: parameter 'x' is never used\n",
    );
    assert_eq!(kt_diags.len(), 2);
    assert_eq!(kt_diags[0].severity, "error");
    assert_eq!(kt_diags[0].line, 12);
    assert_eq!(kt_diags[1].severity, "warning");
}
