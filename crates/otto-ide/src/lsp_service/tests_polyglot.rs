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
