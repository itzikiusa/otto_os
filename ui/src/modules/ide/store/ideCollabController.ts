// Live Collaboration & Runtime IDE Plugin helpers for IdeStore.
// Kept in a focused module so `ideStore.svelte.ts` stays crisp and under the file-loc cap.

import { api, laneFetch } from '../../../lib/api/client';
import { loadErrorText } from '../../../lib/loadError';
import { toasts } from '../../../lib/toast.svelte';
import { toastError } from '../../../lib/toastError';
import { ideCollabNotices } from './ideCollabNotices.svelte';
import { mergeText } from './ideCollabSync';
import {
  collabGuestPost,
  ideBasename,
  streamIdeSse,
  type IdeCollabPeer,
  type IdeCollabRole,
  type IdeCollabRoom,
  type IdeDebugSessionSnapshot,
  type IdeOpenTab,
  type IdePeerCursor,
  type IdePluginCatalogEntry,
  type IdePluginInfo,
  type IdeTargetSpec,
} from './ideTypes';

export interface CollabHostState {
  target: IdeTargetSpec;
  readonly targetPayload: IdeTargetSpec;
  tabs: IdeOpenTab[];
  collabRoom: IdeCollabRoom | null;
  collabRooms: IdeCollabRoom[];
  collabInviteToken: string | null;
  collabShareHash: string | null;
  collabIsGuest: boolean;
  collabPeerId: string | null;
  collabPeerSecret: string | null;
  collabGuestToken: string | null;
  peerCursors: IdePeerCursor[];
  followingPeerId: string | null;
  sharedBuffers: Record<string, { content: string; version: number }>;
  collabLoading: boolean;
  collabError: string | null;
  readonly canEditCollab: boolean;
  plugins: IdePluginInfo[];
  pluginCatalog: IdePluginCatalogEntry[];
  pluginsLoading: boolean;
  pluginsError: string | null;
  onCollabJoinFailed?(errorMsg: string): void;
  clearLiveBuffer?(path: string): void;
  getBuffer?(path: string): string;
  sendCollabEdit?(path: string, content: string): Promise<boolean>;
  openFile(path: string, opts?: { line?: number; col?: number; preview?: boolean }): Promise<void>;
  applyDebugSnapshot(snap: IdeDebugSessionSnapshot): void;
}

/** Events carry the GUEST view of the room (no host ids, a bare target). A guest takes it
 *  whole; the host keeps its own target/ids and only takes the live parts. */
export function mergeEventRoom(store: Pick<CollabHostState, 'collabIsGuest' | 'collabRoom'>, room: IdeCollabRoom): IdeCollabRoom {
  const cur = store.collabRoom;
  if (store.collabIsGuest || !cur || cur.id !== room.id) return room;
  return { ...cur, peers: room.peers ?? cur.peers, default_role: room.default_role ?? cur.default_role, title: room.title || cur.title };
}

