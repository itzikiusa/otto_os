//! Structured O(1)-per-line test output parsers (`go test -v`, `cargo test`,
//! `pytest -v` / `unittest`, `node --test` / `vitest`, Bats TAP, Maven/Gradle).

use std::collections::HashMap;

use crate::types::{IdeRunnerKind, IdeTestCaseResult, IdeTestCaseStatus};

const MAX_PARSED_TESTS: usize = 2_000;

/// Stateful $O(1)$-per-line test parser that retains structured test cases
/// across an entire run even after `output` rolls past `MAX_RUN_OUTPUT_LINES`.
#[derive(Default)]
pub(super) struct IncrementalTestTracker {
    pub(super) results: Vec<IdeTestCaseResult>,
    pub(super) coverage_pct: Option<f64>,
    pub(super) coverage_summary: Option<String>,
    go_index: HashMap<String, usize>,
    last_go_idx: Option<usize>,
    go_cov_sum: f64,
    go_cov_count: usize,
    go_has_total: bool,
}

impl IncrementalTestTracker {
    fn push_capped(&mut self, item: IdeTestCaseResult) {
        if self.results.len() < MAX_PARSED_TESTS {
            self.results.push(item);
        }
    }

    pub(super) fn compact_on_finish(&mut self) {
        self.go_index.clear();
        self.go_index.shrink_to_fit();
        self.results.shrink_to_fit();
    }

    pub(super) fn ingest_line(&mut self, runner: IdeRunnerKind, line: &str) {
        self.ingest_coverage_line(runner, line);
        match runner {
            IdeRunnerKind::GoTest => self.ingest_go_line(line),
            IdeRunnerKind::CargoTest => {
                if let Some(tc) = parse_cargo_test_line(line) {
                    self.push_capped(tc);
                }
            }
            IdeRunnerKind::Pytest => {
                if let Some(tc) = parse_python_test_line(line) {
                    self.push_capped(tc);
                }
            }
            IdeRunnerKind::Node
            | IdeRunnerKind::Vitest
            | IdeRunnerKind::Npm
            | IdeRunnerKind::Shell => {
                if let Some(tc) = parse_js_test_line(line) {
                    self.push_capped(tc);
                }
            }
            IdeRunnerKind::JavaJunit | IdeRunnerKind::Maven | IdeRunnerKind::Gradle => {
                if let Some(tc) = parse_jvm_test_line(line) {
                    self.push_capped(tc);
                }
            }
            _ => {}
        }
    }

