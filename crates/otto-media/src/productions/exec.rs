//! The productions engine on [`MediaService`]: create / steer (one planner
//! turn each, validated, one retry), patch, start (the deterministic
//! executor), cancel, revert, and the restart recovery.
//!
//! Productions make one at a time, in Start order (the rest wait, with their
//! `queue_position`). The executor first takes what Otto Forge's library has
//! (seconds), then runs three lanes side by side: the models through the 3D
//! job path (one in its GPU steps at a time, the next starting when it
//! reaches its CPU game-ready pass; each made model goes into the scene at
//! once, and once every model is in, the characters get their skeletons and
//! their known moves, `{clips}`), the spoken lines through TTS, and sound,
//! music and nonverbal lines through the sound engine ([`crate::audio`],
//! lengths clamped to its limits). One item failing never stops the others.
//! Then the scene is written a last time, with its notes.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use base64::Engine;
use chrono::Utc;
use otto_core::cancel::{CancelGuard, CancelRegistryExt};
use otto_core::{new_id, Error, Result};
use otto_design::service::{Author, CreateInput, SaveOpts};
use serde_json::{json, Value};

use std::path::Path;

use otto_core::Id;

use super::pose::Posed;
use super::progress::{
    eta, finish_dir, generate_step, join_all, preview_path, shape_fraction, step, steps_of,
    waits_for_heavy_slot, work_file, DRAWING, LIBRARY, MOVES, READYING, SHAPING, SKELETON,
};
use super::scene::{glb_bounds, glb_joints, glb_tris, size_m, words, Bounds, Model};
use super::setting::{self, build_scene};
use super::{
    parse_plan_reply, poly_budget, retry_prompt, steer_prompt, validate_plan, CreateProductionReq,
    Creations, Item, ItemResult, PatchProductionReq, Plan, Production, ProductionPlanner, Revision,
    RevisionDraft, SteerReq, Step, ENGINE_MOVES, MAX_DESCRIPTION_CHARS, USES,
};
use super::{project_plan_prompt, reuse};
use crate::audio::{audio_limits, StartAudioJobReq, MAX_AUDIO_PROMPT_CHARS};
use crate::gen3d::{read_optional_manifest, run_cli_argv, validate_3d_output};
use crate::service::MediaService;
use crate::types::{
    Media3dProviderRecord, StartMedia3dJobReq, SynthesizeTtsReq, TtsClip, TtsClipRecord, TtsVoice,
    BUILTIN_MACOS_SAY_ID, BUILTIN_OTTO_FORGE_ID,
};

/// How often a running item reads its 3D job.
const JOB_POLL: Duration = if cfg!(test) {
    Duration::from_millis(40)
} else {
    Duration::from_millis(500)
};
const STOPPED: &str = "Stopped before it was made.";
const FROM_LIBRARY: &str = "Taken from the Otto Forge library.";
/// Shape words a library piece must share with the plan's name to match it.
const SHAPES: &[&str] = &["round", "square", "long", "tall", "low", "short"];
const NO_SOUND_ENGINE: &str = "The sound engine is not installed, so this was not made.";
const NO_VOICE_ENGINE: &str =
    "Spoken with the built-in voice because the voice engine is not installed.";
/// Otto Forge's voice engine, unless its provider's `default_params.voice_engine` names another.
const FORGE_VOICE_ENGINE: &str = "qwen";
/// Words of a line's pace or mood (or a voice) that slow it down.
const SLOW_WORDS: &[&str] = &[
    "slow",
    "slowly",
    "whisper",
    "whispers",
    "whispering",
    "scared",
    "nervous",
    "sad",
    "tired",
    "sleepy",
    "quiet",
    "quietly",
    "soft",
    "softly",
];
/// No line comes out faster than this per word (the engine runs brisk).
const MIN_MS_PER_WORD: u64 = 280;
const STANDING: &str = "In the scene; its skeleton comes once every model is made.";

/// What a waiting item waits for, said on it until its turn comes (GPU turns
/// go pictures, spoken lines, shapes, skeletons, then the sound engine).
fn waits_for(it: &Item) -> &'static str {
    match it.kind.as_str() {
        "sound" | "music" => "Made by the sound engine once every piece has its shape.",
        "voice_line" if it.details.nonverbal == Some(true) => {
            "A sound, not words: made by the sound engine once every piece has its shape."
        }
        "voice_line" => "Spoken right after the pictures are drawn.",
        _ if it.made.as_deref() == Some("mesh") => STANDING,
        _ => "Waits for its turn: every picture first, then the shapes one at a time.",
    }
}
const FIRST_LOOK: &str = "In the scene as a first look; Otto is getting it ready for games.";

/// One of a production's lanes (futures of different kinds, joined).
type Lane<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;

/// The making production, shared by its lanes.
type Shared = Arc<tokio::sync::Mutex<Production>>;

#[path = "pictures.rs"]
mod pictures;
#[path = "running.rs"]
mod running;

enum Outcome {
    /// Made, with an optional plain note for the item (a length that was clamped).
    Done(ItemResult, Option<String>),
    Skipped(String),
    Failed(String, Option<ItemResult>),
}

pub(super) fn plain(e: &Error) -> String {
    e.public_message()
}

impl MediaService {
    // -----------------------------------------------------------------------
    // Reads
    // -----------------------------------------------------------------------

    pub async fn get_production(&self, id: &str) -> Result<Production> {
        let mut p = self.store().require_production(id).await?;
        p.eta_secs = match p.status.as_str() {
            "planning" => Some(60),
            "ready" | "running" => {
                let sound = self.audio_command("sound").await.is_some();
                let music = self.audio_command("music").await.is_some();
                let first = self.pictures_first().await;
                let e = eta(&p, sound, music, first);
                p.pictures_eta_secs = (e.pictures > 0).then_some(e.pictures);
                p.scene_eta_secs = (e.scene > 0).then_some(e.scene);
                // What it would take with every reused piece made.
                let mut made = p.clone();
                made.plan
                    .items
                    .iter_mut()
                    .for_each(|i| i.details.reuse = None);
                let all = eta(&made, sound, music, first).all;
                p.eta_unreused_secs = (all > e.all).then_some(all);
                Some(e.all)
            }
            _ => None,
        };
        let ahead = self.queue_ahead(&p.id) as i64;
        p.queue_position = (p.status == "running" && ahead > 0).then_some(ahead);
        if p.project_id != p.id {
            let first = self.store().require_production(&p.project_id).await;
            p.project_name = first.ok().map(|f| f.plan.title);
        }
        if p.status == "ready" {
            p.project_pieces = self.project_offer(&p).await?.0;
        }
        p.scene_ready = p.scene_artifact_id.is_some()
            && p.plan.items.iter().all(|i| {
                !(i.enabled && matches!(i.kind.as_str(), "character" | "object"))
                    || i.result.is_some()
                    || !matches!(i.status.as_str(), "waiting" | "working")
            });
        // A piece with no picture of its own shows its model's thumbnail,
        // once one was rendered.
        let design = self.design_service();
        for it in p.plan.items.iter_mut().filter(|i| !i.has_preview) {
            let Some(aid) = it.result.as_ref().and_then(|r| r.artifact_id.as_deref()) else {
                continue;
            };
            if let Ok(a) = design.store().require_artifact(aid).await {
                it.has_preview =
                    a.thumb_blob.is_some() || self.imported_picture(&a, it).await.is_some();
                it.preview_rev = it.preview_rev.max(it.has_preview as i64);
            }
        }
        Ok(p.public())
    }

    /// The productions of `workspaces` (newest first, at most `limit`), each
    /// with its workspace's name and its place in the line.
    pub async fn list_productions(
        &self,
        workspaces: &[otto_core::domain::Workspace],
        limit: i64,
    ) -> Result<Vec<super::ProductionSummary>> {
        let ids: Vec<Id> = workspaces.iter().map(|w| w.id.clone()).collect();
        let mut list = self.store().list_productions(&ids, limit).await?;
        for p in &mut list {
            let ahead = self.queue_ahead(&p.id) as i64;
            p.queue_position = (p.status == "running" && ahead > 0).then_some(ahead);
            let ws = workspaces.iter().find(|w| w.id == p.workspace_id);
            p.workspace_name = ws.map(|w| w.name.clone());
        }
        let titles: HashMap<Id, String> = list
            .iter()
            .map(|p| (p.id.clone(), p.title.clone()))
            .collect();
        for p in list.iter_mut().filter(|p| p.project_id != p.id) {
            p.project_name = titles.get(&p.project_id).cloned();
        }
        Ok(list)
    }

    /// The shelf of everything made in Create in `workspaces` (the ones the
    /// caller can see): productions, and the 3D models and clips not inside
    /// one (its scene, or a result or previous result of one of its items),
    /// each newest first, at most `SHELF` of each.
    pub async fn creations(
        &self,
        workspaces: &[otto_core::domain::Workspace],
    ) -> Result<Creations> {
        const SHELF: usize = 60;
        let mut productions = self.list_productions(workspaces, 1_000).await?;
        let inside: HashSet<&Id> = productions.iter().flat_map(|p| &p.pieces).collect();
        let ids: Vec<Id> = workspaces.iter().map(|w| w.id.clone()).collect();
        let filter = otto_design::store::ArtifactFilter {
            workspaces: Some(ids.clone()),
            studio: Some("3d".into()),
            format: Some("glb".into()),
            limit: 200,
            ..Default::default()
        };
        let models = self
            .design_service()
            .store()
            .list_artifacts(&filter)
            .await?;
        let models = models.into_iter().filter(|a| !inside.contains(&a.id));
        let mut clips = Vec::new();
        for id in &ids {
            clips.extend(
                self.store()
                    .list_tts_clips(id, None, None, None, 200)
                    .await?,
            );
        }
        clips.retain(|c| !inside.contains(&c.id));
        clips.sort_by_key(|c| std::cmp::Reverse(c.created_at));
        clips.truncate(SHELF);
        let models = models.take(SHELF).collect();
        productions.truncate(SHELF);
        Ok(Creations {
            productions,
            models,
            clips,
        })
    }

    /// An item's picture (its concept picture, else its model's rendered
    /// thumbnail) and its type.
    pub async fn production_preview(
        &self,
        id: &str,
        item_id: &str,
    ) -> Result<(Vec<u8>, &'static str)> {
        let p = self.store().require_production(id).await?;
        let it = p
            .plan
            .items
            .iter()
            .find(|i| i.id == item_id)
            .ok_or_else(|| Error::NotFound(format!("item {item_id}")))?;
        let none = || Error::NotFound(format!("item {item_id} has no picture"));
        let bytes = match preview_path(self.data_dir(), &p.id, &it.id) {
            Some(path) if it.has_preview && path.is_file() => tokio::fs::read(&path)
                .await
                .map_err(|e| Error::Internal(format!("read picture: {e}")))?,
            _ => {
                let design = self.design_service();
                let aid = it.result.as_ref().and_then(|r| r.artifact_id.as_deref());
                let a = design
                    .store()
                    .require_artifact(aid.ok_or_else(none)?)
                    .await?;
                if let Some(blob) = &a.thumb_blob {
                    design.blobs().get(blob).await?
                } else {
                    let path = self.imported_picture(&a, it).await.ok_or_else(none)?;
                    tokio::fs::read(path).await.map_err(|_| none())?
                }
            }
        };
        let mime = match bytes.starts_with(&[0xff, 0xd8]) {
            true => "image/jpeg",
            false => "image/png",
        };
        Ok((bytes, mime))
    }

    // -----------------------------------------------------------------------
    // Planning (create, steer)
    // -----------------------------------------------------------------------

