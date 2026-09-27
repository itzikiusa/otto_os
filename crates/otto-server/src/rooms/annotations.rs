use super::registry::{forbidden, invalid, Room};
use chrono::Utc;
use otto_core::{api::*, Result};
use std::time::{Duration, Instant};
impl Room {
    pub fn annotation_action(&mut self, actor: &str, action: RoomAction) -> Result<()> {
        if let RoomAction::AnnotationsEnabled { enabled } = action {
            self.host(actor)?;
            self.annotations_enabled = enabled;
            for g in &mut self.grants {
                g.allowed = false;
                g.requested = false;
                g.epoch += 1;
            }
            self.marks.clear();
            for p in &mut self.presentations {
                p.clear_epoch += 1;
            }
            if enabled {
                for g in &mut self.grants {
                    if !g.blocked
                        && self
                            .presentations
                            .iter()
                            .any(|p| p.id == g.source_id && p.member_id == g.member_id)
                    {
                        g.allowed = true;
                    }
                }
            }
            return Ok(());
        }
        let source_id = match &action {
            RoomAction::RequestAnnotation { source_id }
            | RoomAction::GrantAnnotation { source_id, .. }
            | RoomAction::RevokeAnnotation { source_id, .. }
            | RoomAction::Annotation { source_id, .. }
            | RoomAction::ClearAnnotations { source_id }
            | RoomAction::UndoAnnotation { source_id } => source_id,
            _ => return Err(invalid("Invalid annotation action")),
        }
        .clone();
        let source = self
            .presentations
            .iter()
            .find(|p| p.id == source_id)
            .cloned()
            .ok_or_else(|| invalid("Presentation no longer exists"))?;
        match action {
            RoomAction::RequestAnnotation { .. } => {
                if !self.annotations_enabled {
                    return Err(forbidden("Room annotations are disabled"));
                }
                if let Some(g) = self
                    .grants
                    .iter_mut()
                    .find(|g| g.source_id == source_id && g.member_id == actor)
                {
                    if g.blocked {
                        return Err(forbidden("The host revoked this annotation permission"));
                    }
                    g.requested = true;
                } else {
                    self.grants.push(RoomAnnotationGrant {
                        source_id,
                        member_id: actor.into(),
                        requested: true,
                        allowed: false,
                        blocked: false,
                        epoch: 1,
                    });
                }
            }
            RoomAction::GrantAnnotation {
                member_id, allowed, ..
            } => {
                if source.member_id != actor {
                    return Err(forbidden("Only the presenter can approve annotations"));
                }
                self.admitted(&member_id)?;
                if !self.annotations_enabled {
                    return Err(forbidden("Room annotations are disabled"));
                }
                let g = self
                    .grants
                    .iter_mut()
                    .find(|g| g.source_id == source_id && g.member_id == member_id)
                    .ok_or_else(|| invalid("No annotation request"))?;
                if g.blocked {
                    return Err(forbidden("The host revoked this annotation permission"));
                }
                g.allowed = allowed;
                g.requested = false;
                g.epoch += 1;
                self.marks
                    .retain(|m| m.source_id != source_id || m.member_id != member_id);
            }
            RoomAction::RevokeAnnotation {
                member_id, blocked, ..
            } => {
                self.host(actor)?;
                self.admitted(&member_id)?;
                if let Some(g) = self
                    .grants
                    .iter_mut()
                    .find(|g| g.source_id == source_id && g.member_id == member_id)
                {
                    g.allowed = false;
                    g.requested = false;
                    g.blocked = blocked;
                    g.epoch += 1;
                } else {
                    self.grants.push(RoomAnnotationGrant {
                        source_id: source_id.clone(),
                        member_id: member_id.clone(),
                        requested: false,
                        allowed: false,
                        blocked,
                        epoch: 1,
                    });
                }
                self.marks
                    .retain(|m| m.source_id != source_id || m.member_id != member_id);
            }
            RoomAction::Annotation {
                source_generation,
                clear_epoch,
                grant_epoch,
                tool,
                points,
                ..
            } => {
                if !self.annotations_enabled
                    || source.generation != source_generation
                    || source.clear_epoch != clear_epoch
                {
                    return Err(forbidden("Annotation source or clear epoch expired"));
                }
                if !self.grants.iter().any(|g| {
                    g.source_id == source_id
                        && g.member_id == actor
                        && g.allowed
                        && !g.blocked
                        && g.epoch == grant_epoch
                }) {
                    return Err(forbidden("Annotation grant expired"));
                }
                if points.is_empty()
                    || points.len() > 64
                    || points.iter().any(|p| {
                        !p.x.is_finite()
                            || !p.y.is_finite()
                            || !(0.0..=1.0).contains(&p.x)
                            || !(0.0..=1.0).contains(&p.y)
                    })
                {
                    return Err(invalid("Annotations require 1–64 normalized points"));
                }
                if tool == RoomAnnotationTool::Pointer && points.len() != 1 {
                    return Err(invalid("Pointer requires one point"));
                }
                if tool == RoomAnnotationTool::Highlight && points.len() != 2 {
                    return Err(invalid("Highlight requires two corners"));
                }
                let m = self.members.get_mut(actor).unwrap();
                m.annotation_rate.take(32.0, 48.0)?;
                if tool == RoomAnnotationTool::Pointer {
                    if m.pointer_at
                        .is_some_and(|t| t.elapsed() < Duration::from_millis(50))
                    {
                        return Err(invalid("Pointer updates are limited to 20 per second"));
                    }
                    m.pointer_at = Some(Instant::now());
                    self.marks.retain(|m| {
                        m.source_id != source_id
                            || m.member_id != actor
                            || m.tool != RoomAnnotationTool::Pointer
                    });
                }
                self.marks.retain(|m| {
                    chrono::DateTime::parse_from_rfc3339(&m.expires_at)
                        .map(|t| t > Utc::now())
                        .unwrap_or(false)
                });
                if self
                    .marks
                    .iter()
                    .filter(|m| m.source_id == source_id)
                    .count()
                    >= 128
                {
                    return Err(invalid("Too many live annotations on this source"));
                }
                let mark = RoomAnnotation {
                    id: uuid::Uuid::new_v4().to_string(),
                    source_id,
                    source_generation,
                    clear_epoch,
                    grant_epoch,
                    member_id: actor.into(),
                    tool,
                    points,
                    expires_at: (Utc::now()
                        + chrono::Duration::seconds(if tool == RoomAnnotationTool::Pointer {
                            1
                        } else {
                            10
                        }))
                    .to_rfc3339(),
                };
                self.marks.push(mark.clone());
                self.event(None, RoomEvent::Annotation { annotation: mark });
            }
            RoomAction::ClearAnnotations { .. } => {
                if actor != self.host && actor != source.member_id {
                    return Err(forbidden("Only presenter or host can clear this source"));
                }
                let p = self
                    .presentations
                    .iter_mut()
                    .find(|p| p.id == source_id)
                    .unwrap();
                p.clear_epoch += 1;
                let epoch = p.clear_epoch;
                self.marks.retain(|m| m.source_id != source_id);
                self.event(
                    None,
                    RoomEvent::ClearAnnotations {
                        source_id,
                        clear_epoch: epoch,
                    },
                );
            }
            RoomAction::UndoAnnotation { .. } => {
                if let Some(pos) = self
                    .marks
                    .iter()
                    .rposition(|m| m.source_id == source_id && m.member_id == actor)
                {
                    let mark = self.marks.remove(pos);
                    self.event(
                        None,
                        RoomEvent::AnnotationRemoved {
                            source_id,
                            annotation_id: mark.id,
                        },
                    );
                }
            }
            _ => return Err(invalid("Invalid annotation action")),
        }
        Ok(())
    }
}
