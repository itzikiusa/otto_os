//! The `scene3d` document — the agent-editable 3D format of the Design arena
//! (`application/vnd.otto.scene3d+json`), plus its deterministic export to a
//! Blender Python script.
//!
//! Small, declarative, human-readable: the browser renders it (three.js), the
//! agent edits it, the inspector round-trips it. Conventions: units are metres,
//! **y-up**, origin at the floor, rotation in **degrees** (agents and humans
//! think in degrees; the viewer converts). Primitives have a unit bounding box
//! before `scale` (box 1×1×1, sphere Ø1, cylinder/cone Ø1 × h1, plane 1×1, torus
//! major radius 0.5 / minor 0.2). `gltf` objects reference an **`attachment_id`**,
//! never a URL, so there is no URL surface to guard.
//!
//! `validate` runs before every render/save (TS mirror: `scene3d/validate.ts`)
//! and before the Blender export: known `type`s only, finite numbers, bounded
//! array lengths, safe id components. `to_blender_script` is a FIXED template
//! that interpolates only validated numbers / enums / escaped strings — it is
//! generated server-side from a validated document and never from a user or
//! agent file (see `design_blender.rs`).

use std::collections::HashSet;

use otto_core::Error;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Hard caps that keep a hostile document from exhausting the viewer or Blender.
pub const MAX_OBJECTS: usize = 2_000;
pub const MAX_LIGHTS: usize = 64;
pub const MAX_GROUPS: usize = 500;
const MAX_NAME: usize = 200;
const MAX_TEXT: usize = 2_000;
const MAX_NOTES: usize = 4_000;
/// Largest absolute coordinate / scale / intensity we accept (metres, units).
const MAX_MAGNITUDE: f64 = 1.0e6;