    pub async fn create_production(
        self: &Arc<Self>,
        req: CreateProductionReq,
        user_id: &str,
        planner: Arc<dyn ProductionPlanner>,
    ) -> Result<Production> {
        let description = req.description.trim().to_string();
        if description.is_empty() || description.chars().count() > MAX_DESCRIPTION_CHARS {
            return Err(Error::Invalid(format!(
                "description must be 1-{MAX_DESCRIPTION_CHARS} characters"
            )));
        }
        let use_for = req.use_for.as_deref().unwrap_or("pc").trim().to_string();
        if !USES.contains(&use_for.as_str()) {
            return Err(Error::Invalid(format!(
                "use must be one of: {}",
                USES.join(", ")
            )));
        }
        let pick = req.agent();
        let now = Utc::now();
        let id = new_id();
        // A new scene of a project takes the project's id; else it starts one.
        let project_id = match &req.project_id {
            Some(of) => self.store().require_production(of).await?.project_id,
            None => id.clone(),
        };
        let p = Production {
            id,
            workspace_id: req.workspace_id,
            project_id,
            project_name: None,
            project_pieces: Vec::new(),
            eta_unreused_secs: None,
            description,
            use_for,
            status: "planning".into(),
            message: None,
            eta_secs: None,
            pictures_eta_secs: None,
            scene_eta_secs: None,
            plan: Plan::default(),
            scene_artifact_id: None,
            revisions: Vec::new(),
            provider: pick.provider,
            model: pick.model,
            effort: pick.effort,
            queued_at: None,
            agent: None,
            queue_position: None,
            scene_ready: false,
            created_by: user_id.to_string(),
            created_at: now,
            updated_at: now,
        };
        self.store().save_production(&p).await?;
        let (pieces, place) = self.project_offer(&p).await?;
        let project = reuse::prompt(&pieces, place.as_ref());
        let prompt = project_plan_prompt(&p.description, &p.use_for, "plan.json", &project);
        let svc = Arc::clone(self);
        let id = p.id.clone();
        tokio::spawn(otto_core::auth::carry_request_credential(async move {
            let mut planned = svc.plan_turn(&id, prompt, planner.as_ref()).await;
            if let Ok((plan, _)) = &mut planned {
                reuse::resolve(plan, &pieces);
            }
            let _lock = svc.production_lock.lock().await;
            let Ok(mut p) = svc.store().require_production(&id).await else {
                return;
            };
            if p.status != "planning" {
                return; // cancelled meanwhile: the plan is dropped
            }
            match planned {
                Ok((plan, _)) => (p.plan, p.status, p.message) = (plan, "ready".into(), None),
                Err(m) => (p.status, p.message) = ("failed".into(), Some(m)),
            }
            let _ = svc.store().save_production(&p).await;
        }));
        self.get_production(&p.id).await
    }

    /// One planner turn, and one more with the problem when the first answer
    /// does not pass [`validate_plan`]. `Err` is the plain message.
    async fn plan_turn(
        &self,
        id: &str,
        prompt: String,
        planner: &dyn ProductionPlanner,
    ) -> std::result::Result<(Plan, Option<RevisionDraft>), String> {
        let failed = |e: Error| format!("Otto could not make a plan: {}", plain(&e));
        let first = planner.run(id, prompt.clone()).await.map_err(failed)?;
        match parse_plan_reply(&first) {
            Ok(v) => Ok(v),
            Err(problem) => {
                let second = planner
                    .run(id, retry_prompt(&prompt, &problem))
                    .await
                    .map_err(failed)?;
                parse_plan_reply(&second).map_err(|e| {
                    format!("The plan came back wrong twice ({e}). Try describing it again.")
                })
            }
        }
    }

    pub async fn steer_production(
        self: &Arc<Self>,
        id: &str,
        req: SteerReq,
        planner: Arc<dyn ProductionPlanner>,
        user_id: &str,
    ) -> Result<Production> {
        let comment = req.comment.trim().to_string();
        if comment.is_empty() || comment.chars().count() > 2_000 {
            return Err(Error::Invalid("comment must be 1-2000 characters".into()));
        }
        if req.pins.len() > 20 || req.pins.iter().any(|p| p.note.chars().count() > 300) {
            return Err(Error::Invalid(
                "at most 20 pins with notes of ≤ 300 characters".into(),
            ));
        }
        if self.store().require_production(id).await?.status == "running" {
            return self.steer_running(id, comment, req.pins, planner).await;
        }
        let before = {
            let _lock = self.production_lock.lock().await;
            let mut p = self.store().require_production(id).await?;
            if !matches!(p.status.as_str(), "ready" | "done" | "cancelled") {
                return Err(Error::Conflict(format!(
                    "a {} production cannot be steered",
                    p.status
                )));
            }
            let before = p.clone();
            (p.status, p.message) = ("planning".into(), Some("Changing the plan…".into()));
            self.store().save_production(&p).await?;
            before
        };
        let prompt = steer_prompt(&before.plan, &comment, &req.pins, "plan.json");
        let svc = Arc::clone(self);
        let (id, user) = (id.to_string(), user_id.to_string());
        let pid = id.clone();
        tokio::spawn(otto_core::auth::carry_request_credential(async move {
            let planned = svc.plan_turn(&id, prompt, planner.as_ref()).await;
            let _lock = svc.production_lock.lock().await;
            let Ok(mut p) = svc.store().require_production(&id).await else {
                return;
            };
            if p.status != "planning" {
                return;
            }
            match planned {
                Ok((mut plan, draft)) => {
                    let offered = svc.project_offer(&p).await.map(|o| o.0);
                    reuse::resolve(&mut plan, &offered.unwrap_or_default());
                    let rev = apply_revision(
                        &mut p,
                        &before.plan,
                        plan,
                        draft.unwrap_or_default(),
                        comment,
                    );
                    p.revisions.push(rev);
                    (p.status, p.message) = ("ready".into(), None);
                    // Nothing to make (only sizes or places changed): the scene now.
                    let made = p.plan.items.iter().any(|i| i.status == "done");
                    if made
                        && !p
                            .plan
                            .items
                            .iter()
                            .any(|i| i.enabled && i.status == "waiting")
                    {
                        svc.write_scene(&mut p, &user, true).await;
                        p.status = match before.status.as_str() {
                            "cancelled" => "cancelled",
                            _ => "done",
                        }
                        .into();
                    }
                }
                // Nothing changed: back to where it was, with the reason.
                Err(m) => (p.status, p.message) = (before.status.clone(), Some(m)),
            }
            let _ = svc.store().save_production(&p).await;
        }));
        self.get_production(&pid).await
    }

    // -----------------------------------------------------------------------
    // Editing (patch, revert)
    // -----------------------------------------------------------------------

    pub async fn patch_production(&self, id: &str, req: PatchProductionReq) -> Result<Production> {
        let _lock = self.production_lock.lock().await;
        let mut p = self.store().require_production(id).await?;
        if !matches!(p.status.as_str(), "ready" | "cancelled") {
            return Err(Error::Conflict(
                "a plan can be edited only while it is ready or stopped".into(),
            ));
        }
        let switched: Vec<String> = req
            .items
            .iter()
            .filter(|i| i.details.is_some())
            .map(|i| i.id.clone())
            .collect();
        for patch in req.items {
            let it = p
                .plan
                .items
                .iter_mut()
                .find(|i| i.id == patch.id)
                .ok_or_else(|| Error::NotFound(format!("item {}", patch.id)))?;
            if let Some(v) = patch.enabled {
                it.enabled = v;
            }
            if let Some(v) = patch.name {
                it.name = v;
            }
            if let Some(v) = patch.description {
                it.description = v;
            }
            if let Some(v) = patch.details {
                it.details = v;
            }
        }
        validate_plan(&p.plan).map_err(Error::Invalid)?;
        // A piece switched to one of the project's must be one of them.
        let (pieces, _) = self.project_offer(&p).await?;
        let dropped = reuse::resolve(&mut p.plan, &pieces);
        if let Some(id) = dropped.iter().find(|d| switched.contains(d)) {
            return Err(Error::Invalid(format!(
                "item {id}: that is not a made piece of this project"
            )));
        }
        self.store().save_production(&p).await?;
        drop(_lock);
        self.get_production(id).await
    }

