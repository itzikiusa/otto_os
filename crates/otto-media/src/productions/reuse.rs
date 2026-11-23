//! Reuse inside a project (contract v1). Every creation belongs to one
//! project: the id of the project's first creation (its own id when it
//! started one, as every older creation did). A new scene of a project gets
//! the made pieces of the project's other creations (its cast and props, the
//! newest version of each) and its last place in the planner prompt
//! ([`prompt`]); the planner marks an item that IS one of them with
//! `details.reuse`. [`resolve`] checks every mark against those pieces (by
//! the model, never by name), drops one that is not among them, and fills
//! the item in from its piece, so it keeps its look, size, body, moves and
//! voice. The executor then puts it in the scene at once (`reuse_first`):
//! nothing is made for it. A new project starts clean: nothing from another
//! project is offered or taken.

use std::collections::HashSet;

use otto_core::{Id, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{Details, Plan, Production, MAX_NOTES};
use crate::service::MediaService;

/// Which made piece an item is, and (filled in from the piece) where it
/// was made and what it carries along.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Reuse {
    pub artifact_id: Id,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_id: Option<Id>,
    /// The creation it was made in, its item there (its picture) and that
    /// creation's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub production_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Its name there: a character's voice is designed from its name and
    /// voice, so its new lines sound the same.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its skeleton as the engine said (the result's `rig`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rig: Option<Value>,
    /// A line it spoke there: a voice engine that picks a voice speaks its
    /// new lines in that line's voice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Id>,
}

/// One made piece of a project, as it was planned; `details.reuse` says
/// which model it is and where it was made.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Piece {
    pub kind: String,
    pub name: String,
    pub description: String,
    pub details: Details,
}

/// The made characters and objects of `project` (newest creation first),
/// but not of `except`: each model once, from the newest creation that has
/// it (so the newest version of a piece changed later). One that was itself
/// reused keeps where it was first made.
pub fn pieces(project: &[Production], except: &str) -> Vec<Piece> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for p in project.iter().filter(|p| p.id != except) {
        for it in &p.plan.items {
            let model = matches!(it.kind.as_str(), "character" | "object");
            let made = it.result.as_ref().filter(|_| model && it.status == "done");
            let Some((r, aid)) = made.and_then(|r| Some((r, r.artifact_id.clone()?))) else {
                continue;
            };
            if !seen.insert(aid.clone()) {
                continue;
            }
            let spoken = p.plan.items.iter().find(|l| {
                l.details.speaker.as_deref() == Some(it.id.as_str())
                    && l.result.as_ref().is_some_and(|r| r.clip_id.is_some())
            });
            let was = it.details.reuse.clone().unwrap_or_default();
            let d = &it.details;
            let details = Details {
                prompt: d.prompt.clone(),
                voice: d.voice.clone(),
                size_m: d.size_m,
                dims_m: d.dims_m,
                body: d.body.clone(),
                moves: d.moves.clone(),
                reuse: Some(Reuse {
                    artifact_id: aid,
                    version_id: r.version_id.clone(),
                    item_id: was.item_id.or(Some(it.id.clone())),
                    production_id: was.production_id.or(Some(p.id.clone())),
                    from: was.from.or(Some(p.plan.title.clone())),
                    name: was.name.or(Some(it.name.clone())),
                    rig: r.rig.clone().or(was.rig),
                    line: was
                        .line
                        .or(spoken.and_then(|l| l.result.as_ref()?.clip_id.clone())),
                }),
                ..Default::default()
            };
            out.push(Piece {
                kind: it.kind.clone(),
                name: it.name.clone(),
                description: it.description.clone(),
                details,
            });
        }
    }
    out
}

