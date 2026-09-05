//! Probe body parsers. Every format ends up as a flat list of [`Sample`]s
//! (metric name + labels + f64) plus optional pod-level string labels (JSON
//! `label` mappings — build version, commit…) that the collector attaches to
//! every sample of that pod for the cycle.
//!
//! The prometheus parser is a tolerant line parser (counter / gauge /
//! histogram / summary all look the same on the wire); bad lines are counted
//! in `parse_errors` and skipped, non-finite values are dropped, and the
//! series cap (`SERIES_CAP`, spec "Cardinality guard") stops the parse.

use std::collections::BTreeMap;

use serde_json::Value;

use super::probes::{glob_match, Mapping, Unit};

/// Spec: at most this many distinct series per pod per cycle.
pub const SERIES_CAP: usize = 1500;

#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub metric: String,
    pub labels: BTreeMap<String, String>,
    pub value: f64,
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Parsed {
    pub samples: Vec<Sample>,
    /// Pod-level labels from JSON `label` mappings.
    pub labels: BTreeMap<String, String>,
    pub parse_errors: u32,
    pub capped: bool,
}

fn name_selected(name: &str, include: &[String], exclude: &[String]) -> bool {
    let inc = include.is_empty() || include.iter().any(|g| glob_match(g, name));
    inc && !exclude.iter().any(|g| glob_match(g, name))
}

/// Parse one `{k="v",…}` label block (the slice between the braces).
/// Handles `\"`, `\\` and `\n` escapes; returns `None` on malformed input.
fn parse_labels(s: &str) -> Option<BTreeMap<String, String>> {
    let mut out = BTreeMap::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    loop {
        while i < chars.len() && (chars[i] == ' ' || chars[i] == ',') {
            i += 1;
        }
        if i >= chars.len() {
            return Some(out);
        }
        let ks = i;
        while i < chars.len() && chars[i] != '=' {
            i += 1;
        }
        if i >= chars.len() {
            return None;
        }
        let key: String = chars[ks..i].iter().collect::<String>().trim().to_string();
        i += 1; // '='
        if i >= chars.len() || chars[i] != '"' {
            return None;
        }
        i += 1;
        let mut val = String::new();
        loop {
            if i >= chars.len() {
                return None;
            }
            match chars[i] {
                '\\' => {
                    i += 1;
                    match chars.get(i) {
                        Some('n') => val.push('\n'),
                        Some(c) => val.push(*c),
                        None => return None,
                    }
                }
                '"' => break,
                c => val.push(c),
            }
            i += 1;
        }
        i += 1; // closing quote
        if key.is_empty() {
            return None;
        }
        out.insert(key, val);
    }
}

/// Parse a prometheus text-format body. `include`/`exclude` are series-name
/// globs (empty include = all); `cap` bounds the number of samples. When a
/// body overflows the cap, histogram `_bucket` series are dropped FIRST —
/// they are the bulk of any exporter with per-path labels, and Go's
/// exporter lists them before `http_requests_total`, so a plain "first N
/// lines" cap silently lost the request counter on every busy service.
pub fn parse_prometheus(text: &str, include: &[String], exclude: &[String], cap: usize) -> Parsed {
    let mut p = parse_prometheus_all(text, include, exclude, cap.saturating_mul(6).max(cap));
    if p.samples.len() > cap {
        p.capped = true;
        // Tiers: plain counters / gauges (`http_requests_total`, `up`,
        // `process_*`) → histogram `_count` / `_sum` → `_bucket`. An API
        // gateway with hundreds of paths has thousands of `_count`/`_sum`
        // lines that would otherwise bury the request counter.
        let tier = |m: &str| -> u8 {
            if m.ends_with("_bucket") {
                2
            } else if m.ends_with("_count") || m.ends_with("_sum") {
                1
            } else {
                0
            }
        };
        let mut all: Vec<Sample> = std::mem::take(&mut p.samples);
        all.sort_by_key(|s| tier(&s.metric)); // stable: file order within a tier
        all.truncate(cap);
        p.samples = all;
    }
    p
}