    fn ingest_go_line(&mut self, line: &str) {
        let t = line.trim();
        if let Some(name) = t
            .strip_prefix("=== RUN")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            if let Some(&idx) = self.go_index.get(name) {
                self.results[idx] = simple_test_case(name, IdeTestCaseStatus::Running, None);
                self.last_go_idx = Some(idx);
            } else if self.results.len() < MAX_PARSED_TESTS {
                let idx = self.results.len();
                self.results
                    .push(simple_test_case(name, IdeTestCaseStatus::Running, None));
                self.go_index.insert(name.to_string(), idx);
                self.last_go_idx = Some(idx);
            }
        } else if t.starts_with("--- ") {
            if let Some((status, rest)) = t
                .strip_prefix("--- PASS:")
                .map(|r| (IdeTestCaseStatus::Passed, r))
                .or_else(|| {
                    t.strip_prefix("--- FAIL:")
                        .map(|r| (IdeTestCaseStatus::Failed, r))
                })
                .or_else(|| {
                    t.strip_prefix("--- SKIP:")
                        .map(|r| (IdeTestCaseStatus::Skipped, r))
                })
            {
                let rest = rest.trim();
                let (name, dur_ms) = match rest.split_once(' ') {
                    Some((n, tail)) => (n.trim(), parse_seconds_parens(tail)),
                    None => (rest, None),
                };
                if let Some(&idx) = self.go_index.get(name) {
                    self.results[idx].status = status;
                    self.results[idx].duration_ms = dur_ms;
                    self.last_go_idx = Some(idx);
                } else if self.results.len() < MAX_PARSED_TESTS {
                    let idx = self.results.len();
                    self.results.push(simple_test_case(name, status, dur_ms));
                    self.go_index.insert(name.to_string(), idx);
                    self.last_go_idx = Some(idx);
                }
            }
        } else if t.contains(".go:") {
            if let Some((file, ln, msg)) = parse_go_diag_line(t) {
                if let Some(idx) = self.last_go_idx {
                    let entry = &mut self.results[idx];
                    entry.file.get_or_insert(file);
                    entry.line.get_or_insert(ln);
                    entry.message = Some(msg);
                }
            }
        }
    }

    fn ingest_coverage_line(&mut self, _runner: IdeRunnerKind, line: &str) {
        let t = line.trim();
        if t.is_empty() {
            return;
        }
        // Go `go tool cover -func` / total summary: `total: (statements) 82.5%`
        if t.starts_with("total:") && t.contains("(statements)") {
            if let Some(pct) = parse_last_percent_token(t) {
                let r = round_pct(pct);
                self.go_has_total = true;
                self.coverage_pct = Some(r);
                self.coverage_summary = Some(format!("{}% statements", format_pct(r)));
                return;
            }
        }
        // Go `go test -cover`: `coverage: 82.5% of statements` or `ok pkg 0.1s coverage: 82.5% of statements`
        if let Some((_, after)) = t.split_once("coverage:") {
            let after = after.trim_start();
            if let Some((num_s, tail)) = after.split_once('%') {
                if tail.contains("of statements") {
                    if let Ok(pct) = num_s.trim().parse::<f64>() {
                        if !self.go_has_total {
                            self.go_cov_sum += pct;
                            self.go_cov_count += 1;
                            let avg = round_pct(self.go_cov_sum / (self.go_cov_count as f64));
                            self.coverage_pct = Some(avg);
                            self.coverage_summary =
                                Some(format!("{}% statements", format_pct(avg)));
                        }
                        return;
                    }
                }
            }
        }
        // pytest-cov / cargo-llvm-cov: `TOTAL    120    18    85%` or `TOTAL ... 85.42%`
        if t.starts_with("TOTAL ") || t.starts_with("TOTAL\t") {
            if let Some(pct) = parse_last_percent_token(t) {
                let r = round_pct(pct);
                self.coverage_pct = Some(r);
                self.coverage_summary = Some(format!("{}% total", format_pct(r)));
                return;
            }
        }
        // Jest / Vitest / c8 table: `All files |   85.2 | ...`
        if t.starts_with("All files") && t.contains('|') {
            let mut cols = t.split('|');
            let _first = cols.next();
            if let Some(stmt_col) = cols.next() {
                let cleaned = stmt_col.trim().trim_end_matches('%').trim();
                if let Ok(pct) = cleaned.parse::<f64>() {
                    let r = round_pct(pct);
                    self.coverage_pct = Some(r);
                    self.coverage_summary = Some(format!("{}% statements", format_pct(r)));
                }
            }
        }
    }
}

fn round_pct(v: f64) -> f64 {
    (v.clamp(0.0, 100.0) * 10.0).round() / 10.0
}

fn format_pct(v: f64) -> String {
    if (v - v.round()).abs() < 1e-6 {
        format!("{:.1}", v)
    } else {
        format!("{v:.1}")
    }
}

fn parse_last_percent_token(line: &str) -> Option<f64> {
    for tok in line.split_whitespace().rev() {
        let cleaned = tok.trim_matches(|c: char| c == '|' || c == ',' || c == ')');
        if let Some(num) = cleaned.strip_suffix('%') {
            if let Ok(v) = num.trim().parse::<f64>() {
                return Some(v);
            }
        }
    }
    None
}

/// Parse coverage percentage and summary across runner output lines.
#[cfg(test)]
pub fn parse_coverage_output(
    runner: IdeRunnerKind,
    lines: &[String],
) -> (Option<f64>, Option<String>) {
    let mut tracker = IncrementalTestTracker::default();
    for line in lines {
        tracker.ingest_coverage_line(runner, line);
    }
    (tracker.coverage_pct, tracker.coverage_summary)
}

/// Parse raw runner output lines into structured per-test pass/fail/duration items.
pub fn parse_test_output(runner: IdeRunnerKind, lines: &[String]) -> Vec<IdeTestCaseResult> {
    let mut tracker = IncrementalTestTracker::default();
    for line in lines {
        tracker.ingest_line(runner, line);
    }
    tracker.results
}

fn parse_go_diag_line(trimmed: &str) -> Option<(String, u32, String)> {
    let (file_part, rest) = trimmed.split_once(".go:")?;
    if file_part.contains(' ') {
        return None;
    }
    let (line_s, msg) = rest.split_once(':')?;
    let line_no = line_s.trim().parse::<u32>().ok()?;
    Some((format!("{file_part}.go"), line_no, msg.trim().to_string()))
}