/// Check every `details.reuse` of `plan` against `pieces` and fill the item
/// in from its piece: its look (a prompt it does not state), its body,
/// moves and voice, and a size it does not state. A mark that is no piece
/// of the project (or of another kind) is dropped: that item is made. The
/// ids of the dropped ones.
pub fn resolve(plan: &mut Plan, pieces: &[Piece]) -> Vec<String> {
    let (mut dropped, mut notes) = (Vec::new(), Vec::new());
    for it in &mut plan.items {
        let Some(asked) = it.details.reuse.take() else {
            continue;
        };
        let piece = pieces.iter().find(|p| {
            p.kind == it.kind
                && p.details
                    .reuse
                    .as_ref()
                    .is_some_and(|r| r.artifact_id == asked.artifact_id)
        });
        let Some(from) = piece.map(|p| &p.details) else {
            dropped.push(it.id.clone());
            continue;
        };
        let d = &mut it.details;
        d.prompt = d.prompt.take().or(from.prompt.clone());
        d.size_m = d.size_m.or(from.size_m);
        d.dims_m = d.dims_m.or(from.dims_m);
        (d.body, d.voice) = (from.body.clone(), from.voice.clone());
        let lost: Vec<String> = d
            .moves
            .iter()
            .filter(|m| !from.moves.contains(m))
            .cloned()
            .collect();
        if !lost.is_empty() {
            notes.push(format!(
                "{} keeps the moves it has, so it does not {} here.",
                it.name,
                lost.join(" or ")
            ));
        }
        (d.moves, d.reuse) = (from.moves.clone(), from.reuse.clone());
    }
    for n in notes {
        if plan.notes.len() < MAX_NOTES && !plan.notes.contains(&n) {
            plan.notes.push(n);
        }
    }
    dropped
}

/// The planner's part about the project: its pieces, one JSON line each,
/// how to reuse them, and its last place. Empty for a project with nothing
/// made yet.
pub fn prompt(pieces: &[Piece], place: Option<&super::setting::Setting>) -> String {
    if pieces.is_empty() && place.is_none() {
        return String::new();
    }
    let lines: Vec<String> = pieces
        .iter()
        .map(|p| {
            let d = &p.details;
            let r = d.reuse.as_ref();
            let mut v = json!({
                "reuse": {"artifact_id": r.map(|r| &r.artifact_id), "version_id": r.and_then(|r| r.version_id.as_ref())},
                "kind": p.kind, "name": p.name, "description": p.description,
                "size_m": d.size_m, "dims_m": d.dims_m, "body": d.body,
                "moves": d.moves, "voice": d.voice,
            });
            if let Some(o) = v.as_object_mut() {
                o.retain(|_, x| !x.is_null() && x != &json!([]));
            }
            v.to_string()
        })
        .collect();
    let mut out = String::from("\nTHIS SCENE IS PART OF A PROJECT.\n");
    if !lines.is_empty() {
        out += &format!(
            "Its earlier scenes already made these pieces (one JSON object a line):\n{}\n\
Reuse them:\n\
- When the description means one of these pieces (the same name, or clearly the same character or \
thing), the item IS that piece: give it its kind, a name, a one-sentence description and \
\"details\": {{\"reuse\": {{\"artifact_id\": ..., \"version_id\": ...}}}} copied exactly from its line, and \
nothing else in details (its look, size, body, moves and voice come with it; it is not made again). \
Place it in plan.layout like any piece.\n\
- A reused character speaks in its own voice: its lines need only details.speaker. It has only the \
moves on its line; any other move goes into plan.notes.\n\
- Make a new item only for what is new, or when the description clearly changes how a piece looks \
(another outfit, older, another colour): then it is a new item without \"reuse\", and plan.notes \
says so.\n",
            lines.join("\n")
        );
    }
    if let Some(place) = place {
        // Its lights name that scene's items: this plan says its own.
        let mut place = serde_json::to_value(place).unwrap_or_default();
        place.as_object_mut().map(|o| o.remove("lights"));
        let place = place.to_string();
        out += &format!(
            "The project's last scene was set here (plan.setting): {place}\nKeep that place (the same \
ground, indoors or out) unless the description moves somewhere else.\n"
        );
    }
    out
}

impl MediaService {
    /// The creations of `project`, newest first.
    pub(crate) async fn project(&self, project: &str) -> Result<Vec<Production>> {
        self.store().project_productions(project).await
    }

    /// The made pieces of `p`'s project that are not its own, and the
    /// project's last place (its newest other creation's setting).
    pub(crate) async fn project_offer(
        &self,
        p: &Production,
    ) -> Result<(Vec<Piece>, Option<super::setting::Setting>)> {
        let all = self.project(&p.project_id).await?;
        let place = all
            .iter()
            .filter(|o| o.id != p.id)
            .find_map(|o| o.plan.setting.clone());
        Ok((pieces(&all, &p.id), place))
    }
}