fn parse_prometheus_all(text: &str, include: &[String], exclude: &[String], hard_cap: usize) -> Parsed {
    let mut p = Parsed::default();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if p.samples.len() >= hard_cap {
            p.capped = true;
            break;
        }
        // name[{labels}] value [timestamp]
        let (name, rest) = match line.find(|c: char| c == '{' || c.is_whitespace()) {
            Some(idx) => (&line[..idx], &line[idx..]),
            None => {
                p.parse_errors += 1;
                continue;
            }
        };
        if name.is_empty() {
            p.parse_errors += 1;
            continue;
        }
        let (labels, tail) = if let Some(body) = rest.strip_prefix('{') {
            match body.find('}') {
                Some(end) => match parse_labels(&body[..end]) {
                    Some(l) => (l, &body[end + 1..]),
                    None => {
                        p.parse_errors += 1;
                        continue;
                    }
                },
                None => {
                    p.parse_errors += 1;
                    continue;
                }
            }
        } else {
            (BTreeMap::new(), rest)
        };
        let mut parts = tail.split_whitespace();
        let value = match parts.next().map(parse_prom_float) {
            Some(Some(v)) => v,
            _ => {
                p.parse_errors += 1;
                continue;
            }
        };
        // A trailing timestamp is allowed; anything beyond is malformed.
        if parts.next().is_some() && parts.next().is_some() {
            p.parse_errors += 1;
            continue;
        }
        if !name_selected(name, include, exclude) {
            continue;
        }
        if !value.is_finite() {
            continue;
        }
        p.samples.push(Sample {
            metric: name.to_string(),
            labels,
            value,
        });
    }
    p
}

fn parse_prom_float(s: &str) -> Option<f64> {
    match s {
        "+Inf" | "Inf" => Some(f64::INFINITY),
        "-Inf" => Some(f64::NEG_INFINITY),
        "NaN" | "nan" => Some(f64::NAN),
        _ => s.parse::<f64>().ok(),
    }
}

/// Walk a dotted path (`a.b.0.c`); numeric segments index arrays.
fn json_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.') {
        cur = match cur {
            Value::Object(m) => m.get(seg)?,
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(cur)
}

/// Apply JSON mappings to a body. A mapping whose path is missing or whose
/// value does not parse under its unit counts as one parse error.
pub fn parse_json(body: &str, mappings: &[Mapping]) -> Parsed {
    let mut p = Parsed::default();
    let root: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(_) => {
            p.parse_errors += 1;
            return p;
        }
    };
    for m in mappings {
        let Some(v) = json_path(&root, &m.field) else {
            p.parse_errors += 1;
            continue;
        };
        if let Some(label) = &m.label {
            let s = match v {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                other => other.to_string(),
            };
            p.labels.insert(label.clone(), s);
        }
        if let Some(metric) = &m.metric {
            match parse_unit(v, &m.unit) {
                Some(value) => p.samples.push(Sample {
                    metric: metric.clone(),
                    labels: BTreeMap::new(),
                    value,
                }),
                None => p.parse_errors += 1,
            }
        }
    }
    p
}

/// Health probe: `up` (1 on 2xx) + the raw `http_status`.
pub fn parse_health(status: u16) -> Parsed {
    let up = if (200..300).contains(&status) { 1.0 } else { 0.0 };
    Parsed {
        samples: vec![
            Sample {
                metric: "up".into(),
                labels: BTreeMap::new(),
                value: up,
            },
            Sample {
                metric: "http_status".into(),
                labels: BTreeMap::new(),
                value: f64::from(status),
            },
        ],
        ..Parsed::default()
    }
}

/// Split `"27 MB"` / `"512Mi"` / `"1.5GiB"` into (number, lowercase suffix).
fn split_num_suffix(s: &str) -> Option<(f64, String)> {
    let t = s.trim();
    let end = t
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+'))
        .map(|(i, _)| i)
        .unwrap_or(t.len());
    let num: f64 = t[..end].parse().ok()?;
    Some((num, t[end..].trim().to_ascii_lowercase()))
}

/// Convert a JSON value under a [`Unit`]. Human byte suffixes are treated as
/// binary multiples whether spelled `MB` or `MiB` (the Go runtime prints
/// `"27 MB"` for 27·2²⁰); durations accept `1h2m3s`, `250ms`, bare seconds.
pub fn parse_unit(raw: &Value, unit: &Unit) -> Option<f64> {
    match unit {
        Unit::Number | Unit::Bytes => match raw {
            Value::Number(n) => n.as_f64(),
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Value::String(s) => s.trim().parse::<f64>().ok(),
            _ => None,
        },
        Unit::Percent => match raw {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().trim_end_matches('%').trim().parse::<f64>().ok(),
            _ => None,
        },
        Unit::BytesHuman => match raw {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => {
                let (n, suf) = split_num_suffix(s)?;
                let mult: f64 = match suf.as_str() {
                    "" | "b" => 1.0,
                    "k" | "kb" | "ki" | "kib" => 1024.0,
                    "m" | "mb" | "mi" | "mib" => 1024.0 * 1024.0,
                    "g" | "gb" | "gi" | "gib" => 1024.0 * 1024.0 * 1024.0,
                    "t" | "tb" | "ti" | "tib" => 1024.0 * 1024.0 * 1024.0 * 1024.0,
                    _ => return None,
                };
                Some(n * mult)
            }
            _ => None,
        },
        Unit::DurationHuman => match raw {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => parse_duration_secs(s),
            _ => None,
        },
    }
}