    /// Put back what a steering round changed: its changed and removed items
    /// as they were (with their results), without the items it added.
    pub async fn revert_production(
        &self,
        id: &str,
        revision_id: &str,
        user_id: &str,
    ) -> Result<Production> {
        let _lock = self.production_lock.lock().await;
        let mut p = self.store().require_production(id).await?;
        if !matches!(p.status.as_str(), "ready" | "done" | "cancelled") {
            return Err(Error::Conflict(format!(
                "a {} production cannot be reverted",
                p.status
            )));
        }
        let rev = p
            .revisions
            .iter()
            .find(|r| r.id == revision_id)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("revision {revision_id}")))?;
        p.plan.items.retain(|i| !rev.added.contains(&i.id));
        if let Some(kept) = rev.setting_before {
            p.plan.setting = setting::restore(kept);
        }
        for old in rev.before {
            match p.plan.items.iter_mut().find(|i| i.id == old.id) {
                Some(it) => *it = old,
                None => p.plan.items.push(old),
            }
        }
        let enabled = || p.plan.items.iter().filter(|i| i.enabled);
        let waiting = enabled().any(|i| i.status == "waiting");
        let stopped = enabled().any(|i| i.message.as_deref() == Some(STOPPED));
        p.status = match (waiting, stopped) {
            (true, _) => "ready",
            (_, true) => "cancelled",
            _ => "done",
        }
        .into();
        self.write_scene(&mut p, user_id, true).await;
        self.store().save_production(&p).await?;
        drop(_lock);
        self.get_production(id).await
    }

    /// The scene written again from the pieces already made, with the
    /// current scene builder and poses: nothing is made, the plan is unchanged.
    pub async fn rebuild_scene(&self, id: &str, user_id: &str) -> Result<Production> {
        let _lock = self.production_lock.lock().await;
        let mut p = self.store().require_production(id).await?;
        if !matches!(p.status.as_str(), "ready" | "done" | "cancelled") {
            return Err(Error::Conflict(format!(
                "a {} production's scene cannot be rebuilt",
                p.status
            )));
        }
        // With the current engine too: every pose is made again (seconds each).
        for r in p.plan.items.iter_mut().filter_map(|i| i.result.as_mut()) {
            if let Some(pose) = r.pose.as_mut() {
                pose["spec"] = serde_json::Value::Null;
            }
        }
        self.write_scene(&mut p, user_id, true).await;
        self.store().save_production(&p).await?;
        drop(_lock);
        self.get_production(id).await
    }

    // -----------------------------------------------------------------------
    // Running (start, cancel)
    // -----------------------------------------------------------------------

    pub async fn start_production(self: &Arc<Self>, id: &str, user_id: &str) -> Result<Production> {
        let _lock = self.production_lock.lock().await;
        let mut p = self.store().require_production(id).await?;
        if !matches!(p.status.as_str(), "ready" | "done" | "cancelled") {
            return Err(Error::Conflict(format!(
                "a {} production cannot be started",
                p.status
            )));
        }
        for it in &mut p.plan.items {
            if !it.enabled {
                (it.status, it.message) = ("skipped".into(), Some("Turned off.".into()));
            } else if it.status != "done" || it.message == Some(stands_still(it)) {
                // A previously failed shape can have consumed two more drawings.
                // Read its saved reason before replacing the status text for Start.
                if it.status == "failed" {
                    let prefix = format!("The {} could not be made: ", it.name);
                    if let Some(seed) = it
                        .message
                        .as_deref()
                        .and_then(|m| m.strip_prefix(&prefix))
                        .and_then(|why| pictures::next_concept_seed(why, it.seed))
                    {
                        it.seed = Some(seed);
                    }
                }
                // A character whose skeleton or moves could not be added
                // gets them again from its model (nothing else is made).
                if it.status == "done" {
                    it.made = Some("mesh".into());
                }
                (it.status, it.progress_pct) = ("waiting".into(), 0);
                it.message = Some(waits_for(it).into());
                (it.step, it.waiting) = (None, None);
            }
        }
        (p.status, p.message) = ("running".into(), None);
        // Its place in the line (kept when a restart starts it again).
        p.queued_at.get_or_insert_with(Utc::now);
        self.store().save_production(&p).await?;
        // Its place in the line: Start never fails because another one makes.
        self.production_queue().push(p.id.clone());
        let guard = self.production_cancels.register(id);
        let svc = Arc::clone(self);
        let user = user_id.to_string();
        let run = p.clone();
        tokio::spawn(otto_core::auth::carry_request_credential(async move {
            svc.run_production(run, &user, guard).await;
        }));
        drop(_lock);
        self.get_production(id).await
    }

    /// Stop it: a running production's working items are cancelled (a 3D job
    /// with the job's own process-group cancel) and the rest are skipped; one
    /// waiting for its turn leaves the line.
    pub async fn cancel_production(&self, id: &str) -> Result<Production> {
        let _lock = self.production_lock.lock().await;
        let mut p = self.store().require_production(id).await?;
        match p.status.as_str() {
            "running" => {
                self.production_cancels.signal(id);
            }
            "planning" | "ready" => {
                skip_the_rest(&mut p);
                p.status = "cancelled".into();
                self.store().save_production(&p).await?;
            }
            _ => {}
        }
        drop(_lock);
        self.get_production(id).await
    }

    async fn run_production(self: Arc<Self>, p: Production, user_id: &str, guard: CancelGuard) {
        let id = p.id.clone();
        // Its turn: one production makes at a time, in Start order (a Stop frees it).
        while !guard.is_cancelled() && self.queue_ahead(&id) > 0 {
            tokio::time::sleep(JOB_POLL).await;
        }
        let s: Shared = Arc::new(tokio::sync::Mutex::new(p));
        self.runs().insert(id.clone(), s.clone());
        if !guard.is_cancelled() {
            self.reuse_first(&s, user_id).await;
            self.note_unknown_moves(&s).await;
            self.library_first(&s, user_id, &guard).await;
            self.draw_pictures(&s, &guard).await;
            self.speak_lines(&s, user_id, &guard).await;
            // One GPU job at a time (a picture, a spoken line, a shape, a
            // skeleton, a sound), at most one CPU job beside it (a game-ready
            // pass, moves): pictures, then the lines, then every shape, then
            // the skeletons and sounds once every piece has its shape.
            let after = async {
                self.wait_for_shapes(&s, &guard).await;
                tokio::join!(
                    self.make_skeletons(&s, user_id, &guard),
                    self.make_sounds(&s, user_id, &guard)
                );
            };
            tokio::join!(self.make_meshes(&s, user_id, &guard), after);
        }
        // Out of the live set while its state is taken (a steer that comes
        // later goes to the store).
        let mut p = {
            let live = s.lock().await;
            self.runs().remove(&id);
            live.clone()
        };
        if guard.is_cancelled() {
            skip_the_rest(&mut p);
            p.status = "cancelled".into();
        } else {
            let enabled = p.plan.items.iter().filter(|i| i.enabled);
            let failed = enabled.clone().filter(|i| i.status == "failed").count();
            p.status = "done".into();
            // A steer's word on what waits stays when nothing failed.
            let made = (failed > 0).then(|| {
                format!(
                    "Made {} of {}; {failed} could not be made.",
                    enabled.clone().filter(|i| i.status == "done").count(),
                    enabled.count()
                )
            });
            p.message = made.or(p.message.take());
        }
        self.write_scene(&mut p, user_id, true).await;
        p.queued_at = None;
        let _ = self.store().save_production(&p).await;
        self.production_queue().retain(|q| q != &id);
    }

    fn runs(&self) -> std::sync::MutexGuard<'_, HashMap<Id, Shared>> {
        self.production_runs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub(crate) fn production_queue(&self) -> std::sync::MutexGuard<'_, Vec<Id>> {
        self.production_queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// How many running productions are ahead of this one (0: it makes).
    fn queue_ahead(&self, id: &str) -> usize {
        let queue = self.production_queue();
        queue.iter().position(|q| q == id).unwrap_or(0)
    }

    /// Change the making production and save it (its lanes share it).
    async fn edit<R>(&self, s: &Shared, f: impl FnOnce(&mut Production) -> R) -> R {
        let mut p = s.lock().await;
        let r = f(&mut p);
        let _ = self.store().save_production(&p).await;
        r
    }

    async fn settle(&self, s: &Shared, ix: usize, outcome: Outcome) {
        self.edit(s, |p| settle(&mut p.plan.items[ix], outcome))
            .await
    }

    /// The item starts working, on `step`.
    async fn begin(&self, s: &Shared, ix: usize, step: Option<Step>) {
        self.edit(s, |p| {
            let it = &mut p.plan.items[ix];
            (it.status, it.step, it.waiting) = ("working".into(), step, None);
            (it.message, it.working_since) = (None, Some(Utc::now()));
        })
        .await
    }

    /// The enabled items still to make that `pick` takes, in plan order.
    async fn todo(&self, s: &Shared, pick: impl Fn(&Item) -> bool) -> Vec<usize> {
        let p = s.lock().await;
        (0..p.plan.items.len())
            .filter(|&ix| {
                let it = &p.plan.items[ix];
                it.enabled && it.status == "waiting" && pick(it)
            })
            .collect()
    }

    /// The scene with what is made so far (its notes wait for the last write).
    async fn write_scene_now(&self, s: &Shared, user_id: &str) {
        let mut p = s.lock().await;
        self.write_scene(&mut p, user_id, false).await;
        let _ = self.store().save_production(&p).await;
    }

    /// A move the engine cannot make is left out with a plain note.
    async fn note_unknown_moves(&self, s: &Shared) {
        self.edit(s, |p| {
            let notes: Vec<String> = p
                .plan
                .items
                .iter()
                .filter(|i| i.enabled && i.status == "waiting" && i.kind == "character")
                .filter_map(|i| {
                    let (_, unknown) = moves_of(i);
                    (!unknown.is_empty()).then(|| {
                        format!(
                            "{} cannot {} yet, so that was left out.",
                            the(&i.name, "The"),
                            unknown.join(" or ")
                        )
                    })
                })
                .collect();
            notes.into_iter().for_each(|n| push_note(&mut p.plan, n));
        })
        .await
    }

    /// Library first, before anything is generated (seconds a piece): a prop
    /// Otto Forge's CC0 library has, or a dressed character it has (only
    /// Ranger and Peasant outfits; every character ask scores the same there,
    /// so the words decide). One it lacks waits to be made.
    async fn library_first(&self, s: &Shared, user_id: &str, guard: &CancelGuard) {
        let forge = self
            .store()
            .require_3d_provider(BUILTIN_OTTO_FORGE_ID)
            .await
            .ok();
        let Some((bin, forge)) = forge.and_then(|f| Some((f.effective_cli_binary()?, f))) else {
            return;
        };
        let p = s.lock().await.clone();
        let mut took = false;
        for (ix, it) in p.plan.items.iter().enumerate() {
            let Some(kind) = library_kind(it) else {
                continue;
            };
            if guard.is_cancelled() {
                break;
            }
            self.begin(s, ix, step(1, 1, LIBRARY)).await;
            let (moves, _) = moves_of(it);
            let moves: Vec<&str> = moves.iter().map(String::as_str).collect();
            let remake = remake_target(it);
            // The name's head word first ("Chest Key" is a key: the word
            // "chest" picked a sci-fi chest), then the whole name.
            let head = it.name.split_whitespace().last().unwrap_or(&it.name);
            let mut asks = vec![format!("{head}. {}", it.description)];
            asks.push(format!("{}. {}", it.name, it.description));
            asks.dedup();
            let mut outcome = None;
            for ask in &asks {
                let (target, ws) = (remake.as_deref(), &p.workspace_id);
                let got = self
                    .take_library_first(
                        &forge, &bin, it, ask, kind, &moves, target, ws, user_id, guard,
                    )
                    .await;
                match got {
                    Ok(r) => outcome = Some(Outcome::Done(r, Some(FROM_LIBRARY.into()))),
                    Err(_) if guard.is_cancelled() => {
                        outcome = Some(Outcome::Skipped(STOPPED.into()))
                    }
                    Err(_) => continue,
                }
                break;
            }
            match outcome {
                Some(o) => {
                    took |= matches!(o, Outcome::Done(..));
                    self.settle(s, ix, o).await;
                }
                None => {
                    self.edit(s, |p| {
                        let it = &mut p.plan.items[ix];
                        (it.status, it.step, it.working_since) = ("waiting".into(), None, None);
                    })
                    .await
                }
            }
        }
        if took {
            self.write_scene_now(s, user_id).await;
        }
    }

    /// Every model still to make, in plan order, one shape on the GPU at a
    /// time, built from its picture when it has one. An object's shape goes
    /// into the scene at once as a first look (Forge's `--skip-gameready`)
    /// and its game-ready pass (CPU, Blender) runs beside the next shape; a
    /// character's run gets ready inside it, the next shape starting once it
    /// reaches that CPU step. A character then stands in the scene until
    /// every piece has its shape, then gets its skeleton.
    /// The pieces this scene takes from its project, before anything else
    /// (no GPU, no time): each is in the scene at once as the model it is
    /// there, with its skeleton and moves. One whose model was deleted
    /// meanwhile is made instead, and says so.
    async fn reuse_first(&self, s: &Shared, user_id: &str) {
        let p = s.lock().await.clone();
        let design = self.design_service();
        let mut took = false;
        for (ix, it) in p.plan.items.iter().enumerate() {
            let asked = it.details.reuse.as_ref();
            let Some(r) = asked.filter(|_| it.enabled && it.status == "waiting") else {
                continue;
            };
            let from = r.from.as_deref().unwrap_or("an earlier scene");
            let there = match design.store().require_artifact(&r.artifact_id).await {
                Ok(a) => match &r.version_id {
                    Some(v) => design.resolve_version(&a, v).await.is_ok(),
                    None => true,
                },
                Err(_) => false,
            };
            if there {
                let result = ItemResult {
                    artifact_id: Some(r.artifact_id.clone()),
                    version_id: r.version_id.clone(),
                    rig: r.rig.clone(),
                    ..Default::default()
                };
                let said = Some(format!("From “{from}”."));
                self.settle(s, ix, Outcome::Done(result, said)).await;
                took = true;
                continue;
            }
            let note = format!(
                "{} from “{from}” was deleted, so Otto makes it again.",
                the(&it.name, "The")
            );
            self.edit(s, |p| {
                let it = &mut p.plan.items[ix];
                (it.details.reuse, it.message) = (None, Some(note.clone()));
                push_note(&mut p.plan, note);
            })
            .await;
        }
        if took {
            self.write_scene_now(s, user_id).await;
        }
    }

    async fn make_meshes(self: &Arc<Self>, s: &Shared, user_id: &str, guard: &CancelGuard) {
        let shapes = self
            .todo(s, |i| {
                matches!(i.made.as_deref(), None | Some("picture"))
                    && matches!(i.kind.as_str(), "character" | "object")
            })
            .await;
        let drafts = self.todo(s, |i| i.made.as_deref() == Some("draft")).await;
        let each = shapes
            .into_iter()
            .map(|ix| Box::pin(self.make_mesh(s, ix, user_id, guard)) as Lane<'_>)
            .chain(
                drafts
                    .into_iter()
                    .map(|ix| Box::pin(self.ready_draft(s, ix, user_id, guard)) as Lane<'_>),
            );
        join_all(each.collect()).await;
    }

    async fn make_mesh(
        self: &Arc<Self>,
        s: &Shared,
        ix: usize,
        user_id: &str,
        guard: &CancelGuard,
    ) {
        let turn = self.production_gpu.acquire().await.ok();
        if guard.is_cancelled() {
            return;
        }
        let p = s.lock().await.clone();
        let it = &p.plan.items[ix];
        let character = it.kind == "character";
        let of = steps_of(it, !moves_of(it).0.is_empty());
        // Its picture, drawn first: the shape is built from exactly that.
        let picture = match (
            it.made.as_deref(),
            preview_path(self.data_dir(), &p.id, &it.id),
        ) {
            (Some("picture"), Some(path)) => tokio::fs::read(path).await.ok(),
            _ => None,
        };
        let at = match picture {
            Some(_) => (2, of, SHAPING),
            None => (1, of, DRAWING),
        };
        self.begin(s, ix, step(at.0, of, at.2)).await;
        // Its real size goes to generate when the installed Forge takes it;
        // an object is built as a first look when it can be got ready later.
        let bin = self.forge_bin().await.unwrap_or_default();
        let mut params = serde_json::Map::new();
        if forge_takes(&bin, "--height-m").await {
            let axis = if character {
                "height_m"
            } else {
                "longest_side_m"
            };
            params.insert(axis.into(), json!(size_m(&p.plan, it)));
        }
        // A first look first, got ready later: an object on any Forge that
        // skips the game-ready pass, a character on one that finishes its
        // draft exactly (its finishing inputs kept beside the draft).
        let draft = forge_takes(&bin, "--skip-gameready").await
            && (!character || forge_serves_concepts(&bin).await);
        if draft {
            params.insert("skip_gameready".into(), json!(true));
        }
        if draft && character {
            params.insert("kind".into(), json!("character"));
            let keep = finish_dir(self.data_dir(), &p.id, &it.id);
            params.insert("keep_dir".into(), json!(keep));
        }
        if let Some(body) = picture_body(it) {
            if forge_takes(&bin, "--body").await {
                params.insert("body".into(), json!(body));
            }
        }
        let params = (!params.is_empty()).then(|| params.into());
        let mut req = stage_req(&p, it, "generate", None, remake_target(it), params);
        req.seed = it.seed;
        if let Some(png) = &picture {
            req.image_b64 = Some(base64::engine::general_purpose::STANDARD.encode(png));
            req.image_mime = Some("image/png".into());
        }
        let span = if character || draft {
            (0, 55)
        } else {
            (0, 100)
        };
        let made = self
            .run_stage(s, ix, req, user_id, at, span, turn, guard)
            .await;
        let outcome = match made {
            Ok(r) if character || draft => {
                self.edit(s, |p| {
                    let it = &mut p.plan.items[ix];
                    (it.status, it.progress_pct, it.step) = ("waiting".into(), 55, None);
                    let (made, said) = match draft {
                        true => ("draft", FIRST_LOOK),
                        false => ("mesh", STANDING),
                    };
                    (it.result, it.made) = (Some(r), Some(made.into()));
                    it.message = Some(said.into());
                })
                .await;
                None
            }
            Ok(r) => Some(Outcome::Done(r, None)),
            Err(None) => Some(Outcome::Skipped(STOPPED.into())),
            Err(Some(why)) => Some(Outcome::Failed(
                format!("The {} could not be made: {why}", it.name),
                None,
            )),
        };
        if let Some(o) = outcome {
            self.settle(s, ix, o).await;
        }
        self.write_scene_now(s, user_id).await;
        if draft && !guard.is_cancelled() {
            self.ready_draft(s, ix, user_id, guard).await;
        }
    }

    /// A first look gets ready for games (Forge's `gameready`: fewer
    /// triangles, LODs, collision; a character's head, hands and face from
    /// its kept finishing inputs, with the same triangle budget) on the CPU,
    /// beside the next shape, as a new version of the same model. A
    /// character then stands for its skeleton. If that fails the first look
    /// stays, with a plain note.
    async fn ready_draft(
        self: &Arc<Self>,
        s: &Shared,
        ix: usize,
        user_id: &str,
        guard: &CancelGuard,
    ) {
        let cpu = self.production_cpu.acquire().await.ok();
        let p = s.lock().await.clone();
        let it = &p.plan.items[ix];
        let Some(draft) = it.result.clone().filter(|_| !guard.is_cancelled()) else {
            return;
        };
        let character = it.kind == "character";
        let of = steps_of(it, !moves_of(it).0.is_empty());
        self.begin(s, ix, step(3, of, READYING)).await;
        let bin = self.forge_bin().await.unwrap_or_default();
        // A character's draft knows its size; its kept inputs go back beside it.
        let keep = finish_dir(self.data_dir(), &p.id, &it.id);
        let params = match character {
            true => Some(json!({ "keep_dir": keep })),
            false => forge_takes(&bin, "--height-m")
                .await
                .then(|| json!({ "longest_side_m": size_m(&p.plan, it) })),
        };
        let target = draft.artifact_id.clone();
        let req = stage_req(&p, it, "gameready", Some(&draft), target, params);
        let at = (3, of, READYING);
        let readied = self
            .run_stage(s, ix, req, user_id, at, (55, 100), None, guard)
            .await;
        drop(cpu);
        if let (Ok(_), true, Some(keep)) = (&readied, character, &keep) {
            let _ = tokio::fs::remove_dir_all(keep).await;
        }
        let outcome = match readied {
            Ok(r) if character => {
                self.edit(s, |p| {
                    let it = &mut p.plan.items[ix];
                    (it.status, it.progress_pct, it.step) = ("waiting".into(), 70, None);
                    (it.result, it.made) = (Some(r), Some("mesh".into()));
                    it.message = Some(STANDING.into());
                })
                .await;
                self.write_scene_now(s, user_id).await;
                return;
            }
            Ok(r) => Outcome::Done(r, None),
            Err(None) => Outcome::Skipped(STOPPED.into()),
            Err(Some(_)) => {
                let note = format!(
                    "Otto could not get {} ready for games this time; its first look stands in the scene.",
                    the(&it.name, "the")
                );
                Outcome::Done(draft, Some(note))
            }
        };
        self.edit(s, |p| {
            if let Outcome::Done(_, Some(note)) = &outcome {
                push_note(&mut p.plan, note.clone());
            }
            settle(&mut p.plan.items[ix], outcome);
        })
        .await;
        self.write_scene_now(s, user_id).await;
    }

    /// Wait until every 3D piece has its shape and every character is ready
    /// for its skeleton, or never will be (stopped too).
    async fn wait_for_shapes(&self, s: &Shared, guard: &CancelGuard) {
        while !guard.is_cancelled() {
            let shaping = s.lock().await.plan.items.iter().any(|i| {
                i.enabled
                    && matches!(i.kind.as_str(), "character" | "object")
                    && matches!(i.status.as_str(), "waiting" | "working")
                    && (matches!(i.made.as_deref(), None | Some("picture"))
                        || (i.kind == "character" && i.made.as_deref() == Some("draft")))
            });
            if !shaping {
                return;
            }
            tokio::time::sleep(JOB_POLL).await;
        }
    }

    /// Once every model is in the scene: each standing character gets its
    /// skeleton (GPU, one at a time) and then its moves (CPU, beside the next
    /// skeleton), always from the model already made. One whose skeleton or
    /// moves cannot be added stays as it stands, done, with a plain note
    /// (the engine's words stay on its job): the next Start tries them again.
    async fn make_skeletons(self: &Arc<Self>, s: &Shared, user_id: &str, guard: &CancelGuard) {
        let todo = self
            .todo(s, |i| {
                i.kind == "character" && i.made.as_deref() == Some("mesh")
            })
            .await;
        let each = todo
            .into_iter()
            .map(|ix| self.make_skeleton(s, ix, user_id, guard));
        join_all(each.collect()).await;
    }

    async fn make_skeleton(
        self: &Arc<Self>,
        s: &Shared,
        ix: usize,
        user_id: &str,
        guard: &CancelGuard,
    ) {
        let turn = self.production_gpu.acquire().await.ok();
        let p = s.lock().await.clone();
        let it = &p.plan.items[ix];
        let Some(mesh) = it.result.clone().filter(|_| !guard.is_cancelled()) else {
            return;
        };
        let (moves, _) = moves_of(it);
        let of = steps_of(it, !moves.is_empty());
        self.begin(s, ix, step(4, of, SKELETON)).await;
        let span = if moves.is_empty() { (55, 99) } else { (55, 93) };
        let mut req = stage_req(&p, it, "rig", Some(&mesh), None, None);
        if it.details.body.as_deref() == Some("fish") {
            let bin = self.forge_bin().await.unwrap_or_default();
            if forge_help(&bin, &["rig", "--help"])
                .await
                .contains("--swimming-fish")
            {
                req.params = Some(json!({ "swimming_fish": true }));
            }
        }
        let at = (4, of, SKELETON);
        let rigged = self
            .run_stage(s, ix, req, user_id, at, span, turn, guard)
            .await;
        let standing = |r: ItemResult| Outcome::Done(r, Some(stands_still(it)));
        let outcome = match rigged {
            Ok(r) if moves.is_empty() => Outcome::Done(r, None),
            Ok(r) => {
                let _cpu = self.production_cpu.acquire().await.ok();
                self.begin(s, ix, step(5, of, MOVES)).await;
                let clips = json!({ "clips": moves.join(",") });
                let req = stage_req(&p, it, "animate", Some(&r), None, Some(clips));
                let at = (5, of, MOVES);
                let rig = r.rig.clone();
                match self
                    .run_stage(s, ix, req, user_id, at, (93, 99), None, guard)
                    .await
                {
                    Ok(r) => Outcome::Done(ItemResult { rig, ..r }, None),
                    Err(None) => Outcome::Skipped(STOPPED.into()),
                    Err(Some(_)) => standing(r),
                }
            }
            Err(None) => Outcome::Skipped(STOPPED.into()),
            Err(Some(_)) => standing(mesh),
        };
        self.edit(s, |p| {
            if let Outcome::Done(_, Some(note)) = &outcome {
                push_note(&mut p.plan, note.clone());
            }
            settle(&mut p.plan.items[ix], outcome);
        })
        .await;
        self.write_scene_now(s, user_id).await;
    }

    /// Whether the installed Forge draws pictures on their own (every
    /// picture first, then the shapes from them).
    pub(super) async fn pictures_first(&self) -> bool {
        let bin = self.forge_bin().await.unwrap_or_default();
        forge_takes(&bin, "--concept-only").await
    }

    pub(super) async fn forge_bin(&self) -> Option<String> {
        let forge = self
            .store()
            .require_3d_provider(BUILTIN_OTTO_FORGE_ID)
            .await;
        forge.ok()?.effective_cli_binary()
    }

    /// One 3D job of an item, followed to its end: its step, progress and
    /// engine wait on the item, its concept picture copied as soon as it is
    /// drawn, and `turn` (the GPU turn) handed on when it reaches its CPU
    /// step. `Err(None)`: stopped; `Err(Some(why))`: it failed.
    #[allow(clippy::too_many_arguments)]
    async fn run_stage(
        self: &Arc<Self>,
        s: &Shared,
        ix: usize,
        req: StartMedia3dJobReq,
        user_id: &str,
        (n, of, label): (u32, u32, &'static str),
        (lo, hi): (i64, i64),
        mut turn: Option<tokio::sync::SemaphorePermit<'_>>,
        guard: &CancelGuard,
    ) -> std::result::Result<ItemResult, Option<String>> {
        let generate = req.stage.as_deref() == Some("generate");
        let req_has_image = req.image_b64.is_some();
        let job = self
            .start_3d_job(req, user_id, None)
            .await
            .map_err(|e| Some(plain(&e)))?;
        let (pid, item) = {
            let p = s.lock().await;
            (p.id.clone(), p.plan.items[ix].id.clone())
        };
        let (bin, marker) = (self.forge_bin().await, format!("/out/work/{}-", job.id));
        let (mut drawn, mut held, mut polls, mut at) = (!generate || req_has_image, false, 0u32, n);
        let (mut cpu, mut part) = (None, None::<f64>);
        let job = loop {
            tokio::time::sleep(JOB_POLL).await;
            polls += 1;
            if guard.is_cancelled() {
                let _ = self.cancel_3d_job(&job.id).await;
            }
            let Ok(j) = self.store().require_3d_job(&job.id).await else {
                return Err(Some("Its 3D job is gone.".into()));
            };
            let key = self
                .engine_steps()
                .get(&job.id)
                .cloned()
                .unwrap_or_default();
            // A model's step never goes back (the key is gone once its run ends).
            if generate {
                at = at.max(generate_step(&key).0);
            }
            let (n, label) = match generate {
                true => (at, [DRAWING, SHAPING, READYING][at as usize - 1]),
                false => (n, label),
            };
            // Its CPU step: the next shape's GPU turn, once the CPU turn is its.
            if n == 3 && turn.is_some() {
                cpu = cpu.or_else(|| self.production_cpu.try_acquire().ok());
                if cpu.is_some() {
                    turn.take();
                }
            }
            // How far into the 3D shape (Pixal3D's own steps in Forge's log).
            if generate && key == "generate" {
                if let Some(log) = match &bin {
                    Some(bin) => work_file(Path::new(bin), &job.id, "generate.log").await,
                    None => None,
                } {
                    let text = tokio::fs::read_to_string(log).await.unwrap_or_default();
                    part = shape_fraction(&text).map(|f| f.max(part.unwrap_or(0.0)));
                }
            }
            // Queued behind other 3D jobs, or its engine waits for a heavy slot.
            if j.status == "queued" || polls % 4 == 0 {
                held = j.status == "queued" || waits_for_heavy_slot(&marker).await;
            }
            let picture = match (drawn, key.as_str(), &bin) {
                (false, k, Some(bin)) if !k.is_empty() && k != "concept" => {
                    drawn = true;
                    let from = work_file(Path::new(bin), &job.id, "concept.png").await;
                    let to = preview_path(self.data_dir(), &pid, &item);
                    match (from, to) {
                        (Some(from), Some(to)) => copy_picture(&from, &to).await,
                        _ => false,
                    }
                }
                _ => false,
            };
            // The shape's part moves the bar between Forge's 20 % and 80 %.
            let engine = j
                .progress_pct
                .max(part.map_or(0, |f| 20 + (60.0 * f) as i64));
            let pct = (lo + engine.clamp(0, 100) * (hi - lo) / 100).min(99);
            // The engine's "wrote /tmp/…/<job>.glb" is no news for a person.
            let said = match j.stage.starts_with("wrote ") {
                true => "Saving it".to_string(),
                false => j.stage.clone(),
            };
            let mut st = step(n, of, label);
            if let (Some(st), true) = (st.as_mut(), n == 2) {
                st.fraction = part.map(|f| (f * 100.0).round() / 100.0);
            }
            let waits = held.then(|| "engine".to_string());
            let changed = {
                let p = s.lock().await;
                let it = &p.plan.items[ix];
                picture
                    || it.progress_pct != pct
                    || it.message.as_deref() != Some(&said)
                    || it.step != st
                    || it.waiting != waits
            };
            if changed {
                self.edit(s, |p| {
                    let it = &mut p.plan.items[ix];
                    (it.progress_pct, it.message, it.step, it.waiting) =
                        (pct, Some(said), st, waits);
                    if picture {
                        (it.has_preview, it.preview_rev) = (true, it.preview_rev + 1);
                    }
                })
                .await;
            }
            if matches!(j.status.as_str(), "succeeded" | "failed" | "cancelled") {
                break j;
            }
        };
        match job.status.as_str() {
            "succeeded" => Ok(ItemResult {
                artifact_id: job.artifact_id,
                version_id: job.version_id,
                clip_id: None,
                // What the engine says its skeleton is (any body plan).
                rig: job.manifest.as_ref().and_then(|m| {
                    let says = |k: &str| m.get(k).filter(|v| !v.is_null()).cloned();
                    let mut picked: serde_json::Map<String, Value> = ["body_plan", "legs"]
                        .into_iter()
                        .filter_map(|k| Some((k.to_string(), says(k)?)))
                        .collect();
                    if let Some(front) = m
                        .pointer("/body/forward")
                        .and_then(Value::as_array)
                        .filter(|v| v.len() == 3 && v.iter().all(Value::is_number))
                    {
                        picked.insert("forward".into(), Value::Array(front.clone()));
                    }
                    (!picked.is_empty()).then(|| picked.into())
                }),
                pose: None,
            }),
            "cancelled" => Err(None),
            _ => {
                let why = job.error.unwrap_or_else(|| "no reason given".into());
                Err(Some(why.trim_start_matches("upstream: ").to_string()))
            }
        }
    }

    /// Spoken lines (TTS, seconds each), right after the pictures and before
    /// any shape, so the conversation is heard minutes in: the voice does
    /// not need the speaker's model (its clip joins the model once made).
    async fn speak_lines(&self, s: &Shared, user_id: &str, guard: &CancelGuard) {
        let todo = self
            .todo(s, |i| {
                i.kind == "voice_line" && i.details.nonverbal != Some(true)
            })
            .await;
        for ix in todo {
            if guard.is_cancelled() {
                break;
            }
            // Otto Forge's voice model runs on the GPU: the production's turn
            // (the line says it waits for the engine meanwhile).
            let gpu = match self.forge_voice_cmd().await {
                Some(_) => match self.production_gpu.try_acquire() {
                    Ok(turn) => Some(turn),
                    Err(_) => {
                        self.edit(s, |p| p.plan.items[ix].waiting = Some("engine".into()))
                            .await;
                        self.production_gpu.acquire().await.ok()
                    }
                },
                None => None,
            };
            self.begin(s, ix, step(1, 1, "Speaking the line")).await;
            let p = s.lock().await.clone();
            let outcome = self.make_voice(&p, ix, user_id, guard).await;
            drop(gpu);
            self.settle(s, ix, outcome).await;
        }
    }

    /// Sounds, music and nonverbal lines through the sound engine, one at a
    /// time in plan order, each taking the GPU turn (once every piece has
    /// its shape, between the skeletons).
    async fn make_sounds(self: &Arc<Self>, s: &Shared, user_id: &str, guard: &CancelGuard) {
        let todo = self
            .todo(s, |i| {
                matches!(i.kind.as_str(), "sound" | "music")
                    || (i.kind == "voice_line" && i.details.nonverbal == Some(true))
            })
            .await;
        for ix in todo {
            if !self.wait_for_speaker(s, ix, guard).await {
                break;
            }
            let (kind, label) = {
                let p = s.lock().await;
                let kind = p.plan.items[ix].kind.clone();
                let label = if kind == "music" {
                    "Making the music"
                } else {
                    "Making the sound"
                };
                (kind, label)
            };
            let _gpu = self.production_gpu.acquire().await.ok();
            if guard.is_cancelled() {
                break;
            }
            self.begin(s, ix, step(1, 1, label)).await;
            let outcome = match kind.as_str() {
                "voice_line" => self.make_nonverbal(s, ix, user_id, guard).await,
                _ => self.make_audio(s, ix, user_id, guard).await,
            };
            self.settle(s, ix, outcome).await;
        }
    }

    /// Wait until a line's speaker has its model, or never will; `false`: stopped.
    async fn wait_for_speaker(&self, s: &Shared, ix: usize, guard: &CancelGuard) -> bool {
        loop {
            if guard.is_cancelled() {
                return false;
            }
            let ready = {
                let p = s.lock().await;
                let speaker = p.plan.items[ix].details.speaker.as_deref();
                let c = speaker.and_then(|id| p.plan.items.iter().find(|i| i.id == id));
                c.is_none_or(|c| {
                    c.result.is_some()
                        || !(c.enabled && matches!(c.status.as_str(), "waiting" | "working"))
                })
            };
            if ready {
                return true;
            }
            tokio::time::sleep(JOB_POLL).await;
        }
    }

    /// One piece from Otto Forge's library (`forge library`, its docs/library.md
    /// "Smart mode"), saved as the item's model (a new version of `remake`)
    /// with its version marked `source: "library"`, so the scene keeps its
    /// real size. `Err` when the library has none ("no library match") or
    /// the call fails.
    #[allow(clippy::too_many_arguments)]
    async fn take_library_first(
        &self,
        forge: &Media3dProviderRecord,
        bin: &str,
        it: &Item,
        ask: &str,
        kind: &str,
        moves: &[&str],
        remake: Option<&str>,
        workspace_id: &str,
        user_id: &str,
        guard: &CancelGuard,
    ) -> Result<ItemResult> {
        let tmp =
            tempfile::tempdir().map_err(|e| Error::Internal(format!("library tempdir: {e}")))?;
        let out = tmp.path().join("library.glb");
        let mut args = vec![
            "library".to_string(),
            "--prompt".into(),
            ask.into(),
            "--kind".into(),
            kind.into(),
            "--output".into(),
            out.to_string_lossy().into(),
        ];
        if kind == "character" && !moves.is_empty() {
            args.extend(["--clips".into(), moves.join(",")]);
        }
        let workdir = forge
            .cli_workdir
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(std::path::Path::new);
        let timeout = Duration::from_secs(forge.timeout_secs.max(15) as u64);
        run_cli_argv(bin.as_ref(), &args, workdir, timeout, guard.flag(), None).await?;
        let bytes = tokio::fs::read(&out)
            .await
            .map_err(|e| Error::Upstream(format!("the library wrote no model: {e}")))?;
        validate_3d_output("glb", &bytes, forge.effective_max_output_bytes())?;
        let manifest = read_optional_manifest(&out).await.unwrap_or_default();
        let picked: serde_json::Map<String, serde_json::Value> =
            ["id", "pack", "licence", "plain_name", "score", "clips"]
                .iter()
                .filter_map(|k| Some((k.to_string(), manifest.get(*k)?.clone())))
                .collect();
        // "Round wooden table" is not matched by "Table Large": a piece whose
        // name says its shape is made instead when the library's does not say it.
        let plain = manifest
            .get("plain_name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if let Some(why) = no_match(&it.name, plain) {
            return Err(Error::Upstream(format!("no library match: {why}")));
        }
        let provenance = json!({ "source": "library", "library": picked });
        let design = self.design_service();
        let message = "From the Otto Forge library".to_string();
        let saved = match remake {
            Some(aid) => {
                let a = design.store().require_artifact(aid).await?;
                let opts = SaveOpts {
                    base: None,
                    kind: "named".into(),
                    author: Author::user(user_id),
                    message,
                    provenance,
                    force: false,
                    validate: true,
                    change: "content",
                };
                design.commit_bytes(&a, bytes, opts).await?
            }
            None => {
                design
                    .create_artifact(CreateInput {
                        workspace_id: workspace_id.to_string(),
                        project_id: None,
                        studio: Some("3d".into()),
                        format: "glb".into(),
                        title: it.name.clone(),
                        tags: vec!["smart-mode".into(), "library".into()],
                        meta: json!({ "source": "library" }),
                        content: Some(bytes),
                        story_id: None,
                        derived_from: None,
                        created_by: user_id.to_string(),
                        author: Author::user(user_id),
                        message: Some(message),
                        source: None,
                        created_at: None,
                        version_kind: Some("named".into()),
                        validate: true,
                        provenance: Some(provenance),
                    })
                    .await?
            }
        };
        if let Some(path) =
            pictures::library_picture(Path::new(bin), &saved.version.provenance).await
        {
            if let Ok(png) = tokio::fs::read(path).await {
                let _ = design.set_thumbnail(&saved.artifact, &png).await;
            }
        }
        Ok(ItemResult {
            artifact_id: Some(saved.artifact.id),
            version_id: Some(saved.version.id),
            ..Default::default()
        })
    }

    /// A voice line through TTS: a Kokoro (Forge voice) provider when one is
    /// on, else the built-in macOS voices; the voice is picked from the plain
    /// description (the line's, else its speaker's).
    async fn make_voice(
        &self,
        p: &Production,
        ix: usize,
        user_id: &str,
        guard: &CancelGuard,
    ) -> Outcome {
        let it = &p.plan.items[ix];
        let speaker = it
            .details
            .speaker
            .as_deref()
            .and_then(|s| p.plan.items.iter().find(|i| i.id == s));
        let providers = self.list_tts_providers().await.unwrap_or_default();
        // Otto Forge's designed voices first; a Kokoro server the person
        // turned on speaks when that engine is missing or fails a line; else
        // the built-in voices, said plainly.
        let chosen = providers.iter().find(|t| t.enabled && t.kind == "kokoro");
        if let Some(cmd) = self.forge_voice_cmd().await {
            let out = self.forge_voice(p, ix, &cmd, user_id, guard).await;
            if chosen.is_none() || guard.is_cancelled() || !matches!(out, Outcome::Failed(..)) {
                return out;
            }
        }
        let note = chosen.is_none().then(|| NO_VOICE_ENGINE.to_string());
        let provider = chosen
            .or_else(|| providers.iter().find(|t| t.id == BUILTIN_MACOS_SAY_ID))
            .cloned();
        let Some(provider) = provider else {
            return Outcome::Failed("No voice engine is on.".into(), None);
        };
        let voices = match self.list_tts_voices(&provider.id).await {
            Ok(v) => v,
            Err(e) => return Outcome::Failed(plain(&e), None),
        };
        // A character always speaks in its own voice; a line without a
        // speaker gets one for its own description.
        let voices = speaking_voices(&voices);
        let default = provider.default_voice.as_deref();
        // A reused character speaks in the voice of its earlier lines (this engine's).
        let mut kept = HashMap::new();
        for c in p.plan.items.iter().filter(|c| c.kind == "character") {
            let Some(line) = c.details.reuse.as_ref().and_then(|r| r.line.as_deref()) else {
                continue;
            };
            let Ok(rec) = self.store().require_tts_clip(line).await else {
                continue;
            };
            if rec.clip.provider_id == provider.id
                && voices.iter().any(|v| v.id == rec.clip.voice_id)
            {
                kept.insert(c.id.clone(), rec.clip.voice_id);
            }
        }
        let voice = match speaker {
            Some(s) => cast_voices(&p.plan, &voices, default, kept).remove(&s.id),
            None => {
                let wanted = it.details.voice.as_deref().unwrap_or_default();
                pick_voice(&voices, wanted, default, &HashSet::new())
            }
        };
        let Some(voice) = voice else {
            return Outcome::Failed("The voice engine has no voices.".into(), None);
        };
        let text = it
            .details
            .text
            .clone()
            .unwrap_or_else(|| it.description.clone());
        let req = SynthesizeTtsReq {
            workspace_id: p.workspace_id.clone(),
            story_id: None,
            artifact_id: speaker.and_then(|s| s.result.as_ref()?.artifact_id.clone()),
            provider_id: Some(provider.id.clone()),
            voice_id: voice,
            text,
            speed: None,
            format: Some("wav".into()),
            title: Some(it.name.clone()),
            character_name: speaker.map(|s| s.name.clone()),
            model: None,
            voice_settings: None,
        };
        match self.synthesize_tts(req, user_id).await {
            Ok(clip) => Outcome::Done(
                ItemResult {
                    clip_id: Some(clip.id),
                    ..Default::default()
                },
                note,
            ),
            Err(e) => Outcome::Failed(plain(&e), None),
        }
    }

    /// A line spoken before its speaker's model was made joins that model
    /// once it exists (the clip's `artifact_id`).
    async fn link_clips(&self, p: &Production) {
        for it in p.plan.items.iter().filter(|i| i.kind == "voice_line") {
            let speaker = it.details.speaker.as_deref();
            let model = speaker
                .and_then(|s| p.plan.items.iter().find(|i| i.id == s))
                .and_then(|s| s.result.as_ref()?.artifact_id.clone());
            let clip = it.result.as_ref().and_then(|r| r.clip_id.as_deref());
            let (Some(model), Some(clip)) = (model, clip) else {
                continue;
            };
            let Ok(rec) = self.store().require_tts_clip(clip).await else {
                continue;
            };
            if rec.clip.artifact_id.as_deref() != Some(model.as_str()) {
                let _ = self.store().set_tts_clip_artifact(clip, &model).await;
            }
        }
    }

    /// Otto Forge's voice engine (`forge-voice` beside its binary), when
    /// installed: lines are made with it first.
    async fn forge_voice_cmd(&self) -> Option<std::path::PathBuf> {
        self.audio_command("voice").await
    }

    /// A spoken line made by Otto Forge's own voice engine (`forge-voice`
    /// beside its binary), one job at a time with the sound engine (its model
    /// runs on the GPU): the speaker's voice designed from a description that
    /// is the same for every line of that character (the engine then gives
    /// the same voice), at the pace its words say, with its mouth cues.
    async fn forge_voice(
        &self,
        p: &Production,
        ix: usize,
        cmd: &Path,
        user_id: &str,
        guard: &CancelGuard,
    ) -> Outcome {
        let it = &p.plan.items[ix];
        let speaker = it
            .details
            .speaker
            .as_deref()
            .and_then(|s| p.plan.items.iter().find(|i| i.id == s));
        let voice = speaker
            .and_then(|s| s.details.voice.clone())
            .or(it.details.voice.clone())
            .unwrap_or_default();
        // A reused character's voice is designed from its name there.
        let who = speaker.map_or("A storyteller", |s| {
            let name = s.details.reuse.as_ref().and_then(|r| r.name.as_deref());
            name.unwrap_or(&s.name)
        });
        let description = format!("{who}. {voice}").trim().to_string();
        // The line's own pace and mood first (a whisper is slower, never
        // faster), else the character's voice words.
        let line = [&it.details.pace, &it.details.mood].map(|s| s.clone().unwrap_or_default());
        let (line, said) = (words(&line.join(" ")), words(&voice));
        let has = |ws: &[String], w: &[&str]| ws.iter().any(|x| w.contains(&x.as_str()));
        let (slow, fast) = (SLOW_WORDS, ["fast", "quick", "quickly", "rapid", "hurried"]);
        let pace = match () {
            _ if has(&line, slow) => 0.85,
            _ if has(&line, &fast) => 1.1,
            _ if it.details.pace.is_some() => 1.0,
            _ if has(&said, slow) => 0.85,
            _ if has(&said, &fast) => 1.1,
            _ => 1.0,
        };
        let forge = self
            .store()
            .require_3d_provider(BUILTIN_OTTO_FORGE_ID)
            .await;
        let engine = forge
            .ok()
            .and_then(|f| Some(f.default_params.get("voice_engine")?.as_str()?.to_string()))
            .unwrap_or_else(|| FORGE_VOICE_ENGINE.into());
        let text = it.details.text.clone().unwrap_or(it.description.clone());
        let Ok(tmp) = tempfile::tempdir() else {
            return Outcome::Failed("Otto could not make a work folder.".into(), None);
        };
        let out = tmp.path().join("line.wav");
        let args: Vec<String> = [
            "speak",
            "--engine",
            &engine,
            "--voice",
            "designed",
            "--description",
            &description,
            "--speed",
            &pace.to_string(),
            "--text",
            &text,
            "--visemes",
            "--output",
        ]
        .iter()
        .map(|s| s.to_string())
        .chain([out.to_string_lossy().to_string()])
        .collect();
        // Its model runs on the GPU (the line holds the production's turn),
        // one job at a time with the sound engine.
        let turn = self.audio_turn.clone().acquire_owned().await;
        let speak = |args: Vec<String>| async move {
            let limit = Duration::from_secs(600);
            run_cli_argv(cmd, &args, None, limit, guard.flag(), None).await
        };
        let mut used = args.clone();
        let mut ran = speak(used.clone()).await;
        // Mouth cues that can't be lined up with the words (the aligner's
        // "sub-word alignment" or "utterance" failures) are no reason to lose
        // the line: spoken again without them.
        let cues_failed = |e: &Error| ["align", "utterance"].iter().any(|w| plain(e).contains(w));
        if ran.as_ref().is_err_and(cues_failed) && !guard.is_cancelled() {
            used.retain(|a| a != "--visemes");
            ran = speak(used.clone()).await;
        }
        drop(turn);
        match ran {
            Err(_) if guard.is_cancelled() => return Outcome::Skipped(STOPPED.into()),
            Err(e) => {
                return Outcome::Failed(
                    format!("The line could not be spoken: {}", plain(&e)),
                    None,
                )
            }
            Ok(()) => {}
        }
        let mut bytes = match tokio::fs::read(&out).await {
            Ok(b) if b.starts_with(b"RIFF") => b,
            _ => return Outcome::Failed("The voice engine wrote no sound.".into(), None),
        };
        // Too fast for its words: spoken once more, slowed to the floor.
        let floor = MIN_MS_PER_WORD * text.split_whitespace().count() as u64;
        let took = crate::tts::wav_duration_ms(&bytes).map_or(floor, |ms| ms.max(0) as u64);
        let mut pace = pace;
        if took < floor && !guard.is_cancelled() {
            let slowed = f64::from(pace) * took as f64 / floor as f64;
            pace = ((slowed * 100.0).round() / 100.0).max(0.7) as f32;
            if let Some(i) = used.iter().position(|a| a == "--speed") {
                used[i + 1] = pace.to_string();
            }
            let _turn = self.audio_turn.clone().acquire_owned().await;
            if speak(used).await.is_ok() {
                if let Ok(b) = tokio::fs::read(&out).await {
                    bytes = b;
                }
            }
        }
        let cues = tokio::fs::read(format!("{}.visemes.json", out.display())).await;
        let visemes = cues.ok().and_then(|b| serde_json::from_slice(&b).ok());
        let manifest = read_optional_manifest(&out).await;
        let clip_id = new_id();
        let file_name = format!("{clip_id}.wav");
        if let Err(e) = self.write_clip_file(&file_name, &bytes).await {
            return Outcome::Failed(plain(&e), None);
        }
        let rec = TtsClipRecord {
            clip: TtsClip {
                id: clip_id.clone(),
                workspace_id: p.workspace_id.clone(),
                story_id: None,
                artifact_id: speaker.and_then(|s| s.result.as_ref()?.artifact_id.clone()),
                provider_id: BUILTIN_OTTO_FORGE_ID.into(),
                provider_kind: "otto_forge".into(),
                voice_id: "designed".into(),
                title: it.name.clone(),
                character_name: speaker.map(|s| s.name.clone()),
                text,
                format: "wav".into(),
                mime: "audio/wav".into(),
                speed: pace,
                size_bytes: bytes.len() as i64,
                duration_ms: crate::tts::wav_duration_ms(&bytes),
                audio_url: format!("/api/v1/product/tts/clips/{clip_id}/audio"),
                exported_path: None,
                visemes,
                kind: "voice".into(),
                manifest,
                created_by: user_id.to_string(),
                created_at: Utc::now(),
            },
            file_name,
        };
        match self.store().insert_tts_clip(&rec).await {
            Ok(_) => Outcome::Done(
                ItemResult {
                    clip_id: Some(clip_id),
                    ..Default::default()
                },
                None,
            ),
            Err(e) => Outcome::Failed(plain(&e), None),
        }
    }

    /// A grunt, scream or laugh: a short sound from the sound engine; without
    /// it a line with words is spoken, else it is skipped with a plain note.
    async fn make_nonverbal(
        self: &Arc<Self>,
        s: &Shared,
        ix: usize,
        user_id: &str,
        guard: &CancelGuard,
    ) -> Outcome {
        if self.audio_command("sound").await.is_some() {
            return self.make_audio(s, ix, user_id, guard).await;
        }
        let p = s.lock().await.clone();
        let it = &p.plan.items[ix];
        if it
            .details
            .text
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty())
        {
            return self.make_voice(&p, ix, user_id, guard).await;
        }
        Outcome::Skipped(
            "A grunt, scream or laugh is made by the sound engine, which is not installed, so this was not made."
                .into(),
        )
    }

    /// Sound, music (and a nonverbal voice line, as a sound) through the
    /// sound engine ([`crate::audio`]): one job, one at a time beside the 3D
    /// work, its length clamped to the engine's limits with a plain note.
    async fn make_audio(
        self: &Arc<Self>,
        s: &Shared,
        ix: usize,
        user_id: &str,
        guard: &CancelGuard,
    ) -> Outcome {
        let p = s.lock().await.clone();
        let it = p.plan.items[ix].clone();
        let kind = if it.kind == "music" { "music" } else { "sound" };
        if self.audio_command(kind).await.is_none() {
            return Outcome::Skipped(NO_SOUND_ENGINE.into());
        }
        let looped = it.kind != "voice_line" && it.details.looped == Some(true);
        let (lo, hi) = audio_limits(kind, looped);
        let asked = it.details.seconds.unwrap_or(match (kind, looped) {
            ("music", _) => 30.0,
            (_, true) => 20.0,
            _ => 3.0,
        });
        let seconds = asked.clamp(lo, hi);
        let note = (seconds != asked).then(|| {
            let what = match (kind, looped) {
                ("music", true) => "looping music",
                ("music", false) => "music",
                (_, true) => "a looping sound",
                _ => "a sound",
            };
            format!("Made {seconds} seconds long, not {asked}: {what} can be {lo} to {hi} seconds.")
        });
        let speaker = it
            .details
            .speaker
            .as_deref()
            .and_then(|s| p.plan.items.iter().find(|i| i.id == s));
        let prompt = if it.kind == "voice_line" {
            let voice = it
                .details
                .voice
                .clone()
                .or_else(|| speaker.and_then(|s| s.details.voice.clone()));
            [voice, Some(it.description.clone()), it.details.text.clone()]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(", ")
        } else {
            it.details.prompt.clone().unwrap_or(it.description.clone())
        };
        let req = StartAudioJobReq {
            workspace_id: p.workspace_id.clone(),
            kind: kind.into(),
            prompt: prompt.chars().take(MAX_AUDIO_PROMPT_CHARS).collect(),
            seconds,
            looped,
            bpm: None,
            seed: None,
            title: Some(it.name.clone()),
            story_id: None,
            artifact_id: speaker.and_then(|s| s.result.as_ref()?.artifact_id.clone()),
        };
        let job = match self.start_audio_job(req, user_id).await {
            Ok(j) => j,
            Err(e) => return Outcome::Failed(plain(&e), None),
        };
        let job = loop {
            tokio::time::sleep(JOB_POLL).await;
            if guard.is_cancelled() {
                let _ = self.cancel_audio_job(&job.id).await;
            }
            let Ok(j) = self.store().require_audio_job(&job.id).await else {
                return Outcome::Failed("Its sound job is gone.".into(), None);
            };
            // Queued: another sound engine job holds the engine.
            let pct = j.progress_pct.min(99);
            let waits = (j.status == "queued").then(|| "engine".to_string());
            let changed = {
                let p = s.lock().await;
                let item = &p.plan.items[ix];
                item.progress_pct != pct
                    || item.message.as_deref() != Some(&j.stage)
                    || item.waiting != waits
            };
            if changed {
                self.edit(s, |p| {
                    let item = &mut p.plan.items[ix];
                    (item.progress_pct, item.message) = (pct, Some(j.stage.clone()));
                    item.waiting = waits;
                })
                .await;
            }
            if !matches!(j.status.as_str(), "queued" | "running") {
                break j;
            }
        };
        match job.status.as_str() {
            "succeeded" => Outcome::Done(
                ItemResult {
                    clip_id: job.clip_id,
                    ..Default::default()
                },
                note,
            ),
            "cancelled" => Outcome::Skipped(STOPPED.into()),
            "not_installed" => Outcome::Skipped(NO_SOUND_ENGINE.into()),
            _ => Outcome::Failed(
                format!(
                    "The sound could not be made: {}",
                    job.error.unwrap_or_else(|| "no reason given".into())
                ),
                None,
            ),
        }
    }

    /// Every made character and object as the scene builder takes it: the
    /// item's own version, and a character posed for this scene embeds its
    /// posed model (its size still from the model as made).
    pub(super) async fn scene_models(&self, p: &Production) -> Vec<Model> {
        let design = self.design_service();
        let mut models = Vec::new();
        for it in
            (p.plan.items.iter()).filter(|i| matches!(i.kind.as_str(), "character" | "object"))
        {
            let Some(aid) = it.result.as_ref().and_then(|r| r.artifact_id.clone()) else {
                continue;
            };
            let Ok(a) = design.store().require_artifact(&aid).await else {
                continue;
            };
            // The item's own version: a re-make adds versions to the same
            // artifact, and a revert points back at an older one.
            let version = match it.result.as_ref().and_then(|r| r.version_id.as_deref()) {
                Some(v) => design.resolve_version(&a, v).await,
                None => design.head_version(&a).await,
            };
            let (bytes, version, library) = match version {
                Ok(v) => (
                    design.version_bytes(&v).await.unwrap_or_default(),
                    Some(v.seq),
                    v.provenance["source"] == "library",
                ),
                Err(_) => (Vec::new(), None, false),
            };
            let mut m = Model {
                item: it.id.clone(),
                artifact_id: aid,
                version,
                bounds: glb_bounds(&bytes).unwrap_or(Bounds::UNIT),
                joints: glb_joints(&bytes),
                tris: glb_tris(&bytes),
                real_size: library,
                pose: None,
            };
            let posed = it.result.as_ref().and_then(|r| r.pose.as_ref());
            if let Some((pa, v)) = posed.and_then(|v| Some((v["artifact_id"].as_str()?, v))) {
                if let (Ok(a), Some(pose)) = (
                    design.store().require_artifact(pa).await,
                    Posed::of(&v["manifest"]),
                ) {
                    let ver = match v["version_id"].as_str() {
                        Some(x) => design.resolve_version(&a, x).await,
                        None => design.head_version(&a).await,
                    };
                    if let Ok(ver) = ver {
                        (m.artifact_id, m.version) = (pa.to_string(), Some(ver.seq));
                        m.apply_pose(pose, v, it.details.body.as_deref());
                    }
                }
            }
            models.push(m);
        }
        models
    }

    /// Write (or add a version to) the production's one scene artifact from
    /// every made model; a failure is a note, never the production's. The
    /// layout's notes are kept only with `notes` (the last write: while
    /// characters still stand without a skeleton, a neck is not found yet).
    async fn write_scene(&self, p: &mut Production, user_id: &str, notes: bool) {
        self.link_clips(p).await;
        self.pose_characters(p, user_id).await;
        let design = self.design_service();
        let models = self.scene_models(p).await;
        if models.is_empty() {
            return;
        }
        let (doc, said) = build_scene(&p.plan, &models);
        if notes {
            // Said by builds before a fist could hold anything; never true now.
            p.plan
                .notes
                .retain(|n| !n.ends_with(" yet; they stand next to it for now."));
            let gone = std::mem::take(&mut p.plan.scene_notes);
            p.plan
                .notes
                .retain(|n| !gone.contains(n) || said.contains(n));
            for n in &said {
                push_note(&mut p.plan, n.clone());
            }
            p.plan.scene_notes = said;
        }
        let bytes = doc.to_string().into_bytes();
        let existing = match &p.scene_artifact_id {
            Some(id) => design.store().require_artifact(id).await.ok(),
            None => None,
        };
        let saved = match existing {
            Some(a) => design
                .commit_bytes(
                    &a,
                    bytes,
                    SaveOpts {
                        base: None,
                        kind: "named".into(),
                        author: Author::user(user_id),
                        message: "Scene rebuilt by Smart mode".into(),
                        provenance: json!({ "production_id": p.id }),
                        force: true,
                        validate: true,
                        change: "content",
                    },
                )
                .await
                .map(|r| r.artifact.id),
            None => design
                .create_artifact(CreateInput {
                    workspace_id: p.workspace_id.clone(),
                    project_id: None,
                    studio: Some("3d".into()),
                    format: "scene3d".into(),
                    title: format!("{} (scene)", p.plan.title),
                    tags: vec!["smart-mode".into()],
                    meta: json!({ "production_id": p.id }),
                    content: Some(bytes),
                    story_id: None,
                    derived_from: None,
                    created_by: user_id.to_string(),
                    author: Author::user(user_id),
                    message: Some("Scene made by Smart mode".into()),
                    source: None,
                    created_at: None,
                    version_kind: Some("named".into()),
                    validate: true,
                    provenance: Some(json!({ "production_id": p.id })),
                })
                .await
                .map(|r| r.artifact.id),
        };
        match saved {
            Ok(id) => p.scene_artifact_id = Some(id),
            Err(e) => push_note(
                &mut p.plan,
                format!("I could not put the scene together: {}", plain(&e)),
            ),
        }
    }

    /// Boot recovery for 3D jobs and productions (their runners die with the
    /// daemon): a planning production fails (or, when it was being steered,
    /// keeps its plan); a running one (making, or waiting its turn) has its
    /// working items failed and is started again in its old place in the
    /// line, so it makes the rest (one that cannot start stays ready).
    pub async fn recover_interrupted(self: &Arc<Self>) -> Result<()> {
        self.store().fail_interrupted_3d_jobs().await?;
        self.store().fail_interrupted_audio_jobs().await?;
        let mut again = Vec::new();
        for mut p in self.store().interrupted_productions().await? {
            if p.status == "planning" && p.plan.items.is_empty() {
                p.status = "failed".into();
                p.message =
                    Some("Otto restarted while it was planning. Describe the scene again.".into());
            } else if p.status == "planning" {
                let waiting = p
                    .plan
                    .items
                    .iter()
                    .any(|i| i.enabled && i.status == "waiting");
                p.status = if waiting { "ready" } else { "done" }.into();
                p.message =
                    Some("Otto restarted while it was changing the plan; nothing changed.".into());
            } else {
                for it in p.plan.items.iter_mut().filter(|i| i.status == "working") {
                    (it.status, it.message) = (
                        "failed".into(),
                        Some("Otto restarted while this was being made.".into()),
                    );
                    (it.step, it.waiting) = (None, None);
                }
                p.status = "ready".into();
                p.message =
                    Some("Otto restarted while making this. Press Start to make the rest.".into());
                again.push((p.queued_at, p.id.clone(), p.created_by.clone()));
            }
            self.store().save_production(&p).await?;
        }
        again.sort();
        for (_, id, user) in again {
            let _ = self.start_production(&id, &user).await;
        }
        Ok(())
    }
}

/// Whether this Forge CLI's `generate` takes `flag` (`--height-m`,
/// `--concept-only`, `--skip-gameready`…; an older one refuses an unknown
/// Map the planner's body to Forge's drawing recipe. Older plans leave the
/// choice to Forge and props never receive a character pose.
fn picture_body(it: &Item) -> Option<&'static str> {
    if it.kind != "character" {
        return None;
    }
    Some(match it.details.body.as_deref()? {
        "two_legs" => "humanoid",
        "four_legs" => "quadruped",
        "bird" => "bird",
        "fish" | "no_legs" => "legless",
        "many_legs" | "wheels" => "other",
        _ => return None,
    })
}

