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
            IdeRunnerKind::ZigTest => {
                if let Some(tc) = parse_zig_test_line(line) {
                    self.push_capped(tc);
                } else if let Some((file, ln, msg)) = parse_zig_diag_line(line.trim()) {
                    if let Some(last) = self.results.last_mut() {
                        last.file.get_or_insert(file);
                        last.line.get_or_insert(ln);
                        if last.message.is_none() {
                            last.message = Some(msg);
                        }
                    }
                }
            }
            IdeRunnerKind::Ctest | IdeRunnerKind::Meson => {
                if let Some(tc) = parse_cpp_test_line(line) {
                    self.push_capped(tc);
                } else if let Some((file, ln, msg)) = parse_cpp_diag_line(line.trim()) {
                    if let Some(last) = self.results.last_mut() {
                        last.file.get_or_insert(file);
                        last.line.get_or_insert(ln);
                        if last.message.is_none() {
                            last.message = Some(msg);
                        }
                    }
                }
            }
            IdeRunnerKind::DotnetTest => {
                if let Some(tc) = parse_dotnet_test_line(line) {
                    self.push_capped(tc);
                } else if let Some((file, ln)) = parse_dotnet_stack_location(line.trim()) {
                    if let Some(last) = self
                        .results
                        .last_mut()
                        .filter(|t| t.status == IdeTestCaseStatus::Failed)
                    {
                        last.file.get_or_insert(file);
                        last.line.get_or_insert(ln);
                    }
                } else {
                    let trimmed = line.trim();
                    if let Some(last) = self
                        .results
                        .last_mut()
                        .filter(|t| t.status == IdeTestCaseStatus::Failed && t.message.is_none())
                    {
                        if let Some(msg) = trimmed
                            .strip_prefix("Assert.")
                            .map(|_| trimmed)
                            .or_else(|| trimmed.strip_prefix("Expected:").map(|_| trimmed))
                            .or_else(|| trimmed.strip_prefix("System.").map(|_| trimmed))
                        {
                            last.message = Some(msg.to_string());
                        }
                    }
                }
            }
            IdeRunnerKind::Pytest => {
                if let Some(tc) = parse_python_test_line(line) {
                    self.push_capped(tc);
                }
            }
            IdeRunnerKind::Node
            | IdeRunnerKind::Vitest
            | IdeRunnerKind::Jest
            | IdeRunnerKind::Playwright
            | IdeRunnerKind::Karma
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

pub(crate) fn parse_zig_test_line(line: &str) -> Option<IdeTestCaseResult> {
    // Zig test runner output formats:
    //   `1/3 main.test.addition works...OK`
    //   `2/3 main.test.failing case...FAIL (TestExpectedEqual)`
    //   `3/3 main.test.skipped...SKIP`
    let t = line.trim();
    let (prefix, rest) = t.split_once(' ')?;
    let (cur, total) = prefix.split_once('/')?;
    if !cur.chars().all(|c| c.is_ascii_digit())
        || cur.is_empty()
        || !total.chars().all(|c| c.is_ascii_digit())
        || total.is_empty()
    {
        return None;
    }
    let (raw_name, outcome) = rest.rsplit_once("...")?;
    let outcome = outcome.trim();
    let (status, msg) = if outcome == "OK" || outcome.starts_with("OK ") {
        (IdeTestCaseStatus::Passed, None)
    } else if outcome == "FAIL" || outcome.starts_with("FAIL") {
        let detail = outcome
            .strip_prefix("FAIL")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        (IdeTestCaseStatus::Failed, detail)
    } else if outcome == "SKIP" || outcome.starts_with("SKIP") {
        (IdeTestCaseStatus::Skipped, None)
    } else {
        return None;
    };
    let clean_name = raw_name
        .trim()
        .split_once(".test.")
        .map(|(_, after)| after)
        .unwrap_or_else(|| raw_name.trim());
    let mut tc = simple_test_case(clean_name, status, None);
    tc.message = msg;
    Some(tc)
}

