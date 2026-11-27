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
    // BUG-16: Kotest specs execute at spec-class granularity (no broken JUnit leaf selectors).
    assert!(
        kotest_targets
            .iter()
            .all(|t| t.kind != IdeGutterKind::Test),
        "Kotest leaf DSL strings must not emit broken JUnit method gutter items: {kotest_targets:?}"
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

#[test]
fn spring_boot_workspace_config_hover_navigation_diagnostics_actuator_and_devtools_work() {
    use crate::gutter::detect_gutter_items;
    use crate::spring_boot::actuator::{
        parse_actuator_beans, parse_actuator_env, parse_actuator_health, parse_actuator_mappings,
        validate_loopback_actuator_url,
    };
    use crate::spring_boot::detect_spring_boot_workspace;
    use crate::spring_boot::props::{
        analyze_spring_config_diagnostics, extract_config_map, hover_spring_config,
        is_spring_config_path, navigate_spring_config,
    };

    // 1. Workspace detection (pom.xml + @SpringBootApplication + application.properties + application-dev.yml)
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("spring-demo");
    std::fs::create_dir_all(ws.join("src/main/java/com/acme/demo")).unwrap();
    std::fs::create_dir_all(ws.join("src/main/resources")).unwrap();
    std::fs::write(
        ws.join("pom.xml"),
        r#"<project>
  <parent>
    <groupId>org.springframework.boot</groupId>
    <artifactId>spring-boot-starter-parent</artifactId>
    <version>3.4.0</version>
  </parent>
  <dependencies>
    <dependency><artifactId>spring-boot-starter-web</artifactId></dependency>
    <dependency><artifactId>spring-boot-starter-actuator</artifactId></dependency>
    <dependency><artifactId>spring-boot-devtools</artifactId></dependency>
  </dependencies>
</project>"#,
    )
    .unwrap();
    let app_java = ws.join("src/main/java/com/acme/demo/DemoApplication.java");
    let app_src = r#"package com.acme.demo;

import org.springframework.boot.SpringApplication;
import org.springframework.boot.autoconfigure.SpringBootApplication;
import org.springframework.beans.factory.annotation.Value;

@SpringBootApplication
public class DemoApplication {
    @Value("${server.port:8080}")
    private int port;

    public static void main(String[] args) {
        SpringApplication.run(DemoApplication.class, args);
    }
}
"#;
    std::fs::write(&app_java, app_src).unwrap();

    let props_path = ws.join("src/main/resources/application.properties");
    let props_src = "\
server.port=8091
management.server.port=9091
management.endpoints.web.base-path=/ops
spring.devtools.restart.trigger-file=.mytrigger
spring.profiles.active=dev,local
spring.redis.host=localhost
spring.jpa.show-sql=notabool
server.port=8092
spring.jpa.hibernate.ddl-auto=bogus
";
    std::fs::write(&props_path, props_src).unwrap();

    let yml_path = ws.join("src/main/resources/application-dev.yml");
    let yml_src = "\
spring:
  profiles:
    active: dev,local
  datasource:
    url: jdbc:h2:mem:devdb
server:
  shutdown: graceful
";
    std::fs::write(&yml_path, yml_src).unwrap();

    let canon_ws = ws.canonicalize().unwrap();
    let info = detect_spring_boot_workspace(std::slice::from_ref(&canon_ws))
        .expect("Spring Boot workspace detected");
    assert!(info.detected);
    assert!(info.has_actuator);
    assert!(info.has_devtools);
    assert_eq!(info.server_port, 8092);
    assert_eq!(info.management_port, Some(9091));
    assert_eq!(info.actuator_base_path, "/ops");
    assert_eq!(info.actuator_url, "http://127.0.0.1:9091/ops");
    assert_eq!(info.trigger_file.as_deref(), Some(".mytrigger"));
    assert!(info.profiles.iter().any(|p| p == "dev"));
    assert!(info.profiles.iter().any(|p| p == "local"));
    assert_eq!(info.apps.len(), 1);
    assert_eq!(info.apps[0].main_class, "com.acme.demo.DemoApplication");

    // 2. Gutter marks @SpringBootApplication main() clearly
    let gutter = detect_gutter_items(&app_java.to_string_lossy(), app_src, &[]);
    assert!(
        gutter
            .iter()
            .any(|g| g.label == "Spring Boot DemoApplication"),
        "expected Spring Boot gutter label: {gutter:?}"
    );

    // 3. YAML nested dotted map + config hover + bidirectional navigation
    assert!(is_spring_config_path(&props_path.to_string_lossy()));
    assert!(is_spring_config_path(&yml_path.to_string_lossy()));
    let yml_map = extract_config_map(&yml_path.to_string_lossy(), yml_src);
    assert_eq!(
        yml_map.get("spring.datasource.url").map(String::as_str),
        Some("jdbc:h2:mem:devdb")
    );
    assert_eq!(
        yml_map.get("server.shutdown").map(String::as_str),
        Some("graceful")
    );

    let hover_prop = hover_spring_config(
        &props_path.to_string_lossy(),
        props_src,
        1,
        std::slice::from_ref(&canon_ws),
        false,
    )
    .expect("hover on server.port");
    assert!(hover_prop.found);
    assert_eq!(hover_prop.symbol.as_deref(), Some("server.port"));
    assert!(hover_prop
        .signature
        .as_deref()
        .unwrap_or("")
        .contains("integer"));

    let hover_code = hover_spring_config(
        &app_java.to_string_lossy(),
        app_src,
        9,
        std::slice::from_ref(&canon_ws),
        false,
    )
    .expect("hover on @Value(\"${server.port:8080}\")");
    assert!(hover_code.found);
    assert_eq!(hover_code.symbol.as_deref(), Some("server.port"));
    assert!(hover_code
        .full_doc
        .as_deref()
        .unwrap_or("")
        .contains("application.properties"));

    let (nav_key, nav_locs) = navigate_spring_config(
        &app_java.to_string_lossy(),
        app_src,
        9,
        std::slice::from_ref(&canon_ws),
        false,
    )
    .expect("Java @Value -> application.properties");
    assert_eq!(nav_key, "server.port");
    assert!(nav_locs[0].path.ends_with("application.properties"));

    let (rev_key, rev_locs) = navigate_spring_config(
        &props_path.to_string_lossy(),
        props_src,
        1,
        std::slice::from_ref(&canon_ws),
        false,
    )
    .expect("application.properties -> Java @Value");
    assert_eq!(rev_key, "server.port");
    assert!(rev_locs[0].path.ends_with("DemoApplication.java"));

    // 4. Diagnostics: deprecated key + Quick Fix, invalid boolean, duplicate key, invalid enum
    let diags = analyze_spring_config_diagnostics(&props_path.to_string_lossy(), props_src);
    let dep = diags
        .iter()
        .find(|d| d.code == "spring_deprecated_property")
        .expect("deprecated spring.redis.host diagnostic");
    assert_eq!(dep.quick_fixes.len(), 1);
    assert!(dep.quick_fixes[0]
        .replacement
        .contains("spring.data.redis.host"));
    assert!(diags.iter().any(|d| d.code == "spring_invalid_type"));
    assert!(diags.iter().any(|d| d.code == "spring_duplicate_property"));
    assert!(diags.iter().any(|d| d.code == "spring_invalid_enum"));

    // 5. Loopback-only Actuator URL validator (SSRF guard)
    assert!(validate_loopback_actuator_url("http://127.0.0.1:8080/actuator").is_ok());
    assert!(validate_loopback_actuator_url("http://localhost:9090/actuator/").is_ok());
    assert!(validate_loopback_actuator_url("http://[::1]:8080/actuator").is_ok());
    assert!(validate_loopback_actuator_url("http://169.254.169.254/latest/meta-data").is_err());
    assert!(validate_loopback_actuator_url("http://example.com:8080/actuator").is_err());
    assert!(validate_loopback_actuator_url("http://user:pass@127.0.0.1:8080/actuator").is_err());

    // 6. Actuator JSON parsers + automatic secret redaction
    let (health_st, comps) = parse_actuator_health(&serde_json::json!({
        "status": "UP",
        "components": { "db": { "status": "UP", "details": { "database": "PostgreSQL" } } }
    }));
    assert_eq!(health_st.as_deref(), Some("UP"));
    assert_eq!(comps.len(), 1);

    let eps = parse_actuator_mappings(&serde_json::json!({
        "contexts": { "application": { "mappings": { "dispatcherServlets": { "dispatcherServlet": [{
            "handler": "com.acme.OrderController#list()",
            "details": { "requestMappingConditions": { "patterns": ["/api/orders"], "methods": ["GET"] } }
        }] } } } }
    }));
    assert_eq!(eps.len(), 1);
    assert_eq!(eps[0].method, "GET");
    assert_eq!(eps[0].path, "/api/orders");

    let beans = parse_actuator_beans(&serde_json::json!({
        "contexts": { "application": { "beans": { "orderService": {
            "type": "com.acme.OrderService", "scope": "singleton", "dependencies": ["orderRepo"]
        } } } }
    }));
    assert_eq!(beans.len(), 1);
    assert_eq!(beans[0].name, "orderService");

    let (profs, env_props) = parse_actuator_env(&serde_json::json!({
        "activeProfiles": ["dev"],
        "propertySources": [{
            "name": "application.properties",
            "properties": {
                "server.port": { "value": "8092" },
                "spring.datasource.password": { "value": "super-secret" }
            }
        }]
    }));
    assert_eq!(profs, vec!["dev"]);
    let pw = env_props
        .iter()
        .find(|p| p.key == "spring.datasource.password")
        .unwrap();
    assert!(pw.redacted);
    assert_eq!(pw.value, "******");
    // SEC-6: ALL env property values are redacted so URI credentials or custom keys cannot leak.
    let port_prop = env_props.iter().find(|p| p.key == "server.port").unwrap();
    assert!(port_prop.redacted);
    assert_eq!(port_prop.value, "******");
}

#[tokio::test]
async fn spring_boot_review_fixes_devtools_confinement_profiles_multimodule_and_catalog() {
    use crate::build_tools::build_tools_model;
    use crate::spring_boot::actuator::{parse_actuator_env, trigger_devtools_restart};
    use crate::spring_boot::detect_spring_boot_workspace_scoped;
    use crate::spring_boot::props::{
        hover_spring_config, lookup_spring_property, navigate_spring_config,
    };
    use crate::types::IdeBuildToolsModelReq;

    // DUP-1: shared JSON catalog contains both spring.config.import and spring.profiles.default.
    assert!(lookup_spring_property("spring.config.import").is_some());
    assert!(lookup_spring_property("spring.profiles.default").is_some());

    // SEC-6: URI with embedded credentials in non-secret key name is still redacted.
    let (_, env_props) = parse_actuator_env(&serde_json::json!({
        "activeProfiles": [],
        "propertySources": [{
            "name": "application.properties",
            "properties": {
                "spring.datasource.url": { "value": "postgresql://admin:hunter2@db.internal:5432/app" }
            }
        }]
    }));
    assert_eq!(env_props.len(), 1);
    assert!(env_props[0].redacted);
    assert_eq!(env_props[0].value, "******");

    // Multi-module Maven parent with Spring Boot child module (BUG-18), inactive profile isolation (BUG-19),
    // symlink escape confinement (SEC-4), and non-destructive DevTools trigger touch (DATA-2, RED-1).
    let outer = tempfile::tempdir().unwrap();
    let outside = outer.path().join("outside");
    let ws = outer.path().join("aggregator");
    let svc = ws.join("service");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::create_dir_all(svc.join("src/main/java/com/acme/svc")).unwrap();
    std::fs::create_dir_all(svc.join("src/main/resources")).unwrap();

    std::fs::write(
        ws.join("pom.xml"),
        "<project><groupId>com.acme</groupId><artifactId>parent</artifactId><packaging>pom</packaging><modules><module>service</module></modules></project>",
    )
    .unwrap();
    std::fs::write(
        svc.join("pom.xml"),
        "<project><groupId>com.acme</groupId><artifactId>service</artifactId><dependencies><dependency><groupId>org.springframework.boot</groupId><artifactId>spring-boot-starter-web</artifactId></dependency><dependency><groupId>org.springframework.boot</groupId><artifactId>spring-boot-devtools</artifactId></dependency></dependencies></project>",
    )
    .unwrap();
    let svc_app = svc.join("src/main/java/com/acme/svc/SvcApp.java");
    let svc_app_src = "package com.acme.svc;\nimport org.springframework.boot.autoconfigure.SpringBootApplication;\nimport org.springframework.beans.factory.annotation.Value;\n@SpringBootApplication\npublic class SvcApp {\n    @Value(\"${server.port}\") int p;\n    public static void main(String[] args) {}\n}\n";
    std::fs::write(&svc_app, svc_app_src).unwrap();
    std::fs::write(
        svc.join("src/main/resources/application.properties"),
        "server.port=8081\n",
    )
    .unwrap();
    // Inactive profile must NOT override base server.port=8081 (BUG-19).
    std::fs::write(
        svc.join("src/main/resources/application-test.properties"),
        "server.port=9999\nmanagement.server.port=9998\n",
    )
    .unwrap();

    // SEC-4: symlinked config file pointing outside workspace root must not be read.
    let secret_outside = outside.join("leaked.properties");
    std::fs::write(&secret_outside, "server.port=7777\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        &secret_outside,
        svc.join("src/main/resources/application-leak.properties"),
    )
    .unwrap();

    let canon_ws = ws.canonicalize().unwrap();
    let canon_svc = svc.canonicalize().unwrap();

    // BUG-18: build_tools_model on aggregator root detects Spring Boot inside child module `service/`.
    let model = build_tools_model(
        &IdeBuildToolsModelReq {
            roots: vec![canon_ws.to_string_lossy().into_owned()],
            target: None,
        },
        false,
    );
    let sb = model
        .spring_boot
        .expect("Spring Boot detected in Maven sub-module");
    assert_eq!(
        sb.server_port, 8081,
        "inactive application-test.properties must not override base port"
    );
    assert_eq!(
        sb.management_port, None,
        "inactive profile management.server.port must not apply"
    );
    assert!(
        sb.profiles.iter().any(|p| p == "test"),
        "inactive profile still listed"
    );
    assert_eq!(
        sb.module_root.as_deref(),
        Some(canon_svc.to_string_lossy().as_ref())
    );

    // SEC-4: hover & navigation ignore symlink escaping workspace root.
    let hover = hover_spring_config(
        &svc_app.to_string_lossy(),
        svc_app_src,
        6,
        std::slice::from_ref(&canon_ws),
        false,
    )
    .expect("hover on @Value");
    assert!(
        !hover.full_doc.as_deref().unwrap_or("").contains("7777"),
        "outside symlinked properties must not leak into hover: {:?}",
        hover.full_doc
    );
    let (_, nav_locs) = navigate_spring_config(
        &svc_app.to_string_lossy(),
        svc_app_src,
        6,
        std::slice::from_ref(&canon_ws),
        false,
    )
    .expect("nav from @Value");
    assert!(
        nav_locs
            .iter()
            .all(|l| !l.path.contains("application-leak.properties") && !l.path.contains("outside")),
        "outside symlinked properties must not appear in navigation: {nav_locs:?}"
    );

    // Active profile DOES apply when named in spring.profiles.active (BUG-19).
    std::fs::write(
        svc.join("src/main/resources/application.properties"),
        "server.port=8081\nspring.profiles.active=test\n",
    )
    .unwrap();
    let active_sb = detect_spring_boot_workspace_scoped(
        std::slice::from_ref(&canon_svc),
        std::slice::from_ref(&canon_ws),
        false,
    )
    .expect("Spring Boot detected with active profile");
    assert_eq!(active_sb.server_port, 9999);
    assert_eq!(active_sb.management_port, Some(9998));

    // DATA-2 & RED-1: DevTools restart requires compiled classpath output dir (no src/main/resources fallback),
    // rejects traversal/invalid filenames, and preserves existing file contents in-place.
    assert!(
        trigger_devtools_restart(&canon_ws, None, Some(&canon_svc.to_string_lossy()))
            .await
            .is_err(),
        "must error when compiled classes output dir does not exist"
    );
    let classes_dir = canon_svc.join("target/classes");
    std::fs::create_dir_all(&classes_dir).unwrap();
    assert!(trigger_devtools_restart(
        &canon_ws,
        Some("../escape"),
        Some(&canon_svc.to_string_lossy())
    )
    .await
    .is_err());
    assert!(trigger_devtools_restart(
        &canon_ws,
        Some("sub/trigger"),
        Some(&canon_svc.to_string_lossy())
    )
    .await
    .is_err());
    let existing = classes_dir.join("application.properties");
    std::fs::write(&existing, "server.port=8081\n# keep me intact\n").unwrap();
    let resp = trigger_devtools_restart(
        &canon_ws,
        Some("application.properties"),
        Some(&canon_svc.to_string_lossy()),
    )
    .await
    .expect("touch existing classpath file");
    assert!(resp.ok);
    assert!(resp.message.contains("restart check requested"));
    assert_eq!(
        std::fs::read_to_string(&existing).unwrap(),
        "server.port=8081\n# keep me intact\n",
        "touching an existing trigger file must never truncate or overwrite its bytes"
    );
}

#[tokio::test]
async fn spring_boot_actuator_accepts_503_down_health_response() {
    use crate::spring_boot::actuator::inspect_actuator;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv = tokio::spawn(async move {
        for _ in 0..8 {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let mut buf = [0u8; 2048];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]);
            let path = req
                .lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .unwrap_or("/");
            let (status, body) = match path {
                "/actuator" => (
                    "200 OK",
                    r#"{"_links":{"self":{"href":"http://127.0.0.1/actuator"},"health":{"href":"http://127.0.0.1/actuator/health"}}}"#,
                ),
                "/actuator/health" => (
                    "503 Service Unavailable",
                    r#"{"status":"DOWN","components":{"db":{"status":"DOWN"}}}"#,
                ),
                _ => ("404 Not Found", "{}"),
            };
            let resp = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = sock.write_all(resp.as_bytes()).await;
        }
    });

    let base_url = format!("http://{addr}/actuator");
    let res = inspect_actuator(&crate::types::IdeSpringActuatorReq {
        target: None,
        base_url: Some(base_url),
        port: None,
        base_path: None,
    })
    .await
    .expect("inspect actuator");
    srv.abort();
    assert!(
        res.reachable,
        "503 DOWN health must still report reachable=true"
    );
    assert_eq!(res.status.as_deref(), Some("DOWN"));
    assert_eq!(res.health_components.len(), 1);
    assert_eq!(res.health_components[0].name, "db");
    assert_eq!(res.health_components[0].status, "DOWN");
}

