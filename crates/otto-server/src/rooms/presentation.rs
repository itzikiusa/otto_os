use super::registry::{forbidden, invalid, Room};
use otto_core::{api::*, Error, Result};
use std::time::{Duration, Instant};
impl Room {
    pub fn presentation_action(&mut self, actor: &str, action: RoomAction) -> Result<()> {
        match action {
            RoomAction::RequestPresent => {
                self.members
                    .get_mut(actor)
                    .unwrap()
                    .view
                    .presenter_requested = true
            }
            RoomAction::GrantPresent { member_id, allowed } => {
                self.host(actor)?;
                self.admitted(&member_id)?;
                if !self.members[&member_id].view.connected {
                    return Err(forbidden("Presenter is disconnected"));
                }
                if !allowed {
                    let sources: Vec<_> = self
                        .presentations
                        .iter()
                        .filter(|p| p.member_id == member_id)
                        .map(|p| p.id.clone())
                        .collect();
                    for source in sources {
                        self.stop_source(&source);
                    }
                }
                let member = self.members.get_mut(&member_id).unwrap();
                member.view.presenter_allowed = allowed;
                member.view.presenter_requested = false;
                member.present_until = allowed.then(|| Instant::now() + Duration::from_secs(60));
            }
            RoomAction::StartPresent {
                title,
                width,
                height,
            } => {
                let m = self.member(actor)?;
                if actor != self.host
                    && (!m.view.presenter_allowed
                        || m.present_until.is_some_and(|t| t < Instant::now()))
                {
                    return Err(forbidden("Request permission to present"));
                }
                if self.presentations.len() >= 4
                    || self.presentations.iter().any(|p| p.member_id == actor)
                {
                    return Err(Error::Conflict(
                        "One source per participant is supported".into(),
                    ));
                }
                if title.trim().is_empty()
                    || title.len() > 200
                    || width == 0
                    || height == 0
                    || width > 16384
                    || height > 16384
                {
                    return Err(invalid("Invalid presentation title or dimensions"));
                }
                let id = uuid::Uuid::new_v4().to_string();
                self.presentations.push(RoomPresentation {
                    id: id.clone(),
                    member_id: actor.into(),
                    generation: 1,
                    title,
                    width,
                    height,
                    clear_epoch: 0,
                });
                self.grants.push(RoomAnnotationGrant {
                    source_id: id,
                    member_id: actor.into(),
                    requested: false,
                    allowed: self.annotations_enabled,
                    blocked: false,
                    epoch: 1,
                });
                self.members.get_mut(actor).unwrap().present_until = None;
                self.record(RecapEventData::Presentation {
                    operation: "start".into(),
                    presentation: self.presentations.last().unwrap().clone(),
                });
            }
            RoomAction::UpdatePresent {
                source_id,
                width,
                height,
            } => {
                let source = self
                    .presentations
                    .iter_mut()
                    .find(|p| p.id == source_id)
                    .ok_or_else(|| invalid("Presentation no longer exists"))?;
                if source.member_id != actor {
                    return Err(forbidden("Only the presenter can update source geometry"));
                }
                if width == 0 || height == 0 || width > 16384 || height > 16384 {
                    return Err(invalid("Invalid presentation dimensions"));
                }
                if source.width == width && source.height == height {
                    return Ok(());
                }
                source.width = width;
                source.height = height;
                source.generation += 1;
                source.clear_epoch += 1;
                let updated = source.clone();
                self.record(RecapEventData::Presentation {
                    operation: "update".into(),
                    presentation: updated,
                });
                self.marks.retain(|m| m.source_id != source_id);
                for grant in &mut self.grants {
                    if grant.source_id == source_id {
                        grant.epoch += 1;
                        grant.requested = false;
                        grant.allowed =
                            grant.member_id == actor && !grant.blocked && self.annotations_enabled;
                    }
                }
            }
            RoomAction::StopPresent { source_id } => {
                let source = self
                    .presentations
                    .iter()
                    .find(|p| p.id == source_id)
                    .ok_or_else(|| invalid("Presentation no longer exists"))?;
                if source.member_id != actor && actor != self.host {
                    return Err(forbidden("Only presenter or host may stop this source"));
                }
                let owner = source.member_id.clone();
                self.stop_source(&source_id);
                // Host stop revokes presenting permission, so a non-compliant
                // publisher cannot immediately start another source.
                if actor == self.host && owner != actor {
                    let m = self.members.get_mut(&owner).unwrap();
                    m.view.presenter_allowed = false;
                    m.present_until = None;
                }
            }
            RoomAction::Subscribe {
                source_id,
                source_generation,
                tier,
            } => {
                let source = self
                    .presentations
                    .iter()
                    .find(|p| p.id == source_id && p.generation == source_generation)
                    .ok_or_else(|| invalid("Presentation generation expired"))?;
                if source.member_id == actor && tier != RoomSubscriptionTier::Hidden {
                    return Err(invalid("Use your local preview"));
                }
                self.event(
                    Some(self.host.clone()),
                    RoomEvent::Subscription {
                        member_id: actor.into(),
                        source_id,
                        source_generation,
                        tier,
                    },
                );
            }
            _ => return Err(invalid("Invalid presentation action")),
        }
        Ok(())
    }
}
