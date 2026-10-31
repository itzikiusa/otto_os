///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
import { plural } from '../../lib/plural';
import { toastError } from '../../lib/toastError';
import { onTabKey } from './tabKeys';
// One design, open (canvas/studio archetype):
//
//   PageHeader: Design Hall › Project › Title · status ▾ · vN      Compare  ⋯  [Save]
//   ┌ stage toolbar + ArtifactStage ────────────────────────────┬ Otto | Links | References ┐
//   │  existing viewers/editors per format                      │                          │
//   └───────────────────────────────────────────────────────────┴──────────────────────────┘
//   (a `scene3d` design opens the 3D Studio layout instead — studio3d/Studio3D.svelte —
//    over the same working copy, save path and Otto/Links/References panels)
//   version strip: v1 · v2 (Otto) · v3 (you, current) …                  Compare
//
// This view owns the working copy of the source, the base version it was
// loaded from, and every save: `PUT …/content` with `base_version`, so a head
// that moved (an agent, another window) comes back as 409 and the person
// chooses — never a silent overwrite. Versions are the undo model: restore
// saves old bytes as a NEW version. Live `design_*` events refresh what they
// touch; every loader is sequence-guarded against stale responses. The UI
// posts the decisions it makes (restore, keep/replace vs an agent version) as
// design signals; approve/status/edit-after-draft signals are recorded by the
// daemon itself.
import { tick, untrack } from 'svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Icon from '../../lib/components/Icon.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { loadErrorText } from '../../lib/loadError';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { popoutItems } from '../../lib/popoutMenu';
import { confirmer } from '../../lib/confirm.svelte';
import { toasts } from '../../lib/toast.svelte';
import { router } from '../../lib/router.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { registry } from '../../lib/commands.svelte';
import { designBus } from '../../lib/events.svelte';
import { ApiError } from '../../lib/api/client';
import { downloadText } from '../../lib/components/exporters';
import * as api from '../../lib/api/design';
import type {
    DesignArtifact, DesignArtifactDetail, DesignArtifactFormat,
    DesignLinksResp, DesignStatus, DesignVersion,
  } from '../../lib/api/types';
