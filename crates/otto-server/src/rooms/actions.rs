//! Membership, driver and chat transitions. PTY authority effects are applied
//! by the socket owner before publishing the resulting state.
use super::registry::{forbidden, invalid, Room};
use otto_core::{api::*, Result};
use std::time::Instant;
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Effect {
    None,
    Driver(Option<String>),
    End,
}
impl Room {
    pub fn apply(&mut self, actor: &str, action: RoomAction) -> Result<Effect> {
        self.member(actor)?;
        if matches!(action, RoomAction::Heartbeat) {
            self.members.get_mut(actor).unwrap().heartbeat = Instant::now();
            return Ok(Effect::None);
        }
        if matches!(action, RoomAction::Leave) {
            return self.remove_member(actor, actor);
        }
        self.admitted(actor)?;
        self.members
            .get_mut(actor)
            .unwrap()
            .action_rate
            .take(48.0, 80.0)?;
        match action {
            RoomAction::RecapConsent { .. }
            | RoomAction::RecapStart { .. }
            | RoomAction::RecapPause
            | RoomAction::RecapStop => self.recap_action(actor, action)?,
            RoomAction::Admit { member_id, role } => self.admit(actor, &member_id, role)?,
            RoomAction::Reject { member_id } => {
                self.host(actor)?;
                if self.member(&member_id)?.view.admission != RoomAdmission::Pending {
                    return Err(invalid("Member is not waiting"));
                }
                return self.remove_member(actor, &member_id);
            }
            RoomAction::Remove { member_id } => {
                self.host(actor)?;
                return self.remove_member(actor, &member_id);
            }
            RoomAction::End => {
                self.host(actor)?;
                return Ok(Effect::End);
            }
            RoomAction::Role { member_id, role } => {
                self.host(actor)?;
                self.admitted(&member_id)?;
                if member_id == self.host || role == RoomRole::Host {
                    return Err(forbidden("Host ownership cannot be transferred"));
                }
                let m = self.members.get_mut(&member_id).unwrap();
                m.view.role = role;
                m.view.control_requested = false;
                if self.driver == member_id && role == RoomRole::Viewer {
                    self.driver = self.host.clone();
                    return Ok(Effect::Driver(None));
                }
            }
            RoomAction::RequestControl => {
                if self.member(actor)?.view.role != RoomRole::Editor {
                    return Err(forbidden("View-only members cannot control the terminal"));
                }
                self.members.get_mut(actor).unwrap().view.control_requested = true;
            }
            RoomAction::GrantControl { member_id } => {
                self.host(actor)?;
                let target = self.admitted(&member_id)?;
                if !target.view.connected || target.view.role == RoomRole::Viewer {
                    return Err(forbidden("Driver must be a connected editor"));
                }
                self.driver = member_id.clone();
                self.members
                    .get_mut(&member_id)
                    .unwrap()
                    .view
                    .control_requested = false;
                return Ok(Effect::Driver(if member_id == self.host {
                    None
                } else {
                    Some(member_id)
                }));
            }
            RoomAction::ReleaseControl => {
                if self.driver != actor && actor != self.host {
                    return Err(forbidden("You do not control the terminal"));
                }
                self.driver = self.host.clone();
                return Ok(Effect::Driver(None));
            }
            RoomAction::Chat { text, nonce } => {
                if nonce.is_empty()
                    || nonce.len() > 128
                    || text.trim().is_empty()
                    || text.len() > 4096
                {
                    return Err(invalid(
                        "Message must contain 1–4096 UTF8 bytes and a nonce up to 128 bytes",
                    ));
                }
                if self
                    .messages
                    .iter()
                    .any(|m| m.member_id == actor && m.nonce == nonce)
                {
                    return Ok(Effect::None);
                }
                let m = self.members.get_mut(actor).unwrap();
                m.chat_rate.take(5.0, 10.0)?;
                self.next_message += 1;
                self.messages.push_back(RoomMessage {
                    seq: self.next_message,
                    member_id: actor.into(),
                    name: m.view.name.clone(),
                    text,
                    nonce,
                    created_at: chrono::Utc::now().to_rfc3339(),
                });
                self.record(RecapEventData::Chat {
                    message: self.messages.back().unwrap().clone(),
                });
                if self.messages.len() > 200 {
                    self.messages.pop_front();
                }
            }
            RoomAction::AudioApplied { epoch } => {
                self.host(actor)?;
                if epoch != self.audio_epoch {
                    return Err(invalid("Audio coordination epoch expired"));
                }
                self.audio_enforced_epoch = epoch;
                self.audio_pending_since = None;
            }
            RoomAction::Audio { joined, muted } => {
                if joined && actor != self.host && !self.members[&self.host].view.audio_joined {
                    return Err(forbidden("The host must join audio first"));
                }
                let m = self.members.get_mut(actor).unwrap();
                m.view.audio_joined = joined;
                m.view.muted = muted || !joined || m.view.room_muted;
                if actor == self.host && !joined {
                    for m in self.members.values_mut() {
                        m.view.audio_joined = false;
                        m.view.muted = true;
                    }
                }
                self.audio_changed();
                self.recap_audio_changed();
            }
            RoomAction::Mute { member_id, muted } => {
                self.host(actor)?;
                self.admitted(&member_id)?;
                let m = self.members.get_mut(&member_id).unwrap();
                m.view.room_muted = muted;
                m.view.muted = true;
                self.audio_changed();
                self.recap_audio_changed();
            }
            RoomAction::Signal {
                to,
                generation,
                media,
            } => {
                let source = self.admitted(actor)?;
                let target = self.admitted(&to)?;
                if actor == to
                    || (actor != self.host && to != self.host)
                    || !source.view.connected
                    || !target.view.connected
                    || generation != target.view.generation
                {
                    return Err(forbidden(
                        "Signaling requires a live host-to-guest connection",
                    ));
                }
                if serde_json::to_vec(&media)
                    .map_err(|_| invalid("Invalid signaling"))?
                    .len()
                    > 32768
                {
                    return Err(invalid("Signaling exceeds 32 KiB"));
                }
                self.event(
                    Some(to.clone()),
                    RoomEvent::Signal {
                        from: actor.into(),
                        to,
                        generation: source.view.generation,
                        media,
                    },
                );
            }
            RoomAction::RequestPresent
            | RoomAction::GrantPresent { .. }
            | RoomAction::StartPresent { .. }
            | RoomAction::StopPresent { .. }
            | RoomAction::UpdatePresent { .. }
            | RoomAction::Subscribe { .. } => self.presentation_action(actor, action)?,
            RoomAction::RequestAnnotation { .. }
            | RoomAction::GrantAnnotation { .. }
            | RoomAction::RevokeAnnotation { .. }
            | RoomAction::Annotation { .. }
            | RoomAction::ClearAnnotations { .. }
            | RoomAction::UndoAnnotation { .. }
            | RoomAction::AnnotationsEnabled { .. } => self.annotation_action(actor, action)?,
            RoomAction::RequestIce => {
                self.members
                    .get_mut(actor)
                    .unwrap()
                    .ice_rate
                    .take(1.0 / 30.0, 2.0)?;
            }
            RoomAction::Heartbeat | RoomAction::Leave => unreachable!(),
        }
        Ok(Effect::None)
    }
    fn remove_member(&mut self, actor: &str, id: &str) -> Result<Effect> {
        let was_admitted = self.member(id)?.view.admission == RoomAdmission::Admitted;
        if actor != id {
            self.host(actor)?;
        }
        if id == self.host {
            if actor != self.host {
                return Err(forbidden("Cannot remove the host"));
            }
            return Ok(Effect::End);
        }
        self.withdraw_media(id);
        self.members.remove(id);
        if was_admitted {
            self.recap_interrupt("A participant left or was removed", false);
        }
        if self.driver == id {
            self.driver = self.host.clone();
            Ok(Effect::Driver(None))
        } else {
            Ok(Effect::None)
        }
    }
}
