import type { RecapCapture } from './room-recap-types';
/** Mirrors crates/otto-core/src/api/rooms.rs and docs/contracts/rooms.md. */
export type RoomRole = 'host' | 'viewer' | 'editor';
export type RoomAdmission = 'pending' | 'admitted';
export interface RoomCredential { room_id: string; member_id: string; token: string }
export interface RoomInvite { invite: string; url: string; expires_at: string }
export interface RoomMember {
  id: string; name: string; role: RoomRole; admission: RoomAdmission; connected: boolean;
  generation: number; control_requested: boolean; audio_joined: boolean; muted: boolean;
  room_muted: boolean; presenter_requested: boolean; presenter_allowed: boolean;
}
export interface RoomMessage { seq: number; member_id: string; name: string; text: string; nonce: string; created_at: string }
export interface RoomPresentation { id: string; member_id: string; generation: number; title: string; width: number; height: number; clear_epoch: number }
export interface RoomAnnotationGrant { source_id: string; member_id: string; requested: boolean; allowed: boolean; blocked: boolean; epoch: number }
export type RoomAnnotationTool = 'pointer' | 'highlight' | 'pen';
export interface RoomPoint { x: number; y: number }
export interface RoomAnnotation { id: string; source_id: string; source_generation: number; clear_epoch: number; grant_epoch: number; member_id: string; tool: RoomAnnotationTool; points: RoomPoint[]; expires_at: string }
export interface RoomSnapshot {
  room_id: string; member_id: string; admission: RoomAdmission; recap?: RecapCapture;
  session_id?: string; session_title?: string; provider?: string; host_member_id?: string;
  driver_member_id?: string; grant_epoch?: number; audio_epoch?: number; audio_enforced_epoch?: number; expires_at?: string;
  members?: RoomMember[]; messages?: RoomMessage[]; presentations?: RoomPresentation[];
  annotation_grants?: RoomAnnotationGrant[]; annotations?: RoomAnnotation[]; annotations_enabled?: boolean;
}
export type RoomSubscriptionTier = 'hidden' | 'preview' | 'grid' | 'full';
export type RoomAction =
  | { type: 'heartbeat' | 'request_ice' | 'recap_pause' | 'recap_stop' | 'request_control' | 'release_control' | 'leave' | 'end' | 'request_present' }
  | { type: 'admit' | 'role'; member_id: string; role: RoomRole }
  | { type: 'reject' | 'grant_control' | 'remove'; member_id: string }
  | { type: 'chat'; text: string; nonce: string }
  | { type: 'recap_consent'; epoch: number; allow: boolean }
  | { type: 'recap_start'; epoch: number }
  | { type: 'audio_applied'; epoch: number }
  | { type: 'audio'; joined: boolean; muted: boolean }
  | { type: 'mute'; member_id: string; muted: boolean }
  | { type: 'grant_present'; member_id: string; allowed: boolean }
  | { type: 'update_present'; source_id: string; width: number; height: number }
  | { type: 'start_present'; title: string; width: number; height: number }
  | { type: 'stop_present' | 'request_annotation' | 'clear_annotations' | 'undo_annotation'; source_id: string }
  | { type: 'subscribe'; source_id: string; source_generation: number; tier: RoomSubscriptionTier }
  | { type: 'annotations_enabled'; enabled: boolean }
  | { type: 'signal'; to: string; generation: number; media: unknown }
  | { type: 'grant_annotation'; source_id: string; member_id: string; allowed: boolean }
  | { type: 'revoke_annotation'; source_id: string; member_id: string; blocked: boolean }
  | { type: 'annotation'; source_id: string; source_generation: number; clear_epoch: number; grant_epoch: number; tool: RoomAnnotationTool; points: RoomPoint[] };
export type RoomEvent =
  | { type: 'snapshot'; room: RoomSnapshot }
  | { type: 'heartbeat' }
  | { type: 'ice'; ice_servers: RoomIceServer[]; relay_configured: boolean; relay_only: boolean; expires_at: string }
  | { type: 'signal'; from: string; to: string; generation: number; media: unknown }
  | { type: 'subscription'; member_id: string; source_id: string; source_generation: number; tier: RoomSubscriptionTier }
  | { type: 'annotation'; annotation: RoomAnnotation }
  | { type: 'clear_annotations'; source_id: string; clear_epoch: number }
  | { type: 'annotation_removed'; source_id: string; annotation_id: string }
  | { type: 'ended'; reason: string }
  | { type: 'error'; code: string; message: string };

export interface RoomIceServer { urls: string[]; username?: string; credential?: string }
export interface RoomSettings { public_origin: string; stun_urls: string[]; turn_urls: string[]; relay_only: boolean; turn_secret_configured: boolean }
export interface RoomSettingsUpdate { public_origin: string; stun_urls: string[]; turn_urls: string[]; relay_only: boolean; turn_secret?: string }
