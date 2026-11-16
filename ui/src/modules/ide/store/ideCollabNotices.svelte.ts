// Calm, inline states for a shared (collab) IDE session, instead of raw toasts:
// - `roomSync`: an edit was refused because the room's event stream fell far behind (409
//   "not keeping up"); the guest's text stays, the room state is re-read and the edit resent.
// - `saveRefusedByPath`: the host refused a guest's save of a protected file (403 with the
//   reason: git/agent config, instruction files, `.venv/bin`, launch configs, …).
// - `debug`: the host-only debug action a guest tried (Restart), said where the button is.

class IdeCollabNotices {
  roomSync = $state<'reconnecting' | 'failed' | null>(null);
  saveRefusedByPath = $state<Record<string, string>>({});
  debug = $state<string | null>(null);

  refuseSave(path: string, reason: string): void {
    this.saveRefusedByPath = { ...this.saveRefusedByPath, [path]: reason };
  }

  clearSave(path: string): void {
    if (!(path in this.saveRefusedByPath)) return;
    const next = { ...this.saveRefusedByPath };
    delete next[path];
    this.saveRefusedByPath = next;
  }

  reset(): void {
    this.roomSync = null;
    this.saveRefusedByPath = {};
    this.debug = null;
  }
}

export const ideCollabNotices = new IdeCollabNotices();
