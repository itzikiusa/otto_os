//! Mobile Simulators & Emulators slice of the outward MCP surface (`otto.sim_*`):
//! - Reads (`Browser: View`): `sim_list` (`DEFAULT_ENABLED`) and `sim_logs` (opt-in:
//!   device logs carry app tokens and PII, like `k8s_logs`).
//! - Controls (`DANGEROUS`, reversible, `Browser: Edit`): `sim_action` (boot/shutdown/
//!   open_url/orientation/appearance/launch_app/terminate_app) and `sim_input`
//!   (tap/swipe/text/key_event).
//!
//! No screenshot tool: the governed path returns text only, and a frame is
//! megabytes of base64. Kept compact (4 tools) so `ottod`'s `tools/list` byte budget stays comfortably
//! under its ceiling, and kept in its own module so `catalog::otto_tool_specs`
//! and `exec::route_for` stay strictly under their LOC ratchets.

use otto_core::Error;
use serde_json::{json, Value};

use super::catalog::preview;
use super::exec::{arg_str, opt_query, seg, SelfCall};

const CAT: &str = "Simulators";

/// Tool specs appended to `otto_tool_specs()`.
pub fn specs() -> Vec<Value> {
    let dev = json!({"type":"string"});
    vec![
        json!({"name":"otto.sim_list","mutating":false,"category":CAT,
            "description":"List iOS simulators and Android emulators: state, screen, capabilities, host readiness. Read-only.",
            "inputSchema":{"type":"object","properties":{
                "platform":{"type":"string","enum":["ios","android"]},
                "state":{"type":"string","enum":["booted","booting","shutdown","offline"]},
                "refresh":{"type":"boolean"}}}}),
        json!({"name":"otto.sim_logs","mutating":false,"category":CAT,
            "description":"Recent device log lines (Android logcat; iOS: the last 2 minutes). Read-only.",
            "inputSchema":{"type":"object","required":["device_id"],"properties":{
                "device_id":dev,"lines":{"type":"integer"},
                "level":{"type":"string","enum":["verbose","debug","info","warn","error","fatal"]},
                "filter":{"type":"string"}}}}),
        json!({"name":"otto.sim_action","mutating":true,"category":CAT,
            "description":"Control a simulator. boot stays embedded unless open_gui; open_url forwards localhost ports on Android. Approval-gated.",
            "inputSchema":{"type":"object","required":["device_id","action"],"properties":{
                "device_id":dev,
                "action":{"type":"string","enum":["boot","shutdown","open_url","orientation","appearance","launch_app","terminate_app"]},
                "url":{"type":"string"},"auto_forward_localhost":{"type":"boolean"},
                "orientation":{"type":"string","enum":["portrait","landscape","portrait_upside_down","landscape_right"]},
                "appearance":{"type":"string","enum":["light","dark"]},
                "app_id":{"type":"string"},"args":{"type":"array","items":{"type":"string"}},
                "wait":{"type":"boolean"},"open_gui":{"type":"boolean"}}}}),
        json!({"name":"otto.sim_input","mutating":true,"category":CAT,
            "description":"Tap, swipe, type text or press a key (HOME, BACK, ENTER, DEL…) on a booted simulator; normalized:true takes 0..1 coordinates. Approval-gated.",
            "inputSchema":{"type":"object","required":["device_id","type"],"properties":{
                "device_id":dev,"type":{"type":"string","enum":["tap","swipe","text","key_event"]},
                "x":{"type":"number"},"y":{"type":"number"},
                "x1":{"type":"number"},"y1":{"type":"number"},"x2":{"type":"number"},"y2":{"type":"number"},
                "duration_ms":{"type":"integer"},"normalized":{"type":"boolean"},
                "text":{"type":"string"},"key":{"type":"string"}}}}),
    ]
}