import { DEVICES, type DeviceKind } from '../../lib/design/DeviceFrame.svelte';
import Studio3D from './studio3d/Studio3D.svelte';
import ArtifactStage from './ArtifactStage.svelte';
import VersionStrip from './VersionStrip.svelte';
import CompareModal, { type CompareSide } from './CompareModal.svelte';
import LinksPanel from './LinksPanel.svelte';
import ReferencesPanel from './ReferencesPanel.svelte';
import OttoPanel from './assist/OttoPanel.svelte';
import type { AssistSelection } from './assist/model';
import StatusPill from './StatusPill.svelte';
import StudioBadge from './StudioBadge.svelte';
import { formatLabel, isTextFormat, newerHeadAfterTypedLoad, renderKind, seqLookup, splitLinks, statusLabel, studioInfo } from './model';
import { library } from './library.svelte';
import { originOf } from './nav';
import { guardUnsaved } from '../../lib/leaveGuard';
;

  interface Props {
    id: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { id }: Props = $props();

  // ── State ─────────────────────────────────────────────────────────────────
  let detail = $state<DesignArtifactDetail | null>(null);
  let phase = $state<'loading' | 'ready' | 'error' | 'gone'>('loading');
  let loadError = $state<string | null>(null);
  /** Working copy (text formats) and what it was loaded from. */
  let source = $state<string | null>(null);
  let baseSource = $state<string | null>(null);
  let baseVersionId = $state<string | null>(null);
  let blobUrl = $state<string | null>(null);
  let saving = $state(false);
  /** A newer head arrived while there were unsaved edits. */
  let newerHead = $state<string | null>(null);

  let versions = $state<DesignVersion[]>([]);
  let versionsLoading = $state(false);
  let versionsError = $state<string | null>(null);
  let links = $state<DesignLinksResp | null>(null);
  let linksLoading = $state(false);
  let linksError = $state<string | null>(null);

  let selected = $state<string[]>([]);
  let compare = $state<{ left: CompareSide; right: CompareSide } | null>(null);
  let detailsOpen = $state(false);
  let detailsButton: HTMLButtonElement | undefined = $state();
  let designButton: HTMLButtonElement | undefined = $state();
  async function setDetails(open: boolean): Promise<void> {
    detailsOpen = open;
    await tick();
    const target = open ? designButton : detailsButton;
    if (target?.getClientRects().length) target.focus();
  }
  let rightTab = $state<'otto' | 'links' | 'references'>('links');
  let showSource = $state(false);
  let device = $state<DeviceKind>('none');

  const artifact = $derived<DesignArtifact | null>(detail?.artifact ?? null);
  const kind = $derived(artifact ? renderKind(artifact.format) : 'other');
  const textual = $derived(!!artifact && isTextFormat(artifact.format));
  const origin = $derived(artifact ? originOf(artifact) : null);
  /** Imported rows mirror Product/Canvas — they're edited at the origin (or forked). */
  const imported = $derived(!!artifact?.source_kind);
  const canEdit = $derived(!!artifact && auth.can('design', 'edit') && artifact.status !== 'archived');
  const readonly = $derived(!canEdit || imported || !textual);
  const dirty = $derived(textual && source !== null && source !== baseSource);
  // Leaving the page (sidebar, link, back) with unsaved source edits asks first.
  $effect(() => guardUnsaved(() => dirty, { what: 'this design' }));
  const head = $derived<DesignVersion | null>(detail?.head ?? null);
  const project = $derived(library.projectOf(artifact?.project_id));
  /** Seqs of OTHER artifacts' versions that links pin (resolved lazily below). */
  let pinnedSeqs = $state<Record<string, number>>({});
  const ownSeq = $derived(seqLookup(versions));
  const seqOf = $derived((vid: string): number | null => ownSeq(vid) ?? pinnedSeqs[vid] ?? null);
  const split = $derived(artifact ? splitLinks(artifact.id, links?.links ?? [], links?.artifacts ?? [], storyLabel) : { uses: [], usedIn: [] });
  const meId = $derived(auth.me?.id ?? null);
  const brief = $derived(typeof artifact?.meta?.brief === 'string' ? (artifact.meta.brief as string) : null);

  function storyLabel(kind: string, dstId: string): string {
    if (kind === 'story') {
      const s = library.stories[dstId];
      return s ? `${s.source_key} · ${s.title}` : 'A product story';
    }
    return dstId;
  }

  // ── Loading (sequence-guarded) ────────────────────────────────────────────
  let loadSeq = 0;
  let versionsSeq = 0;
  let linksSeq = 0;

  function setBlob(u: string | null): void {
    if (blobUrl && blobUrl !== u) URL.revokeObjectURL(blobUrl);
    blobUrl = u;
  }

  async function load(target: string): Promise<void> {
    const my = ++loadSeq;
    const same = detail?.artifact.id === target;
    // Snapshot the editor: a live >64 KB `content` event or a resync reloads
    // while the user may keep typing. If the text moved during the fetch, the
    // fetched head must NOT replace it — flag it as a newer version instead
    // (S18-09). An explicit discard/"load theirs" sets `source` BEFORE calling,
    // so its snapshot matches and the load applies.
    const typedAt = same ? source : null;
    phase = same ? phase : 'loading';
    try {
      const d = await api.getArtifact(target, { content: true });
      if (my !== loadSeq) return;
      let text: string | null = null;
      if (isTextFormat(d.artifact.format)) {
        if (d.content != null && !d.content_truncated) text = d.content;
        else if (d.head) text = (await api.fetchContent(target, { asText: true })).text;
        if (my !== loadSeq) return;
        setBlob(null);
      } else if (d.head) {
        const c = await api.fetchContent(target, { asText: false });
        if (my !== loadSeq) {
          if (c.blobUrl) URL.revokeObjectURL(c.blobUrl);
          return;
        }
        setBlob(c.blobUrl);
      }
      if (same && source !== typedAt) {
        detail = d;
        newerHead = newerHeadAfterTypedLoad(d.content_version_id ?? d.head?.id ?? null, baseVersionId);
        phase = 'ready';
        loadError = null;
        return;
      }
      detail = d;
      source = text;
      baseSource = text;
      baseVersionId = d.content_version_id ?? d.head?.id ?? null;
      newerHead = null;
      phase = 'ready';
      loadError = null;
    } catch (e) {
      if (my !== loadSeq) return;
      if (e instanceof ApiError && e.status === 404) phase = 'gone';
      else {
        phase = 'error';
        loadError = loadErrorText(e);
      }
    }
  }

  /** Metadata only (status / approved / title) — keeps the working copy. */
  // Its own sequence: a metadata refresh must never cancel an in-flight full
  // load (which would leave the view stuck on its skeleton).
  let metaSeq = 0;
  async function refreshMeta(): Promise<void> {
    const my = ++metaSeq;
    const loadAt = loadSeq;
    try {
      const d = await api.getArtifact(id);
      if (my !== metaSeq || loadAt !== loadSeq || !detail) return;
      detail = { ...d, content: detail.content, content_version_id: detail.content_version_id };
    } catch {
      /* the next full load reports errors */
    }
  }

  async function loadVersions(): Promise<void> {
    const my = ++versionsSeq;
    versionsLoading = true;
    try {
      const v = await api.listVersions(id);
      if (my !== versionsSeq) return;
      versions = v;
      versionsError = null;
      selected = selected.filter((s) => v.some((x) => x.id === s));
    } catch (e) {
      if (my === versionsSeq) versionsError = loadErrorText(e);
    } finally {
      if (my === versionsSeq) versionsLoading = false;
    }
  }

  /**
   * A live `content` event only ADDS versions at the head: fetch the newest
   * few and prepend the unseen ones instead of re-listing the whole history.
   * No overlap with what we hold (a gap, or nothing loaded yet) → full reload.
   */
  async function loadNewVersions(): Promise<void> {
    if (versions.length === 0) return loadVersions();
    const my = ++versionsSeq;
    try {
      const v = await api.listVersions(id, { limit: 10 });
      if (my !== versionsSeq) return;
      const have = new Set(versions.map((x) => x.id));
      const fresh = v.filter((x) => !have.has(x.id));
      if (fresh.length === v.length) return loadVersions();
      if (fresh.length) versions = [...fresh, ...versions];
      versionsError = null;
    } catch {
      if (my === versionsSeq) return loadVersions();
    } finally {
      // This bumped versionsSeq, so a superseded full load won't clear it.
      if (my === versionsSeq) versionsLoading = false;
    }
  }

  async function loadLinks(): Promise<void> {
    const my = ++linksSeq;
    linksLoading = true;
    try {
      const l = await api.getLinks(id, 'both');
      if (my !== linksSeq) return;
      links = l;
      linksError = null;
    } catch (e) {
      if (my === linksSeq) linksError = loadErrorText(e);
    } finally {
      if (my === linksSeq) linksLoading = false;
    }
  }

  // Pinned links name a version of the TARGET ("pinned v9 · v10 available"):
  // resolve those seqs from the targets' version lists (a few targets at most).
  const triedTargets = new Set<string>();
  $effect(() => {
    const l = links;
    if (!l) return;
    const targets = new Set<string>();
    for (const link of l.links) {
      if (!link.pinned_version_id || link.dst_kind !== 'artifact' || link.src_artifact_id !== id) continue;
      if (untrack(() => ownSeq(link.pinned_version_id!) ?? pinnedSeqs[link.pinned_version_id!]) != null) continue;
      if (!triedTargets.has(link.dst_id)) targets.add(link.dst_id);
    }
    for (const t of [...targets].slice(0, 8)) {
      triedTargets.add(t);
      void api.listVersions(t).then(
        (vs) => (pinnedSeqs = { ...pinnedSeqs, ...Object.fromEntries(vs.map((v) => [v.id, v.seq])) }),
        () => {},
      );
    }
  });

  // (Re)load everything when the route's id changes.
  $effect(() => {
    const target = id;
    untrack(() => {
      detail = null;
      links = null;
      versions = [];
      selected = [];
      compare = null;
      // `#/design/a/<id>/otto` (the lobby's Generate hand-off) opens on Otto.
      rightTab = router.parts[3] === 'otto' ? 'otto' : 'links';
      detailsOpen = router.parts[3] === 'otto';
      phase = 'loading';
      void load(target);
      void loadVersions();
      void loadLinks();
      if (!library.loaded) void library.load();
    });
  });
  $effect(() => () => setBlob(null));

  // ── Live updates ──────────────────────────────────────────────────────────
  let seen = designBus.seq;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      const evs = designBus.since(seen);
      seen = now;
      let reloadLinks = false;
      for (const ev of evs) {
        if (ev.type === 'design_artifact_updated' && ev.artifact_id === id) {
          if (ev.change === 'deleted') {
            phase = 'gone';
            continue;
          }
          if (ev.change === 'content' || ev.change === 'created') {
            void loadNewVersions();
            if (ev.version_id && ev.version_id === baseVersionId) continue; // our own save
            if (dirty) {
              newerHead = ev.version_id;
            } else if (ev.content != null && textual) {
              source = ev.content;
              baseSource = ev.content;
              baseVersionId = ev.version_id;
              void refreshMeta();
            } else {
              void load(id);
            }
          } else {
            void refreshMeta();
            if (ev.change === 'approved') void loadVersions();
          }
        } else if (ev.type === 'design_artifact_updated' && links?.artifacts.some((a) => a.id === ev.artifact_id)) {
          reloadLinks = true;
        } else if (ev.type === 'design_link_updated' && (ev.artifact_id === id || ev.target_artifact_id === id)) {
          reloadLinks = true;
        }
      }
      if (reloadLinks) void loadLinks();
    });
  });
  $effect(() => {
    const t = designBus.resyncTick;
    if (t === 0) return;
    untrack(() => {
      if (dirty) void loadVersions();
      else void load(id);
      void loadLinks();
    });
  });

  // ── Saving ────────────────────────────────────────────────────────────────
  async function save(message?: string, named = false): Promise<void> {
    if (!artifact || readonly || saving || source === null) return;
    if (!dirty && !named) return;
    saving = true;
    const body = { content: source, base_version: baseVersionId ?? '', ...(message ? { message } : {}) };
    try {
      const res = named ? await api.commitVersion(id, { ...body, message: message ?? '' }) : await api.putContent(id, body);
      applySaved(res, body.content);
      if (!res.created) toasts.info('No changes to save', 'The content matches the current version.');
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) await resolveConflict(named ? message : undefined);
      else toastError('Couldn’t save the design', e);
    } finally {
      saving = false;
    }
  }

  function applySaved(res: import('../../lib/api/types').DesignSaveResult, savedText = source): void {
    baseSource = savedText;
    baseVersionId = res.version.id;
    newerHead = null;
    if (detail) detail = { ...detail, artifact: res.artifact, head: res.version };
    if (res.created && !versions.some((v) => v.id === res.version.id)) versions = [res.version, ...versions];
    const r = res.links;
    if (r.broken.length || r.cycles.length) {
      const parts: string[] = [];
      if (r.broken.length) parts.push(plural(r.broken.length, 'broken otto://design reference'));
      if (r.cycles.length) parts.push(`${plural(r.cycles.length, 'embed')} skipped (would loop)`);
      toasts.warn('Saved with link problems', parts.join(' · '));
    }
    void loadLinks();
  }

  /**
   * The head moved since this copy was loaded. Show who wrote the newer
   * version and let the person pick: take theirs (drop local edits) or keep
   * theirs in history and save these edits on top. Both paths lose nothing
   * that was committed; the choice vs an AGENT version is a learning signal.
   */
  async function resolveConflict(namedMessage?: string): Promise<void> {
    let latest: DesignVersion | null = null;
    try {
      latest = (await api.listVersions(id, { limit: 1 }))[0] ?? null;
    } catch {
      /* fall through with an unknown author */
    }
    const byAgent = latest?.author_kind === 'agent';
    const who = latest ? `v${latest.seq}${byAgent ? ' by Otto' : ''}` : 'a newer version';
    const { value } = await confirmer.choose(
      `Someone saved ${who} while you were editing. Your edits aren’t saved yet.`,
      {
        title: 'This design changed',
        options: [
          { label: 'Save mine on top', value: 'mine', kind: 'primary' },
          { label: `Load ${latest ? `v${latest.seq}` : 'latest'} (discard my edits)`, value: 'theirs', kind: 'danger' },
        ],
      },
    );
    if (value === 'theirs') {
      if (byAgent && latest) {
        api.captureSignal({ artifact_id: id, kind: 'variant_chosen', version_id: latest.id, payload: { source: 'conflict', chosen_version_id: latest.id, chosen_seq: latest.seq, over: 'unsaved local edits' } });
      }
      await load(id);
      void loadVersions();
    } else if (value === 'mine') {
      if (!latest) {
        toasts.error('Couldn’t read the current version', 'Your edits are kept. Try Save again to retry.');
        return;
      }
      try {
        const body = { content: source ?? '', base_version: latest.id, ...(namedMessage ? { message: namedMessage } : {}) };
        const res = namedMessage ? await api.commitVersion(id, { ...body, message: namedMessage }) : await api.putContent(id, body);
        applySaved(res, body.content);
        if (byAgent) {
          api.captureSignal({ artifact_id: id, kind: 'variant_rejected', version_id: latest.id, payload: { source: 'conflict', rejected_version_id: latest.id, rejected_seq: latest.seq, kept_version_id: res.version.id, reason: 'kept my edits' } });
        }
        void loadVersions();
      } catch (e) {
        toastError('Couldn’t save the design', e);
      }
    }
  }

  async function saveNamed(): Promise<void> {
    const msg = await confirmer.promptText('Describe this version (shown in history).', {
      title: 'Save named version',
      confirmLabel: 'Save version',
      placeholder: 'Hero v2 — bolder headline',
    });
    if (msg) await save(msg, true);
  }

  // ── Restore (from Compare) ────────────────────────────────────────────────
  async function restore(versionId: string, seq: number): Promise<void> {
    if (!artifact || !head) return;
    if (dirty) {
      toasts.warn('Save or discard your edits first', 'Restoring replaces the working copy.');
      return;
    }
    const next = (versions[0]?.seq ?? head.seq) + 1;
    const ok = await confirmer.ask(
      `Restore v${seq}? Its content is saved as a new version v${next}. v${head.seq} and every other version stay in history — nothing is lost.`,
      { title: 'Restore version', confirmLabel: `Restore v${seq}`, danger: false },
    );
    if (!ok) return;
    try {
      // Server-side: the old blob is re-committed as-is (no download +
      // base64 re-upload of the bytes through the webview).
      const res = await api.restoreVersion(id, versionId, { base_version: head.id, message: `Restored v${seq}` });
      api.captureSignal({
        artifact_id: id,
        kind: 'restored',
        version_id: res.version.id,
        payload: { from_version_id: versionId, from_seq: seq, over_version_id: head.id, over_seq: head.seq, new_seq: res.version.seq },
      });
      compare = null;
      selected = [];
      await load(id);
      void loadVersions();
      toasts.success(`Restored v${seq} as v${res.version.seq}`);
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) toasts.error('Couldn’t restore', 'The design changed meanwhile. Refresh and try again.');
      else toastError('Couldn’t restore', e);
    }
  }

  // ── Status (approve is human-only and explicit) ───────────────────────────
  function statusMenu(e: MouseEvent): void {
    if (!artifact) return;
    const cur = artifact.status;
    const set = (s: DesignStatus) => void setStatus(s);
    const items: MenuItem[] = [
      { label: 'Draft', icon: 'edit', disabled: cur === 'draft' || !canEdit, action: () => set('draft') },
      { label: 'In review', icon: 'eye', disabled: cur === 'review' || !canEdit, action: () => set('review') },
      {
        label: dirty ? 'Approve (save first)' : `Approve v${head?.seq ?? ''}…`,
        icon: 'check',
        disabled: !canEdit || dirty || !head || (cur === 'approved' && artifact.approved_version_id === head?.id),
        action: () => void approve(),
      },
      { label: 'Shipped', icon: 'send', disabled: cur === 'shipped' || !canEdit, action: () => set('shipped') },
    ];
    ctxMenu.show(e, items);
  }

  async function setStatus(s: DesignStatus): Promise<void> {
    try {
      const a = await api.updateArtifact(id, { status: s });
      if (detail) detail = { ...detail, artifact: a };
    } catch (e) {
      toastError(`Couldn’t move to ${statusLabel(s)}`, e);
    }
  }

  async function approve(): Promise<void> {
    if (!artifact || !head) return;
    // Approval belongs to the version the person sees in this sheet. Live
    // content updates may move head while the confirmation is open.
    const target = id;
    const version = head;
    const followers = split.usedIn.filter((r) => r.link.policy === 'follow_approved').length;
    const ok = await confirmer.ask(
      `Approve v${version.seq} of “${artifact.title}”?` +
        (followers ? ` ${plural(followers, 'design')} that follow its approved version will update to v${version.seq}.` : ''),
      { title: 'Approve design', confirmLabel: `Approve v${version.seq}`, danger: false },
    );
    if (!ok) return;
    try {
      const a = await api.approveArtifact(target, version.id);
      if (detail) detail = { ...detail, artifact: a, approved: version };
      const follow = split.usedIn.filter((r) => r.link.policy === 'follow_approved').map((r) => r.label);
      const pinned = split.usedIn.filter((r) => r.link.policy === 'pinned').map((r) => r.label);
      const parts = [
        follow.length ? `${follow.join(', ')} ${follow.length === 1 ? 'follows' : 'follow'} Approved → now shows v${version.seq}` : '',
        pinned.length ? `${pinned.join(', ')} ${pinned.length === 1 ? 'pins' : 'pin'} an older version (update available)` : '',
      ].filter(Boolean);
      toasts.success(`Approved v${version.seq}`, parts.join(' · ') || undefined);
    } catch (e) {
      toastError('Couldn’t approve', e);
    }
  }

  // ── Header ⋯ actions ──────────────────────────────────────────────────────
  async function rename(): Promise<void> {
    if (!artifact) return;
    const t = await confirmer.promptText('New title', { title: 'Rename design', confirmLabel: 'Rename', initial: artifact.title });
    if (!t || t === artifact.title) return;
    try {
      const a = await api.updateArtifact(id, { title: t });
      if (detail) detail = { ...detail, artifact: a };
    } catch (e) {
      toastError('Couldn’t rename', e);
    }
  }

  function moveMenu(e: MouseEvent): void {
    const items: MenuItem[] = library.projects
      .filter((p) => !p.archived)
      .map((p) => ({ label: p.name, icon: 'folder', disabled: p.id === artifact?.project_id, action: () => void moveTo(p.id) }));
    items.push({ separator: true }, { label: 'Unfiled', icon: 'archive', disabled: !artifact?.project_id, action: () => void moveTo('') });
    ctxMenu.show(e, items, { filter: items.length > 10, maxVisible: 12 });
  }
  async function moveTo(projectId: string): Promise<void> {
    try {
      const a = await api.updateArtifact(id, { project_id: projectId });
      if (detail) detail = { ...detail, artifact: a };
      void library.load();
    } catch (e) {
      toastError('Couldn’t move the design', e);
    }
  }

  async function duplicateHere(): Promise<void> {
    if (!artifact) return;
    try {
      const res = await api.createArtifact({
        workspace_id: artifact.workspace_id,
        format: artifact.format as DesignArtifactFormat,
        studio: artifact.studio,
        title: `${artifact.title} (copy)`,
        project_id: artifact.project_id ?? undefined,
        derived_from: { artifact_id: artifact.id, version_id: head?.id },
        message: `Copied from ${artifact.title}`,
      });
      api.captureSignal({
        artifact_id: res.artifact.id,
        kind: 'forked',
        version_id: res.version.id,
        payload: { source_artifact_id: artifact.id, source_version_id: head?.id, source: 'copy' },
      });
      router.go(`design/a/${encodeURIComponent(res.artifact.id)}`);
    } catch (e) {
      toastError('Couldn’t make a copy', e);
    }
  }

  async function archive(): Promise<void> {
    if (!artifact) return;
    const ok = await confirmer.ask(
      `Archive “${artifact.title}”? It leaves the lobby and search; every version is kept and links to it keep working.`,
      { title: 'Archive design', confirmLabel: 'Archive', danger: false },
    );
    if (!ok) return;
    try {
      await api.archiveArtifact(id);
      toasts.success('Design archived');
      router.go('design');
    } catch (e) {
      toastError('Couldn’t archive', e);
    }
  }

  function download(): void {
    if (!artifact) return;
    if (source !== null) {
      const ext = ({ html: 'html', svg: 'svg', mermaid: 'mmd', d2: 'd2' } as Record<string, string>)[artifact.format] ?? 'json';
      downloadText(source, `${artifact.title.replace(/[^\w.-]+/g, '-')}.${ext}`, artifact.mime || 'text/plain');
    } else if (blobUrl) {
      const a = document.createElement('a');
      a.href = blobUrl;
      a.download = artifact.title;
      a.click();
    }
  }

  async function discard(): Promise<void> {
    const ok = await confirmer.ask('Discard your unsaved edits? The design goes back to the last saved version.', {
      title: 'Discard edits',
      danger: true, confirmLabel: 'Discard',
    });
    if (ok) {
      source = baseSource;
      if (newerHead) void load(id);
    }
  }

  // ── Compare ───────────────────────────────────────────────────────────────
  function toggleSelect(vid: string): void {
    if (selected.includes(vid)) selected = selected.filter((s) => s !== vid);
    else selected = [...selected, vid].slice(-2);
  }
  function openCompare(): void {
    if (!artifact || !head || !versions.length) return;
    const bySeq = (x: string) => versions.find((v) => v.id === x)?.seq ?? 0;
    let [a, b] = selected.length >= 2 ? [...selected].sort((x, y) => bySeq(x) - bySeq(y)) : [selected[0], head.id];
    if (!a) a = versions[1]?.id ?? head.id;
    if (a === b) b = head.id;
    compare = { left: { artifact, versionId: a }, right: { artifact, versionId: b } };
  }
  function compareWithRef(ref: DesignArtifact): void {
    if (!artifact || !head) return;
    const refVersion = ref.approved_version_id ?? ref.head_version_id;
    if (!refVersion) return;
    compare = { left: { artifact: ref, versionId: refVersion }, right: { artifact, versionId: head.id } };
  }
  function comparePinned(target: DesignArtifact, pinned: string): void {
    if (!target.head_version_id) return;
    compare = { left: { artifact: target, versionId: pinned }, right: { artifact: target, versionId: target.head_version_id } };
  }

  // What the Otto tab focuses a turn on: the 3D Studio passes its selected
  // object (the `ottoPanel` snippet's argument); Site Studio's section
  // selection plugs in the same way when it lands.
  const assistBlocked = $derived(
    !canEdit
      ? 'You can view this design, but asking Otto to change it needs edit access.'
      : imported
        ? 'Mirrored from Product/Canvas — make an editable copy to work on it with Otto.'
        : !textual
          ? 'Otto edits text and JSON designs. Images, PDFs and 3D models can’t be changed by an agent.'
          : null,
  );

  // ── ⌘K commands (only while a design is open) ─────────────────────────────
  $effect(() => {
    if (!artifact) return registry.register('design-artifact', []);
    return registry.register('design-artifact', [
      { id: 'design.save', title: 'Save design', group: 'Design Hall', shortcut: '⌘S', keywords: 'version commit', run: () => void save() },
      { id: 'design.named', title: 'Save named version…', group: 'Design Hall', keywords: 'commit message', run: () => void saveNamed() },
      { id: 'design.compare', title: 'Compare versions', group: 'Design Hall', keywords: 'diff history restore', run: openCompare },
      { id: 'design.references', title: 'Find references', group: 'Design Hall', keywords: 'inspiration library search', run: () => { rightTab = 'references'; void setDetails(true); } },
      { id: 'design.otto', title: 'Ask Otto about this design', group: 'Design Hall', keywords: 'agent assist variants accessibility refine', run: () => { rightTab = 'otto'; void setDetails(true); } },
    ]);
  });

  // ⌘S saves while focus is anywhere inside this view (editor, panels) — a
  // scoped listener, not a global shortcut (plain ⌘S stays free elsewhere).
  let rootEl = $state<HTMLDivElement | null>(null);
  function onKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 's') {
      e.preventDefault();
      void save();
    }
  }
  $effect(() => {
    const el = rootEl;
    if (!el) return;
    el.addEventListener('keydown', onKeydown);
    return () => el.removeEventListener('keydown', onKeydown);
  });

  function moreMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'Save named version…', icon: 'commit', disabled: readonly, action: () => void saveNamed() },
      { label: 'Discard edits…', icon: 'refresh', disabled: !dirty, action: () => void discard() },
      { label: 'Compare versions…', icon: 'columns', disabled: versions.length < 2, action: openCompare },
      { separator: true },
      { label: 'Rename…', icon: 'edit', disabled: !canEdit, action: () => void rename() },
      { label: 'Move to project…', icon: 'folder', disabled: !canEdit, action: () => queueMicrotask(() => moveMenu(e)) },
      { label: 'Make an editable copy', icon: 'copy', action: () => void duplicateHere() },
      { label: 'Download', icon: 'download', action: download },
      ...(artifact ? popoutItems(`design/a/${encodeURIComponent(artifact.id)}`, artifact.title) : []),
      { separator: true },
      { label: 'Archive…', icon: 'archive', danger: true, disabled: !canEdit, action: () => void archive() },
    ];
    ctxMenu.show(e, items);
  }

  // An unfiled design has no project crumb: "Unfiled" only led back to the
  // lobby (the first crumb already does) and ate the title's room.
  const crumbs = $derived([
    { label: 'Design Hall', onclick: () => router.go('design') },
    ...(project ? [{ label: project.name, onclick: () => router.go(`design/p/${encodeURIComponent(project.id)}`) }] : []),
  ]);