#[test]
fn java_deep_submodule_junit5_gutter_and_workspace_resolution_preserved() {
    use crate::gutter::detect_gutter_items;
    use crate::java_workspace::{infer_java_fqcn_from_file, scan_java_workspace};
    use crate::types::IdeGutterKind;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("multi");
    let core = root.join("core");
    let deep_dir = core.join("src/test/java/com/acme/core/deep/inner");
    std::fs::create_dir_all(&deep_dir).unwrap();
    std::fs::write(
        root.join("pom.xml"),
        "<project><groupId>com.acme</groupId><artifactId>multi</artifactId><packaging>pom</packaging><modules><module>core</module></modules></project>",
    )
    .unwrap();
    std::fs::write(
        core.join("pom.xml"),
        "<project><groupId>com.acme</groupId><artifactId>core</artifactId></project>",
    )
    .unwrap();
    let deep_test = deep_dir.join("DeepTest.java");
    let src = "package com.acme.core.deep.inner;\nimport org.junit.jupiter.api.Test;\nclass DeepTest {\n    @Test\n    void adds() {\n        int sum = 1 + 2;\n    }\n}\n";
    std::fs::write(&deep_test, src).unwrap();
    let canon_root = root.canonicalize().unwrap();
    let canon_core = core.canonicalize().unwrap();
    let canon_test = deep_test.canonicalize().unwrap();

    let items = detect_gutter_items(&canon_test.to_string_lossy(), src, &[]);
    let test_item = items
        .iter()
        .find(|i| i.kind == IdeGutterKind::Test && i.label.contains("adds"))
        .expect("JUnit 5 @Test gutter item");
    assert_eq!(
        test_item.run_spec.debug_program.as_deref(),
        Some("com.acme.core.deep.inner.DeepTest")
    );
    assert_eq!(
        test_item.run_spec.test_filter.as_deref(),
        Some("com.acme.core.deep.inner.DeepTest#adds")
    );

    let graph = scan_java_workspace(std::slice::from_ref(&canon_root));
    let owner = graph
        .owning_module(&canon_test)
        .expect("owning module core");
    assert_eq!(owner.dir, canon_core);
    assert_eq!(
        infer_java_fqcn_from_file(&canon_test).as_deref(),
        Some("com.acme.core.deep.inner.DeepTest")
    );
}