fn parse_zig_diag_line(trimmed: &str) -> Option<(String, u32, String)> {
    let (file_part, rest) = trimmed.split_once(".zig:")?;
    if file_part.contains(' ') {
        return None;
    }
    let (line_s, tail) = rest.split_once(':')?;
    let line_no = line_s.trim().parse::<u32>().ok()?;
    let msg = tail
        .split_once(':')
        .map(|(_, m)| m.trim())
        .unwrap_or_else(|| tail.trim());
    Some((format!("{file_part}.zig"), line_no, msg.to_string()))
}

pub(crate) fn parse_cpp_test_line(line: &str) -> Option<IdeTestCaseResult> {
    let t = line.trim();
    // GoogleTest:
    //   `[       OK ] MathTest.Adds (2 ms)`
    //   `[  FAILED  ] MathTest.Divides (0 ms)`
    //   `[  SKIPPED ] MathTest.Pending (0 ms)`
    if let Some((status, rest)) = t
        .strip_prefix("[       OK ]")
        .map(|r| (IdeTestCaseStatus::Passed, r))
        .or_else(|| {
            t.strip_prefix("[  FAILED  ]")
                .map(|r| (IdeTestCaseStatus::Failed, r))
        })
        .or_else(|| {
            t.strip_prefix("[  SKIPPED ]")
                .map(|r| (IdeTestCaseStatus::Skipped, r))
        })
    {
        let rest = rest.trim();
        // Skip summary lines like `[  FAILED  ] 1 test, listed below:`
        if rest.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            return None;
        }
        let dur_ms = parse_ms_suffix(rest);
        let full_name = rest
            .split_once(" (")
            .map(|(n, _)| n.trim())
            .unwrap_or(rest)
            .trim_end_matches(',');
        if full_name.is_empty() {
            return None;
        }
        let (suite, name) = match full_name.split_once('.') {
            Some((s, n)) => (Some(s.to_string()), n),
            None => (None, full_name),
        };
        let mut tc = simple_test_case(name, status, dur_ms);
        tc.suite = suite;
        return Some(tc);
    }

    // CTest:
    //   `1/3 Test #1: MathTest .......................   Passed    0.01 sec`
    //   `2/3 Test #2: FailTest .......................***Failed    0.02 sec`
    //   `3/3 Test #3: SkipTest .......................***Skipped   0.00 sec`
    if let Some((_, after_test)) = t.split_once("Test #") {
        let (_, after_colon) = after_test.split_once(':')?;
        let after_colon = after_colon.trim();
        let (name_raw, tail) = after_colon
            .split_once("..")
            .or_else(|| after_colon.split_once("***"))?;
        let name = name_raw.trim();
        if name.is_empty() {
            return None;
        }
        let tail_clean = tail.trim_start_matches(['.', '*', ' ']);
        let mut parts = tail_clean.split_whitespace();
        let outcome = parts.next()?;
        let status = match outcome {
            "Passed" => IdeTestCaseStatus::Passed,
            "Failed" | "Timeout" | "Subprocess" => IdeTestCaseStatus::Failed,
            "Skipped" | "Not" => IdeTestCaseStatus::Skipped,
            _ => return None,
        };
        let dur_ms = parts
            .find_map(|tok| tok.parse::<f64>().ok())
            .map(|sec| (sec * 1000.0).round() as u64);
        return Some(simple_test_case(name, status, dur_ms));
    }

    // Meson test:
    //   `1/2 math_test        OK              0.01s`
    //   `2/2 broken_test      FAIL            0.02s   exit status 1`
    if let Some((prefix, rest)) = t.split_once(' ') {
        if let Some((cur, total)) = prefix.split_once('/') {
            if !cur.is_empty()
                && cur.chars().all(|c| c.is_ascii_digit())
                && !total.is_empty()
                && total.chars().all(|c| c.is_ascii_digit())
                && !rest.contains("...")
            {
                let tokens: Vec<&str> = rest.split_whitespace().collect();
                if tokens.len() >= 2 {
                    let status_pos = tokens.iter().position(|tok| {
                        matches!(*tok, "OK" | "FAIL" | "SKIP" | "TIMEOUT" | "EXPECTEDFAIL")
                    });
                    if let Some(pos) = status_pos {
                        if pos > 0 {
                            let name = tokens[..pos].join(" ");
                            let status = match tokens[pos] {
                                "OK" | "EXPECTEDFAIL" => IdeTestCaseStatus::Passed,
                                "SKIP" => IdeTestCaseStatus::Skipped,
                                _ => IdeTestCaseStatus::Failed,
                            };
                            let dur_ms = tokens.get(pos + 1).and_then(|s| {
                                s.strip_suffix('s')
                                    .and_then(|n| n.parse::<f64>().ok())
                                    .map(|sec| (sec * 1000.0).round() as u64)
                            });
                            return Some(simple_test_case(&name, status, dur_ms));
                        }
                    }
                }
            }
        }
    }

    // Catch2 verbose / reporter: `test case: "adds numbers" - passed` / `FAILED:`
    if let Some(rest) = t.strip_prefix("FAILED:") {
        let msg = rest.trim();
        if !msg.is_empty() {
            return Some(simple_test_case(msg, IdeTestCaseStatus::Failed, None));
        }
    }
    None
}