/// Map `sim_*` tools to their `/api/v1/simulators/*` self-calls.
pub fn route_for(tool: &str, args: &Value) -> Result<SelfCall, Error> {
    Ok(match tool {
        "sim_list" => {
            let q = opt_query(
                args,
                &[
                    ("platform", "platform"),
                    ("state", "state"),
                    ("refresh", "refresh"),
                ],
            );
            SelfCall::get(with_query("/api/v1/simulators/devices", &q))
        }
        "sim_logs" => {
            let id = seg(&arg_str(args, "device_id")?);
            let q = opt_query(
                args,
                &[("lines", "lines"), ("level", "level"), ("filter", "filter")],
            );
            SelfCall::get(with_query(
                &format!("/api/v1/simulators/devices/{id}/logs"),
                &q,
            ))
        }
        "sim_action" => route_sim_action(args)?,
        "sim_input" => {
            let id = seg(&arg_str(args, "device_id")?);
            let kind = arg_str(args, "type")?;
            let mut body = json!({"type": kind});
            for k in [
                "x",
                "y",
                "x1",
                "y1",
                "x2",
                "y2",
                "duration_ms",
                "normalized",
                "text",
                "key",
            ] {
                if let Some(v) = args.get(k).filter(|v| !v.is_null()) {
                    body[k] = v.clone();
                }
            }
            SelfCall::post(format!("/api/v1/simulators/devices/{id}/input"), body)
        }
        other => return Err(Error::Invalid(format!("unknown otto tool '{other}'"))),
    })
}

fn route_sim_action(args: &Value) -> Result<SelfCall, Error> {
    let id = seg(&arg_str(args, "device_id")?);
    let action = arg_str(args, "action")?;
    Ok(match action.as_str() {
        "boot" => {
            // The route's own defaults decide the window (embedded unless open_gui).
            let mut body = json!({"wait": args.get("wait").and_then(Value::as_bool).unwrap_or(true)});
            if let Some(g) = args.get("open_gui").and_then(Value::as_bool) {
                body["open_gui"] = json!(g);
            }
            SelfCall::post(format!("/api/v1/simulators/devices/{id}/boot"), body)
        }
        "shutdown" => SelfCall::post(format!("/api/v1/simulators/devices/{id}/shutdown"), json!({})),
        "open_url" => {
            let url = arg_str(args, "url")?;
            let auto_fwd = args
                .get("auto_forward_localhost")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            SelfCall::post(
                format!("/api/v1/simulators/devices/{id}/open-url"),
                json!({"url": url, "auto_forward_localhost": auto_fwd}),
            )
        }
        "orientation" => SelfCall::post(
            format!("/api/v1/simulators/devices/{id}/orientation"),
            json!({"orientation": arg_str(args, "orientation")?}),
        ),
        "appearance" => SelfCall::post(
            format!("/api/v1/simulators/devices/{id}/appearance"),
            json!({"appearance": arg_str(args, "appearance")?}),
        ),
        "launch_app" => {
            let mut body = json!({"app_id": arg_str(args, "app_id")?});
            if let Some(a) = args.get("args").filter(|v| !v.is_null()) {
                body["args"] = a.clone();
            }
            SelfCall::post(format!("/api/v1/simulators/devices/{id}/apps/launch"), body)
        }
        "terminate_app" => SelfCall::post(
            format!("/api/v1/simulators/devices/{id}/apps/terminate"),
            json!({"app_id": arg_str(args, "app_id")?}),
        ),
        other => {
            return Err(Error::Invalid(format!(
                "unknown sim_action '{other}'; expected boot, shutdown, open_url, orientation, appearance, launch_app, or terminate_app"
            )))
        }
    })
}

fn with_query(path: &str, q: &str) -> String {
    if q.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{q}")
    }
}