export function attachCollabEventStream(
  store: CollabHostState,
  roomId: string,
  token: string,
  peerId: string,
  prevCtl: AbortController | null,
): AbortController {
  prevCtl?.abort();
  const ctl = new AbortController();
  const path = `/ide/collab/guest/${encodeURIComponent(roomId)}/events?peer_id=${encodeURIComponent(peerId)}`;
  void streamIdeSse(path, { 'X-Ide-Collab-Token': token }, ctl.signal, (raw) => {
    try {
      const ev = JSON.parse(raw) as Record<string, unknown>;
      if (ev.type === 'room_closed') {
        store.collabRoom = null;
        store.collabError = 'The host ended this shared session.';
        ctl.abort();
        return;
      }
      if (ev.room && typeof ev.room === 'object') store.collabRoom = mergeEventRoom(store, ev.room as IdeCollabRoom);
      if (ev.type === 'peer_left') {
        // A guest left (or its seat was freed after 90 s without a stream): drop its cursor.
        const gone = String(ev.peer_id ?? '');
        store.peerCursors = store.peerCursors.filter((p) => p.peer_id !== gone);
        if (store.followingPeerId === gone) store.followingPeerId = null;
        return;
      }
      if (ev.type === 'cursor_moved' || ev.kind === 'cursor' || ev.type === 'cursor') {
        const c = ev as unknown as IdePeerCursor;
        if (c.peer_id && c.peer_id !== store.collabPeerId) {
          store.peerCursors = [...store.peerCursors.filter((p) => p.peer_id !== c.peer_id), c];
          if (store.followingPeerId === c.peer_id && c.path) {
            void store.openFile(c.path, { line: c.active_line || 1, preview: true });
          }
        }
      } else if (ev.type === 'buffer_updated' || ev.kind === 'edit' || ev.type === 'edit') {
        const editPath = String(ev.path ?? '');
        const content = String(ev.content ?? '');
        if (editPath && ev.peer_id !== store.collabPeerId) {
          const tab = store.tabs.find((t) => t.path === editPath);
          // Text typed here and not yet in the room is merged with the peer's, not overwritten.
          const base = store.sharedBuffers[editPath]?.content;
          const mine = tab ? (store.getBuffer?.(editPath) ?? tab.content) : content;
          const merged = base === undefined ? content : mergeText(base, mine, content);
          store.sharedBuffers = { ...store.sharedBuffers, [editPath]: { content, version: Number(ev.version ?? 0) } };
          store.clearLiveBuffer?.(editPath);
          if (tab) {
            tab.content = merged ?? content;
            if (merged === null) toasts.info(`A peer edited the same lines of ${ideBasename(editPath)}: their version is kept`);
            else if (merged !== content) void store.sendCollabEdit?.(editPath, merged);
            if (ev.saved) {
              tab.savedContent = content;
              tab.sha256 = typeof ev.sha256 === 'string' ? ev.sha256 : undefined;
              tab.diskEols = undefined; // a peer's save: its per-line endings are unknown here
            }
            tab.dirty = tab.content !== tab.savedContent;
          } else if (store.collabIsGuest && store.tabs.length === 0) {
            void store.openFile(editPath);
          }
        }
      } else if (ev.type === 'debug_updated' || ev.active_debug || ev.session) {
        store.applyDebugSnapshot((ev.active_debug ?? ev.session) as IdeDebugSessionSnapshot);
      }
    } catch {
      /* ignore heartbeat or malformed event */
    }
  });
  return ctl;
}

export async function startCollabSession(
  store: CollabHostState,
  defaultRole: IdeCollabRole,
  title: string | undefined,
  prevCtl: AbortController | null,
): Promise<AbortController | null> {
  if (!store.target.root) return prevCtl;
  store.collabLoading = true;
  store.collabError = null;
  try {
    const resp = await api.post<{
      room: IdeCollabRoom;
      invite_token: string;
      share_hash: string;
      host_peer_id?: string;
      host_peer_secret?: string;
    }>('/ide/collab/rooms', {
      target: store.targetPayload,
      title: title || ideBasename(store.target.root) || 'IDE Session',
      default_role: defaultRole,
    });
    store.collabRoom = resp.room;
    store.collabInviteToken = resp.invite_token;
    store.collabShareHash =
      resp.share_hash || `#/ide?collab=${encodeURIComponent(resp.room.id)}&token=${encodeURIComponent(resp.invite_token)}`;
    store.collabIsGuest = false;
    store.collabGuestToken = resp.invite_token;
    const hostPeerId = resp.host_peer_id ?? null;
    const hostPeerSecret = resp.host_peer_secret ?? null;
    store.collabPeerId = hostPeerId;
    store.collabPeerSecret = hostPeerSecret;
    if (hostPeerId && hostPeerSecret) {
      for (let i = 0; i < store.tabs.length; i++) {
        const tab = store.tabs[i];
        if (tab.loading) continue;
        try {
          const seeded = await collabGuestPost<{ version: number }>(
            `/ide/collab/guest/${encodeURIComponent(resp.room.id)}/edit`,
            resp.invite_token,
            { peer_id: hostPeerId, peer_secret: hostPeerSecret, path: tab.path, content: tab.content, save: false },
          );
          store.sharedBuffers = { ...store.sharedBuffers, [tab.path]: { content: tab.content, version: seeded.version } };
        } catch {
          /* best-effort initial buffer seed */
        }
      }
      return attachCollabEventStream(store, resp.room.id, resp.invite_token, hostPeerId, prevCtl);
    }
    return prevCtl;
  } catch (e) {
    store.collabError = loadErrorText(e);
    toastError('Couldn’t start shared session', e);
    return prevCtl;
  } finally {
    store.collabLoading = false;
  }
}

