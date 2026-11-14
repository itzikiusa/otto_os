use super::*;
use std::collections::BTreeMap;

#[test]
fn document_symbols_hierarchical_across_5_languages() {
    let rs = "pub struct Engine {\n    id: u64,\n}\n\nimpl Engine {\n    pub fn new() -> Self {\n        Self { id: 1 }\n    }\n    pub fn run(&self) {}\n}\n";
    let rs_syms = parse_document_symbols("src/lib.rs", "rust", rs);
    assert_eq!(rs_syms.len(), 2);
    assert_eq!(rs_syms[1].name, "impl Engine");
    assert_eq!(rs_syms[1].children.len(), 2);
    assert_eq!(rs_syms[1].children[1].name, "run");

    let go_src = "package calc\ntype Adder struct {\n\tbase int\n}\nfunc (a *Adder) Add(v int) int {\n\treturn a.base + v\n}\n";
    let go_syms = parse_document_symbols("calc.go", "go", go_src);
    assert_eq!(go_syms.len(), 2);
    assert_eq!(go_syms[1].name, "Add");

    let java_src = "public class OrderService {\n    public int createOrder(String item) {\n        return 1;\n    }\n}\n";
    let java_syms = parse_document_symbols("OrderService.java", "java", java_src);
    assert_eq!(java_syms[0].children[0].name, "createOrder");

    let ts_src = "export class Store {\n  load(id: string) {\n    return id;\n  }\n}\nexport const helper = (x: number) => x + 1;\n";
    let ts_syms = parse_document_symbols("store.ts", "typescript", ts_src);
    assert_eq!(ts_syms.len(), 2);
    assert_eq!(ts_syms[0].children[0].name, "load");

    let py_src = "MAX_RETRIES = 3\n\nclass Worker:\n    def __init__(self):\n        self.ok = True\n    async def execute(self, job):\n        return job\n";
    let py_syms = parse_document_symbols("worker.py", "python", py_src);
    assert_eq!(py_syms[1].children.len(), 2);
}

#[test]
fn references_and_rename_skip_comments_strings_and_substrings() {
    let src = "// computeTax comment mentions computeTax\nconst label = \"computeTax\";\nfunction computeTax(amount) {\n  return computeTaxHelper(amount) + computeTax(0);\n}\n";
    assert_eq!(
        extract_identifier_at(src, 3, 12).as_deref(),
        Some("computeTax")
    );
    let refs = find_symbol_occurrences_in_text("tax.ts", "typescript", src, "computeTax");
    assert_eq!(refs.len(), 2);
    let (renamed, edit) =
        rename_identifier_in_text("tax.ts", "typescript", src, "computeTax", "calcDuty")
            .expect("rename");
    assert_eq!(edit.occurrences, 2);
    assert!(renamed.contains("// computeTax comment"));
    assert!(renamed.contains("\"computeTax\""));
    assert!(renamed.contains("computeTaxHelper(amount)"));
    assert!(renamed.contains("function calcDuty(amount)"));
}

#[test]
fn code_actions_populate_concrete_edits_and_fallback_formatter_works() {
    let messy_go =
        "package main\nfunc calc(x int) (int, error) {\nres, err := step(x)\nreturn res, err\n}\n";
    assert!(format_builtin_fallback("go", messy_go).contains("\tres, err := step(x)"));
    let actions = generate_code_actions("main.go", "go", messy_go, 3, 2, &[]);
    let err_act = actions
        .iter()
        .find(|a| a.id == "refactor.go.wrap_err_check")
        .expect("wrap_err_check");
    assert!(!err_act.edits.is_empty());
    assert_eq!(err_act.edits[0].path, "main.go");

    let rs_src = "use std::sync::Arc;\nuse std::collections::HashMap;\n\npub fn parse_num(s: &str) -> i32 {\n    s.parse::<i32>().unwrap()\n}\n";
    let rs_actions = generate_code_actions("lib.rs", "rust", rs_src, 5, 10, &[]);
    assert!(rs_actions
        .iter()
        .any(|a| a.id == "refactor.organize_imports" && !a.edits.is_empty()));
    assert!(rs_actions
        .iter()
        .any(|a| a.id == "quickfix.rust.unwrap_to_question" && !a.edits.is_empty()));
}