pub const DOC_TYPE: &str = "otto-scene3d";
pub const DOC_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Document types (serde; unknown keys are ignored for forward compatibility)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene3d {
    #[serde(rename = "type")]
    pub doc_type: String,
    pub version: u32,
    #[serde(default)]
    pub background: Option<String>,
    #[serde(default)]
    pub grid: Option<bool>,
    #[serde(default)]
    pub camera: Option<Camera>,
    #[serde(default)]
    pub lights: Vec<Light>,
    #[serde(default)]
    pub objects: Vec<Object3d>,
    #[serde(default)]
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Camera {
    pub position: [f64; 3],
    #[serde(default = "zero3")]
    pub target: [f64; 3],
    #[serde(default)]
    pub fov: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Light {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: LightKind,
    #[serde(default)]
    pub position: Option<[f64; 3]>,
    #[serde(default)]
    pub target: Option<[f64; 3]>,
    #[serde(default)]
    pub intensity: Option<f64>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub shadow: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LightKind {
    Directional,
    Ambient,
    Point,
    Spot,
    Hemisphere,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Object3d {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub kind: ObjectKind,
    #[serde(default = "zero3")]
    pub position: [f64; 3],
    #[serde(default = "zero3")]
    pub rotation: [f64; 3],
    #[serde(default = "one3")]
    pub scale: [f64; 3],
    #[serde(default)]
    pub material: Option<Material>,
    /// `gltf` only — the `.glb`/`.gltf` attachment to load.
    #[serde(default)]
    pub attachment_id: Option<String>,
    /// `text` only — the string to extrude.
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub visible: Option<bool>,
    /// Free-form per-object notes (the 3D stand-in for pinned annotations).
    #[serde(default)]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObjectKind {
    Box,
    Sphere,
    Cylinder,
    Cone,
    Torus,
    Plane,
    Text,
    Gltf,
    Group,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Material {
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub metalness: Option<f64>,
    #[serde(default)]
    pub roughness: Option<f64>,
    #[serde(default)]
    pub opacity: Option<f64>,
    #[serde(default)]
    pub emissive: Option<String>,
    #[serde(default)]
    pub wireframe: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub children: Vec<String>,
}

fn zero3() -> [f64; 3] {
    [0.0, 0.0, 0.0]
}
fn one3() -> [f64; 3] {
    [1.0, 1.0, 1.0]
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Parse + validate a `scene3d` JSON document. Every failure is
/// `Error::Invalid` (HTTP 400) with a path-ish message.
pub fn validate(doc: &Value) -> Result<Scene3d, Error> {
    let scene: Scene3d =
        serde_json::from_value(doc.clone()).map_err(|e| Error::Invalid(format!("scene3d: {e}")))?;
    validate_scene(&scene)?;
    Ok(scene)
}

/// Parse + validate from raw bytes (the PUT / assist / render paths).
pub fn validate_bytes(bytes: &[u8]) -> Result<Scene3d, Error> {
    let doc: Value =
        serde_json::from_slice(bytes).map_err(|e| Error::Invalid(format!("scene3d: {e}")))?;
    validate(&doc)
}

fn validate_scene(s: &Scene3d) -> Result<(), Error> {
    if s.doc_type != DOC_TYPE {
        return Err(bad(format!(
            "type must be {DOC_TYPE:?}, got {:?}",
            s.doc_type
        )));
    }
    if s.version != DOC_VERSION {
        return Err(bad(format!(
            "unsupported version {} (expected {DOC_VERSION})",
            s.version
        )));
    }
    if let Some(bg) = &s.background {
        check_color("background", bg)?;
    }
    if let Some(c) = &s.camera {
        check_vec3("camera.position", &c.position)?;
        check_vec3("camera.target", &c.target)?;
        if let Some(fov) = c.fov {
            if !fov.is_finite() || !(1.0..=179.0).contains(&fov) {
                return Err(bad("camera.fov must be within 1..=179"));
            }
        }
    }
    if s.lights.len() > MAX_LIGHTS {
        return Err(bad(format!(
            "too many lights ({} > {MAX_LIGHTS})",
            s.lights.len()
        )));
    }
    if s.objects.len() > MAX_OBJECTS {
        return Err(bad(format!(
            "too many objects ({} > {MAX_OBJECTS})",
            s.objects.len()
        )));
    }
    if s.groups.len() > MAX_GROUPS {
        return Err(bad(format!(
            "too many groups ({} > {MAX_GROUPS})",
            s.groups.len()
        )));
    }

    let mut ids: HashSet<&str> = HashSet::new();
    for (i, l) in s.lights.iter().enumerate() {
        let at = format!("lights[{i}]");
        check_id(&at, &l.id, &mut ids)?;
        if let Some(p) = &l.position {
            check_vec3(&format!("{at}.position"), p)?;
        }
        if let Some(t) = &l.target {
            check_vec3(&format!("{at}.target"), t)?;
        }
        if let Some(v) = l.intensity {
            check_num(&format!("{at}.intensity"), v, 0.0, MAX_MAGNITUDE)?;
        }
        if let Some(c) = &l.color {
            check_color(&format!("{at}.color"), c)?;
        }
    }
    for (i, o) in s.objects.iter().enumerate() {
        let at = format!("objects[{i}]");
        check_id(&at, &o.id, &mut ids)?;
        if let Some(n) = &o.name {
            check_str(&format!("{at}.name"), n, MAX_NAME)?;
        }
        check_vec3(&format!("{at}.position"), &o.position)?;
        check_vec3(&format!("{at}.rotation"), &o.rotation)?;
        check_vec3(&format!("{at}.scale"), &o.scale)?;
        if let Some(m) = &o.material {
            if let Some(c) = &m.color {
                check_color(&format!("{at}.material.color"), c)?;
            }
            if let Some(c) = &m.emissive {
                check_color(&format!("{at}.material.emissive"), c)?;
            }
            for (k, v) in [
                ("metalness", m.metalness),
                ("roughness", m.roughness),
                ("opacity", m.opacity),
            ] {
                if let Some(v) = v {
                    check_num(&format!("{at}.material.{k}"), v, 0.0, 1.0)?;
                }
            }
        }
        match o.kind {
            ObjectKind::Gltf => match o.attachment_id.as_deref() {
                Some(aid) if otto_core::paths::safe_component(aid).is_some() => {}
                Some(aid) => {
                    return Err(bad(format!("{at}.attachment_id {aid:?} is not a safe id")))
                }
                None => return Err(bad(format!("{at}: gltf objects require attachment_id"))),
            },
            _ => {
                if o.attachment_id.is_some() {
                    return Err(bad(format!(
                        "{at}.attachment_id is only valid on gltf objects"
                    )));
                }
            }
        }
        if let Some(t) = &o.text {
            check_str(&format!("{at}.text"), t, MAX_TEXT)?;
        }
        if let Some(n) = &o.notes {
            check_str(&format!("{at}.notes"), n, MAX_NOTES)?;
        }
    }
    for (i, g) in s.groups.iter().enumerate() {
        let at = format!("groups[{i}]");
        check_id(&at, &g.id, &mut ids)?;
        if let Some(n) = &g.name {
            check_str(&format!("{at}.name"), n, MAX_NAME)?;
        }
        if g.children.len() > MAX_OBJECTS {
            return Err(bad(format!("{at}.children is too long")));
        }
    }
    // Group membership must point at known objects/groups (never itself).
    let object_ids: HashSet<&str> = s.objects.iter().map(|o| o.id.as_str()).collect();
    let group_ids: HashSet<&str> = s.groups.iter().map(|g| g.id.as_str()).collect();
    for (i, g) in s.groups.iter().enumerate() {
        for c in &g.children {
            if c == &g.id {
                return Err(bad(format!("groups[{i}] contains itself")));
            }
            if !object_ids.contains(c.as_str()) && !group_ids.contains(c.as_str()) {
                return Err(bad(format!("groups[{i}] references unknown id {c:?}")));
            }
        }
    }
    Ok(())
}

fn bad(msg: impl Into<String>) -> Error {
    Error::Invalid(format!("scene3d: {}", msg.into()))
}

fn check_id<'a>(at: &str, id: &'a str, seen: &mut HashSet<&'a str>) -> Result<(), Error> {
    if otto_core::paths::safe_component(id).is_none() || id.len() > 128 {
        return Err(bad(format!("{at}.id {id:?} is not a safe id")));
    }
    if !seen.insert(id) {
        return Err(bad(format!("{at}.id {id:?} is duplicated")));
    }
    Ok(())
}

fn check_str(at: &str, s: &str, max: usize) -> Result<(), Error> {
    if s.chars().count() > max {
        return Err(bad(format!("{at} exceeds {max} characters")));
    }
    Ok(())
}

fn check_num(at: &str, v: f64, lo: f64, hi: f64) -> Result<(), Error> {
    if !v.is_finite() || v < lo || v > hi {
        return Err(bad(format!(
            "{at} must be a finite number within {lo}..={hi}"
        )));
    }
    Ok(())
}

fn check_vec3(at: &str, v: &[f64; 3]) -> Result<(), Error> {
    for (i, x) in v.iter().enumerate() {
        check_num(&format!("{at}[{i}]"), *x, -MAX_MAGNITUDE, MAX_MAGNITUDE)?;
    }
    Ok(())
}

/// `#rgb` or `#rrggbb` (lowercase or uppercase hex) — the only colour syntax.
fn check_color(at: &str, c: &str) -> Result<(), Error> {
    let hex = c.strip_prefix('#').unwrap_or("");
    let ok = (hex.len() == 3 || hex.len() == 6) && hex.chars().all(|ch| ch.is_ascii_hexdigit());
    if !ok {
        return Err(bad(format!(
            "{at} must be a #rgb / #rrggbb colour, got {c:?}"
        )));
    }
    Ok(())
}

/// Parse a validated colour into linear-ish 0..1 RGB (sRGB values as-is —
/// Blender's Principled Base Color socket expects sRGB-encoded floats from the
/// UI, which is what agents author).
fn rgb(c: &str) -> (f64, f64, f64) {
    let hex = c.trim_start_matches('#');
    let (r, g, b) = if hex.len() == 3 {
        let d = |i: usize| u8::from_str_radix(&hex[i..i + 1].repeat(2), 16).unwrap_or(0);
        (d(0), d(1), d(2))
    } else {
        let d = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0);
        (d(0), d(2), d(4))
    };
    (
        f64::from(r) / 255.0,
        f64::from(g) / 255.0,
        f64::from(b) / 255.0,
    )
}

// ---------------------------------------------------------------------------
// Starter document
// ---------------------------------------------------------------------------

/// A minimal valid starter scene: floor, a sun + ambient light and a camera —
/// what a brand-new 3D artifact holds before the agent or the user adds objects.
pub fn empty_scene_json(story_title: &str) -> String {
    let name: String = story_title.chars().take(MAX_NAME).collect();
    serde_json::json!({
        "type": DOC_TYPE,
        "version": DOC_VERSION,
        "background": "#0f172a",
        "grid": true,
        "camera": { "position": [6, 5, 8], "target": [0, 1, 0], "fov": 50 },
        "lights": [
            { "id": "sun", "type": "directional", "position": [5, 10, 5], "intensity": 1.2, "color": "#ffffff", "shadow": true },
            { "id": "amb", "type": "ambient", "intensity": 0.4 }
        ],
        "objects": [
            { "id": "floor", "name": "Floor", "type": "plane", "position": [0, 0, 0], "rotation": [-90, 0, 0],
              "scale": [20, 20, 1], "material": { "color": "#334155", "roughness": 0.9 },
              "notes": format!("Blockout for {name}") }
        ],
        "groups": []
    })
    .to_string()
}

// ---------------------------------------------------------------------------
// Blender export
// ---------------------------------------------------------------------------

/// Deterministic `scene3d → blender.py`. The script builds the scene from
/// scratch (factory settings, empty file), renders `render.png` (Eevee,
/// 1280×720) and exports `scene.glb` into the directory passed after `--` as
/// `--out <dir>`:
///
/// ```text
/// blender -b --python <generated.py> -- --out <dir>
/// ```
///
/// Axis mapping: the document is y-up (three.js), Blender is z-up, so
/// `(x, y, z) → (x, -z, y)`; Euler XYZ in doc axes becomes Blender `XZY` with
/// `(rx, -rz, ry)`; flat primitives (plane / torus / text) get the +90° X bake
/// so an un-rotated doc plane faces doc +Z exactly as in the viewer. `gltf`
/// objects become named empties (the asset lives in Otto, not on disk).
pub fn to_blender_script(scene: &Scene3d) -> String {
    let mut py = String::new();
    py.push_str(PRELUDE);

    // World / background (+ ambient/hemisphere lights fold into the world).
    let (br, bg, bb) = rgb(scene.background.as_deref().unwrap_or("#0f172a"));
    let mut ambient = 0.0;
    for l in &scene.lights {
        if matches!(l.kind, LightKind::Ambient | LightKind::Hemisphere) {
            ambient += l.intensity.unwrap_or(1.0);
        }
    }
    py.push_str(&format!(
        "set_world(({}, {}, {}), {})\n",
        num(br),
        num(bg),
        num(bb),
        num(ambient.max(0.05))
    ));

    // Camera.
    match &scene.camera {
        Some(c) => py.push_str(&format!(
            "add_camera({}, {}, {})\n",
            vec3(&c.position),
            vec3(&c.target),
            num(c.fov.unwrap_or(50.0))
        )),
        None => py.push_str("add_camera((6.0, 5.0, 8.0), (0.0, 1.0, 0.0), 50.0)\n"),
    }

    // Lights (non-ambient).
    for l in &scene.lights {
        let kind = match l.kind {
            LightKind::Directional => "SUN",
            LightKind::Point => "POINT",
            LightKind::Spot => "SPOT",
            LightKind::Ambient | LightKind::Hemisphere => continue,
        };
        let (r, g, b) = rgb(l.color.as_deref().unwrap_or("#ffffff"));
        py.push_str(&format!(
            "add_light({}, \"{}\", {}, {}, {}, ({}, {}, {}), {})\n",
            py_str(&l.id),
            kind,
            vec3(&l.position.unwrap_or([5.0, 10.0, 5.0])),
            vec3(&l.target.unwrap_or([0.0, 0.0, 0.0])),
            num(l.intensity.unwrap_or(1.0)),
            num(r),
            num(g),
            num(b),
            py_bool(l.shadow.unwrap_or(true))
        ));
    }

    // Objects.
    for o in &scene.objects {
        let kind = match o.kind {
            ObjectKind::Box => "box",
            ObjectKind::Sphere => "sphere",
            ObjectKind::Cylinder => "cylinder",
            ObjectKind::Cone => "cone",
            ObjectKind::Torus => "torus",
            ObjectKind::Plane => "plane",
            ObjectKind::Text => "text",
            ObjectKind::Gltf => "gltf",
            ObjectKind::Group => "group",
        };
        let name = o.name.as_deref().unwrap_or(&o.id);
        let m = o.material.clone().unwrap_or_default();
        let (cr, cg, cb) = rgb(m.color.as_deref().unwrap_or("#cbd5e1"));
        let (er, eg, eb) = rgb(m.emissive.as_deref().unwrap_or("#000000"));
        py.push_str(&format!(
            "add_object({}, {}, \"{}\", {}, {}, {}, dict(color=({}, {}, {}), metalness={}, roughness={}, opacity={}, emissive=({}, {}, {}), wireframe={}), text={}, visible={})\n",
            py_str(&o.id),
            py_str(name),
            kind,
            vec3(&o.position),
            vec3(&o.rotation),
            vec3(&o.scale),
            num(cr),
            num(cg),
            num(cb),
            num(m.metalness.unwrap_or(0.0)),
            num(m.roughness.unwrap_or(0.6)),
            num(m.opacity.unwrap_or(1.0)),
            num(er),
            num(eg),
            num(eb),
            py_bool(m.wireframe.unwrap_or(false)),
            py_str(o.text.as_deref().unwrap_or("")),
            py_bool(o.visible.unwrap_or(true)),
        ));
    }

    // Groups → empties with parented children.
    for g in &scene.groups {
        let children: Vec<String> = g.children.iter().map(|c| py_str(c)).collect();
        py.push_str(&format!(
            "add_group({}, {}, [{}])\n",
            py_str(&g.id),
            py_str(g.name.as_deref().unwrap_or(&g.id)),
            children.join(", ")
        ));
    }

    py.push_str(EPILOGUE);
    py
}

/// Python float literal for a validated finite number.
fn num(v: f64) -> String {
    let s = format!("{v}");
    if s.contains(['.', 'e', 'E']) {
        s
    } else {
        format!("{s}.0")
    }
}

fn vec3(v: &[f64; 3]) -> String {
    format!("({}, {}, {})", num(v[0]), num(v[1]), num(v[2]))
}

fn py_bool(b: bool) -> &'static str {
    if b {
        "True"
    } else {
        "False"
    }
}

/// Python double-quoted string literal. Escapes backslash, quote, newline /
/// return / tab and every other control character — nothing from the document
/// can break out of the literal.
fn py_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

const PRELUDE: &str = r#"# Generated by Otto from a validated scene3d document. Do not edit by hand —
# re-export from the Design arena instead.
#   blender -b --python this.py -- --out <dir>
import bpy, math, os, sys
from mathutils import Matrix, Vector

argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
OUT = argv[argv.index("--out") + 1] if "--out" in argv else os.getcwd()
os.makedirs(OUT, exist_ok=True)

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
try:
    scene.render.engine = "BLENDER_EEVEE_NEXT"
except TypeError:
    scene.render.engine = "BLENDER_EEVEE"
scene.render.resolution_x = 1280
scene.render.resolution_y = 720
scene.render.resolution_percentage = 100
scene.render.image_settings.file_format = "PNG"

OBJECTS = {}

def yup(v):
    # doc (x, y, z) y-up  ->  Blender (x, -z, y) z-up
    return (v[0], -v[2], v[1])

def yup_rot(r):
    # doc Euler XYZ (degrees) -> Blender XZY (radians) under the axis swap
    return (math.radians(r[0]), math.radians(-r[2]), math.radians(r[1]))

def yup_scale(s):
    return (s[0], s[2], s[1])

def set_world(color, strength):
    world = bpy.data.worlds.new("World")
    scene.world = world
    world.use_nodes = True
    bg = world.node_tree.nodes.get("Background")
    if bg is not None:
        bg.inputs["Color"].default_value = (color[0], color[1], color[2], 1.0)
        bg.inputs["Strength"].default_value = strength

def add_camera(position, target, fov):
    cam = bpy.data.cameras.new("Camera")
    cam.sensor_fit = "VERTICAL"
    cam.angle_y = math.radians(fov)
    obj = bpy.data.objects.new("Camera", cam)
    scene.collection.objects.link(obj)
    obj.location = yup(position)
    direction = Vector(yup(target)) - Vector(yup(position))
    obj.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()
    scene.camera = obj

def add_light(name, kind, position, target, intensity, color, shadow):
    light = bpy.data.lights.new(name, kind)
    light.color = color
    light.use_shadow = shadow
    if kind == "SUN":
        light.energy = intensity * 3.0
    else:
        light.energy = intensity * 100.0
    obj = bpy.data.objects.new(name, light)
    scene.collection.objects.link(obj)
    obj.location = yup(position)
    direction = Vector(yup(target)) - Vector(yup(position))
    if direction.length > 0:
        obj.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()

def make_material(name, m):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    if bsdf is not None:
        c = m["color"]
        bsdf.inputs["Base Color"].default_value = (c[0], c[1], c[2], 1.0)
        bsdf.inputs["Metallic"].default_value = m["metalness"]
        bsdf.inputs["Roughness"].default_value = m["roughness"]
        bsdf.inputs["Alpha"].default_value = m["opacity"]
        e = m["emissive"]
        if "Emission Color" in bsdf.inputs:
            bsdf.inputs["Emission Color"].default_value = (e[0], e[1], e[2], 1.0)
        elif "Emission" in bsdf.inputs:
            bsdf.inputs["Emission"].default_value = (e[0], e[1], e[2], 1.0)
        if "Emission Strength" in bsdf.inputs:
            bsdf.inputs["Emission Strength"].default_value = 1.0 if max(e) > 0 else 0.0
    if m["opacity"] < 1.0:
        try:
            mat.blend_method = "BLEND"
        except Exception:
            pass
    return mat

FLAT = Matrix.Rotation(math.radians(90.0), 4, "X")

def add_object(oid, name, kind, position, rotation, scale, material, text="", visible=True):
    if kind == "box":
        bpy.ops.mesh.primitive_cube_add(size=1.0)
    elif kind == "sphere":
        bpy.ops.mesh.primitive_uv_sphere_add(radius=0.5)
    elif kind == "cylinder":
        bpy.ops.mesh.primitive_cylinder_add(radius=0.5, depth=1.0)
    elif kind == "cone":
        bpy.ops.mesh.primitive_cone_add(radius1=0.5, depth=1.0)
    elif kind == "torus":
        bpy.ops.mesh.primitive_torus_add(major_radius=0.5, minor_radius=0.2)
    elif kind == "plane":
        bpy.ops.mesh.primitive_plane_add(size=1.0)
    elif kind == "text":
        bpy.ops.object.text_add()
    else:
        bpy.ops.object.empty_add(type="PLAIN_AXES")
    obj = bpy.context.active_object
    obj.name = name
    if kind in ("plane", "torus", "text") and obj.data is not None:
        obj.data.transform(FLAT)
    if kind == "text":
        obj.data.body = text
        obj.data.extrude = 0.05
    obj.location = yup(position)
    obj.rotation_mode = "XZY"
    obj.rotation_euler = yup_rot(rotation)
    obj.scale = yup_scale(scale)
    obj.hide_render = not visible
    obj.hide_viewport = not visible
    if obj.type in ("MESH", "FONT"):
        obj.data.materials.append(make_material(name + "_mat", material))
        if material["wireframe"]:
            obj.modifiers.new("Wireframe", "WIREFRAME")
    OBJECTS[oid] = obj
    return obj

def add_group(gid, name, children):
    bpy.ops.object.empty_add(type="PLAIN_AXES")
    obj = bpy.context.active_object
    obj.name = name
    for cid in children:
        child = OBJECTS.get(cid)
        if child is not None:
            child.parent = obj
    OBJECTS[gid] = obj
    return obj

"#;

const EPILOGUE: &str = r#"
scene.render.filepath = os.path.join(OUT, "render.png")
bpy.ops.render.render(write_still=True)
bpy.ops.export_scene.gltf(filepath=os.path.join(OUT, "scene.glb"), export_format="GLB")
print("otto-scene3d: wrote", OUT)
"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample() -> Value {
        json!({
            "type": "otto-scene3d", "version": 1,
            "background": "#0f172a", "grid": true,
            "camera": { "position": [6, 5, 8], "target": [0, 1, 0], "fov": 50 },
            "lights": [
                { "id": "sun", "type": "directional", "position": [5, 10, 5], "intensity": 1.2, "color": "#ffffff", "shadow": true },
                { "id": "amb", "type": "ambient", "intensity": 0.4 }
            ],
            "objects": [
                { "id": "floor", "name": "Floor", "type": "plane", "position": [0, 0, 0], "rotation": [-90, 0, 0],
                  "scale": [20, 20, 1], "material": { "color": "#334155", "roughness": 0.9 } },
                { "id": "crate", "name": "Crate", "type": "box", "position": [0, 0.5, 0], "rotation": [0, 30, 0],
                  "scale": [1, 1, 1], "material": { "color": "#f59e0b", "metalness": 0.1, "roughness": 0.6 } },
                { "id": "hero", "name": "Hero", "type": "gltf", "attachment_id": "01HXYZ",
                  "position": [2, 0, 0], "rotation": [0, 0, 0], "scale": [1, 1, 1] }
            ],
            "groups": [ { "id": "props", "name": "Props", "children": ["crate", "hero"] } ]
        })
    }

    #[test]
    fn validates_the_spec_example_and_the_empty_scene() {
        let s = validate(&sample()).unwrap();
        assert_eq!(s.objects.len(), 3);
        assert_eq!(s.objects[1].kind, ObjectKind::Box);
        let empty: Value = serde_json::from_str(&empty_scene_json("Kiosk")).unwrap();
        assert!(validate(&empty).is_ok());
    }

    #[test]
    fn rejects_unknown_types_bad_numbers_and_unsafe_ids() {
        let mut d = sample();
        d["objects"][0]["type"] = json!("teapot");
        assert!(validate(&d).is_err(), "unknown object type");

        let mut d = sample();
        d["objects"][1]["position"] = json!([0, "1", 0]);
        assert!(validate(&d).is_err(), "non-numeric coordinate");

        let mut d = sample();
        d["objects"][1]["position"] = json!([0, 1e300, 0]);
        assert!(validate(&d).is_err(), "out-of-range coordinate");

        let mut d = sample();
        d["objects"][1]["material"]["roughness"] = json!(1.5);
        assert!(validate(&d).is_err(), "roughness > 1");

        let mut d = sample();
        d["objects"][2]["attachment_id"] = json!("../etc/passwd");
        assert!(validate(&d).is_err(), "traversing attachment id");

        let mut d = sample();
        d["objects"][2]["attachment_id"] = Value::Null;
        assert!(validate(&d).is_err(), "gltf without attachment_id");

        let mut d = sample();
        d["objects"][1]["attachment_id"] = json!("01HXYZ");
        assert!(validate(&d).is_err(), "attachment_id on a box");

        let mut d = sample();
        d["objects"][1]["id"] = json!("floor");
        assert!(validate(&d).is_err(), "duplicate id");

        let mut d = sample();
        d["groups"][0]["children"] = json!(["ghost"]);
        assert!(validate(&d).is_err(), "group references unknown id");

        let mut d = sample();
        d["background"] = json!("red");
        assert!(validate(&d).is_err(), "non-hex colour");

        let mut d = sample();
        d["type"] = json!("otto-scene2d");
        assert!(validate(&d).is_err(), "wrong doc type");

        let mut d = sample();
        d["camera"]["fov"] = json!(0);
        assert!(validate(&d).is_err(), "fov out of range");
    }

    #[test]
    fn rejects_oversized_documents() {
        let mut d = sample();
        let many: Vec<Value> = (0..=MAX_OBJECTS)
            .map(|i| json!({ "id": format!("o{i}"), "type": "box" }))
            .collect();
        d["objects"] = Value::Array(many);
        d["groups"] = json!([]);
        let err = validate(&d).unwrap_err();
        assert!(err.to_string().contains("too many objects"), "{err}");
    }

    #[test]
    fn python_literals_are_escaped() {
        assert_eq!(py_str("plain"), "\"plain\"");
        assert_eq!(py_str("a\"b\\c\nd"), "\"a\\\"b\\\\c\\nd\"");
        assert_eq!(py_str("x\u{1}y"), "\"x\\x01y\"");
        assert_eq!(num(1.0), "1.0");
        assert_eq!(num(0.5), "0.5");
        assert_eq!(num(-90.0), "-90.0");
        assert_eq!(vec3(&[0.0, 0.5, 0.0]), "(0.0, 0.5, 0.0)");
    }

    /// Golden: the generated script is deterministic and interpolates exactly
    /// the validated values (names escaped) between the fixed prelude/epilogue.
    #[test]
    fn blender_script_golden() {
        let scene = validate(&json!({
            "type": "otto-scene3d", "version": 1,
            "background": "#000000",
            "camera": { "position": [0, 2, 5], "target": [0, 0, 0], "fov": 45 },
            "lights": [
                { "id": "key", "type": "point", "position": [1, 2, 3], "intensity": 2, "color": "#ff0000" },
                { "id": "amb", "type": "ambient", "intensity": 0.25 }
            ],
            "objects": [
                { "id": "b", "name": "Crate \"A\"", "type": "box", "position": [0, 0.5, 0], "rotation": [0, 45, 0],
                  "material": { "color": "#ffffff", "opacity": 0.5, "wireframe": true } },
                { "id": "t", "type": "text", "text": "hi\nthere", "position": [1, 0, 0] }
            ],
            "groups": [ { "id": "g", "children": ["b", "t"] } ]
        }))
        .unwrap();
        let py = to_blender_script(&scene);
        assert_eq!(py, to_blender_script(&scene), "deterministic");
        let body = py
            .strip_prefix(PRELUDE)
            .and_then(|s| s.strip_suffix(EPILOGUE))
            .expect("prelude + epilogue are fixed");
        let expected = "\
set_world((0.0, 0.0, 0.0), 0.25)
add_camera((0.0, 2.0, 5.0), (0.0, 0.0, 0.0), 45.0)
add_light(\"key\", \"POINT\", (1.0, 2.0, 3.0), (0.0, 0.0, 0.0), 2.0, (1.0, 0.0, 0.0), True)
add_object(\"b\", \"Crate \\\"A\\\"\", \"box\", (0.0, 0.5, 0.0), (0.0, 45.0, 0.0), (1.0, 1.0, 1.0), dict(color=(1.0, 1.0, 1.0), metalness=0.0, roughness=0.6, opacity=0.5, emissive=(0.0, 0.0, 0.0), wireframe=True), text=\"\", visible=True)
add_object(\"t\", \"t\", \"text\", (1.0, 0.0, 0.0), (0.0, 0.0, 0.0), (1.0, 1.0, 1.0), dict(color=(0.796078431372549, 0.8352941176470589, 0.8823529411764706), metalness=0.0, roughness=0.6, opacity=1.0, emissive=(0.0, 0.0, 0.0), wireframe=False), text=\"hi\\nthere\", visible=True)
add_group(\"g\", \"g\", [\"b\", \"t\"])
";
        assert_eq!(body, expected);
        // Fixed parts really are fixed and carry the render/export contract.
        assert!(py.contains("scene.render.resolution_x = 1280"));
        assert!(py.contains("\"render.png\""));
        assert!(py.contains("\"scene.glb\""));
        assert!(py.contains("--out"));
    }

    #[test]
    fn gltf_objects_export_as_named_empties_not_paths() {
        let scene = validate(&sample()).unwrap();
        let py = to_blender_script(&scene);
        assert!(py.contains("add_object(\"hero\", \"Hero\", \"gltf\""));
        // The attachment id never reaches the script (no fs path surface).
        assert!(!py.contains("01HXYZ"));
    }
}
