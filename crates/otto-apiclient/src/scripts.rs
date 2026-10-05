//! Server-side pre-request / post-response script runtime for API-client
//! automations. Runs the user's JS through `boa_engine` with a `pm` object
//! whose surface is ported verbatim from the interactive runner
//! (`ui/src/lib/api/scripts.ts`), so a script behaves the same whether the
//! user clicks Send or an automation replays the stored request. Like the UI
//! runtime this is a convenience engine, not a security sandbox.
//!
//! boa's loop limit is PER LOOP (nested loops multiply it) and it has no
//! wall-clock or heap bound, so a script from an imported collection could pin
//! a thread forever or abort the process on allocation failure. The daemon
//! therefore runs every script through [`run_pre_request_isolated`] /
//! [`run_post_response_isolated`]: in a child `ottod apiclient-script`
//! process (registered with [`set_script_host`]) that is killed after
//! [`SCRIPT_TIMEOUT`], so a runaway or OOM script costs one child, never the
//! daemon. At most [`MAX_CONCURRENT_SCRIPTS`] run at once. Without a
//! registered host (unit tests, other binaries) the script runs in-process on
//! the blocking pool — never on an async worker.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use boa_engine::{Context, Source};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Semaphore;

/// Mutable request view a pre-request script works on. `headers` keeps the
/// wire `[{key,value,enabled}]` shape so mutations round-trip losslessly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptRequest {
    pub method: String,
    pub url: String,
    pub headers: Value,
    pub body: String,
}

/// Response view handed to a post-response script.
#[derive(Debug, Clone, Serialize)]
pub struct ScriptResponse {
    pub code: u16,
    pub status: String,
    pub response_time: i64,
    /// Lower-cased header name → value (mirrors the UI runner).
    pub headers: BTreeMap<String, String>,
    pub body_text: String,
}

/// One `pm.test(...)` result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScriptTest {
    pub name: String,
    pub passed: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// Outcome of one script run. `error` is a script-level failure (syntax or
/// uncaught throw); individual test failures land in `tests` instead.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ScriptOutcome {
    pub logs: Vec<String>,
    pub error: Option<String>,
    pub tests: Vec<ScriptTest>,
    pub vars: BTreeMap<String, String>,
    pub request: Option<ScriptRequest>,
}

#[derive(Deserialize)]
struct RawOutcome {
    #[serde(default)]
    logs: Vec<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    tests: Vec<ScriptTest>,
    #[serde(default)]
    vars: BTreeMap<String, String>,
    #[serde(default)]
    req: Option<ScriptRequest>,
}

/// The `pm` definition, ported from `ui/src/lib/api/scripts.ts`. Expects
/// `__ctx` (vars + req/resp) to be defined; leaves `pm`, `__logs`, `__tests`,
/// `console` in scope.
const PRELUDE: &str = r#"
const __logs = [];
const __tests = [];
const __vars = __ctx.vars;
const __str = (x) => (typeof x === 'string' ? x : JSON.stringify(x));
const console = {
  log: (...a) => __logs.push(a.map(__str).join(' ')),
  error: (...a) => __logs.push('ERROR: ' + a.map(String).join(' ')),
  warn: (...a) => __logs.push('WARN: ' + a.map(String).join(' ')),
  info: (...a) => __logs.push(a.map(String).join(' ')),
};
const __v = {
  get: (k) => __vars[k],
  set: (k, v) => { __vars[k] = typeof v === 'string' ? v : JSON.stringify(v); },
  unset: (k) => { delete __vars[k]; },
  has: (k) => k in __vars,
  toObject: () => Object.assign({}, __vars),
};
function __expect(actual) {
  const assert = (cond, msg) => { if (!cond) throw new Error(msg); };
  const eq = (e) => assert(JSON.stringify(actual) === JSON.stringify(e),
    'expected ' + JSON.stringify(actual) + ' to equal ' + JSON.stringify(e));
  return {
    toBe: (e) => assert(actual === e, 'expected ' + JSON.stringify(actual) + ' to be ' + JSON.stringify(e)),
    toEqual: eq,
    eql: eq,
    toContain: (e) => assert(
      (typeof actual === 'string' && actual.includes(String(e))) || (Array.isArray(actual) && actual.includes(e)),
      'expected ' + JSON.stringify(actual) + ' to contain ' + JSON.stringify(e)),
    toBeTruthy: () => assert(!!actual, 'expected ' + JSON.stringify(actual) + ' to be truthy'),
    toBeFalsy: () => assert(!actual, 'expected ' + JSON.stringify(actual) + ' to be falsy'),
    above: (n) => assert(Number(actual) > n, 'expected ' + actual + ' to be above ' + n),
    below: (n) => assert(Number(actual) < n, 'expected ' + actual + ' to be below ' + n),
  };
}
"#;

