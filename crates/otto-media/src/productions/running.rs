//! Changing a production while it makes: a steer or a "not this one" on a
//! piece whose 3D shape is not built yet, and which is not being worked on
//! right now, changes only that piece; it is drawn again when its turn
//! comes, nothing else starts over. Pieces are never added or removed
//! mid-run (the lanes hold their places), and a built or working piece is
//! not changed: that part of a steer waits, said plainly.

use std::sync::Arc;

use chrono::Utc;
use otto_core::{new_id, Error, Result};

use super::{setting, steer_prompt, Item, Plan, Production, ProductionPlanner, Revision, Shared};
use crate::productions::Pin;
use crate::service::MediaService;

/// A piece a change can still reach: a 3D piece not built, not working.
fn open(it: &Item) -> bool {
    matches!(it.kind.as_str(), "character" | "object")
        && matches!(it.made.as_deref(), None | Some("picture"))
        && it.result.is_none()
        && !matches!(it.status.as_str(), "working" | "done")
}

impl MediaService {
    /// Change a production through its live state while it makes, else in
    /// the store (it ended meanwhile).
    async fn change<R>(&self, id: &str, f: impl FnOnce(&mut Production) -> R) -> Result<R> {
        let live: Option<Shared> = self.runs().get(id).cloned();
        if let Some(s) = live {
            let mut p = s.lock().await;
            // Still live under its lock: its last state is not taken yet.
            if self.runs().contains_key(id) {
                let r = f(&mut p);
                self.store().save_production(&p).await?;
                return Ok(r);
            }
        }
        let _lock = self.production_lock.lock().await;
        let mut p = self.store().require_production(id).await?;
        let r = f(&mut p);
        self.store().save_production(&p).await?;
        Ok(r)
    }

    /// "Not this one": the piece is drawn again with another seed (only its
    /// picture is new; the rest goes on). While the production makes, when
    /// its turn comes; else at the next Start.
    pub async fn redraw_item(&self, id: &str, item_id: &str) -> Result<Production> {
        self.change(id, |p| {
            let it = p
                .plan
                .items
                .iter_mut()
                .find(|i| i.id == item_id)
                .ok_or_else(|| Error::NotFound(format!("item {item_id}")))?;
            if !open(it) {
                return Err(Error::Conflict(
                    "Only a piece whose 3D shape is not built yet, and that is not being made right now, can be drawn again.".into(),
                ));
            }
            it.seed = Some(it.seed.unwrap_or(42) + 1);
            it.made = None;
            Ok(())
        })
        .await??;
        self.get_production(id).await
    }

    /// A steer while it makes: one planner turn; what it changes on pieces a
    /// change can still reach (their words, prompt and details) is taken,
    /// each drawn again at its turn; the layout is taken for pieces that
    /// exist; the rest waits for the production to end, said plainly.
    pub(super) async fn steer_running(
        self: &Arc<Self>,
        id: &str,
        comment: String,
        pins: Vec<Pin>,
        planner: Arc<dyn ProductionPlanner>,
    ) -> Result<Production> {
        let before = self
            .change(id, |p| {
                p.message = Some("Changing the plan…".into());
                p.plan.clone()
            })
            .await?;
        let prompt = steer_prompt(&before, &comment, &pins, "plan.json");
        let (svc, pid) = (Arc::clone(self), id.to_string());
        tokio::spawn(otto_core::auth::carry_request_credential(async move {
            let planned = svc.plan_turn(&pid, prompt, planner.as_ref()).await;
            let _ = svc
                .change(&pid, |p| match planned {
                    Ok((plan, draft)) => {
                        take_running(p, plan, comment, draft.unwrap_or_default().summary)
                    }
                    Err(m) => p.message = Some(m),
                })
                .await;
        }));
        self.get_production(id).await
    }
}

/// Take what a running production can take from a revised plan (see
/// [`MediaService::steer_running`]), as a revision, with a plain message
/// for what waits.
fn take_running(p: &mut Production, plan: Plan, comment: String, summary: String) {
    let (mut changed, mut before, mut waits) = (Vec::new(), Vec::new(), Vec::new());
    // Where and when it is ("make it night") changes no piece: taken at once,
    // the scene drawn with it as the pieces come.
    let (done, setting_before) = (
        setting::said(&p.plan, &plan),
        setting::before(&p.plan, &plan),
    );
    p.plan.setting = plan.setting.clone();
    for new in &plan.items {
        let Some(it) = p.plan.items.iter_mut().find(|i| i.id == new.id) else {
            waits.push(new.name.clone());
            continue;
        };
        let same = it.name == new.name
            && it.description == new.description
            && it.details == new.details
            && it.enabled == new.enabled;
        if same {
            continue;
        }
        if !open(it) || it.kind != new.kind {
            waits.push(it.name.clone());
            continue;
        }
        before.push(it.clone());
        changed.push(it.id.clone());
        (it.name, it.description) = (new.name.clone(), new.description.clone());
        (it.details, it.enabled, it.made) = (new.details.clone(), new.enabled, None);
        if !it.enabled {
            (it.status, it.message) = ("skipped".into(), Some("Turned off.".into()));
        }
    }
    waits.extend(
        p.plan
            .items
            .iter()
            .filter(|i| !plan.items.iter().any(|n| n.id == i.id))
            .map(|i| i.name.clone()),
    );
    let exists = |id: &str| p.plan.items.iter().any(|i| i.id == id);
    let layout: Vec<_> = plan
        .layout
        .into_iter()
        .filter(|l| exists(&l.item))
        .collect();
    if !layout.is_empty() {
        p.plan.layout = layout;
    }
    p.message = (!waits.is_empty()).then(|| {
        format!(
            "Changed while making. This waits until it is made, then steer again: {}.",
            waits.join(", ")
        )
    });
    p.revisions.push(Revision {
        id: new_id(),
        comment,
        summary,
        changed,
        added: Vec::new(),
        removed: Vec::new(),
        done,
        at: Utc::now(),
        before,
        setting_before,
    });
}
