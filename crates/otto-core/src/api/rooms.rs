//! Ephemeral session-room contracts; mirrored by UI room-types.ts.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomRole {
    Host,
    Viewer,
    Editor,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomAdmission {
    Pending,
    Admitted,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRoomReq {
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinRoomReq {
    pub room_id: String,
    pub invite: String,
    pub name: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomInviteReq {
    pub role: RoomRole,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomCredential {
    pub room_id: String,
    pub member_id: String,
    pub token: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomInvite {
    pub invite: String,
    pub url: String,
    pub expires_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMember {
    pub id: String,
    pub name: String,
    pub role: RoomRole,
    pub admission: RoomAdmission,
    pub connected: bool,
    pub generation: u64,
    pub control_requested: bool,
    pub audio_joined: bool,
    pub muted: bool,
    pub room_muted: bool,
    pub presenter_requested: bool,
    pub presenter_allowed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomMessage {
    pub seq: u64,
    pub member_id: String,
    pub name: String,
    pub text: String,
    pub nonce: String,
    pub created_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomPresentation {
    pub id: String,
    pub member_id: String,
    pub generation: u64,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub clear_epoch: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomAnnotationGrant {
    pub source_id: String,
    pub member_id: String,
    pub requested: bool,
    pub allowed: bool,
    pub blocked: bool,
    pub epoch: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomAnnotationTool {
    Pointer,
    Highlight,
    Pen,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomPoint {
    pub x: f64,
    pub y: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomAnnotation {
    pub id: String,
    pub source_id: String,
    pub source_generation: u64,
    pub clear_epoch: u64,
    pub grant_epoch: u64,
    pub member_id: String,
    pub tool: RoomAnnotationTool,
    pub points: Vec<RoomPoint>,
    pub expires_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomSnapshot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recap: Option<super::RecapCapture>,
    pub room_id: String,
    pub member_id: String,
    pub admission: RoomAdmission,
    // Pending projections omit all fields below instead of returning placeholders.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_member_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub driver_member_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_epoch: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub members: Option<Vec<RoomMember>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<RoomMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentations: Option<Vec<RoomPresentation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotation_grants: Option<Vec<RoomAnnotationGrant>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<Vec<RoomAnnotation>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations_enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_epoch: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audio_enforced_epoch: Option<u64>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomSubscriptionTier {
    Hidden,
    Preview,
    Grid,
    Full,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum RoomAction {
    Heartbeat,
    RecapConsent {
        epoch: u64,
        allow: bool,
    },
    RecapStart {
        epoch: u64,
    },
    RecapPause,
    RecapStop,
    RequestIce,
    AudioApplied {
        epoch: u64,
    },
    Admit {
        member_id: String,
        role: RoomRole,
    },
    Reject {
        member_id: String,
    },
    Role {
        member_id: String,
        role: RoomRole,
    },
    RequestControl,
    GrantControl {
        member_id: String,
    },
    ReleaseControl,
    Chat {
        text: String,
        nonce: String,
    },
    Leave,
    End,
    Remove {
        member_id: String,
    },
    Audio {
        joined: bool,
        muted: bool,
    },
    Mute {
        member_id: String,
        muted: bool,
    },
    RequestPresent,
    GrantPresent {
        member_id: String,
        allowed: bool,
    },
    StartPresent {
        title: String,
        width: u32,
        height: u32,
    },
    StopPresent {
        source_id: String,
    },
    UpdatePresent {
        source_id: String,
        width: u32,
        height: u32,
    },
    Subscribe {
        source_id: String,
        source_generation: u64,
        tier: RoomSubscriptionTier,
    },
    Signal {
        to: String,
        generation: u64,
        media: serde_json::Value,
    },
    RequestAnnotation {
        source_id: String,
    },
    GrantAnnotation {
        source_id: String,
        member_id: String,
        allowed: bool,
    },
    RevokeAnnotation {
        source_id: String,
        member_id: String,
        blocked: bool,
    },
    Annotation {
        source_id: String,
        source_generation: u64,
        clear_epoch: u64,
        grant_epoch: u64,
        tool: RoomAnnotationTool,
        points: Vec<RoomPoint>,
    },
    ClearAnnotations {
        source_id: String,
    },
    UndoAnnotation {
        source_id: String,
    },
    AnnotationsEnabled {
        enabled: bool,
    },
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RoomEvent {
    Snapshot {
        room: Box<RoomSnapshot>,
    },
    Heartbeat,
    Ice {
        ice_servers: Vec<RoomIceServer>,
        relay_configured: bool,
        relay_only: bool,
        expires_at: String,
    },
    Signal {
        from: String,
        to: String,
        generation: u64,
        media: serde_json::Value,
    },
    Subscription {
        member_id: String,
        source_id: String,
        source_generation: u64,
        tier: RoomSubscriptionTier,
    },
    Annotation {
        annotation: RoomAnnotation,
    },
    ClearAnnotations {
        source_id: String,
        clear_epoch: u64,
    },
    AnnotationRemoved {
        source_id: String,
        annotation_id: String,
    },
    Ended {
        reason: String,
    },
    Error {
        code: String,
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomIceServer {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoomSettings {
    pub public_origin: String,
    pub stun_urls: Vec<String>,
    pub turn_urls: Vec<String>,
    pub relay_only: bool,
    pub turn_secret_configured: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomSettingsReq {
    pub public_origin: String,
    pub stun_urls: Vec<String>,
    pub turn_urls: Vec<String>,
    pub relay_only: bool,
    pub turn_secret: Option<String>,
}