fn parse_cargo_test_line(line: &str) -> Option<IdeTestCaseResult> {
    let rest = line.trim().strip_prefix("test ")?;
    let (name, outcome) = rest.rsplit_once(" ... ")?;
    let status = if outcome.starts_with("ok") {
        IdeTestCaseStatus::Passed
    } else if outcome.starts_with("FAILED") {
        IdeTestCaseStatus::Failed
    } else if outcome.starts_with("ignored") {
        IdeTestCaseStatus::Skipped
    } else {
        return None;
    };
    Some(simple_test_case(name.trim(), status, None))
}

fn parse_python_test_line(line: &str) -> Option<IdeTestCaseResult> {
    let t = line.trim();
    if t.contains("::") {
        let mut parts = t.split_whitespace();
        if let (Some(first), Some(second)) = (parts.next(), parts.next()) {
            let st = match second {
                "PASSED" => Some(IdeTestCaseStatus::Passed),
                "FAILED" | "ERROR" => Some(IdeTestCaseStatus::Failed),
                "SKIPPED" | "XFAIL" => Some(IdeTestCaseStatus::Skipped),
                _ => None,
            };
            if let Some(status) = st {
                let (file, name) = first.split_once("::").unwrap_or(("", first));
                let mut item = simple_test_case(name, status, None);
                if !file.is_empty() {
                    item.suite = Some(file.to_string());
                    item.file = Some(file.to_string());
                }
                return Some(item);
            }
        }
    }
    let (left, right) = t.rsplit_once(" ... ")?;
    let status = match right.trim() {
        "ok" => IdeTestCaseStatus::Passed,
        "FAIL" | "ERROR" => IdeTestCaseStatus::Failed,
        s if s.starts_with("skipped") => IdeTestCaseStatus::Skipped,
        _ => return None,
    };
    Some(simple_test_case(left.trim(), status, None))
}

fn parse_js_test_line(line: &str) -> Option<IdeTestCaseResult> {
    let t = line.trim();
    if let Some(rest) = t
        .strip_prefix("ok ")
        .or_else(|| t.strip_prefix("✓ "))
        .or_else(|| t.strip_prefix("✔ "))
    {
        let name = rest
            .split_once(" - ")
            .map(|(_, n)| n)
            .unwrap_or(rest)
            .trim();
        if !name.is_empty() {
            return Some(simple_test_case(
                name,
                IdeTestCaseStatus::Passed,
                parse_ms_suffix(name),
            ));
        }
    } else if let Some(rest) = t
        .strip_prefix("not ok ")
        .or_else(|| t.strip_prefix("✖ "))
        .or_else(|| t.strip_prefix("× "))
    {
        let name = rest
            .split_once(" - ")
            .map(|(_, n)| n)
            .unwrap_or(rest)
            .trim();
        if !name.is_empty() {
            return Some(simple_test_case(
                name,
                IdeTestCaseStatus::Failed,
                parse_ms_suffix(name),
            ));
        }
    }
    None
}

fn parse_jvm_test_line(line: &str) -> Option<IdeTestCaseResult> {
    let t = line
        .trim()
        .trim_start_matches("[INFO] ")
        .trim_start_matches("[ERROR] ");
    if t.contains(" > ") {
        if let Some((left, right)) = t.rsplit_once(' ') {
            if left.contains(" > ") {
                let st = match right {
                    "PASSED" => Some(IdeTestCaseStatus::Passed),
                    "FAILED" => Some(IdeTestCaseStatus::Failed),
                    "SKIPPED" => Some(IdeTestCaseStatus::Skipped),
                    _ => None,
                };
                if let Some(status) = st {
                    let (suite, name) = left.split_once(" > ").unwrap_or(("", left));
                    let mut item = simple_test_case(name.trim(), status, None);
                    item.suite = Some(suite.trim().to_string());
                    return Some(item);
                }
            }
        }
    }
    if t.starts_with("Tests run:") && t.contains("-- in ") {
        if let Some((_, suite)) = t.split_once("-- in ") {
            let failed = !t.contains("Failures: 0, Errors: 0");
            let status = if failed {
                IdeTestCaseStatus::Failed
            } else {
                IdeTestCaseStatus::Passed
            };
            let mut item = simple_test_case(suite.trim(), status, None);
            item.suite = Some(suite.trim().to_string());
            return Some(item);
        }
    }
    None
}