/// flag): its `generate --help`, read once per binary.
pub(super) async fn forge_takes(bin: &str, flag: &str) -> bool {
    forge_help(bin, &["generate", "--help"])
        .await
        .contains(flag)
}

/// Whether this Forge has `forge concept-server`; the same engine finishes a
/// character draft exactly (it keeps `<draft>.glb.finish/` beside it).
pub(super) async fn forge_serves_concepts(bin: &str) -> bool {
    forge_help(bin, &["--help"])
        .await
        .contains("concept-server")
}

pub(super) async fn forge_help(bin: &str, args: &[&str]) -> String {
    static HELP: LazyLock<Mutex<HashMap<String, String>>> = LazyLock::new(Default::default);
    let key = format!("{bin} {}", args.join(" "));
    let known = HELP
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .cloned();
    if let Some(help) = known {
        return help;
    }
    let ask = tokio::process::Command::new(bin)
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let help = match tokio::time::timeout(Duration::from_secs(20), ask).await {
        Ok(Ok(out)) => String::from_utf8_lossy(&out.stdout).into_owned(),
        _ => String::new(),
    };
    HELP.lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(key, help.clone());
    help
}

/// A character's note when its skeleton or moves could not be added.
fn stands_still(it: &Item) -> String {
    format!(
        "Otto could not add moves to {} this time, so it stands still.",
        the(&it.name, "the")
    )
}