const PRE_PM: &str = r#"
const __req = __ctx.req;
const __headers = {
  add: (h) => __req.headers.push({ key: h.key, value: h.value, enabled: true }),
  upsert: (h) => {
    const i = __req.headers.findIndex((x) => x.key.toLowerCase() === h.key.toLowerCase());
    if (i >= 0) __req.headers[i] = Object.assign({}, __req.headers[i], { value: h.value });
    else __req.headers.push({ key: h.key, value: h.value, enabled: true });
  },
  remove: (k) => { __req.headers = __req.headers.filter((x) => x.key.toLowerCase() !== k.toLowerCase()); },
  get: (k) => { const f = __req.headers.find((x) => x.key.toLowerCase() === k.toLowerCase()); return f ? f.value : undefined; },
};
const pm = {
  environment: __v, variables: __v, globals: __v, expect: __expect,
  request: {
    get method() { return __req.method; }, set method(m) { __req.method = m; },
    get url() { return __req.url; }, set url(u) { __req.url = u; },
    get body() { return __req.body; }, set body(b) { __req.body = b; },
    headers: __headers,
    addHeader: __headers.add,
  },
};
"#;

const POST_PM: &str = r#"
const __resp = __ctx.resp;
const pm = {
  environment: __v, variables: __v, globals: __v, expect: __expect,
  response: {
    code: __resp.code,
    status: __resp.status,
    responseTime: __resp.response_time,
    headers: __resp.headers,
    text: () => __resp.body_text,
    json: () => JSON.parse(__resp.body_text),
  },
  test: (name, fn) => {
    try { fn(); __tests.push({ name: name, passed: true }); }
    catch (e) { __tests.push({ name: name, passed: false, error: e instanceof Error ? e.message : String(e) }); }
  },
};
"#;

/// Wrap user code so an uncaught throw is captured, then serialize the whole
/// outcome as the eval result.
fn epilogue(kind: &str) -> String {
    format!(
        r#"
let __err = null;
try {{ __user(pm, console); }}
catch (e) {{ __err = (e && e.name) ? (e.name + ': ' + e.message) : String(e); }}
JSON.stringify({{ logs: __logs, tests: __tests, vars: __vars, error: __err, req: {} }});
"#,
        if kind == "pre" { "__req" } else { "null" }
    )
}