fn parse_duration_secs(s: &str) -> Option<f64> {
    let t = s.trim();
    if let Ok(v) = t.parse::<f64>() {
        return Some(v);
    }
    let mut total = 0.0;
    let mut num = String::new();
    let mut unit = String::new();
    let mut any = false;
    let flush = |num: &mut String, unit: &mut String, total: &mut f64| -> Option<()> {
        if num.is_empty() {
            return if unit.is_empty() { Some(()) } else { None };
        }
        let n: f64 = num.parse().ok()?;
        let m = match unit.as_str() {
            "h" => 3600.0,
            "m" => 60.0,
            "s" => 1.0,
            "ms" => 0.001,
            "us" | "µs" => 0.000_001,
            "ns" => 0.000_000_001,
            _ => return None,
        };
        *total += n * m;
        num.clear();
        unit.clear();
        Some(())
    };
    for c in t.chars() {
        if c.is_ascii_digit() || c == '.' {
            if !unit.is_empty() {
                flush(&mut num, &mut unit, &mut total)?;
            }
            num.push(c);
            any = true;
        } else if c.is_alphabetic() || c == 'µ' {
            unit.push(c);
        } else if c.is_whitespace() {
            continue;
        } else {
            return None;
        }
    }
    flush(&mut num, &mut unit, &mut total)?;
    if any {
        Some(total)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn prometheus_counter_gauge_histogram_with_labels_and_escapes() {
        let t = r#"# HELP http_requests_total x
# TYPE http_requests_total counter
http_requests_total{method="GET",path="/a\"b",code="200"} 1027 1395066363000
http_requests_total{method="POST"} 3
http_request_duration_seconds_bucket{le="0.1"} 5
http_request_duration_seconds_bucket{le="+Inf"} 9
http_request_duration_seconds_sum 1.5
http_request_duration_seconds_count 9
go_goroutines 84
bad line here
nan_metric NaN
"#;
        let p = parse_prometheus(t, &["http_*".into()], &[], SERIES_CAP);
        assert_eq!(p.parse_errors, 1);
        assert!(p.samples.iter().all(|s| s.metric.starts_with("http_")));
        let get = p
            .samples
            .iter()
            .find(|s| s.labels.get("method").map(String::as_str) == Some("GET"))
            .unwrap();
        assert_eq!(get.value, 1027.0);
        assert_eq!(get.labels["path"], "/a\"b");
        assert_eq!(get.labels["code"], "200");
        assert!(p.samples.iter().any(|s| s.metric == "http_request_duration_seconds_bucket"
            && s.labels["le"] == "+Inf"
            && s.value == 9.0));
        let p2 = parse_prometheus(t, &[], &["*_bucket".into()], SERIES_CAP);
        assert!(!p2.samples.iter().any(|s| s.metric.ends_with("_bucket")));
        assert!(!p2.samples.iter().any(|s| s.value.is_nan()), "NaN dropped");
        assert!(p2.samples.iter().any(|s| s.metric == "go_goroutines" && s.value == 84.0));
        assert!(!p2.capped);
    }

    #[test]
    fn prometheus_cap() {
        let mut t = String::new();
        for i in 0..600 {
            t.push_str(&format!("m{{i=\"{i}\"}} 1\n"));
        }
        let p = parse_prometheus(&t, &[], &[], 500);
        assert_eq!(p.samples.len(), 500);
        assert!(p.capped);
    }

    #[test]
    fn prometheus_cap_drops_buckets_before_counters() {
        // Go exporter order: buckets (many) come before http_requests_total.
        let mut t = String::new();
        for i in 0..40 {
            for le in ["0.1", "0.5", "1", "+Inf"] {
                t.push_str(&format!("http_request_duration_seconds_bucket{{path=\"/p{i}\",le=\"{le}\"}} 1\n"));
            }
        }
        // …then per-path _count / _sum (an API gateway has hundreds of paths)…
        for i in 0..40 {
            t.push_str(&format!("http_request_duration_seconds_count{{path=\"/p{i}\"}} 9\n"));
            t.push_str(&format!("http_request_duration_seconds_sum{{path=\"/p{i}\"}} 1.5\n"));
        }
        // …and only then the request counter.
        for i in 0..40 {
            t.push_str(&format!("http_requests_total{{path=\"/p{i}\",code=\"200\"}} 5\n"));
        }
        let p = parse_prometheus(&t, &[], &[], 100);
        assert!(p.capped);
        assert_eq!(p.samples.len(), 100);
        assert_eq!(p.samples.iter().filter(|s| s.metric == "http_requests_total").count(), 40, "tier 0 kept whole");
        assert_eq!(
            p.samples.iter().filter(|s| s.metric.ends_with("_count") || s.metric.ends_with("_sum")).count(),
            60,
            "tier 1 fills the rest"
        );
        assert_eq!(p.samples.iter().filter(|s| s.metric.ends_with("_bucket")).count(), 0, "buckets go first");
    }

    #[test]
    fn prometheus_malformed_labels_counted() {
        let p = parse_prometheus("a{x=\"unterminated} 1\nb{=\"v\"} 2\nc{x=\"v\" 3\n", &[], &[], 10);
        assert_eq!(p.samples.len(), 0);
        assert_eq!(p.parse_errors, 3);
    }

    #[test]
    fn json_mappings_units_and_labels() {
        let body = r#"{"build_info":{"version":"5.02.25","commit":"c0c4dce"},"go_routines_num":88,"memory_stats":{"alloc":"12 MB","sys":"27 MB","total_alloc":"10729 MB"},"vcpu":16,"arr":[{"v":7}]}"#;
        let m = |f: &str, metric: Option<&str>, label: Option<&str>, unit: Unit| Mapping {
            field: f.into(),
            metric: metric.map(String::from),
            label: label.map(String::from),
            unit,
        };
        let p = parse_json(
            body,
            &[
                m("memory_stats.sys", Some("mem_sys_bytes"), None, Unit::BytesHuman),
                m("go_routines_num", Some("goroutines"), None, Unit::Number),
                m("build_info.version", None, Some("version"), Unit::Number),
                m("arr.0.v", Some("arr_v"), None, Unit::Number),
                m("nope.x", Some("x"), None, Unit::Number),
            ],
        );
        assert_eq!(
            p.samples.iter().find(|s| s.metric == "mem_sys_bytes").unwrap().value,
            27.0 * 1024.0 * 1024.0
        );
        assert_eq!(p.samples.iter().find(|s| s.metric == "goroutines").unwrap().value, 88.0);
        assert_eq!(p.samples.iter().find(|s| s.metric == "arr_v").unwrap().value, 7.0);
        assert_eq!(p.labels["version"], "5.02.25");
        assert_eq!(p.parse_errors, 1);
        assert_eq!(parse_json("not json", &[]).parse_errors, 1);
    }

    #[test]
    fn units() {
        assert_eq!(parse_unit(&json!("1.5 GiB"), &Unit::BytesHuman), Some(1.5 * 1073741824.0));
        assert_eq!(parse_unit(&json!("512Mi"), &Unit::BytesHuman), Some(512.0 * 1048576.0));
        assert_eq!(parse_unit(&json!("2GB"), &Unit::BytesHuman), Some(2.0 * 1073741824.0));
        assert_eq!(parse_unit(&json!("1024"), &Unit::BytesHuman), Some(1024.0));
        assert_eq!(parse_unit(&json!(4096), &Unit::BytesHuman), Some(4096.0));
        assert_eq!(parse_unit(&json!("3 parsecs"), &Unit::BytesHuman), None);
        assert_eq!(parse_unit(&json!("1m30s"), &Unit::DurationHuman), Some(90.0));
        assert_eq!(parse_unit(&json!("250ms"), &Unit::DurationHuman), Some(0.25));
        assert_eq!(parse_unit(&json!("1h"), &Unit::DurationHuman), Some(3600.0));
        assert_eq!(parse_unit(&json!("12"), &Unit::DurationHuman), Some(12.0));
        assert_eq!(parse_unit(&json!("45%"), &Unit::Percent), Some(45.0));
        assert_eq!(parse_unit(&json!(true), &Unit::Number), Some(1.0));
        assert_eq!(parse_unit(&json!("x"), &Unit::Number), None);
        assert_eq!(parse_unit(&json!(null), &Unit::Number), None);
    }

    #[test]
    fn health_probe() {
        let p = parse_health(503);
        assert!(p.samples.iter().any(|s| s.metric == "up" && s.value == 0.0));
        assert!(p.samples.iter().any(|s| s.metric == "http_status" && s.value == 503.0));
        assert!(parse_health(200).samples.iter().any(|s| s.metric == "up" && s.value == 1.0));
    }
}