/// Settle an item with what came of it.
fn settle(it: &mut Item, outcome: Outcome) {
    (it.step, it.waiting) = (None, None);
    match outcome {
        Outcome::Done(r, note) => {
            (it.status, it.progress_pct, it.message) = ("done".into(), 100, note);
            (it.result, it.made) = (Some(r), None);
        }
        Outcome::Skipped(m) if m == STOPPED => stop(it),
        Outcome::Skipped(m) => (it.status, it.message) = ("skipped".into(), Some(m)),
        Outcome::Failed(m, r) => {
            (it.status, it.message) = ("failed".into(), Some(m));
            it.result = r.or(it.result.take());
        }
    }
}

/// A character's moves the engine makes (each once; they reach `{clips}`),
/// and the ones it cannot.
fn moves_of(it: &Item) -> (Vec<String>, Vec<String>) {
    let (mut known, mut unknown) = (Vec::new(), Vec::new());
    for m in &it.details.moves {
        match ENGINE_MOVES.contains(&m.as_str()) {
            true if !known.contains(m) => known.push(m.clone()),
            true => {}
            false => unknown.push(m.clone()),
        }
    }
    (known, unknown)
}

/// Why a library piece named `plain` is no match for the plan's `name`:
/// it shares none of its words ("Banner 1" for "Forest tree 1", matched on
/// the number), or it lacks a shape the name says ("Table Large" for a
/// "Round wooden table"). A piece without a name is taken as it is.
fn no_match(name: &str, plain: &str) -> Option<String> {
    if plain.trim().is_empty() {
        return None;
    }
    let have = super::scene::words(plain);
    let said = super::scene::words(name);
    let real = |w: &&String| w.len() > 2 && w.chars().all(|c| c.is_alphabetic());
    if !said.iter().filter(real).any(|w| have.contains(w)) {
        return Some(format!("\"{plain}\" is not a {name}"));
    }
    (said.iter())
        .find(|w| SHAPES.contains(&w.as_str()) && !have.contains(w))
        .map(|w| format!("\"{plain}\" is not {w}"))
}