#[tokio::test]
async fn rg_local_candidate_files_filters_ignored_and_unmatched() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
    std::fs::create_dir_all(root.join("target/debug")).unwrap();
    std::fs::write(root.join("src/origin.rs"), "pub fn computeInvoice() {}\n").unwrap();
    std::fs::write(root.join("src/caller.rs"), "fn f() { computeInvoice(); }\n").unwrap();
    std::fs::write(root.join("src/unrelated.rs"), "fn other() {}\n").unwrap();
    std::fs::write(
        root.join("node_modules/pkg/index.js"),
        "computeInvoice();\n",
    )
    .unwrap();
    std::fs::write(root.join("target/debug/out.rs"), "computeInvoice();\n").unwrap();

    if let Some(files) =
        rg_local_candidate_files(&root, false, "computeInvoice", "src/origin.rs").await
    {
        assert_eq!(files, vec!["src/origin.rs", "src/caller.rs"]);
        let none_match =
            rg_local_candidate_files(&root, false, "neverExistsAnywhere", "src/origin.rs")
                .await
                .unwrap();
        assert_eq!(none_match, vec!["src/origin.rs"]);
    }
}

#[test]
fn document_symbols_and_formatters_cover_shell_json_jsonl_and_csv_tsv() {
    let sh_src = "#!/usr/bin/env bash\nexport DEPLOY_ENV=\"prod\"\n\ndeploy_service() {\n  if [ -n \"$DEPLOY_ENV\" ]; then\n  echo \"ok\"\n  fi\n}\n";
    let sh_syms = parse_document_symbols("scripts/deploy.sh", "shell", sh_src);
    assert_eq!(sh_syms.len(), 2);
    assert_eq!(sh_syms[0].name, "DEPLOY_ENV");
    assert_eq!(sh_syms[1].name, "deploy_service");
    let sh_fmt = format_builtin_fallback("shell", sh_src);
    assert!(sh_fmt.contains("  if [ -n \"$DEPLOY_ENV\" ]; then\n    echo \"ok\"\n  fi"));

    let jsonc_src = "{\n  // service port\n  \"port\": 8080,\n  \"routes\": [\n    { \"path\": \"/health\" }\n  ]\n}\n";
    let json_syms = parse_document_symbols("config.jsonc", "jsonc", jsonc_src);
    assert_eq!(json_syms.len(), 2);

    let json_pretty = format_builtin_fallback("json", "{\"b\":1,\"a\":[true,false]}");
    assert!(json_pretty.contains("  \"b\": 1"));

    let jsonl_src = "{\"id\":\"evt-1\",\"ok\":true}\n{\"id\":\"evt-2\",\"ok\":false}\n";
    let jsonl_syms = parse_document_symbols("events.jsonl", "jsonl", jsonl_src);
    assert_eq!(jsonl_syms.len(), 2);

    let csv_src = "player_id,brand_id,balance\n1001,20,50.25\n1002,20,99.00\n";
    let csv_syms = parse_document_symbols("players.csv", "csv", csv_src);
    assert_eq!(csv_syms.len(), 3);
}