/// Human-facing approval detail for simulator control tools.
pub fn dangerous_detail(short: &str, args: &Value) -> Option<String> {
    let s = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or("?");
    let dev = s("device_id");
    Some(match short {
        "sim_action" => match s("action") {
            "boot" => {
                let mode = if args.get("open_gui").and_then(Value::as_bool) == Some(true) {
                    "native window"
                } else {
                    "embedded/headless"
                };
                format!("Boot simulator '{dev}' ({mode})")
            }
            "shutdown" => format!("Shut down simulator '{dev}'"),
            "open_url" => format!("Open URL '{}' on simulator '{dev}'", preview(s("url"), 240)),
            "orientation" => format!("Rotate simulator '{dev}' to {}", s("orientation")),
            "appearance" => format!("Set simulator '{dev}' appearance to {}", s("appearance")),
            "launch_app" => format!("Launch app '{}' on simulator '{dev}'", s("app_id")),
            "terminate_app" => format!("Terminate app '{}' on simulator '{dev}'", s("app_id")),
            act => format!("Simulator action '{act}' on '{dev}'"),
        },
        "sim_input" => {
            let n = |k: &str| args.get(k).map_or("?".into(), Value::to_string);
            match s("type") {
                "text" => format!("Type '{}' on simulator '{dev}'", preview(s("text"), 120)),
                "key_event" => format!("Send key '{}' to simulator '{dev}'", s("key")),
                "tap" => format!("Tap ({}, {}) on simulator '{dev}'", n("x"), n("y")),
                "swipe" => format!(
                    "Swipe ({}, {}) → ({}, {}) on simulator '{dev}'",
                    n("x1"),
                    n("y1"),
                    n("x2"),
                    n("y2")
                ),
                kind => format!("Send '{kind}' input to simulator '{dev}'"),
            }
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outward::{
        dangerous_detail as catalog_detail, otto_tool_specs, pin_global, route_for as exec_route,
        Method, DANGEROUS, DEFAULT_ENABLED, IRREVERSIBLE, OPT_IN_READS,
    };

    #[test]
    fn simulator_tools_present_classified_pinned_and_routed() {
        let all = otto_tool_specs();
        for name in ["sim_list", "sim_logs", "sim_action", "sim_input"] {
            let spec = all
                .iter()
                .find(|s| s["name"] == format!("otto.{name}"))
                .unwrap_or_else(|| panic!("missing spec otto.{name}"));
            assert_eq!(spec["category"], json!(CAT));
            assert!(pin_global(name), "{name} must be host-global in pin_global");
        }
        assert!(!all.iter().any(|s| s["name"] == "otto.sim_screenshot"));
        assert!(DEFAULT_ENABLED.contains(&"sim_list") && !DANGEROUS.contains(&"sim_list"));
        // Device logs are console content (app tokens, PII): opt-in, like k8s_logs.
        assert!(OPT_IN_READS.contains(&"sim_logs"));
        assert!(!DEFAULT_ENABLED.contains(&"sim_logs") && !DANGEROUS.contains(&"sim_logs"));
        for w in ["sim_action", "sim_input"] {
            assert!(DANGEROUS.contains(&w), "{w} must be DANGEROUS");
            assert!(
                !DEFAULT_ENABLED.contains(&w),
                "{w} must not be default-enabled"
            );
            assert!(!IRREVERSIBLE.contains(&w), "{w} is reversible");
        }

        // Boot sends only `wait` (+ `open_gui` when given): the route's own
        // defaults pick the window, so the approval card says what will happen.
        let dev = "android:avd:Pixel_Phone";
        let boot = exec_route("sim_action", &json!({"device_id":dev,"action":"boot"})).unwrap();
        assert_eq!(boot.method, Method::Post);
        assert_eq!(
            boot.path,
            "/api/v1/simulators/devices/android%3Aavd%3APixel_Phone/boot"
        );
        assert_eq!(boot.body.unwrap(), json!({"wait":true}));
        assert!(
            catalog_detail("otto.sim_action", &json!({"device_id":dev,"action":"boot"}))
                .contains("embedded/headless")
        );
        let gui = json!({"device_id":dev,"action":"boot","open_gui":true});
        assert_eq!(
            exec_route("sim_action", &gui).unwrap().body.unwrap(),
            json!({"wait":true,"open_gui":true})
        );
        assert!(catalog_detail("otto.sim_action", &gui).contains("native window"));

        let logs = exec_route(
            "sim_logs",
            &json!({"device_id":"android:serial:emulator-5554","lines":50,"level":"error","filter":"crash"}),
        )
        .unwrap();
        assert_eq!(
            logs.path,
            "/api/v1/simulators/devices/android%3Aserial%3Aemulator-5554/logs?lines=50&level=error&filter=crash"
        );

        let tap = json!({"device_id":dev,"type":"tap","x":0.5,"y":0.25,"normalized":true});
        assert_eq!(
            exec_route("sim_input", &tap).unwrap().body.unwrap(),
            json!({"type":"tap","x":0.5,"y":0.25,"normalized":true})
        );
        assert_eq!(
            catalog_detail("otto.sim_input", &tap),
            "Tap (0.5, 0.25) on simulator 'android:avd:Pixel_Phone'"
        );
    }
}