export async function joinCollabSession(
  store: CollabHostState,
  roomId: string,
  token: string,
  displayName: string,
  prevCtl: AbortController | null,
): Promise<AbortController | null> {
  store.collabIsGuest = true;
  ideCollabNotices.reset();
  store.collabLoading = true;
  store.collabError = null;
  store.collabGuestToken = token;
  try {
    const joined = await collabGuestPost<{ peer_id: string; peer_secret: string; room: IdeCollabRoom }>(
      `/ide/collab/guest/${encodeURIComponent(roomId)}/join`,
      token,
      { display_name: displayName, token },
    );
    store.collabPeerId = joined.peer_id;
    store.collabPeerSecret = joined.peer_secret;
    store.collabRoom = joined.room;
    store.target = joined.room.target;
    const stateResp = await laneFetch(
      'int',
      `/ide/collab/guest/${encodeURIComponent(roomId)}/state`,
      { headers: { 'X-Ide-Collab-Token': token } },
    );
    if (stateResp.ok) {
      const st = (await stateResp.json()) as {
        room: IdeCollabRoom;
        shared_buffers: Record<string, { content: string; version: number }>;
        active_debug?: IdeDebugSessionSnapshot | null;
      };
      store.collabRoom = st.room ?? store.collabRoom;
      store.sharedBuffers = st.shared_buffers ?? {};
      if (st.active_debug) store.applyDebugSnapshot(st.active_debug);
      const firstBuf = Object.keys(store.sharedBuffers)[0];
      if (firstBuf) await store.openFile(firstBuf);
    }
    return attachCollabEventStream(store, roomId, token, joined.peer_id, prevCtl);
  } catch (e) {
    const msg = loadErrorText(e);
    if (store.onCollabJoinFailed) {
      store.onCollabJoinFailed(msg);
    } else {
      store.collabRoom = null;
      store.collabIsGuest = false;
      store.collabPeerId = null;
      store.collabPeerSecret = null;
      store.collabGuestToken = null;
      store.peerCursors = [];
      store.followingPeerId = null;
      store.sharedBuffers = {};
      store.collabError = msg;
    }
    toastError('Couldn’t join shared session', e);
    return prevCtl;
  } finally {
    store.collabLoading = false;
  }
}

export function followCollabPeer(store: CollabHostState, peerId: string | null): void {
  store.followingPeerId = peerId;
  if (!peerId) return;
  const peer = store.peerCursors.find((c) => c.peer_id === peerId) ?? store.collabRoom?.peers.find((p) => p.peer_id === peerId);
  const path = peer && 'path' in peer ? peer.path : (peer as IdeCollabPeer | undefined)?.active_path;
  const line = (peer as { active_line?: number | null } | undefined)?.active_line ?? 1;
  if (path) void store.openFile(path, { line: line || 1, preview: true });
}

export async function loadIdePlugins(store: CollabHostState): Promise<void> {
  store.pluginsLoading = true;
  store.pluginsError = null;
  try {
    const resp = await api.get<{ plugins: IdePluginInfo[]; catalog: IdePluginCatalogEntry[] }>('/ide/plugins');
    store.plugins = resp.plugins ?? [];
    store.pluginCatalog = resp.catalog ?? [];
  } catch (e) {
    store.pluginsError = loadErrorText(e);
  } finally {
    store.pluginsLoading = false;
  }
}

export async function installIdePlugin(store: CollabHostState, source: string): Promise<boolean> {
  const clean = source.trim();
  if (!clean) return false;
  try {
    await api.long.post<{ plugin: IdePluginInfo }>('/ide/plugins/install', { source: clean });
    await loadIdePlugins(store);
    toasts.success('IDE plugin installed');
    return true;
  } catch (e) {
    toastError('Couldn’t install IDE plugin', e);
    return false;
  }
}