#[test]
fn external_sdk_and_go_mod_replace_navigation_prioritizes_local_replace() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("go_admission");
    let local_dep = tmp.path().join("local_sdk");
    let goroot = tmp.path().join("goroot");
    let gomodcache = tmp.path().join("pkg/mod");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&local_dep).unwrap();
    std::fs::create_dir_all(goroot.join("src/fmt")).unwrap();
    std::fs::create_dir_all(gomodcache.join("github.com/!azure/sdk@v1.2.0")).unwrap();

    std::fs::write(
        ws.join("go.mod"),
        "module bitbucket.org/org/go_admission\n\nrequire (\n\tgithub.com/Azure/sdk v1.2.0\n)\n\nreplace github.com/Azure/sdk => ../local_sdk\n",
    )
    .unwrap();
    std::fs::write(
        local_dep.join("go.mod"),
        "module github.com/Azure/sdk\n\ngo 1.22\n",
    )
    .unwrap();
    std::fs::write(
        local_dep.join("client.go"),
        "package sdk\n\nfunc NewClient() string { return \"local\" }\n",
    )
    .unwrap();
    let main_go = ws.join("main.go");
    let main_src = "package main\n\nimport (\n\t\"fmt\"\n\t\"github.com/Azure/sdk\"\n)\n\nfunc main() {\n\tfmt.Println(sdk.NewClient())\n}\n";
    std::fs::write(&main_go, main_src).unwrap();
    std::fs::write(
        goroot.join("src/fmt/print.go"),
        "package fmt\n\nfunc Println(a ...any) (n int, err error) { return 0, nil }\n",
    )
    .unwrap();
    std::fs::write(
        gomodcache.join("github.com/!azure/sdk@v1.2.0/client.go"),
        "package sdk\n\nfunc NewClient() string { return \"stale_cache\" }\n",
    )
    .unwrap();

    let mut env = BTreeMap::new();
    env.insert("GOROOT".into(), goroot.to_string_lossy().into_owned());
    env.insert(
        "GOMODCACHE".into(),
        gomodcache.to_string_lossy().into_owned(),
    );
    let roots = vec![ws.canonicalize().unwrap()];
    let ro = vec![
        goroot.canonicalize().unwrap(),
        gomodcache.canonicalize().unwrap(),
    ];

    let sdk_dirs = navigation_ext::resolve_go_package_dirs(
        "github.com/Azure/sdk",
        &main_go,
        &roots,
        &ro,
        &env,
    );
    assert_eq!(sdk_dirs, vec![local_dep.canonicalize().unwrap()]);
    let sdk_defs = navigation_ext::search_go_package_dirs(&sdk_dirs, "NewClient");
    assert_eq!(sdk_defs.len(), 1);
    assert!(sdk_defs[0].path.contains("local_sdk"));
}

#[test]
fn java_diagnostics_catch_unresolved_imports_and_honor_lombok_slf4j() {
    let java_src = r#"package com.acme.demo;

import lombok.extern.slf4j.Slf4j;
import com.nonexistent.bogus.DoesNotExist;
import java.util.UUID;

@Slf4j
public class DemoService {
    public String run() {
        log.debug("starting service");
        DoesNotExist helper = null;
        return "ok";
    }
}
"#;
    let diags = java_diag::analyze_java_diagnostics(
        "/tmp/src/main/java/com/acme/demo/DemoService.java",
        java_src,
        &[],
        None,
        &BTreeMap::new(),
    );
    assert!(
        diags.iter().any(|d| d.code == "unresolved_import"
            && d.message.contains("DoesNotExist")
            && !d.quick_fixes[0].edits.is_empty()),
        "expected unresolved_import diagnostic with concrete edits, got: {diags:?}"
    );
    assert!(
        diags
            .iter()
            .any(|d| d.code == "unused_import" && d.message.contains("java.util.UUID")),
        "expected unused_import on java.util.UUID, got: {diags:?}"
    );
    assert!(
        !diags
            .iter()
            .any(|d| d.message.contains("Slf4j") || d.message.contains("log")),
        "Lombok @Slf4j and synthesized log must not trigger errors, got: {diags:?}"
    );
}

#[test]
fn hover_isolates_single_go_var_doc_without_neighbor_bleed() {
    let decimal_src = r#"package decimal

// DivisionPrecision is the number of decimal places in the result when it
// doesn't divide exactly.
var DivisionPrecision = 16

// PowPrecisionNegativeExponent specifies the maximum precision when computing
// negative exponents.
var PowPrecisionNegativeExponent = 16
"#;
    let defs = navigation::find_definitions_in_content(
        "decimal.go",
        "go",
        decimal_src,
        "DivisionPrecision",
    );
    assert_eq!(defs.len(), 1);
    let hov = hover::extract_hover_from_definition(
        "DivisionPrecision",
        "decimal.go",
        defs[0].line,
        "go",
        decimal_src,
    );
    assert!(hov.found);
    assert_eq!(hov.package_name.as_deref(), Some("decimal"));
    assert_eq!(hov.signature.as_deref(), Some("var DivisionPrecision = 16"));
    let doc = hov.full_doc.unwrap_or_default();
    assert!(doc.contains("DivisionPrecision is the number of decimal places"));
    assert!(
        !doc.contains("PowPrecisionNegativeExponent"),
        "hover doc must never bleed into subsequent symbols: {doc}"
    );
}
