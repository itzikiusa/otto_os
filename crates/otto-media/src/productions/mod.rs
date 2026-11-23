//! Smart mode — **productions** (contract v1, docs/contracts/api.md). One
//! plain description of a whole scene becomes a plan of items (characters,
//! objects, voice lines, sounds, music) and a layout. ONE planner agent turn
//! writes the plan; it is validated strictly here and retried once. Making the
//! items is deterministic server code ([`exec`]): 3D items through the 3D job
//! path one at a time, voice lines through TTS, sound and music through the
//! sound engine ([`crate::audio`]). The made models are assembled into one `scene3d` artifact
//! ([`scene`]). Steering re-plans with a comment (and 3D pins) and makes only
//! the items it changed or added.

pub mod agent;
pub mod e2e;
mod effects;
pub mod exec;
pub mod geometry;
pub mod http;
pub mod pose;
pub mod progress;
pub mod relation;
pub mod reuse;
mod reveal;
pub mod scene;
pub mod setting;
mod store;

use std::collections::HashSet;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use otto_core::{Id, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use relation::Rel;

pub const ITEM_KINDS: &[&str] = &["character", "object", "voice_line", "sound", "music"];
pub const USES: &[&str] = &["phone", "pc", "closeup"];
pub const PLACES: &[&str] = &["left", "center", "right", "front", "back", "far", "wall"];
pub const SCALE_HINTS: &[&str] = &["small", "normal", "big"];
/// Where a character wears or holds a prop (`details.holds[].socket`).
pub const SOCKETS: &[&str] = &[
    "right_hand",
    "left_hand",
    "back",
    "hip_left",
    "hip_right",
    "neck",
];
/// Interaction templates a character may name in `details.actions` (the fit
/// stage being built makes them; until then they are carried through).
pub const ACTION_TEMPLATES: &[&str] = &["shoot_bow", "swing_sword", "pull_sword"];
/// The moves the local engine makes today (`forge animate --clips`); a plan
/// may name others, which the executor leaves out with a plain note.
pub const ENGINE_MOVES: &[&str] = &["idle", "walk", "run"];
pub const MAX_ITEMS: usize = 24;
const MAX_NOTES: usize = 20;
pub const MAX_DESCRIPTION_CHARS: usize = 4_000;

/// One planner agent turn: the full prompt in, the agent's plan JSON out (the
/// file it wrote, or its reply). Production: [`agent::AgentPlanner`].
#[async_trait]
pub trait ProductionPlanner: Send + Sync {
    async fn run(&self, production_id: &str, prompt: String) -> Result<String>;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Plan {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub items: Vec<Item>,
    #[serde(default)]
    pub notes: Vec<String>,
    #[serde(default)]
    pub layout: Vec<Placement>,
    /// Main performers in audience-left to audience-right order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lineup: Vec<String>,
    /// Where and when the scene is ([`setting`]); an older plan has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setting: Option<setting::Setting>,
    /// The notes the scene's last build said (among `notes`): the next build
    /// takes back the ones no longer true.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Item {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub description: String,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub details: Details,
    #[serde(default = "waiting")]
    pub status: String,
    #[serde(default)]
    pub progress_pct: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<ItemResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_result: Option<ItemResult>,
    /// While working: which step of how many, in plain words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<Step>,
    /// When it last started working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_since: Option<DateTime<Utc>>,
    /// `"engine"`: it is next but waits for the engine's turn (another job holds it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting: Option<String>,
    /// A picture of it exists (its concept picture as soon as it is drawn);
    /// `preview_rev` counts each new one.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_preview: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub preview_rev: i64,
    /// How far a model got while it is not done (Start goes on from there,
    /// never makes it again): `"picture"` its concept picture is drawn (the
    /// 3D shape is built from it); `"draft"` an object's first 3D shape is in
    /// the scene, its game-ready pass is still to come; `"mesh"` a
    /// character's model is made, its skeleton and moves are not yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub made: Option<String>,
    /// The seed its picture and shape are drawn with (`None`: 42); a
    /// "not this one" redraw picks another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seed: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Step {
    pub n: u32,
    pub of: u32,
    pub label: String,
    /// How far into this step it is (0–1), when the engine says so (the 3D
    /// shape's own sub-steps).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fraction: Option<f64>,
}

fn yes() -> bool {
    true
}
fn is_zero(n: &i64) -> bool {
    *n == 0
}
fn waiting() -> String {
    "waiting".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Details {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    /// Visible moisture on this object at scene start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<effects::Effect>,
    /// A plain description of the voice ("deep, a bit broken").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<String>,
    /// The id of the character who speaks a voice line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonverbal: Option<bool>,
    #[serde(rename = "loop", default, skip_serializing_if = "Option::is_none")]
    pub looped: Option<bool>,
    /// A requested musical performance plays to its end in the scene player.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub featured: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<f64>,
    /// Its real size in metres: a character's height, an object's longest side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_m: Option<f64>,
    /// Its planned proportions, [width, height, depth] in metres (a deck, a
    /// table, a road): the scene scales the made model toward them per axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dims_m: Option<[f64; 3]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub moves: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach_to: Option<Attach>,
    /// Props a character holds or wears (each its own `object` item).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<Hold>,
    /// Interaction template ids ([`ACTION_TEMPLATES`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    /// A character's body ([`relation::BODIES`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// A voice line's pace: slow, normal or fast.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pace: Option<String>,
    /// A voice line's mood, a few plain words ("tired, kind").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mood: Option<String>,
    /// The voice line this one answers (the conversation's order).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replies_to: Option<String>,
    /// A character's pose in this scene ([`pose`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose: Option<pose::Pose>,
    /// A character or object that IS a made piece of the production's
    /// project ([`reuse`]): it goes into the scene as it is, nothing is made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reuse: Option<reuse::Reuse>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Attach {
    pub item: String,
    #[serde(rename = "where", default)]
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Hold {
    pub item: String,
    pub socket: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Placement {
    pub item: String,
    /// Where a free piece stands; empty for one with a `rel`.
    #[serde(default)]
    pub place: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attach_to: Option<Attach>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_hint: Option<String>,
    /// Where it is relative to another piece ([`relation::Rel`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rel: Option<relation::Rel>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ItemResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clip_id: Option<Id>,
    /// What its skeleton is, as the engine said (`body_plan`, `legs`, and its optional forward direction).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rig: Option<Value>,
    /// This scene's posed model ([`pose`]): `{spec, artifact_id?, version_id?,
    /// manifest?, failed?}` (the result's own model stays unposed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose: Option<Value>,
}

/// One steering round. `before` (stored, never sent) is what the changed and
/// removed items were, so a revert can put them back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Revision {
    pub id: Id,
    pub comment: String,
    pub summary: String,
    pub changed: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// What the change already shows without making anything, one plain
    /// sentence a piece ("The monster is bigger.").
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub done: Vec<String>,
    pub at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub before: Vec<Item>,
    /// The setting it replaced (stored, never sent; [`setting::before`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setting_before: Option<setting::Setting>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Production {
    pub id: Id,
    pub workspace_id: Id,
    /// Its project: the id of the project's first creation (its own id when
    /// it started one, as every older creation did).
    pub project_id: Id,
    /// The project's name (its first creation's title), when that is another creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    /// While it is `ready`: the made pieces of its project's other creations
    /// (the newest version of each), any item can be switched to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub project_pieces: Vec<reuse::Piece>,
    pub description: String,
    #[serde(rename = "use")]
    pub use_for: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_secs: Option<i64>,
    /// Of that, until every piece's picture is drawn, and until the scene
    /// can be looked at (every piece has a shape), while still to come.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pictures_eta_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_eta_secs: Option<i64>,
    /// When pieces are reused: what `eta_secs` would be with every piece made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eta_unreused_secs: Option<i64>,
    pub plan: Plan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_artifact_id: Option<Id>,
    pub revisions: Vec<Revision>,
    /// The agent and model every planner turn of it runs with (picked on
    /// create; `None`: the workspace default agent, the provider's default model).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The reasoning effort of every planner turn (`None`: the model's default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// The pick in one place (read only: `provider`, `model`, `effort`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentPick>,
    /// Running and waiting for its turn: how many productions are ahead of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_position: Option<i64>,
    /// When it was started, while it runs (its place in the line, kept across
    /// a restart); never sent.
    #[serde(skip)]
    pub queued_at: Option<DateTime<Utc>>,
    /// Every 3D piece is in the scene (characters may still be getting their
    /// skeleton and moves): it can be looked at.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scene_ready: bool,
    pub created_by: Id,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The agent, model and effort a production plans with (`None`: the default).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentPick {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

impl Production {
    /// The API shape: revisions without their stored `before` items.
    pub fn public(mut self) -> Self {
        self.revisions.iter_mut().for_each(|r| {
            r.before.clear();
            r.setting_before = None;
        });
        let pick = self.pick();
        self.agent = (pick != AgentPick::default()).then_some(pick);
        self
    }

    pub fn pick(&self) -> AgentPick {
        AgentPick {
            provider: self.provider.clone(),
            model: self.model.clone(),
            effort: self.effort.clone(),
        }
    }
}

/// `GET /productions` row: no plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProductionSummary {
    pub id: Id,
    pub workspace_id: Id,
    pub project_id: Id,
    /// The project's name (its first creation's title), when that is another
    /// creation the list holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    pub description: String,
    #[serde(rename = "use")]
    pub use_for: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub title: String,
    /// Its workspace's name (the list across workspaces says where each is).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_name: Option<String>,
    /// Running and waiting for its turn: how many productions are ahead of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_position: Option<i64>,
    /// Its enabled items: how many are made, of how many.
    pub items_done: i64,
    pub items_total: i64,
    /// Its scene (a thumbnail for its card), once there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_artifact_id: Option<Id>,
    /// Its scene and every model and clip its items made (never sent): what
    /// the shelf does not show loose.
    #[serde(skip)]
    pub pieces: Vec<Id>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// `GET /design/creations`: everything made in Create, across workspaces.
#[derive(Debug, Serialize)]
pub struct Creations {
    pub productions: Vec<ProductionSummary>,
    /// 3D models not inside a production.
    pub models: Vec<otto_design::types::DesignArtifact>,
    /// Voice, sound and music clips not inside a production.
    pub clips: Vec<crate::types::TtsClip>,
}

#[derive(Debug, Deserialize)]
pub struct CreateProductionReq {
    pub workspace_id: Id,
    pub description: String,
    #[serde(rename = "use", default)]
    pub use_for: Option<String>,
    /// The agent that plans (`None`: the workspace default) and its model
    /// (`None`: the provider's default).
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// The reasoning effort (`None`: the model's default).
    #[serde(default)]
    pub effort: Option<String>,
    /// A creation of the project it is a new scene of (`None`: a new project).
    #[serde(default)]
    pub project_id: Option<Id>,
}

impl CreateProductionReq {
    /// The picked agent, model and effort, a blank one as unset.
    pub fn agent(&self) -> AgentPick {
        fn pick(s: &Option<String>) -> Option<String> {
            s.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        }
        AgentPick {
            provider: pick(&self.provider),
            model: pick(&self.model),
            effort: pick(&self.effort),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct PatchProductionReq {
    pub items: Vec<ItemPatch>,
}

#[derive(Debug, Deserialize)]
pub struct ItemPatch {
    pub id: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub details: Option<Details>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pin {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifact_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub point: Option<[f64; 3]>,
    pub note: String,
}

#[derive(Debug, Deserialize)]
pub struct SteerReq {
    pub comment: String,
    #[serde(default)]
    pub pins: Vec<Pin>,
}

#[derive(Debug, Deserialize)]
pub struct RevertReq {
    pub revision_id: Id,
}

/// What a steering turn says it changed (ids of the revised plan).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RevisionDraft {
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub changed: Vec<String>,
    #[serde(default)]
    pub added: Vec<String>,
    #[serde(default)]
    pub removed: Vec<String>,
}

// ---------------------------------------------------------------------------
// Reading and checking the planner's answer
// ---------------------------------------------------------------------------

/// The JSON object in a planner answer: the whole text, else its first fenced
/// block, else the outermost `{…}`.
fn json_object(text: &str) -> std::result::Result<Value, String> {
    let t = text.trim();
    if let Some(fenced) = otto_agent_run::fence::extract_fenced(t, "json") {
        if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(&fenced) {
            return Ok(v);
        }
    }
    let outer = t.find('{').zip(t.rfind('}')).map(|(a, b)| &t[a..=b]);
    for c in [Some(t), outer].into_iter().flatten() {
        if let Ok(v @ Value::Object(_)) = serde_json::from_str::<Value>(c) {
            return Ok(v);
        }
    }
    Err("the answer is not a JSON object".into())
}

/// Parse a planner answer: a plan (or `{plan, revision}` from a steering
/// turn), validated. The message of an `Err` goes back to the planner once.
pub fn parse_plan_reply(text: &str) -> std::result::Result<(Plan, Option<RevisionDraft>), String> {
    let v = json_object(text)?;
    let (plan_v, rev_v) = match v.get("plan") {
        Some(p) => (p.clone(), v.get("revision").cloned()),
        None => (v, None),
    };
    let mut plan: Plan = serde_json::from_value(plan_v).map_err(|e| format!("plan: {e}"))?;
    for it in &mut plan.items {
        // Progress is the server's, never the planner's.
        it.status = "waiting".into();
        it.progress_pct = 0;
        (it.message, it.result, it.previous_result) = (None, None, None);
        (it.step, it.working_since, it.waiting, it.made) = (None, None, None, None);
        (it.has_preview, it.preview_rev) = (false, 0);
    }
    validate_plan(&plan)?;
    let rev = rev_v
        .map(serde_json::from_value::<RevisionDraft>)
        .transpose()
        .map_err(|e| format!("revision: {e}"))?;
    Ok((plan, rev))
}

fn safe_item_id(id: &str) -> bool {
    (1..=40).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

fn text_ok(s: &str, max: usize) -> bool {
    !s.trim().is_empty() && s.chars().count() <= max
}

/// The strict plan check (the planner gets the first problem back once).
pub fn validate_plan(plan: &Plan) -> std::result::Result<(), String> {
    if !text_ok(&plan.title, 120) {
        return Err("plan.title must be 1-120 characters".into());
    }
    if plan.summary.chars().count() > 600 {
        return Err("plan.summary is at most 600 characters".into());
    }
    if plan.items.is_empty() || plan.items.len() > MAX_ITEMS {
        return Err(format!("a plan has 1-{MAX_ITEMS} items"));
    }
    if plan.notes.len() > MAX_NOTES || plan.notes.iter().any(|n| !text_ok(n, 300)) {
        return Err(format!(
            "plan.notes: at most {MAX_NOTES} notes of 1-300 characters"
        ));
    }
    let mut ids = HashSet::new();
    for it in &plan.items {
        if !safe_item_id(&it.id) || !ids.insert(it.id.as_str()) {
            return Err(format!(
                "item id {:?} must be unique, 1-40 of a-z 0-9 _ -",
                it.id
            ));
        }
    }
    let kind_of = |id: &str| {
        plan.items
            .iter()
            .find(|i| i.id == id)
            .map(|i| i.kind.as_str())
    };
    for it in &plan.items {
        let at = format!("item {}", it.id);
        if !ITEM_KINDS.contains(&it.kind.as_str()) {
            return Err(format!(
                "{at}: kind must be one of {}",
                ITEM_KINDS.join(", ")
            ));
        }
        if !text_ok(&it.name, 80) || !text_ok(&it.description, 300) {
            return Err(format!("{at}: name 1-80 and description 1-300 characters"));
        }
        let d = &it.details;
        let character = it.kind == "character";
        if let Some(s) = &d.speaker {
            if kind_of(s) != Some("character") {
                return Err(format!("{at}: speaker {s:?} is not a character item"));
            }
        }
        if it.kind == "voice_line"
            && d.nonverbal != Some(true)
            && !d.text.as_deref().is_some_and(|t| text_ok(t, 1_000))
        {
            return Err(format!(
                "{at}: a voice line needs its text (1-1000 characters)"
            ));
        }
        if d.seconds.is_some_and(|s| !(s > 0.0 && s <= 600.0)) {
            return Err(format!("{at}: seconds must be within 0-600"));
        }
        if d.size_m.is_some_and(|s| !(0.01..=100.0).contains(&s)) {
            return Err(format!("{at}: size_m must be within 0.01-100 metres"));
        }
        if d.reuse.is_some() && !matches!(it.kind.as_str(), "character" | "object") {
            return Err(format!("{at}: only a character or object reuses a piece"));
        }
        if !character && !(d.moves.is_empty() && d.holds.is_empty() && d.actions.is_empty()) {
            return Err(format!(
                "{at}: only a character has moves, holds or actions"
            ));
        }
        if let Some(a) = d
            .actions
            .iter()
            .find(|a| !ACTION_TEMPLATES.contains(&a.as_str()))
        {
            return Err(format!(
                "{at}: action {a:?} is not one of {}",
                ACTION_TEMPLATES.join(", ")
            ));
        }
        for h in &d.holds {
            if kind_of(&h.item) != Some("object") || !SOCKETS.contains(&h.socket.as_str()) {
                return Err(format!(
                    "{at}: holds {:?} must name an object item and a socket ({})",
                    h.item,
                    SOCKETS.join(", ")
                ));
            }
        }
        if let Some(a) = &d.attach_to {
            if a.item == it.id || kind_of(&a.item).is_none() {
                return Err(format!("{at}: attach_to {:?} is not another item", a.item));
            }
        }
    }
    let mut placed = HashSet::new();
    for p in &plan.layout {
        let at = format!("layout {}", p.item);
        if !matches!(kind_of(&p.item), Some("character" | "object")) || !placed.insert(&p.item) {
            return Err(format!("{at}: places each character or object item once"));
        }
        let parts: Vec<&str> = p
            .place
            .split(['-', ' ', '_', '+'])
            .filter(|s| !s.is_empty())
            .collect();
        let related = p.rel.is_some() || p.attach_to.is_some();
        if (parts.is_empty() && !related) || parts.iter().any(|s| !PLACES.contains(s)) {
            return Err(format!(
                "{at}: place is {} or a combination (or the piece has a rel)",
                PLACES.join(", ")
            ));
        }
        let other = |id: &Option<String>| {
            id.as_deref()
                .is_some_and(|f| f == p.item || kind_of(f).is_none())
        };
        if other(&p.facing) || other(&p.attach_to.as_ref().map(|a| a.item.clone())) {
            return Err(format!("{at}: facing and attach_to name another item"));
        }
        if p.scale_hint
            .as_deref()
            .is_some_and(|s| !SCALE_HINTS.contains(&s))
        {
            return Err(format!("{at}: scale_hint is small, normal or big"));
        }
    }
    relation::validate(plan)?;
    pose::validate(plan)?;
    setting::validate(plan)
}

// ---------------------------------------------------------------------------
// The planner prompt (bundled)
// ---------------------------------------------------------------------------

const PLAN_RULES: &str = r#"You plan a small game scene for Otto. Otto makes each item itself:
- "character": a 3D model of a person or creature. It is generated EMPTY-HANDED in a neutral pose:
  never say in its own description that it holds a prop. details.body says what its body is:
  two_legs (a person, a robot, a bear standing up), four_legs (a fox, a dog, a horse), bird, fish (any
  swimmer), no_legs (a snowman, a snake), many_legs (an octopus, a spider, a six-legged robot), wheels.
  details.moves lists the moves to make, only from: idle, walk, run. details.voice describes its
  voice in plain words. details.prompt is the 3D prompt (its looks and the clothes it wears, no pose, no props). details.size_m is its real height in metres (a man 1.8, a child 1.2, a big monster
  2.5-3, a fox 0.5 long).
  details.pose {"kind": stand|sit|kneel|crouch|hold_out|lie|curl|lean|peek, "on"?: the seat or leaning support item id,
  "play"?: item id of a standing double bass, cello, or a drum kit a many-limbed drummer strikes,
  "sing_into"?: the microphone item id for a singer at a mic,
  "plant"?: item id of a flagpole or stake being planted, kept independently upright with its base on the ground and the actor beside it,
  "arms"?: crossed (folded across the chest), "look_at"?: an item id or "camera", "hand"?: left|right (hold_out)} whenever the description says
  it sits, kneels, crouches, lies down, is curled up or asleep, holds out a hand or looks at
  something ("the children sit on logs and look up at the owl": each child sit on its log, look_at
  the owl). Every stated look ("looks up at", "watches", "stares at", "turns to", "faces") is
  details.pose.look_at, alongside any other pose (sit and look_at together; a look alone is kind
  stand with look_at). "X on the palm of Y": Y has pose hold_out (and hand), X has rel held by Y in
  that hand. A group performing together on stage normally forms a row: put all performer ids
  in plan.lineup from audience-left to audience-right, unless the request gives another
  formation. Keep stands, instruments and held props in each player's layout as usual.
  Choose the body and neutral model shape for the action: a tool-using animal supported on its
  hind feet with two free forepaws has body two_legs. Describe that anatomy in details.prompt.
  For an ordinary kit drummer using two hands, plan two separate sticks held left and right,
  each with rel.toward the kit. Leave that player standing unless the scene calls for a seat.
  A drummer using bare tentacles strikes with pose.play and no sticks.
  Preserve explicitly requested anatomy and playing technique.
  Preserve a lean against a named object as kind lean, on that object, rel beside
  its side. Preserve folded or crossed arms as arms crossed, even with another body pose.
  Keep an exposed feature (an open hood or a doorway) clear when placing someone beside it.
  A thing it holds (sword, lantern, spoon, stick) is its own "object" item; the character lists it in
  details.holds [{"item": <object id>, "socket": "right_hand"|"left_hand"|"back"|"hip_left"|"hip_right"|"neck"}]
  and what it does with it in details.actions (only: shoot_bow, swing_sword, pull_sword).
  A planted shaft stands on its own base: use pose.plant and place the character beside the shaft.
  A pail being carried or a spoon being used to stir is held by the character's hand.
  A creature peeking from under furniture uses kind peek and rel under that furniture.
  When only a small part of a hidden character is visible, give that feature enough contrast
  to read in the scene. Carry it into the character's own model prompt. For eyes peeking from
  darkness, describe prominent pale irises, dark pupils and reflected light on a shadowed face.
  Keep the colours and visibility the person specifies, including deliberately unlit eyes.
  Clothing it wears (an apron, a scarf, a coat, a cape, a cloak, a dress, a shawl, a belt, gloves,
  boots, glasses) is part of the character: say it in its details.prompt ("an old grandmother
  wearing a white apron"), never as an item. Only a hat, cap, crown or helmet (rel worn, part head),
  a backpack or quiver (rel worn, part back) and a small thing on a string round the neck (a key,
  a medal, a bell: rel worn, part neck) are their own "object" items, NOT in details.holds.
- For visible steam rising from an object, set that object's details.effect to "steam".
  For rain visible on a window use "rain_on_glass" on the window, even indoors. Keep the room dry.
  A bedside or poolside piece uses rel beside its bed or pool, with a side when specified.
- "object": a 3D prop or set piece; details.prompt is its 3D prompt; details.size_m is its real
  longest side in metres (a sword 1.0, a chest 0.6, a coin pile 0.3, a key 0.08). details.dims_m
  [width, height, depth] in metres when its shape matters (a pier deck [8, 1.2, 3], a table
  [1.2, 0.75, 1.2], a bed, a car, a bookshelf).
  When the text names contents that should be seen (apples in a basket, pancakes on a plate, a teddy
  bear in a bed) the container and the contents are separate items, the contents placed
  with rel in or on the container. Describe each empty support or container by its own materials,
  shape and finish. Keep the separate occupant and its name out of its 3D prompt; the layout
  supplies that relationship. For an occupant tucked beneath a separate cover, place the cover
  on its support and the occupant under the cover, at the edge where its head shows.
- "voice_line": one spoken line: details.text (the words), details.speaker (a character id),
  details.nonverbal true for a grunt, scream or laugh, details.voice if it differs from the speaker,
  details.pace slow|normal|fast, details.mood a few plain words (warm, tired, cheeky, whispering,
  shouting), details.replies_to the id of the line it answers (lines in a conversation each answer
  the one before). Every character that speaks has details.voice: one stable description all its
  lines share, in plain words: timbre, age, pace, accent ("deep, tired, old male robot, slow").
  Fill them from the description: "speaks slowly in a deep, tired voice" gives the character the
  voice "deep, tired, old male, slow" and its line pace slow, mood "tired, kind".
  A character can answer with a sound instead of words. For a beep, meow, clap or other sound
  that follows a spoken cue, use a voice_line with that character as speaker, nonverbal true and
  replies_to set to the cue line's id. Describe the sound and its count in the item's description,
  and omit text. The sound engine makes it and Play waits for the cue to end before it starts.
- "sound": an independent sound effect (details.prompt, details.seconds, details.loop for ambience).
  These begin when the scene starts.
- "music": details.prompt, details.seconds, details.loop. Set details.featured true when the
  user asks to hear a musical performance as the main event. Play lets that whole track finish.
  Leave featured unset for background music.
Rules:
- Names and descriptions are plain words a child understands; description is one sentence.
- Item ids are short, lowercase, a-z 0-9 _ - and unique. Every item has "enabled": true.
- The PLACE ITSELF is never an item: a road, a meadow, a beach, a forest clearing, a garden, a
  kitchen, a bedroom, a room, the moon's surface, the sea floor, the sky are plan.setting (below).
  Items are only real things in the place. A pier, a stage, a bridge or a boat that things stand on IS
  an item (a made thing), with dims_m.
- Every detail the description states or clearly implies (a window, birds outside, steam, a
  spotlight, curtains, "looking up at him") is planned: in plan.setting, as an item, or as a move,
  facing or rel; what Otto cannot show yet goes into plan.notes as one plain sentence saying what
  you did instead. Never drop a detail silently.
- Honest limits: anything you cannot express with these kinds, moves and actions (for example a
  move that is not idle, walk or run, like "pulling the sword out") goes into
  plan.notes as one plain sentence saying what you did instead.
  The animation library has combat, idle, emote, death, locomotion and interaction moves, but
  Otto cannot put them on a character yet: only idle, walk and run can be made now.
- plan.layout places each character and object exactly once: {"item", "place"?, "facing"?: item id,
  "rel"?: {...}, "scale_hint"?: small|normal|big}.
  A piece standing free on the ground has "place": left|center|right|front|back|far or a combination
  like "front-left" ("far": in the background; "wall": indoors, against the room's back wall: a
  window, a painting, a clock or a mirror hangs on it, a bookshelf or a wardrobe stands at it).
  A piece on, in, under, beside, behind, in front of, above or around another piece, or held or worn
  by a character, has "rel" INSTEAD of a place:
  {"to": <item id>, "how": on|under|in|beside|behind|in_front_of|above|around|held|worn,
   "spot"?: for on, in, under: where on the target, left|center|right and/or front|middle|back
            joined with "-" ("front-left", "back", "center-middle"),
   "side"?: for beside: left|right|front|back (the target's side as the camera sees it),
   "hand"?: for held: right_hand|left_hand|both,
   "toward"?: for held: the item id it is held over or pointed at (a stick held over the fire),
   "part"?: for worn: head|neck|back|hip|shoulders}.
  Each field holds only those words, never a sentence. A held or worn piece has no place. Things
  around a centre (children round a fire) each have rel how "around" to the centre piece.
- At most 24 items. Keep it to what the description asks for and implies.
Answer ONLY by writing the JSON file named below (no prose), shaped:
{"title": "...", "summary": "one or two plain sentences", "items": [{"id","kind","name",
"description","enabled":true,"details":{...}}], "notes": ["..."], "layout": [{...}],
"setting": {...}}"#;

/// The poly budget the local engine aims for, per kind and use.
pub fn poly_budget(use_for: &str, kind: &str) -> u32 {
    match (use_for, kind == "character") {
        ("phone", true) => 8_000,
        ("phone", false) => 3_000,
        ("closeup", true) => 60_000,
        ("closeup", false) => 25_000,
        (_, true) => 25_000,
        (_, false) => 10_000,
    }
}

pub fn plan_prompt(description: &str, use_for: &str, file: &str) -> String {
    project_plan_prompt(description, use_for, file, "")
}

/// The plan prompt with the project's pieces and place ([`reuse::prompt`]; empty: none).
pub fn project_plan_prompt(description: &str, use_for: &str, file: &str, project: &str) -> String {
    format!(
        "{PLAN_RULES}\n{}\n{project}\nThe scene will be used on: {use_for}.\nWrite the plan to the file `{file}` in the current directory.\n\nThe description:\n<<<\n{description}\n>>>",
        setting::RULES
    )
}

pub fn steer_prompt(plan: &Plan, comment: &str, pins: &[Pin], file: &str) -> String {
    let plan_json = serde_json::to_string_pretty(&strip_progress(plan)).unwrap_or_default();
    let pins_json = serde_json::to_string_pretty(pins).unwrap_or_default();
    format!(
        "{PLAN_RULES}\n{}\n\nYou are REVISING an existing plan after the person looked at what was made. \
Change as little as possible: keep every item id that stays, keep unchanged items exactly as they \
are, give a new item a new id. A pin's note is about the item its item_id names: change that \
item for it, and say so in the summary. To make a piece bigger or smaller, or move it, change \
only its details.size_m or its layout entry and keep its details.prompt: it is not made again. \
{} Write to the file `{file}` in the current directory: \
{{\"plan\": <the whole revised plan>, \"revision\": {{\"summary\": \"one plain sentence saying what \
will change\", \"changed\": [ids], \"added\": [ids], \"removed\": [ids]}}}}\n\n\
The current plan:\n{plan_json}\n\nThe person's comment:\n<<<\n{comment}\n>>>\n\n\
Pins they placed on the models (point is x, y, z in the scene, metres):\n{pins_json}",
        setting::RULES,
        setting::STEER
    )
}

/// The plan as the planner sees it: content only, no progress or results.
fn strip_progress(plan: &Plan) -> Value {
    let mut v = serde_json::to_value(plan).unwrap_or_default();
    if let Some(items) = v["items"].as_array_mut() {
        for it in items.iter_mut().filter_map(Value::as_object_mut) {
            for k in [
                "status",
                "progress_pct",
                "message",
                "result",
                "previous_result",
                "step",
                "working_since",
                "waiting",
                "has_preview",
                "preview_rev",
                "made",
                "seed",
            ] {
                it.remove(k);
            }
        }
    }
    v
}

pub fn retry_prompt(first: &str, problem: &str) -> String {
    format!(
        "{first}\n\nYour previous plan was refused: {problem}. Write the corrected JSON to the same file."
    )
}

#[cfg(test)]
pub(crate) mod tests;