/// What the library is asked for an item still to make: any prop or scenery, a
/// character only in the words of the outfits it has (Ranger, Peasant).
fn library_kind(it: &Item) -> Option<&'static str> {
    if !it.enabled || it.status != "waiting" || it.made.is_some() {
        return None;
    }
    let prompt = it.details.prompt.as_deref().unwrap_or(&it.description);
    let says = |w: &[&str]| {
        [it.name.as_str(), &it.description, prompt]
            .iter()
            .any(|t| words(t).iter().any(|x| w.contains(&x.as_str())))
    };
    match it.kind.as_str() {
        // Props and scenery (a tree, a hedge, a rock): both are the library's.
        "object" => Some("any"),
        "character" if says(&["ranger", "peasant"]) => Some("character"),
        _ => None,
    }
}

/// A re-make is a new version of the artifact it made before.
fn remake_target(it: &Item) -> Option<String> {
    let r = it.result.as_ref().or(it.previous_result.as_ref());
    r.and_then(|r| r.artifact_id.clone())
}

/// One 3D stage of an item: `generate` from its prompt (a new version of
/// `target`), or `rig` / `animate` on the exact version `source` names.
fn stage_req(
    p: &Production,
    it: &Item,
    stage: &str,
    source: Option<&ItemResult>,
    target: Option<String>,
    params: Option<serde_json::Value>,
) -> StartMedia3dJobReq {
    let prompt = if stage == "generate" {
        pictures::visual_prompt(&p.plan, it)
    } else {
        it.details
            .prompt
            .as_ref()
            .unwrap_or(&it.description)
            .clone()
    };
    StartMedia3dJobReq {
        workspace_id: p.workspace_id.clone(),
        project_id: None,
        stage: Some(stage.to_string()),
        source_artifact_id: source.and_then(|r| r.artifact_id.clone()),
        source_version_id: source.and_then(|r| r.version_id.clone()),
        target_artifact_id: target,
        title: Some(it.name.clone()),
        provider_id: BUILTIN_OTTO_FORGE_ID.into(),
        prompt: Some(prompt),
        image_b64: None,
        image_mime: None,
        image_artifact_id: None,
        output_format: Some("glb".into()),
        preset: None,
        poly_budget: matches!(stage, "generate" | "gameready")
            .then(|| poly_budget(&p.use_for, &it.kind)),
        seed: None,
        // `{clips}` in the provider's argument template picks the moves.
        params,
        auto_save_artifact: Some(true),
        export_relative_path: None,
        then_stages: Vec::new(),
    }
}

