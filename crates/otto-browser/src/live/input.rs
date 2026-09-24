//! Viewer input → CDP `Input.*` commands. Pure: each function returns the
//! `(method, params)` pairs to send, so the mapping is unit-tested without a
//! browser. Coordinates arrive in viewport CSS px (the client maps them from
//! the drawn frame using the frame header).

use serde_json::{json, Value};

use super::protocol::{KeyAction, MouseAction, MouseButton, MAX_PASTE_CHARS};

/// CDP modifier bits.
pub const MOD_ALT: u32 = 1;
pub const MOD_CTRL: u32 = 2;
pub const MOD_META: u32 = 4;
pub const MOD_SHIFT: u32 = 8;

pub type Command = (&'static str, Value);

fn finite(v: f64) -> f64 {
    if v.is_finite() {
        v
    } else {
        0.0
    }
}

#[allow(clippy::too_many_arguments)]
pub fn mouse(
    action: MouseAction,
    x: f64,
    y: f64,
    button: MouseButton,
    buttons: u32,
    click_count: u32,
    delta_x: f64,
    delta_y: f64,
    modifiers: u32,
) -> Command {
    let kind = match action {
        MouseAction::Move => "mouseMoved",
        MouseAction::Down => "mousePressed",
        MouseAction::Up => "mouseReleased",
        MouseAction::Wheel => "mouseWheel",
    };
    let mut p = json!({
        "type": kind,
        "x": finite(x).max(0.0),
        "y": finite(y).max(0.0),
        "modifiers": modifiers & 0xF,
        "button": button.cdp(),
        "buttons": buttons & 0x1F,
    });
    match action {
        MouseAction::Down | MouseAction::Up => {
            p["clickCount"] = json!(click_count.clamp(1, 3));
        }
        MouseAction::Wheel => {
            p["deltaX"] = json!(finite(delta_x).clamp(-10_000.0, 10_000.0));
            p["deltaY"] = json!(finite(delta_y).clamp(-10_000.0, 10_000.0));
        }
        MouseAction::Move => {}
    }
    ("Input.dispatchMouseEvent", p)
}

/// Windows virtual-key code for a DOM `code` (what Chromium keys shortcuts,
/// Enter/Backspace/arrows etc. on). `None` for codes we don't know — the
/// event still carries `key`/`code`/`text`.
pub fn virtual_key_code(code: &str) -> Option<u32> {
    if let Some(c) = code.strip_prefix("Key") {
        let b = c.as_bytes();
        if b.len() == 1 && b[0].is_ascii_uppercase() {
            return Some(b[0] as u32);
        }
    }
    if let Some(d) = code.strip_prefix("Digit") {
        let b = d.as_bytes();
        if b.len() == 1 && b[0].is_ascii_digit() {
            return Some(b[0] as u32);
        }
    }
    if let Some(d) = code.strip_prefix("Numpad") {
        let b = d.as_bytes();
        if b.len() == 1 && b[0].is_ascii_digit() {
            return Some(96 + (b[0] - b'0') as u32);
        }
    }
    if let Some(n) = code.strip_prefix('F') {
        if let Ok(n) = n.parse::<u32>() {
            if (1..=24).contains(&n) {
                return Some(111 + n);
            }
        }
    }
    Some(match code {
        "Backspace" => 8,
        "Tab" => 9,
        "Enter" | "NumpadEnter" => 13,
        "ShiftLeft" | "ShiftRight" => 16,
        "ControlLeft" | "ControlRight" => 17,
        "AltLeft" | "AltRight" => 18,
        "Pause" => 19,
        "CapsLock" => 20,
        "Escape" => 27,
        "Space" => 32,
        "PageUp" => 33,
        "PageDown" => 34,
        "End" => 35,
        "Home" => 36,
        "ArrowLeft" => 37,
        "ArrowUp" => 38,
        "ArrowRight" => 39,
        "ArrowDown" => 40,
        "Insert" => 45,
        "Delete" => 46,
        "MetaLeft" | "OSLeft" => 91,
        "MetaRight" | "OSRight" => 92,
        "ContextMenu" => 93,
        "NumpadMultiply" => 106,
        "NumpadAdd" => 107,
        "NumpadSubtract" => 109,
        "NumpadDecimal" => 110,
        "NumpadDivide" => 111,
        "Semicolon" => 186,
        "Equal" => 187,
        "Comma" => 188,
        "Minus" => 189,
        "Period" => 190,
        "Slash" => 191,
        "Backquote" => 192,
        "BracketLeft" => 219,
        "Backslash" => 220,
        "BracketRight" => 221,
        "Quote" => 222,
        _ => return None,
    })
}

/// Editing command for a platform shortcut (headless Chrome on macOS does not
/// run the native key bindings, so ⌘A etc. must be sent as `commands`). A
/// remote viewer on another OS sends Ctrl — accept both.
pub fn editing_command(code: &str, modifiers: u32) -> Option<&'static str> {
    let primary = modifiers & (MOD_META | MOD_CTRL) != 0;
    if !primary || modifiers & MOD_ALT != 0 {
        return None;
    }
    let shift = modifiers & MOD_SHIFT != 0;
    Some(match (code, shift) {
        ("KeyA", false) => "selectAll",
        ("KeyC", false) => "copy",
        ("KeyX", false) => "cut",
        ("KeyV", false) => "paste",
        ("KeyZ", false) => "undo",
        ("KeyZ", true) | ("KeyY", false) => "redo",
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments)]
pub fn key(
    action: KeyAction,
    key: &str,
    code: &str,
    text: Option<&str>,
    key_code: Option<u32>,
    location: u32,
    repeat: bool,
    modifiers: u32,
) -> Command {
    let modifiers = modifiers & 0xF;
    let vk = key_code.or_else(|| virtual_key_code(code)).unwrap_or(0);
    // Enter's character is "\r" in Chromium's key pipeline.
    let text: Option<String> = match text {
        Some("\n") | Some("\r\n") => Some("\r".into()),
        Some(t) if !t.is_empty() => Some(t.chars().take(16).collect()),
        _ if code == "Enter" || code == "NumpadEnter" => Some("\r".into()),
        _ => None,
    };
    // A shortcut (Ctrl/⌘ held) produces no character.
    let shortcut = modifiers & (MOD_CTRL | MOD_META) != 0;
    let mut p = json!({
        "modifiers": modifiers,
        "key": key.chars().take(32).collect::<String>(),
        "code": code.chars().take(32).collect::<String>(),
        "windowsVirtualKeyCode": vk,
        "nativeVirtualKeyCode": vk,
        "autoRepeat": repeat,
        "location": location.min(3),
    });
    match action {
        KeyAction::Up => {
            p["type"] = json!("keyUp");
        }
        KeyAction::Down => match text {
            Some(t) if !shortcut => {
                p["type"] = json!("keyDown");
                p["text"] = json!(t.clone());
                p["unmodifiedText"] = json!(t);
            }
            _ => {
                p["type"] = json!("rawKeyDown");
                if let Some(cmd) = editing_command(code, modifiers) {
                    p["commands"] = json!([cmd]);
                }
            }
        },
    }
    ("Input.dispatchKeyEvent", p)
}

/// Committed text (IME commit, paste): `Input.insertText`, length-capped.
pub fn insert_text(text: &str) -> Command {
    let t: String = text.chars().take(MAX_PASTE_CHARS).collect();
    ("Input.insertText", json!({ "text": t }))
}

/// In-progress IME composition.
pub fn ime_composition(text: &str, selection_start: i64, selection_end: i64) -> Command {
    let t: String = text.chars().take(1024).collect();
    let n = t.chars().count() as i64;
    (
        "Input.imeSetComposition",
        json!({
            "text": t,
            "selectionStart": selection_start.clamp(0, n),
            "selectionEnd": selection_end.clamp(0, n),
        }),
    )
}

/// Map a CSS `cursor` value (from the element under the pointer) to the
/// set the contract names; unknown values fall back to `default`.
pub fn normalize_cursor(css: &str) -> &'static str {
    let first = css.split(',').next_back().unwrap_or("").trim();
    match first {
        "pointer" => "pointer",
        "text" | "vertical-text" => "text",
        "move" | "all-scroll" => "move",
        "grab" => "grab",
        "grabbing" => "grabbing",
        "not-allowed" | "no-drop" => "not-allowed",
        "wait" => "wait",
        "progress" => "progress",
        "crosshair" => "crosshair",
        "help" => "help",
        "col-resize" => "col-resize",
        "row-resize" => "row-resize",
        "ew-resize" | "e-resize" | "w-resize" => "ew-resize",
        "ns-resize" | "n-resize" | "s-resize" => "ns-resize",
        "nesw-resize" | "ne-resize" | "sw-resize" => "nesw-resize",
        "nwse-resize" | "nw-resize" | "se-resize" => "nwse-resize",
        "zoom-in" => "zoom-in",
        "zoom-out" => "zoom-out",
        "copy" => "copy",
        "context-menu" => "context-menu",
        "cell" => "cell",
        "none" => "none",
        _ => "default",
    }
}