fn simple_test_case(
    name: &str,
    status: IdeTestCaseStatus,
    duration_ms: Option<u64>,
) -> IdeTestCaseResult {
    IdeTestCaseResult {
        name: name.to_string(),
        suite: None,
        status,
        duration_ms,
        file: None,
        line: None,
        message: None,
    }
}

fn parse_seconds_parens(s: &str) -> Option<u64> {
    let inner = s
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .trim_end_matches('s');
    inner
        .parse::<f64>()
        .ok()
        .map(|sec| (sec * 1000.0).round() as u64)
}

fn parse_ms_suffix(s: &str) -> Option<u64> {
    let start = s.rfind('(')?;
    let inner = s[start + 1..].trim_end_matches(')').trim();
    inner
        .strip_suffix("ms")?
        .trim()
        .parse::<f64>()
        .ok()
        .map(|v| v.round() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_structured_test_results_across_go_rust_python_js_shell_and_jvm() {
        let go_out = vec![
            "=== RUN   TestAdd".to_string(),
            "--- PASS: TestAdd (0.02s)".to_string(),
            "=== RUN   TestSub".to_string(),
            "    calc_test.go:19: expected 3, got 2".to_string(),
            "--- FAIL: TestSub (0.01s)".to_string(),
        ];
        let go_res = parse_test_output(IdeRunnerKind::GoTest, &go_out);
        assert_eq!(go_res.len(), 2);
        assert_eq!(go_res[0].status, IdeTestCaseStatus::Passed);
        assert_eq!(go_res[0].duration_ms, Some(20));
        assert_eq!(go_res[1].status, IdeTestCaseStatus::Failed);
        assert_eq!(go_res[1].line, Some(19));

        let cargo_res = parse_test_output(
            IdeRunnerKind::CargoTest,
            &[
                "test math::adds ... ok".into(),
                "test math::fails ... FAILED".into(),
            ],
        );
        assert_eq!(cargo_res.len(), 2);
        assert_eq!(cargo_res[0].status, IdeTestCaseStatus::Passed);
        assert_eq!(cargo_res[1].status, IdeTestCaseStatus::Failed);

        let py_res = parse_test_output(
            IdeRunnerKind::Pytest,
            &["tests/test_app.py::test_ok PASSED [100%]".into()],
        );
        assert_eq!(py_res[0].name, "test_ok");
        assert_eq!(py_res[0].status, IdeTestCaseStatus::Passed);

        let bats_res = parse_test_output(
            IdeRunnerKind::Shell,
            &[
                "ok 1 healthcheck passes".into(),
                "not ok 2 missing env fails".into(),
            ],
        );
        assert_eq!(bats_res.len(), 2);
        assert_eq!(bats_res[0].status, IdeTestCaseStatus::Passed);
        assert_eq!(bats_res[1].status, IdeTestCaseStatus::Failed);

        let jvm_res = parse_test_output(
            IdeRunnerKind::Gradle,
            &["com.acme.CalcTest > addsNumbers() PASSED".into()],
        );
        assert_eq!(jvm_res.len(), 1);
        assert_eq!(jvm_res[0].status, IdeTestCaseStatus::Passed);
    }

    #[test]
    fn parses_coverage_percentages_across_go_pytest_llvm_cov_and_vitest() {
        let (go_pct, go_sum) = parse_coverage_output(
            IdeRunnerKind::GoTest,
            &[
                "ok  \texample.com/pkg1\t0.12s\tcoverage: 80.0% of statements".into(),
                "ok  \texample.com/pkg2\t0.09s\tcoverage: 85.0% of statements".into(),
            ],
        );
        assert_eq!(go_pct, Some(82.5));
        assert_eq!(go_sum.as_deref(), Some("82.5% statements"));

        let (py_pct, py_sum) = parse_coverage_output(
            IdeRunnerKind::Pytest,
            &["TOTAL                                    140     21    85%".into()],
        );
        assert_eq!(py_pct, Some(85.0));
        assert_eq!(py_sum.as_deref(), Some("85.0% total"));

        let (vt_pct, vt_sum) = parse_coverage_output(
            IdeRunnerKind::Vitest,
            &["All files          |   85.2 |    78.4 |   90.0 |   85.2 |".into()],
        );
        assert_eq!(vt_pct, Some(85.2));
        assert_eq!(vt_sum.as_deref(), Some("85.2% statements"));
    }
}