/// Copy a drawn concept picture to the production's own folder.
pub(super) async fn copy_picture(from: &Path, to: &Path) -> bool {
    if let Some(dir) = to.parent() {
        let _ = tokio::fs::create_dir_all(dir).await;
    }
    tokio::fs::copy(from, to).await.is_ok()
}

/// "The man" from "Man" or "The Man" (never "The The Man").
fn the(name: &str, article: &str) -> String {
    let bare = name
        .strip_prefix("The ")
        .or(name.strip_prefix("the "))
        .unwrap_or(name);
    format!("{article} {bare}")
}

fn push_note(plan: &mut Plan, note: String) {
    if !plan.notes.contains(&note) && plan.notes.len() < 40 {
        plan.notes.push(note);
    }
}

fn skip_the_rest(p: &mut Production) {
    p.plan
        .items
        .iter_mut()
        .filter(|i| matches!(i.status.as_str(), "waiting" | "working"))
        .for_each(stop);
}

/// An item stopped before it was made. One that already has a result was
/// being made again after it failed: it stays failed, so Start tries again.
fn stop(it: &mut Item) {
    let status = if it.result.is_some() {
        "failed"
    } else {
        "skipped"
    };
    (it.status, it.message) = (status.into(), Some(STOPPED.into()));
}