fn run(kind: &str, user_code: &str, ctx_json: &Value) -> ScriptOutcome {
    let pm_def = if kind == "pre" { PRE_PM } else { POST_PM };
    let src = format!(
        "const __ctx = {ctx};\n{prelude}\n{pm}\nconst __user = function(pm, console) {{\n{code}\n}};\n{epi}",
        ctx = ctx_json,
        prelude = PRELUDE,
        pm = pm_def,
        code = user_code,
        epi = epilogue(kind),
    );

    let mut context = Context::default();
    // A hostile/buggy loop errors out instead of pinning the daemon.
    context
        .runtime_limits_mut()
        .set_loop_iteration_limit(1_000_000);
    context.runtime_limits_mut().set_recursion_limit(512);

    match context.eval(Source::from_bytes(src.as_bytes())) {
        Ok(v) => {
            let raw = v
                .to_string(&mut context)
                .map(|s| s.to_std_string_escaped())
                .unwrap_or_default();
            match serde_json::from_str::<RawOutcome>(&raw) {
                Ok(o) => ScriptOutcome {
                    logs: o.logs,
                    error: o.error,
                    tests: o.tests,
                    vars: o.vars,
                    request: o.req,
                },
                Err(e) => ScriptOutcome {
                    error: Some(format!("script outcome parse failed: {e}")),
                    ..Default::default()
                },
            }
        }
        // Syntax error in the user code (it is parsed as part of the whole
        // source) or an engine-level failure (e.g. loop limit).
        Err(e) => ScriptOutcome {
            error: Some(format!("{e}")),
            ..Default::default()
        },
    }
}

/// Run a pre-request script: may mutate the request and set variables.
pub fn run_pre_request(
    code: &str,
    request: &ScriptRequest,
    vars: &BTreeMap<String, String>,
) -> ScriptOutcome {
    if code.trim().is_empty() {
        return ScriptOutcome {
            vars: vars.clone(),
            request: Some(request.clone()),
            ..Default::default()
        };
    }
    let ctx = serde_json::json!({ "vars": vars, "req": request });
    run("pre", code, &ctx)
}

/// Run a post-response (tests) script: reads the response, may set variables.
pub fn run_post_response(
    code: &str,
    response: &ScriptResponse,
    vars: &BTreeMap<String, String>,
) -> ScriptOutcome {
    if code.trim().is_empty() {
        return ScriptOutcome {
            vars: vars.clone(),
            ..Default::default()
        };
    }
    let ctx = serde_json::json!({ "vars": vars, "resp": response });
    run("post", code, &ctx)
}

/// argv[1] that makes `ottod` run ONE script from stdin ([`child_main`]).
pub const CHILD_ARG: &str = "apiclient-script";
/// Wall clock a script may run before its child process is killed.
pub const SCRIPT_TIMEOUT: Duration = Duration::from_secs(10);
/// Scripts running at once, daemon-wide (each may burn a core until killed).
pub const MAX_CONCURRENT_SCRIPTS: usize = 4;
/// Cap on a child's stdout (the serialized outcome).
const CHILD_OUTPUT_CAP: u64 = 32 * 1024 * 1024;

static SCRIPT_HOST: OnceLock<PathBuf> = OnceLock::new();
static SCRIPT_SLOTS: Semaphore = Semaphore::const_new(MAX_CONCURRENT_SCRIPTS);

/// Register the executable that runs scripts out of process (`ottod` itself,
/// dispatching [`CHILD_ARG`] to [`child_main`]). First call wins.
pub fn set_script_host(exe: PathBuf) {
    let _ = SCRIPT_HOST.set(exe);
}

/// One script job, as sent to the child on stdin.
#[derive(Serialize, Deserialize)]
struct ChildJob {
    kind: String,
    code: String,
    ctx: Value,
}

/// Entry point of `ottod apiclient-script`: read one [`ChildJob`] from stdin,
/// run it, write the [`ScriptOutcome`] JSON to stdout. Returns success.
pub fn child_main() -> bool {
    use std::io::{Read, Write};
    let mut input = Vec::new();
    if std::io::stdin().read_to_end(&mut input).is_err() {
        return false;
    }
    let Ok(job) = serde_json::from_slice::<ChildJob>(&input) else {
        return false;
    };
    let outcome = run(&job.kind, &job.code, &job.ctx);
    let Ok(out) = serde_json::to_vec(&outcome) else {
        return false;
    };
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&out).is_ok() && stdout.flush().is_ok()
}

fn failed(error: String) -> ScriptOutcome {
    ScriptOutcome {
        error: Some(error),
        ..Default::default()
    }
}