;
async () => {

 { const $$_div0 = svelteHTML.createElement("div", {  "class":`view`,});rootEl = $$_div0;
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "title":artifact?.title ?? 'Design',"crumbs":viewport.isPhone ? [] : crumbs,leading:() => { async ()/*Ωignore_positionΩ*/ => {
      if(viewport.isPhone){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => router.go(project ? `design/p/${encodeURIComponent(project.id)}` : 'design'),"aria-label":`Back`,"title":`Back`,});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});}
         }
      }
    };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
      if(artifact){
         { svelteHTML.createElement("span", { "class":`badges`,});
          if(!viewport.isPhone){ { const $$_egdaBoidutS1C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS1C({ target: __sveltets_2_any(), props: {  "studio":artifact.studio,}});}}
           { const $$_lliPsutatS1C = __sveltets_2_ensureComponent(StatusPill); new $$_lliPsutatS1C({ target: __sveltets_2_any(), props: {    "status":artifact.status === 'approved' && head?.id !== artifact.approved_version_id ? 'draft' : artifact.status,"onclick":statusMenu,}});}
          if(head && !viewport.isPhone){ { svelteHTML.createElement("span", {   "class":`ver`,"title":`Current version: v${head.seq}`,}); head.seq; }}
          if(dirty && !viewport.isPhone){ { svelteHTML.createElement("span", {   "class":`chip edited`,...__sveltets_2_empty({"data-testid":`design-dirty`}),});  }}
         }
      }
    };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
      if(artifact){
        
        if(!readonly){
           { svelteHTML.createElement("button", {         "class":`btn small primary`,"onclick":() => void save(),"disabled":!dirty || saving,"title":`Save a new version (⌘S)`,...__sveltets_2_empty({"data-testid":`design-save`}),});
            saving ? 'Saving…' : 'Save';
           }
        }
         { svelteHTML.createElement("button", {                 "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-10`}),...__sveltets_2_empty({"data-icon":`more`}),...__sveltets_2_empty({"data-label":`More actions`}),"onclick":moreMenu,"aria-label":`More actions`,"title":`More actions`,"aria-haspopup":`menu`,...__sveltets_2_empty({"data-testid":`design-more`}),});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, badge, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
    
    
   PageHeader}

   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
    if(phase === 'loading'){
       { svelteHTML.createElement("div", { "class":`pad`,}); { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":4,"height":64,}});} }
    } else if (phase === 'gone'){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {            "variant":`page`,"icon":`designHall`,"title":`This design isn’t available`,"body":`It was deleted, or you don’t have access to its workspace.`,"actionLabel":`Back to Design Hall`,"onaction":() => router.go('design'),}});}
    } else if (phase === 'error' || !artifact){
       { svelteHTML.createElement("div", {   "class":`pad err`,"role":`alert`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":16,}});}
         { svelteHTML.createElement("div", {});
           { svelteHTML.createElement("strong", {});    }
           { svelteHTML.createElement("div", { "class":`dim`,});loadError; }
         }
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void load(id),});  }
       }
    }else{
        const notices/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
        if(newerHead){
           {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('span'),('A newer version was saved.')));{ svelteHTML.createElement("span", __sveltets_2_union($$action_0), {   "class":`notice warnc`,});
             { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}     
             { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":() => void discard(),});    }
           }}
        }
        if(imported){
           { svelteHTML.createElement("span", { "class":`notice`,});
             { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}   artifact.source_kind === 'canvas_scene' ? 'Canvas' : 'Product';         
            if(origin){ { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":origin.open,});origin.label; }}
             { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":() => void duplicateHere(),});    }
           }
        } else if (!canEdit){
           { svelteHTML.createElement("span", { "class":`notice`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});}  }
        }
      };return __sveltets_2_any(0)};
       const ottoPanel/*Ωignore_positionΩ*/ = (selection: AssistSelection | null)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
         { const $$_lenaPottO0C = __sveltets_2_ensureComponent(OttoPanel); new $$_lenaPottO0C({ target: __sveltets_2_any(), props: {           artifact,versions,head,"uses":split.uses,dirty,selection,"readonlyReason":assistBlocked,"oncompare":(left, right) => (compare = { left, right }),}});}
      };return __sveltets_2_any(0)};
        const linksPanel/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
         { const $$_lenaPskniL0C = __sveltets_2_ensureComponent(LinksPanel); new $$_lenaPskniL0C({ target: __sveltets_2_any(), props: {                artifact,"uses":split.uses,"usedIn":split.usedIn,"loading":linksLoading,"error":linksError,"readonly":!canEdit,seqOf,"onreload":() => void loadLinks(),"oncompare":comparePinned,}});}
      };return __sveltets_2_any(0)};
        const referencesPanel/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
         { const $$_lenaPsecnerefeR0C = __sveltets_2_ensureComponent(ReferencesPanel); new $$_lenaPsecnerefeR0C({ target: __sveltets_2_any(), props: {          artifact,"uses":split.uses,"readonly":!canEdit,seqOf,"onreload":() => void loadLinks(),"oncompare":compareWithRef,}});}
      };return __sveltets_2_any(0)};
      if(kind === 'scene3d'){
        artifact.id; {
           { const $$_D3oidutS2C = __sveltets_2_ensureComponent(Studio3D); new $$_D3oidutS2C({ target: __sveltets_2_any(), props: {                      artifact,source,readonly,dirty,head,"usedIn":split.usedIn,"linkCount":split.uses.length + split.usedIn.length,seqOf,"openOtto":rightTab === 'otto',"onchange":(s) => (source = s),"otto":ottoPanel,"links":linksPanel,"references":referencesPanel,notices,}});}
        }
      }else{
       { svelteHTML.createElement("div", {   "class":`studio`,});rightTab === 'otto';detailsOpen;
         { svelteHTML.createElement("section", {   "class":`center`,"aria-label":`Design`,});
           { svelteHTML.createElement("div", { "class":`toolbar`,});
            if(kind === 'html'){
               { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Device frame`,});
                   for(let d of __sveltets_2_ensureArray(DEVICES)){d.id;
                   { svelteHTML.createElement("button", {    "aria-pressed":device === d.id,"onclick":() => (device = d.id),});device === d.id;d.label; }
                }
               }
            }
            if(textual){
               { svelteHTML.createElement("button", {        "class":`btn small ghost`,"aria-pressed":showSource,"onclick":() => (showSource = !showSource),...__sveltets_2_empty({"data-testid":`design-source-toggle`}),});showSource;
                 { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});} 
               }
            }
             { svelteHTML.createElement("span", { "class":`fmt`,});formatLabel(artifact.format);  studioInfo(artifact.studio).name; }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { const $$_button5 = svelteHTML.createElement("button", {      "class":`btn small ghost compact-details`,"aria-label":`Show design details`,"onclick":() => void setDetails(true),});detailsButton = $$_button5; { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}  }
            ;__sveltets_2_ensureSnippet(notices());
           }
           { svelteHTML.createElement("div", { "class":`stage-host`,});
            artifact.id; {
               { const $$_egatStcafitrA5C = __sveltets_2_ensureComponent(ArtifactStage); new $$_egatStcafitrA5C({ target: __sveltets_2_any(), props: {            artifact,source,dirty,blobUrl,"noVersion":!head,readonly,showSource,device,"onchange":(s) => (source = s),}});}
            }
           }
         }
         { svelteHTML.createElement("aside", {   "class":`right`,"aria-label":`Design details`,});
           { const $$_button4 = svelteHTML.createElement("button", {    "class":`btn small ghost compact-details back-design`,"onclick":() => void setDetails(false),});designButton = $$_button4;   }
           { svelteHTML.createElement("div", {     "class":`tabs segmented`,"role":`tablist`,"aria-label":`Details panel`,});
             { svelteHTML.createElement("button", {                "role":`tab`,"id":`dh-tab-otto`,"aria-controls":`dh-panel`,"aria-selected":rightTab === 'otto',"tabindex":rightTab === 'otto' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (rightTab = 'otto'),...__sveltets_2_empty({"data-testid":`design-tab-otto`}),});rightTab === 'otto';
              
             }
             { svelteHTML.createElement("button", {                "role":`tab`,"id":`dh-tab-links`,"aria-controls":`dh-panel`,"aria-selected":rightTab === 'links',"tabindex":rightTab === 'links' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (rightTab = 'links'),...__sveltets_2_empty({"data-testid":`design-tab-links`}),});rightTab === 'links';
                { svelteHTML.createElement("span", { "class":`count`,});split.uses.length + split.usedIn.length; }
             }
             { svelteHTML.createElement("button", {                "role":`tab`,"id":`dh-tab-references`,"aria-controls":`dh-panel`,"aria-selected":rightTab === 'references',"tabindex":rightTab === 'references' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (rightTab = 'references'),...__sveltets_2_empty({"data-testid":`design-tab-references`}),});rightTab === 'references';
              
             }
           }
           { svelteHTML.createElement("div", {       "class":`panel`,"role":`tabpanel`,"id":`dh-panel`,"aria-labelledby":`dh-tab-${rightTab}`,});
            if(brief && rightTab !== 'otto'){
               { svelteHTML.createElement("div", { "class":`brief`,});
                 { svelteHTML.createElement("span", { "class":`k`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});}  }
                 { svelteHTML.createElement("p", {});brief; }
               }
            }
            if(rightTab === 'otto'){
              ;__sveltets_2_ensureSnippet(ottoPanel(null));
            } else if (rightTab === 'links'){
              ;__sveltets_2_ensureSnippet(linksPanel());
            }else{
              ;__sveltets_2_ensureSnippet(referencesPanel());
            }
           }
         }
       }
      }
       { const $$_pirtSnoisreV2C = __sveltets_2_ensureComponent(VersionStrip); new $$_pirtSnoisreV2C({ target: __sveltets_2_any(), props: {                  versions,"headId":head?.id ?? null,"approvedId":artifact.approved_version_id,meId,selected,"loading":versionsLoading,"error":versionsError,"onselect":toggleSelect,"oncompare":openCompare,"onretry":() => void loadVersions(),}});}
    }
   PageBody}
 }

if(compare && artifact){
   { const $$_ladoMerapmoC0C = __sveltets_2_ensureComponent(CompareModal); new $$_ladoMerapmoC0C({ target: __sveltets_2_any(), props: {              "left":compare.left,"right":compare.right,"currentId":artifact.id,"headId":head?.id ?? null,meId,"onclose":() => (compare = null),"onrestore":(v, s) => void restore(v, s),}});}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ArtifactView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ArtifactView__SvelteComponent_ = ReturnType<typeof ArtifactView__SvelteComponent_>;
/*Ωignore_endΩ*/export default ArtifactView__SvelteComponent_;