/// JS evaluated (in an isolated world) to read the effective cursor at a
/// point: the computed `cursor`, with `auto` resolved the way the browser
/// would (links → pointer, editable text → text).
pub fn cursor_probe_expr(x: f64, y: f64) -> String {
    format!(
        "(function(){{var e=document.elementFromPoint({x},{y});if(!e)return 'default';\
         var c=getComputedStyle(e).cursor;if(c&&c!=='auto')return c;\
         if(e.closest('a[href],button,[role=button],label,select,summary'))return 'pointer';\
         if(e.isContentEditable||e.closest('input:not([type=button]):not([type=submit]):not([type=checkbox]):not([type=radio]),textarea'))return 'text';\
         return 'default';}})()",
        x = finite(x),
        y = finite(y)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_events_map_to_cdp() {
        let (m, p) = mouse(MouseAction::Down, 10.5, 20.0, MouseButton::Left, 1, 2, 0.0, 0.0, MOD_SHIFT);
        assert_eq!(m, "Input.dispatchMouseEvent");
        assert_eq!(p["type"], "mousePressed");
        assert_eq!(p["button"], "left");
        assert_eq!(p["clickCount"], 2);
        assert_eq!(p["modifiers"], 8);
        let (_, p) = mouse(MouseAction::Wheel, 1.0, 1.0, MouseButton::None, 0, 0, 0.0, 1e9, 0);
        assert_eq!(p["type"], "mouseWheel");
        assert_eq!(p["deltaY"], 10_000.0);
        assert!(p.get("clickCount").is_none());
        let (_, p) = mouse(MouseAction::Move, f64::NAN, -5.0, MouseButton::None, 0, 0, 0.0, 0.0, 0xFF);
        assert_eq!(p["x"], 0.0);
        assert_eq!(p["y"], 0.0);
        assert_eq!(p["modifiers"], 0xF);
    }

    #[test]
    fn printable_keydown_carries_text() {
        let (_, p) = key(KeyAction::Down, "a", "KeyA", Some("a"), None, 0, false, 0);
        assert_eq!(p["type"], "keyDown");
        assert_eq!(p["text"], "a");
        assert_eq!(p["windowsVirtualKeyCode"], 65);
        let (_, p) = key(KeyAction::Up, "a", "KeyA", Some("a"), None, 0, false, 0);
        assert_eq!(p["type"], "keyUp");
        assert!(p.get("text").is_none());
    }

    #[test]
    fn enter_and_non_printing_keys() {
        let (_, p) = key(KeyAction::Down, "Enter", "Enter", None, None, 0, false, 0);
        assert_eq!(p["type"], "keyDown");
        assert_eq!(p["text"], "\r");
        assert_eq!(p["windowsVirtualKeyCode"], 13);
        let (_, p) = key(KeyAction::Down, "ArrowLeft", "ArrowLeft", None, None, 0, false, 0);
        assert_eq!(p["type"], "rawKeyDown");
        assert_eq!(p["windowsVirtualKeyCode"], 37);
    }

    #[test]
    fn shortcuts_become_editing_commands_without_text() {
        let (_, p) = key(KeyAction::Down, "a", "KeyA", Some("a"), None, 0, false, MOD_META);
        assert_eq!(p["type"], "rawKeyDown");
        assert!(p.get("text").is_none());
        assert_eq!(p["commands"], json!(["selectAll"]));
        let (_, p) = key(KeyAction::Down, "z", "KeyZ", None, None, 0, false, MOD_CTRL | MOD_SHIFT);
        assert_eq!(p["commands"], json!(["redo"]));
        assert_eq!(editing_command("KeyA", MOD_META | MOD_ALT), None);
        assert_eq!(editing_command("KeyA", 0), None);
    }

    #[test]
    fn virtual_key_codes() {
        assert_eq!(virtual_key_code("KeyZ"), Some(90));
        assert_eq!(virtual_key_code("Digit0"), Some(48));
        assert_eq!(virtual_key_code("Numpad7"), Some(103));
        assert_eq!(virtual_key_code("F12"), Some(123));
        assert_eq!(virtual_key_code("F99"), None);
        assert_eq!(virtual_key_code("Backspace"), Some(8));
        assert_eq!(virtual_key_code("IntlRo"), None);
    }

    #[test]
    fn text_ime_and_paste_are_capped() {
        let big = "x".repeat(MAX_PASTE_CHARS + 50);
        let (m, p) = insert_text(&big);
        assert_eq!(m, "Input.insertText");
        assert_eq!(p["text"].as_str().unwrap().len(), MAX_PASTE_CHARS);
        let (m, p) = ime_composition("にほ", 5, -1);
        assert_eq!(m, "Input.imeSetComposition");
        assert_eq!(p["selectionStart"], 2);
        assert_eq!(p["selectionEnd"], 0);
    }

    #[test]
    fn cursor_normalization() {
        assert_eq!(normalize_cursor("pointer"), "pointer");
        assert_eq!(normalize_cursor("url(x.png) 2 2, pointer"), "pointer");
        assert_eq!(normalize_cursor("e-resize"), "ew-resize");
        assert_eq!(normalize_cursor("something-odd"), "default");
        assert!(cursor_probe_expr(1.0, f64::INFINITY).contains("elementFromPoint(1,0)"));
    }
}
