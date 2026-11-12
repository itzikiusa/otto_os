// Who may change what on Settings ▸ Slack, Telegram & webhooks, mirroring the
// daemon's gates (crates/otto-channels/src/http.rs): setting up or changing a
// bridge (`PUT …/integrations/{channel}` — tokens, allowed users, enable) is
// OWNER-only on top of workspace Editor, because its sessions run as the
// owner (`require_owner_for_bridge`); sending a test message and removing a
// bridge need workspace Editor; reading the cards needs Viewer. Pure, so
// `node --test` can load it.

export type WorkspaceRoleLike = 'viewer' | 'editor' | 'admin';

export interface ChannelAccess {
  /** Set up / edit / enable / allow a sender (owner only). */
  configure: boolean;
  /** Send a test message, remove a bridge (workspace Editor+). */
  editWorkspace: boolean;
  /** One line saying why controls are missing, or '' when none are. */
  note: string;
}

export function channelAccess(isRoot: boolean, role: WorkspaceRoleLike): ChannelAccess {
  if (isRoot) return { configure: true, editWorkspace: true, note: '' };
  if (role !== 'viewer') {
    return {
      configure: false,
      editWorkspace: true,
      note: 'Only the owner can set up or change a bridge, because its sessions run as the owner. You can send a test message or remove one.',
    };
  }
  return {
    configure: false,
    editWorkspace: false,
    note: 'View only: you are a viewer in this workspace. Ask the owner to change its channels.',
  };
}