/// Run `job` in `program args…`, killing it after `timeout`. Any failure
/// (spawn, crash, OOM abort, timeout, garbage output) is a script error.
async fn run_in_child(
    program: &OsStr,
    args: &[&str],
    job: &ChildJob,
    timeout: Duration,
) -> ScriptOutcome {
    let payload = match serde_json::to_vec(job) {
        Ok(p) => p,
        Err(e) => return failed(format!("script encode failed: {e}")),
    };
    let mut child = match tokio::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => return failed(format!("script runner failed to start: {e}")),
    };
    let (Some(mut stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        return failed("script runner has no stdio".into());
    };
    let exchange = async {
        // A child that dies early closes stdin; its exit status says why.
        let _ = stdin.write_all(&payload).await;
        drop(stdin);
        let mut out = Vec::new();
        stdout.take(CHILD_OUTPUT_CAP).read_to_end(&mut out).await?;
        let status = child.wait().await?;
        Ok::<_, std::io::Error>((status, out))
    };
    let result = tokio::time::timeout(timeout, exchange).await;
    match result {
        Ok(Ok((status, out))) if status.success() => serde_json::from_slice(&out)
            .unwrap_or_else(|e| failed(format!("script outcome parse failed: {e}"))),
        Ok(Ok((status, _))) => failed(format!(
            "script runner failed ({status}) — the script may have exhausted memory"
        )),
        Ok(Err(e)) => failed(format!("script runner I/O failed: {e}")),
        Err(_) => {
            let _ = child.start_kill();
            failed(format!(
                "script timed out after {} s and was stopped",
                timeout.as_secs()
            ))
        }
    }
}

async fn run_isolated(kind: &str, code: &str, ctx: Value) -> ScriptOutcome {
    let Ok(_slot) = SCRIPT_SLOTS.acquire().await else {
        return failed("script runner unavailable".into());
    };
    let job = ChildJob {
        kind: kind.to_string(),
        code: code.to_string(),
        ctx,
    };
    match SCRIPT_HOST.get() {
        Some(exe) => run_in_child(exe.as_os_str(), &[CHILD_ARG], &job, SCRIPT_TIMEOUT).await,
        None => tokio::task::spawn_blocking(move || run(&job.kind, &job.code, &job.ctx))
            .await
            .unwrap_or_else(|e| failed(format!("script task failed: {e}"))),
    }
}

/// [`run_pre_request`], isolated (out of process when a host is registered,
/// bounded by [`SCRIPT_TIMEOUT`]) — what the daemon calls.
pub async fn run_pre_request_isolated(
    code: &str,
    request: &ScriptRequest,
    vars: &BTreeMap<String, String>,
) -> ScriptOutcome {
    if code.trim().is_empty() {
        return run_pre_request(code, request, vars);
    }
    let ctx = serde_json::json!({ "vars": vars, "req": request });
    run_isolated("pre", code, ctx).await
}