/// macOS voices carry no gender: the common English ones by name.
const MALE_VOICES: &[&str] = &[
    "Aaron", "Alex", "Arthur", "Daniel", "Eddy", "Evan", "Fred", "Gordon", "Grandpa", "Lee",
    "Nathan", "Oliver", "Ralph", "Reed", "Rishi", "Rocko", "Tom",
];
const FEMALE_VOICES: &[&str] = &[
    "Allison", "Ava", "Fiona", "Flo", "Grandma", "Karen", "Kate", "Kathy", "Moira", "Nicky",
    "Samantha", "Sandy", "Serena", "Shelley", "Susan", "Tessa", "Veena", "Victoria", "Zoe",
];
/// Sound-effect and singing voices never speak for a character.
const NOVELTY_VOICES: &[&str] = &[
    "Albert",
    "Bad News",
    "Bahh",
    "Bells",
    "Boing",
    "Bubbles",
    "Cellos",
    "Deranged",
    "Good News",
    "Hysterical",
    "Jester",
    "Organ",
    "Pipe Organ",
    "Superstar",
    "Trinoids",
    "Whisper",
    "Wobble",
    "Zarvox",
];

/// A voice's own name: macOS calls Samantha "Samantha (English (US))".
fn voice_name(id: &str) -> &str {
    id.split(" (").next().unwrap_or(id).trim()
}

fn voice_gender(v: &TtsVoice) -> Option<&str> {
    let name = voice_name(&v.id);
    v.gender.as_deref().or(if MALE_VOICES.contains(&name) {
        Some("male")
    } else if FEMALE_VOICES.contains(&name) {
        Some("female")
    } else {
        None
    })
}

/// The voices that may speak: no novelty voice, English ones when there are.
fn speaking_voices(voices: &[TtsVoice]) -> Vec<&TtsVoice> {
    let usable: Vec<&TtsVoice> = voices
        .iter()
        .filter(|v| !NOVELTY_VOICES.contains(&voice_name(&v.id)))
        .collect();
    let english = |v: &&TtsVoice| v.locale.as_deref().is_none_or(|l| l.starts_with("en"));
    match () {
        _ if usable.iter().any(english) => usable.into_iter().filter(english).collect(),
        _ if !usable.is_empty() => usable,
        _ => voices.iter().collect(),
    }
}

/// A voice for a plain description, not one in `taken` while another fits:
/// never the other gender than the words ask for, then a free voice, one of
/// the asked gender, the provider's default (by its own name), the first.
fn pick_voice(
    voices: &[&TtsVoice],
    wanted: &str,
    default: Option<&str>,
    taken: &HashSet<String>,
) -> Option<String> {
    let w = format!(" {} ", wanted.to_lowercase());
    let says = |words: &[&str]| words.iter().any(|x| w.contains(x));
    let gender = if says(&[
        " man ", " male", " deep", " boy", " his ", " he ", " king", " father", " monster",
    ]) {
        Some("male")
    } else if says(&[
        " woman", " female", " girl", " her ", " she ", " queen", " mother",
    ]) {
        Some("female")
    } else {
        None
    };
    voices
        .iter()
        .min_by_key(|v| {
            let g = voice_gender(v);
            (
                gender.is_some() && g.is_some() && g != gender,
                taken.contains(&v.id),
                gender.is_some() && g.is_none(),
                !default.is_some_and(|d| v.id == d || voice_name(&v.id) == d),
            )
        })
        .map(|v| v.id.clone())
}

/// One voice per character, in plan order, each a different one while the
/// engine has enough (from its voice, name and description); a `kept` one
/// (a reused character's) is its own.
fn cast_voices(
    plan: &Plan,
    voices: &[&TtsVoice],
    default: Option<&str>,
    kept: HashMap<String, String>,
) -> HashMap<String, String> {
    let mut taken: HashSet<String> = kept.values().cloned().collect();
    let mut cast = kept;
    for c in plan.items.iter().filter(|i| i.kind == "character") {
        if cast.contains_key(&c.id) {
            continue;
        }
        let words = format!(
            "{} {} {}",
            c.details.voice.as_deref().unwrap_or_default(),
            c.name,
            c.description
        );
        if let Some(v) = pick_voice(voices, &words, default, &taken) {
            taken.insert(v.clone());
            cast.insert(c.id.clone(), v);
        }
    }
    cast
}

/// Fold a steering answer into the production: kept items keep their
/// results; changed ones remember theirs as `previous_result` and wait to be
/// remade (a made model whose maker input is the same keeps its result: a
/// size or place change is the scene's, said in `done`; asked to make it
/// again with nothing different, it would come out the same, so it is not
/// made and reads as the same); added ones wait. The ids are worked out from the two plans (the
/// planner's own `changed` list may only add to them).
fn apply_revision(
    p: &mut Production,
    old: &Plan,
    mut new: Plan,
    draft: RevisionDraft,
    comment: String,
) -> Revision {
    let content = |i: &Item| {
        (
            i.kind.clone(),
            i.name.clone(),
            i.description.clone(),
            i.enabled,
            i.details.clone(),
        )
    };
    // What the 3D maker is given: a model whose input is the same (only its
    // size, place or words changed) keeps its result, and the scene shows it
    // at the new size; making it again would give the same model.
    let model_input = |i: &Item| {
        let prompt = i.details.prompt.as_ref().unwrap_or(&i.description);
        (
            i.kind.clone(),
            prompt.clone(),
            picture_body(i),
            i.details.moves.clone(),
        )
    };
    new.scene_notes = old.scene_notes.clone();
    let new_ids: HashSet<String> = new.items.iter().map(|i| i.id.clone()).collect();
    let sizes: HashMap<String, f64> = new
        .items
        .iter()
        .map(|i| (i.id.clone(), size_m(&new, i)))
        .collect();
    let placed = |plan: &Plan, id: &str| plan.layout.iter().find(|l| l.item == id).cloned();
    let moved: HashSet<String> = new
        .items
        .iter()
        .filter(|i| placed(old, &i.id) != placed(&new, &i.id))
        .map(|i| i.id.clone())
        .collect();
    let (mut changed, mut added, mut before, mut done) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for it in &mut new.items {
        let Some(prev) = old.items.iter().find(|o| o.id == it.id) else {
            added.push(it.id.clone());
            continue;
        };
        let same = content(prev) == content(it);
        // Its picture count goes on (a new picture never reads as an old one).
        it.preview_rev = prev.preview_rev;
        if same && !draft.changed.contains(&it.id) {
            (it.status, it.progress_pct, it.message) =
                (prev.status.clone(), prev.progress_pct, prev.message.clone());
            (it.result, it.previous_result) = (prev.result.clone(), prev.previous_result.clone());
            (it.made, it.has_preview) = (prev.made.clone(), prev.has_preview);
            continue;
        }
        changed.push(it.id.clone());
        before.push(prev.clone());
        let model = matches!(it.kind.as_str(), "character" | "object");
        if model && prev.result.is_some() && model_input(prev) == model_input(it) {
            (it.status, it.progress_pct, it.message) =
                (prev.status.clone(), prev.progress_pct, prev.message.clone());
            (it.result, it.made, it.has_preview) =
                (prev.result.clone(), prev.made.clone(), prev.has_preview);
            let (was, now) = (size_m(old, prev), sizes[&it.id]);
            let (is, has) = if it.name.ends_with('s') {
                ("are", "have")
            } else {
                ("is", "has")
            };
            if now > was * 1.01 {
                done.push(format!("{} {is} bigger.", it.name));
            } else if now < was * 0.99 {
                done.push(format!("{} {is} smaller.", it.name));
            } else if moved.contains(&it.id) {
                done.push(format!("{} {has} moved.", it.name));
            } else if same {
                // Nothing to make differently: the same words give the same model.
                it.previous_result = prev.result.clone();
            }
        } else {
            // Changed again before it was remade: the last made one stays.
            it.previous_result = prev.result.clone().or(prev.previous_result.clone());
            // A reused piece whose look changes is made anew.
            if model && model_input(prev) != model_input(it) {
                it.details.reuse = None;
            }
        }
    }
    let removed: Vec<String> = old
        .items
        .iter()
        .filter(|o| !new_ids.contains(&o.id))
        .map(|o| {
            before.push(o.clone());
            o.id.clone()
        })
        .collect();
    let summary = if draft.summary.trim().is_empty() {
        format!(
            "{} changed, {} added, {} removed.",
            changed.len(),
            added.len(),
            removed.len()
        )
    } else {
        draft.summary.trim().to_string()
    };
    done.extend(setting::said(old, &new));
    let setting_before = setting::before(old, &new);
    p.plan = new;
    Revision {
        id: new_id(),
        comment,
        summary,
        changed,
        added,
        removed,
        done,
        at: Utc::now(),
        before,
        setting_before,
    }
}

#[cfg(test)]
mod tests {
    /// A library piece sharing no word with the plan's name, or missing the
    /// shape it says, is no match (the real campfire and breakfast picks).
    #[test]
    fn a_library_piece_named_otherwise_is_no_match() {
        use super::no_match;
        for (name, plain) in [
            ("Round wooden table", "Table Large"),
            ("Forest tree 1", "Banner 1"),
            ("Log seat 3", "Rope 3"),
        ] {
            assert!(no_match(name, plain).is_some(), "{plain} for {name}");
        }
        for (name, plain) in [
            ("Round wooden table", "Table Round"),
            ("Wooden table", "Table Large"),
            ("Log seat 1", "Log Large"),
            ("Old chest", ""),
        ] {
            assert_eq!(no_match(name, plain), None, "{plain} for {name}");
        }
    }

    use super::*;

    /// Acceptance defect 3: macOS voices carry no gender and their ids add
    /// the language; every character got the first voice, the novelty
    /// "Albert". Now each speaks in its own fitting voice.
    #[test]
    fn each_character_gets_its_own_fitting_macos_voice() {
        let voices: Vec<TtsVoice> = [
            ("Albert", "en_US"),
            ("Bad News", "en_US"),
            ("Amélie", "fr_CA"),
            ("Eddy (English (US))", "en_US"),
            ("Flo (English (US))", "en_US"),
            ("Fred", "en_US"),
            ("Samantha (English (US))", "en_US"),
        ]
        .map(|(id, locale)| TtsVoice {
            id: id.into(),
            name: id.into(),
            locale: Some(locale.into()),
            gender: None,
            preview_text: None,
            provider_id: "macos-say".into(),
        })
        .to_vec();
        let plan: Plan = serde_json::from_value(json!({
            "title": "Quest",
            "items": [
                {"id": "man", "kind": "character", "name": "Man", "description": "A tired fighter.",
                 "details": {"voice": "A deep, low man's voice, a little broken."}},
                {"id": "monster", "kind": "character", "name": "The Monster", "description": "A horned beast.",
                 "details": {"voice": "growling"}},
                {"id": "girl", "kind": "character", "name": "Girl", "description": "A brave girl.",
                 "details": {"voice": "bright"}},
                {"id": "ghost", "kind": "character", "name": "Ghost", "description": "A ghost.",
                 "details": {"voice": "soft"}}
            ]
        }))
        .unwrap();
        let speaking = speaking_voices(&voices);
        let cast = cast_voices(&plan, &speaking, Some("Samantha"), HashMap::new());
        assert_eq!(cast["man"], "Eddy (English (US))");
        assert_eq!(cast["monster"], "Fred");
        // The provider's default, found by its own name, comes first among
        // the women; then a voice nobody has yet.
        assert_eq!(cast["girl"], "Samantha (English (US))");
        assert_eq!(cast["ghost"], "Flo (English (US))");
        // More men than male voices: one is shared rather than a woman's.
        let three: Plan = serde_json::from_value(json!({
            "title": "Men",
            "items": (["a", "b", "c"].map(|id| json!({"id": id, "kind": "character", "name": "Man", "description": "A man."})))
        }))
        .unwrap();
        let cast = cast_voices(&three, &speaking, None, HashMap::new());
        assert!(
            ["a", "b", "c"]
                .iter()
                .all(|id| ["Eddy (English (US))", "Fred"].contains(&cast[*id].as_str())),
            "{cast:?}"
        );
    }
}