fn parse_cpp_diag_line(trimmed: &str) -> Option<(String, u32, String)> {
    for ext in [".cpp:", ".cc:", ".cxx:", ".c:", ".hpp:", ".h:"] {
        if let Some((file_part, rest)) = trimmed.split_once(ext) {
            if file_part.contains(' ') {
                continue;
            }
            let (line_s, msg) = rest.split_once(':')?;
            let line_no = line_s.trim().parse::<u32>().ok()?;
            let clean_ext = ext.trim_end_matches(':');
            return Some((
                format!("{file_part}{clean_ext}"),
                line_no,
                msg.trim().to_string(),
            ));
        }
    }
    None
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

pub(crate) fn parse_js_test_line(line: &str) -> Option<IdeTestCaseResult> {
    let t = line.trim();
    // Playwright list reporter:
    //   `✓  1 [chromium] › e2e/home.spec.ts:12:5 › Home › renders hero (120ms)`
    //   `✘  2 [chromium] › e2e/home.spec.ts:20:5 › Home › submits form (1.2s)`
    //   `-  3 [chromium] › e2e/home.spec.ts:28:5 › Home › skipped`
    if let Some((status, rest)) = t
        .strip_prefix("✓ ")
        .or_else(|| t.strip_prefix("✔ "))
        .or_else(|| t.strip_prefix("ok "))
        .map(|r| (IdeTestCaseStatus::Passed, r))
        .or_else(|| {
            t.strip_prefix("✘ ")
                .or_else(|| t.strip_prefix("✕ "))
                .or_else(|| t.strip_prefix("✖ "))
                .or_else(|| t.strip_prefix("× "))
                .or_else(|| t.strip_prefix("not ok "))
                .map(|r| (IdeTestCaseStatus::Failed, r))
        })
        .or_else(|| {
            t.strip_prefix("○ skipped ")
                .or_else(|| t.strip_prefix("↓ "))
                .or_else(|| t.strip_prefix("-  "))
                .map(|r| (IdeTestCaseStatus::Skipped, r))
        })
    {
        if rest.contains(" › ") {
            let parts: Vec<&str> = rest.split(" › ").map(str::trim).collect();
            if parts.len() >= 2 {
                let raw_tail = parts[1..].join(" › ");
                let dur = parse_ms_suffix(&raw_tail);
                let mut tc = simple_test_case(raw_tail.trim(), status, dur);
                // Extract `file:line:col` if present in second segment.
                let file_seg = parts.get(1).copied().unwrap_or("");
                if let Some((f, rest_loc)) = file_seg.split_once(':') {
                    if f.contains('.') {
                        tc.file = Some(f.to_string());
                        if let Some(ln) = rest_loc
                            .split(':')
                            .next()
                            .and_then(|s| s.parse::<u32>().ok())
                        {
                            tc.line = Some(ln);
                        }
                    }
                }
                return Some(tc);
            }
        }
        let name = rest
            .split_once(" - ")
            .map(|(_, n)| n)
            .unwrap_or(rest)
            .trim();
        if !name.is_empty() {
            return Some(simple_test_case(name, status, parse_ms_suffix(name)));
        }
    }
    // Karma / Jasmine spec lines:
    //   `Chrome Headless 120.0.0.0 (Mac OS 10.15.7) AppComponent should create FAILED`
    //   `Chrome Headless ...: Executed 5 of 5 SUCCESS (0.123 secs / 0.1 secs)`
    if let Some(before_failed) = t.strip_suffix(" FAILED") {
        if before_failed.contains(") ") {
            let spec = before_failed
                .rsplit_once(") ")
                .map(|(_, s)| s.trim())
                .unwrap_or(before_failed.trim());
            if !spec.is_empty() {
                return Some(simple_test_case(spec, IdeTestCaseStatus::Failed, None));
            }
        }
    }
    if t.contains(": Executed ") && (t.contains(" SUCCESS") || t.contains(" FAILED")) {
        let status = if t.contains(" FAILED") {
            IdeTestCaseStatus::Failed
        } else {
            IdeTestCaseStatus::Passed
        };
        return Some(simple_test_case(t, status, None));
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

/// Per-test results from the JUnit XML reports (`TEST-*.xml`) Surefire/Failsafe and Gradle
/// write under `cwd` (`**/target/surefire-reports`, `**/build/test-results/**`), counting only
/// files modified since `since` (this run). The console only prints one row per class
/// (Maven) or nothing for passing tests (Gradle). Blocking: call off the async runtime.
#[allow(clippy::disallowed_methods)] // bounded walk, run inside spawn_blocking
pub(super) fn collect_junit_xml_results(
    cwd: &std::path::Path,
    since: std::time::SystemTime,
) -> Vec<IdeTestCaseResult> {
    fn walk(
        dir: &std::path::Path,
        depth: u8,
        since: std::time::SystemTime,
        out: &mut Vec<IdeTestCaseResult>,
    ) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_dir() {
                if depth < 6 && !name.starts_with('.') && name != "src" && name != "node_modules" {
                    walk(&path, depth + 1, since, out);
                }
            } else if name.starts_with("TEST-")
                && name.ends_with(".xml")
                && entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .is_ok_and(|m| m >= since)
            {
                if let Ok(xml) = std::fs::read_to_string(&path) {
                    out.extend(parse_junit_xml(&xml));
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(cwd, 0, since, &mut out);
    out.truncate(MAX_PARSED_TESTS);
    out
}

/// `<testcase name classname time>` entries of one JUnit XML report, with the `<failure>`
/// / `<error>` message or `<skipped>`.
pub(super) fn parse_junit_xml(xml: &str) -> Vec<IdeTestCaseResult> {
    static RE_CASE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static RE_ATTR: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static RE_FAIL: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re_case = RE_CASE.get_or_init(|| {
        regex::Regex::new(r"(?s)<testcase\b([^>]*?)(?:/>|>(.*?)</testcase>)").unwrap()
    });
    let re_attr = RE_ATTR.get_or_init(|| regex::Regex::new(r#"\b([a-z]+)="([^"]*)""#).unwrap());
    let re_fail = RE_FAIL.get_or_init(|| regex::Regex::new(r"<(failure|error)\b([^>]*)").unwrap());
    let attr = |attrs: &str, key: &str| {
        re_attr
            .captures_iter(attrs)
            .find(|c| &c[1] == key)
            .map(|c| xml_unescape(&c[2]))
    };
    re_case
        .captures_iter(xml)
        .map(|c| {
            let attrs = &c[1];
            let body = c.get(2).map_or("", |m| m.as_str());
            let failure = re_fail.captures(body);
            let status = if failure.is_some() {
                IdeTestCaseStatus::Failed
            } else if body.contains("<skipped") {
                IdeTestCaseStatus::Skipped
            } else {
                IdeTestCaseStatus::Passed
            };
            let duration_ms = attr(attrs, "time")
                .and_then(|t| t.parse::<f64>().ok())
                .map(|sec| (sec * 1000.0).round() as u64);
            let mut item = simple_test_case(
                &attr(attrs, "name").unwrap_or_default(),
                status,
                duration_ms,
            );
            item.suite = attr(attrs, "classname");
            item.message = failure.and_then(|f| attr(&f[2], "message"));
            item
        })
        .collect()
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#10;", "\n")
        .replace("&amp;", "&")
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

fn parse_dotnet_duration_bracket(s: &str) -> Option<u64> {
    let open = s.rfind('[')?;
    let close = s[open + 1..].find(']')?;
    let raw = s[open + 1..open + 1 + close].trim();
    if raw.starts_with('<') && raw.contains("ms") {
        return Some(1);
    }
    if let Some(ms) = raw.strip_suffix("ms") {
        return ms.trim().parse::<f64>().ok().map(|v| v.round() as u64);
    }
    if let Some(sec) = raw.strip_suffix('s') {
        return sec
            .trim()
            .parse::<f64>()
            .ok()
            .map(|v| (v * 1000.0).round() as u64);
    }
    None
}

/// Parse `dotnet test --verbosity normal` result lines:
/// - `Passed MyApp.Tests.MathTests.Adds [12 ms]`
/// - `Failed MyApp.Tests.MathTests.Divides [< 1 ms]`
/// - `Skipped MyApp.Tests.MathTests.Pending`
pub(crate) fn parse_dotnet_test_line(line: &str) -> Option<IdeTestCaseResult> {
    let t = line.trim();
    let (status, rest) = if let Some(r) = t.strip_prefix("Passed ") {
        (IdeTestCaseStatus::Passed, r.trim())
    } else if let Some(r) = t.strip_prefix("Failed ") {
        (IdeTestCaseStatus::Failed, r.trim())
    } else {
        let r = t.strip_prefix("Skipped ")?;
        (IdeTestCaseStatus::Skipped, r.trim())
    };
    // Skip summary lines like `Passed!  - Failed:     0, Passed:     2, ...`
    if rest.is_empty() || rest.starts_with('-') || rest.contains("Failed:") {
        return None;
    }
    let duration_ms = parse_dotnet_duration_bracket(rest);
    let raw_name = match rest.rfind('[') {
        Some(idx) => rest[..idx].trim(),
        None => rest,
    };
    if raw_name.is_empty() {
        return None;
    }
    let (suite, name) = match raw_name.rsplit_once('.') {
        Some((s, n)) if !n.is_empty() => (Some(s.to_string()), n.to_string()),
        _ => (None, raw_name.to_string()),
    };
    Some(IdeTestCaseResult {
        name,
        suite,
        status,
        duration_ms,
        file: None,
        line: None,
        message: None,
    })
}

/// Parse .NET stack frame location: `at MyApp.Tests.MathTests.Divides() in /path/MathTests.cs:line 42`
fn parse_dotnet_stack_location(trimmed: &str) -> Option<(String, u32)> {
    let (_, after_in) = trimmed.rsplit_once(" in ")?;
    let (file, line_s) = after_in.rsplit_once(":line ")?;
    let ln = line_s.trim().parse::<u32>().ok()?;
    let clean_file = file.trim();
    if clean_file.ends_with(".cs") || clean_file.ends_with(".fs") {
        Some((clean_file.to_string(), ln))
    } else {
        None
    }
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