#[test]
fn kotlin_review_fixes_trust_main_stem_deps_guard_remote_probe_sdk_kind_and_ktfmt() {
    use crate::dap::java_bridge::infer_kotlin_class_stem;
    use crate::deps_inspector::actions::bump_manifest_version_in_place_scoped;
    use crate::deps_inspector::{parse_gradle_dependencies, parse_gradle_version_catalog};
    use crate::environments::module_sdks::apply_module_sdk_to_project_env;
    use crate::lsp_host::ws_bridge::{
        kotlin_lsp_execution_trusted, TRUST_KOTLIN_LSP_ENV, TRUST_PROJECT_EXECUTION_ENV,
    };
    use crate::toolchains::remote_probe::{parse_ssh_probe_output, REMOTE_PROBE_SCRIPT};
    use crate::toolchains_catalog::EcosystemKind;
    use crate::types::{IdeEnvVarEntry, IdeProjectEnvironment, IdeRunnerKind};

    // SEC-3: kotlin-language-server requires explicit non-secret project trust opt-in.
    let tmp = tempfile::tempdir().unwrap();
    let envs = crate::environments::EnvironmentManager::new(tmp.path().to_path_buf());
    let repo = tmp.path().join("kt-repo");
    let repo_s = repo.to_string_lossy().into_owned();
    assert!(!kotlin_lsp_execution_trusted(&envs, &repo_s, None));
    envs.seed_snapshot(
        &repo_s,
        IdeProjectEnvironment {
            env_vars: vec![IdeEnvVarEntry {
                key: TRUST_KOTLIN_LSP_ENV.into(),
                value: Some("1".into()),
                secret: true,
                has_secret: true,
            }],
            ..Default::default()
        },
    );
    assert!(!kotlin_lsp_execution_trusted(&envs, &repo_s, None));
    envs.seed_snapshot(
        &repo_s,
        IdeProjectEnvironment {
            env_vars: vec![IdeEnvVarEntry {
                key: TRUST_KOTLIN_LSP_ENV.into(),
                value: Some("1".into()),
                secret: false,
                has_secret: false,
            }],
            ..Default::default()
        },
    );
    assert!(kotlin_lsp_execution_trusted(&envs, &repo_s, None));
    assert!(!kotlin_lsp_execution_trusted(&envs, &repo_s, Some("ssh-1")));
    envs.seed_snapshot(
        &format!("ssh:ssh-1:{repo_s}"),
        IdeProjectEnvironment {
            env_vars: vec![IdeEnvVarEntry {
                key: TRUST_PROJECT_EXECUTION_ENV.into(),
                value: Some("1".into()),
                secret: false,
                has_secret: false,
            }],
            ..Default::default()
        },
    );
    assert!(kotlin_lsp_execution_trusted(&envs, &repo_s, Some("ssh-1")));

    // BUG-15: top-level `fun main` in a file that ALSO declares `class DemoApplication`
    // MUST still resolve to `DemoApplicationKt` (file facade).
    let spring_kt = "package com.acme\n\n@SpringBootApplication\nclass DemoApplication\n\nfun main(args: Array<String>) {\n    runApplication<DemoApplication>(*args)\n}\n";
    assert_eq!(
        infer_kotlin_class_stem("DemoApplication", spring_kt),
        "DemoApplicationKt"
    );
    let test_cls_kt = "package com.acme\nclass MathTest {\n    fun helper() {}\n}\n";
    assert_eq!(infer_kotlin_class_stem("MathTest", test_cls_kt), "MathTest");

    // DATA-1: plugin-managed `kotlin("stdlib")` & versionless catalog entries cannot be corrupted by in-place bump.
    let gradle_kts = "dependencies {\n    implementation(kotlin(\"stdlib\"))\n}\n";
    let gdeps = parse_gradle_dependencies(gradle_kts);
    let stdlib = gdeps
        .iter()
        .find(|d| d.name == "org.jetbrains.kotlin:kotlin-stdlib")
        .unwrap();
    assert_eq!(stdlib.version_col, 0);
    assert_eq!(
        stdlib.managed_in.as_ref().map(|m| m.path.as_str()),
        Some("kotlin-plugin")
    );
    let ws = tmp.path().join("ws-deps");
    std::fs::create_dir_all(&ws).unwrap();
    let canon_ws = ws.canonicalize().unwrap();
    assert!(
        bump_manifest_version_in_place_scoped(
            &canon_ws.join("build.gradle.kts"),
            "build.gradle.kts",
            gradle_kts,
            "org.jetbrains.kotlin:kotlin-stdlib",
            "2.1.20",
            std::slice::from_ref(&canon_ws),
            false,
        )
        .is_none(),
        "versionless kotlin(\"stdlib\") must never be rewritten in place"
    );
    let unversioned_cat = "[libraries]\nfoo = { module = \"com.acme:foo\" }\n";
    let cdeps = parse_gradle_version_catalog(unversioned_cat);
    assert_eq!(cdeps[0].version_col, 0);
    assert!(bump_manifest_version_in_place_scoped(
        &canon_ws.join("gradle/libs.versions.toml"),
        "libs.versions.toml",
        unversioned_cat,
        "com.acme:foo",
        "1.2.3",
        std::slice::from_ref(&canon_ws),
        false,
    )
    .is_none());

    // API-2: SSH remote probe script + parser discover Kotlin SDK, LSP, and shared DAP.
    assert!(REMOTE_PROBE_SCRIPT.contains("SDK:kotlin:"));
    assert!(REMOTE_PROBE_SCRIPT.contains("kotlin-language-server"));
    assert!(REMOTE_PROBE_SCRIPT.contains("DAP:kotlin:"));
    let probe_out = "UNAME:Linux x86_64\nSDK:kotlin:otto:2.1.20:/home/u/.otto/toolchains/kotlin/sdks/2.1.20/kotlinc\nLSP:kotlin:/home/u/.otto/toolchains/kotlin/bin/kotlin-language-server\nDAP:kotlin:/home/u/.otto/toolchains/java/dap/com.microsoft.java.debug.plugin-0.53.1.jar\n";
    let (_, probed) = parse_ssh_probe_output(probe_out);
    let kt_probe = probed.get(&EcosystemKind::Kotlin).expect("Kotlin probed");
    assert_eq!(kt_probe.installed.len(), 1);
    assert_eq!(kt_probe.installed[0].version, "2.1.20");
    assert!(kt_probe.lsp.as_ref().is_some_and(|t| t.available));
    assert!(kt_probe.dap.as_ref().is_some_and(|t| t.available));

    // BUG-34: Selecting a Kotlin SDK sets `kotlin_home` and preserves `java_home`.
    let kt_sdk = tmp.path().join("toolchains/kotlin/sdks/2.1.20/kotlinc");
    std::fs::create_dir_all(kt_sdk.join("bin")).unwrap();
    std::fs::write(kt_sdk.join("bin/kotlinc"), "#!/bin/sh\n").unwrap();
    let mut env = IdeProjectEnvironment {
        java_home: Some("/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home".into()),
        ..Default::default()
    };
    apply_module_sdk_to_project_env(
        &mut env,
        &canon_ws.to_string_lossy(),
        &kt_sdk.to_string_lossy(),
        Some(IdeRunnerKind::Gradle),
        Some("src/main/kotlin/App.kt"),
    );
    assert_eq!(
        env.kotlin_home.as_deref(),
        Some(kt_sdk.to_string_lossy().as_ref())
    );
    assert_eq!(
        env.java_home.as_deref(),
        Some("/Library/Java/JavaVirtualMachines/temurin-21.jdk/Contents/Home")
    );

    // OBS-2: ktfmt uses documented `--stdin-name` flag (never invalid `--stdinpath`).
    let kt_fmts = formatting::formatter_candidates("kotlin", "src/App.kt", false, false);
    let ktfmt = kt_fmts
        .iter()
        .find(|(name, _, _)| *name == "ktfmt")
        .unwrap();
    assert!(ktfmt.2.iter().any(|a| a == "--stdin-name"), "{:?}", ktfmt.2);
    assert!(!ktfmt.2.iter().any(|a| a == "--stdinpath"), "{:?}", ktfmt.2);
}