/// [`run_post_response`], isolated like [`run_pre_request_isolated`].
pub async fn run_post_response_isolated(
    code: &str,
    response: &ScriptResponse,
    vars: &BTreeMap<String, String>,
) -> ScriptOutcome {
    if code.trim().is_empty() {
        return run_post_response(code, response, vars);
    }
    let ctx = serde_json::json!({ "vars": vars, "resp": response });
    run_isolated("post", code, ctx).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn job() -> ChildJob {
        ChildJob {
            kind: "pre".into(),
            code: "for (;;) { for (;;) {} }".into(),
            ctx: json!({}),
        }
    }

    /// S6-04: a child that never finishes is killed at the timeout and the
    /// caller gets a script error — the daemon is never pinned.
    #[tokio::test]
    async fn hung_script_child_is_killed_at_the_timeout() {
        let started = std::time::Instant::now();
        let out = run_in_child(
            OsStr::new("/bin/sleep"),
            &["30"],
            &job(),
            Duration::from_millis(300),
        )
        .await;
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(out.error.unwrap().contains("timed out"));
    }

    /// S6-04: a child that dies (e.g. aborts on allocation failure) is a
    /// script error, not a daemon crash.
    #[tokio::test]
    async fn crashed_script_child_is_a_script_error() {
        let out = run_in_child(
            OsStr::new("/usr/bin/false"),
            &[],
            &job(),
            Duration::from_secs(5),
        )
        .await;
        assert!(out.error.unwrap().contains("script runner failed"));
    }

    /// The child protocol round-trips: what `child_main` writes is what the
    /// parent parses.
    #[test]
    fn outcome_round_trips_over_the_child_protocol() {
        let out = run_pre_request("pm.environment.set('a', 'b');", &req(), &BTreeMap::new());
        let wire = serde_json::to_vec(&out).unwrap();
        let back: ScriptOutcome = serde_json::from_slice(&wire).unwrap();
        assert_eq!(back.vars.get("a").unwrap(), "b");
        assert_eq!(back.request.unwrap().url, req().url);
    }

    /// Without a registered host the isolated runner still works (blocking
    /// pool), so tests and non-daemon binaries keep running scripts.
    #[tokio::test]
    async fn isolated_runner_falls_back_in_process() {
        let out =
            run_pre_request_isolated("pm.environment.set('k', 'v');", &req(), &BTreeMap::new())
                .await;
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.vars.get("k").unwrap(), "v");
    }

    fn req() -> ScriptRequest {
        ScriptRequest {
            method: "GET".into(),
            url: "https://api.test/users".into(),
            headers: json!([{ "key": "Accept", "value": "application/json", "enabled": true }]),
            body: String::new(),
        }
    }

    #[test]
    fn pre_mutates_request_and_vars() {
        let out = run_pre_request(
            "pm.environment.set('who', 'otto');\n\
             pm.request.headers.upsert({ key: 'X-Trace', value: 'on' });\n\
             pm.request.url = pm.request.url + '?page=2';\n\
             console.log('ready', pm.environment.get('who'));",
            &req(),
            &BTreeMap::new(),
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.vars.get("who").unwrap(), "otto");
        let r = out.request.unwrap();
        assert!(r.url.ends_with("?page=2"));
        let hdrs = r.headers.as_array().unwrap();
        assert!(hdrs
            .iter()
            .any(|h| h["key"] == "X-Trace" && h["value"] == "on"));
        assert_eq!(out.logs, vec!["ready otto"]);
    }

    #[test]
    fn post_tests_and_extraction() {
        let resp = ScriptResponse {
            code: 200,
            status: "OK".into(),
            response_time: 12,
            headers: BTreeMap::from([("content-type".to_string(), "application/json".to_string())]),
            body_text: r#"{"token":"abc","n":3}"#.into(),
        };
        let out = run_post_response(
            "const b = pm.response.json();\n\
             pm.environment.set('auth', b.token);\n\
             pm.test('status ok', () => pm.expect(pm.response.code).toBe(200));\n\
             pm.test('n large', () => pm.expect(b.n).above(10));",
            &resp,
            &BTreeMap::new(),
        );
        assert!(out.error.is_none(), "{:?}", out.error);
        assert_eq!(out.vars.get("auth").unwrap(), "abc");
        assert_eq!(out.tests.len(), 2);
        assert!(out.tests[0].passed);
        assert!(!out.tests[1].passed);
    }

    #[test]
    fn syntax_error_is_captured_not_panicking() {
        let out = run_pre_request("this is not js {", &req(), &BTreeMap::new());
        assert!(out.error.is_some());
    }

    #[test]
    fn runaway_loop_hits_limit() {
        let out = run_pre_request("while (true) {}", &req(), &BTreeMap::new());
        assert!(out.error.is_some());
    }

    #[test]
    fn empty_script_passes_through() {
        let vars = BTreeMap::from([("a".to_string(), "1".to_string())]);
        let out = run_pre_request("   ", &req(), &vars);
        assert!(out.error.is_none());
        assert_eq!(out.vars.get("a").unwrap(), "1");
    }
}