#[cfg(unix)]
#[test]
fn kotlin_gradle_and_standalone_kotlinc_execution_plans_work() {
    use crate::java_workspace::prepare_java_execution;
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let bin = root.join("bin");
    std::fs::create_dir_all(&bin).unwrap();

    // Fake `gradle` that compiles & writes classpath to the init-script target path.
    let gradle_seen = root.join("gradle-call");
    let fake_gradle = bin.join("gradle");
    std::fs::write(
        &fake_gradle,
        format!(
            "#!/bin/sh\necho \"$*\" > {}\ninit=\"\"\nprev=\"\"\nfor a in \"$@\"; do\n  if [ \"$prev\" = \"--init-script\" ]; then init=\"$a\"; fi\n  prev=\"$a\"\ndone\ncpfile=$(sed -n \"s/.*new File('\\([^']*\\)').*/\\1/p\" \"$init\" | head -n 1)\nmkdir -p build/classes/kotlin/main\nprintf '/gradle/kotlin-stdlib.jar' > \"$cpfile\"\n",
            gradle_seen.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&fake_gradle, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mod_dir = root.join("kt-gradle");
    let src_dir = mod_dir.join("src/main/kotlin/com/acme");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(
        mod_dir.join("settings.gradle.kts"),
        "rootProject.name = \"kt\"\n",
    )
    .unwrap();
    std::fs::write(
        mod_dir.join("build.gradle.kts"),
        "plugins { kotlin(\"jvm\") version \"2.1.20\" }\n",
    )
    .unwrap();
    let app_kt = src_dir.join("App.kt");
    std::fs::write(
        &app_kt,
        "package com.acme\nfun main() { println(\"ok\") }\n",
    )
    .unwrap();

    let env = BTreeMap::from([(
        "PATH".to_string(),
        format!("{}:/usr/bin:/bin", bin.display()),
    )]);
    let plan = prepare_java_execution(
        &app_kt,
        None,
        false,
        std::slice::from_ref(&mod_dir),
        &env,
        "/nonexistent/javac",
        &root.join("out-gradle"),
    )
    .expect("Gradle Kotlin execution plan");
    assert_eq!(plan.main_class, "com.acme.AppKt");
    assert!(
        plan.classpath.contains("/gradle/kotlin-stdlib.jar"),
        "{}",
        plan.classpath
    );
    assert!(
        plan.classpath.contains("build/classes/kotlin/main"),
        "{}",
        plan.classpath
    );
    let call = std::fs::read_to_string(&gradle_seen).unwrap();
    assert!(
        call.contains("classes") && call.contains("ottoPrintClasspath"),
        "{call}"
    );

    // Standalone `.kt` file compiles via `kotlinc -include-runtime -d <jar>`.
    let kotlinc_seen = root.join("kotlinc-call");
    let fake_kotlinc = bin.join("kotlinc");
    std::fs::write(
        &fake_kotlinc,
        format!("#!/bin/sh\necho \"$*\" > {}\n", kotlinc_seen.display()),
    )
    .unwrap();
    std::fs::set_permissions(&fake_kotlinc, std::fs::Permissions::from_mode(0o755)).unwrap();
    let solo_dir = root.join("solo-kt");
    std::fs::create_dir_all(&solo_dir).unwrap();
    let solo_kt = solo_dir.join("Hello.kt");
    std::fs::write(&solo_kt, "fun main() { println(\"hi\") }\n").unwrap();
    let solo_plan = prepare_java_execution(
        &solo_kt,
        None,
        false,
        std::slice::from_ref(&solo_dir),
        &env,
        "/nonexistent/javac",
        &root.join("out-solo"),
    )
    .expect("standalone kotlinc plan");
    assert_eq!(solo_plan.main_class, "HelloKt");
    assert!(
        solo_plan.classpath.ends_with(".jar"),
        "{}",
        solo_plan.classpath
    );
    let kcall = std::fs::read_to_string(&kotlinc_seen).unwrap();
    assert!(kcall.contains("-include-runtime"), "{kcall}");
}

#[cfg(unix)]
#[tokio::test]
async fn cpp_review_fixes_clangd_llvm_codelldb_core_unreal_cc_override_cmake_and_conan() {
    use crate::dap::adapters_cpp::build_codelldb_request;
    use crate::deps_inspector::actions::run_deps_action_scoped;
    use crate::environments::db_store::apply_run_config_sdk_override;
    use crate::environments::runner_cmd::build_runner_command;
    use crate::local_history::LocalHistoryStore;
    use crate::toolchains::installer_remote::build_remote_install_script;
    use crate::toolchains::installer_tools::codelldb_stage_dir;
    use crate::toolchains_catalog::{
        clangd_download_url, curated_catalog, resolve_sdk_download, EcosystemKind, TargetArch,
        TargetOs, TargetPlatform, CLANGD_RELEASE_VERSION,
    };
    use crate::types::{
        IdeDepsActionReq, IdeInstallComponent, IdeRunConfig, IdeRunnerKind, IdeTargetKind,
    };
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;

    // BUG-8 & BUG-9: pinned clangd 19.1.2 (rejects Linux arm64), no broken LLVM tarball downloads.
    let mac_arm = TargetPlatform {
        os: TargetOs::Macos,
        arch: TargetArch::Aarch64,
    };
    let lin_x64 = TargetPlatform {
        os: TargetOs::Linux,
        arch: TargetArch::X86_64,
    };
    let lin_arm = TargetPlatform {
        os: TargetOs::Linux,
        arch: TargetArch::Aarch64,
    };
    assert!(clangd_download_url(mac_arm)
        .unwrap()
        .contains(&format!("clangd-mac-{CLANGD_RELEASE_VERSION}.zip")));
    assert!(clangd_download_url(lin_x64)
        .unwrap()
        .contains(&format!("clangd-linux-{CLANGD_RELEASE_VERSION}.zip")));
    assert!(clangd_download_url(lin_arm).is_err());
    assert!(curated_catalog(EcosystemKind::Cpp, mac_arm).is_empty());
    assert!(resolve_sdk_download(EcosystemKind::Cpp, Some("19.1.7"), mac_arm, &[]).is_err());

    // CONC-2: job-scoped staging dirs for local + PID-scoped staging for remote CodeLLDB.
    let dl = std::path::Path::new("/tmp/tc/.downloads");
    assert_ne!(
        codelldb_stage_dir(dl, "job-a"),
        codelldb_stage_dir(dl, "job-b")
    );
    let remote_dap =
        build_remote_install_script(EcosystemKind::Cpp, IdeInstallComponent::Dap, lin_x64, None)
            .unwrap();
    assert!(remote_dap.contains("codelldb-stage-$$"), "{remote_dap}");

    // API-9: CodeLLDB core dumps use `launch` + `target create <exe> -c <core>` (not `attach`).
    let (cmd, args) = build_codelldb_request(
        "/app/build/demo",
        &[],
        "/app",
        &BTreeMap::new(),
        &[],
        Some("/app/core.123"),
        None,
        None,
        None,
    );
    assert_eq!(cmd, "launch");
    assert!(args.get("pid").is_none());
    let init_cmds = args["initCommands"].as_array().unwrap();
    assert!(init_cmds.iter().any(|v| v
        .as_str()
        .unwrap_or("")
        .contains("target create '/app/build/demo' -c '/app/core.123'")));

    // BUG-41: per-run C++ SDK override updates CC and CXX (overriding project-level CC/CXX).
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let gcc_tc = root.join("gcc-14");
    std::fs::create_dir_all(gcc_tc.join("bin")).unwrap();
    std::fs::write(gcc_tc.join("bin/gcc"), "").unwrap();
    std::fs::write(gcc_tc.join("bin/g++"), "").unwrap();
    let mut env_map = BTreeMap::from([
        ("CC".to_string(), "/usr/bin/clang".to_string()),
        ("CXX".to_string(), "/usr/bin/clang++".to_string()),
    ]);
    let mut path_dirs = Vec::new();
    apply_run_config_sdk_override(
        IdeRunnerKind::Cmake,
        &gcc_tc.to_string_lossy(),
        IdeTargetKind::Local,
        &mut env_map,
        &mut path_dirs,
    );
    assert!(env_map["CC"].ends_with("/gcc-14/bin/gcc"));
    assert!(env_map["CXX"].ends_with("/gcc-14/bin/g++"));

    // API-7, BUG-10, BUG-11, BUG-12: CMake/CTest/Meson target_path, fresh rebuild, GTest/Catch2, gutter main.
    let proj = root.join("cpp-proj");
    std::fs::create_dir_all(proj.join("src")).unwrap();
    std::fs::create_dir_all(proj.join("tests")).unwrap();
    std::fs::create_dir_all(proj.join("build")).unwrap();
    std::fs::write(
        proj.join("CMakeLists.txt"),
        "cmake_minimum_required(VERSION 3.20)\nproject(demo)\nadd_executable(demo src/main.cpp src/helper.cpp)\nadd_executable(math_tests tests/math_test.cpp)\n",
    )
    .unwrap();
    let stale_bin = proj.join("build/demo");
    std::fs::write(&stale_bin, "#!/bin/sh\necho stale\n").unwrap();
    std::fs::set_permissions(&stale_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let proj_s = proj.to_string_lossy().into_owned();

    let mk_cfg = |runner: IdeRunnerKind,
                  target_path: Option<&str>,
                  test_filter: Option<&str>|
     -> IdeRunConfig {
        let mut cfg: IdeRunConfig = serde_json::from_value(serde_json::json!({
            "id": "c1",
            "name": "cfg",
            "target_path": target_path,
            "test_filter": test_filter
        }))
        .unwrap();
        cfg.runner = runner;
        cfg
    };

    // API-7
    let (p, a) = build_runner_command(
        &mk_cfg(IdeRunnerKind::Cmake, Some("demo"), None),
        IdeTargetKind::Local,
        &proj_s,
        &BTreeMap::new(),
    );
    assert_eq!(p, "cmake");
    assert_eq!(a, vec!["--build", "build", "--target", "demo"]);
    let (p, a) = build_runner_command(
        &mk_cfg(IdeRunnerKind::Cmake, Some("preset:debug"), None),
        IdeTargetKind::Local,
        &proj_s,
        &BTreeMap::new(),
    );
    assert_eq!(p, "cmake");
    assert_eq!(a, vec!["--build", "--preset", "debug"]);
    let (p, a) = build_runner_command(
        &mk_cfg(IdeRunnerKind::Ctest, Some("out/build"), None),
        IdeTargetKind::Local,
        &proj_s,
        &BTreeMap::new(),
    );
    assert_eq!(p, "ctest");
    assert!(
        a.windows(2).any(|w| w == ["--test-dir", "out/build"]),
        "{a:?}"
    );
    let (p, a) = build_runner_command(
        &mk_cfg(IdeRunnerKind::Meson, Some("build-rel"), None),
        IdeTargetKind::Local,
        &proj_s,
        &BTreeMap::new(),
    );
    assert_eq!(p, "meson");
    assert_eq!(a, vec!["compile", "-C", "build-rel"]);

    // BUG-12: existing `build/demo` MUST still rebuild incrementally via `cmake --build ... --target demo`.
    let (p, a) = build_runner_command(
        &mk_cfg(IdeRunnerKind::CppRun, Some("demo"), None),
        IdeTargetKind::Local,
        &proj_s,
        &BTreeMap::new(),
    );
    assert_eq!(p, "sh");
    assert!(
        a[1].contains("cmake --build") && a[1].contains("--target demo"),
        "{}",
        a[1]
    );

    // BUG-10: gutter Run on `src/main.cpp` in a CMake project builds & runs `demo` instead of standalone `clang++`.
    let (p, a) = build_runner_command(
        &mk_cfg(IdeRunnerKind::CppRun, Some("src/main.cpp"), None),
        IdeTargetKind::Local,
        &proj_s,
        &BTreeMap::new(),
    );
    assert_eq!(p, "sh");
    assert!(
        a[1].contains("cmake --build") && a[1].contains("--target demo"),
        "{}",
        a[1]
    );

    // BUG-11: GoogleTest gutter run in CMake project builds `math_tests` and passes `--gtest_filter=MathSuite.Adds`.
    let (p, a) = build_runner_command(
        &mk_cfg(
            IdeRunnerKind::Ctest,
            Some("tests/math_test.cpp"),
            Some("MathSuite.Adds"),
        ),
        IdeTargetKind::Local,
        &proj_s,
        &BTreeMap::new(),
    );
    assert_eq!(p, "sh");
    assert!(a[1].contains("--target math_tests"), "{}", a[1]);
    assert!(
        a.iter().any(|x| x == "--gtest_filter=MathSuite.Adds"),
        "{a:?}"
    );

    // API-1: `conanfile.txt` `sync` runs `conan install . --build=missing`.
    let bin = root.join("conan-bin");
    std::fs::create_dir_all(&bin).unwrap();
    let conan_seen = root.join("conan-args");
    let fake_conan = bin.join("conan");
    std::fs::write(
        &fake_conan,
        format!("#!/bin/sh\necho \"$*\" > {}\n", conan_seen.display()),
    )
    .unwrap();
    std::fs::set_permissions(&fake_conan, std::fs::Permissions::from_mode(0o755)).unwrap();
    let conanfile = proj.join("conanfile.txt");
    std::fs::write(&conanfile, "[requires]\nfmt/11.0.2\n").unwrap();
    let hist = LocalHistoryStore::new(root.join("hist"));
    let conan_env = BTreeMap::from([(
        "PATH".to_string(),
        format!("{}:/usr/bin:/bin", bin.display()),
    )]);
    let resp = run_deps_action_scoped(
        &proj,
        std::slice::from_ref(&proj),
        &conanfile,
        &IdeDepsActionReq {
            target: None,
            path: conanfile.to_string_lossy().into_owned(),
            action: "sync".into(),
            module: None,
            version: None,
        },
        &conan_env,
        &hist,
        false,
    )
    .await
    .expect("conanfile.txt sync succeeds");
    assert!(resp.ok);
    assert_eq!(
        std::fs::read_to_string(&conan_seen).unwrap().trim(),
        "install . --build=missing"
    );
}
