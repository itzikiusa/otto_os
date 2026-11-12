<script lang="ts">
  import { announceShown } from '../../lib/announce.svelte';
  import { plural } from '../../lib/plural';
  import { toastError } from '../../lib/toastError';
  // One design, open (canvas/studio archetype):
  //
  //   PageHeader: Design Hall › Project › Title · status ▾ · vN      Saved ✓  ⋯
  //   [mirror banner: "This design mirrors a story attachment" · Edit here]
  //   ┌ stage toolbar + ArtifactStage ────────────────────────────┬ Otto | Versions | Links ┐
  //   │  existing viewers/editors per format                      │ (DetailsPanel)          │
  //   └───────────────────────────────────────────────────────────┴─────────────────────────┘
  //   (a `scene3d` design opens the 3D Studio layout and an `otto-site` design
  //    Site Studio's own three-column layout instead — each over the same
  //    working copy, save path and Otto/Versions/Links panels)
  //
  // This view owns the working copy of the source, the base version it was
  // loaded from, and every save: an autosave ~1.5 s after the last edit
  // (`PUT …/content` with `base_version`; the daemon labels and squashes
  // autosaves) and ⌘S / ⋯ for a named version. A head that moved (an agent,
  // another window) comes back as 409 and the person chooses — never a silent
  // overwrite. Versions are the undo model: restore saves old bytes as a NEW
  // version. A mirror of a Product/Canvas source is read-only until "Edit
  // here" adopts it. Live `design_*` events refresh what they touch; every
  // loader is sequence-guarded against stale responses. The UI posts the
  // decisions it makes (restore, keep/replace vs an agent version) as design
  // signals; approve/status/edit-after-draft signals are recorded by the
  // daemon itself.
  import { tick, untrack, type Snippet } from 'svelte';
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
  import * as api from '../../lib/api/design';
  import type {
    DesignArtifact, DesignArtifactDetail, DesignArtifactFormat,
    DesignLinksResp, DesignStatus, DesignVersion,
  } from '../../lib/api/types';
  import { DEVICES, type DeviceKind } from '../../lib/design/DeviceFrame.svelte';
  import Studio3D from './studio3d/Studio3D.svelte';
  import SiteStudio from './site/SiteStudio.svelte';
  import ArtifactStage, { d2Svg } from './ArtifactStage.svelte';
  import VersionsPanel from './VersionsPanel.svelte';
  import DetailsPanel, { type DetailsTab } from './DetailsPanel.svelte';
  import LinksTab from './LinksTab.svelte';
  import MirrorBanner from './MirrorBanner.svelte';
  import SaveIndicator from './SaveIndicator.svelte';
  import ViewingBar, { type Viewing } from './ViewingBar.svelte';
  import CompareModal, { type CompareSide } from './CompareModal.svelte';
  import LinksPanel from './LinksPanel.svelte';
  import ReferencesPanel from './ReferencesPanel.svelte';
  import OttoPanel from './assist/OttoPanel.svelte';
  import type { AssistSelection } from './assist/model';
  import StatusPill from './StatusPill.svelte';
  import StudioBadge from './StudioBadge.svelte';
  import { formatLabel, isTextFormat, newerHeadAfterTypedLoad, renderKind, seqLookup, splitLinks, statusLabel, studioInfo } from './model';
  import { canAutosave, isMirrored, isMirroredError, mirrorSourceLabel, statusPillOf } from './versionRows';
  import { VersionList } from './versionList.svelte';
  import { Autosaver } from './autosave.svelte';
  import { exportItems } from './exportMenu';
  import { library } from './library.svelte';
  import { openStoryInProduct } from './nav';
  import { guardUnsaved } from '../../lib/leaveGuard';

  interface Props {
    id: string;
  }
  let { id: routeId }: Props = $props();
  // `#/design/a/<id>?from=story/<storyId>` (opened from a Product story): the
  // id is the part before `?`; the story is read from the hash.
  const id = $derived(routeId.split('?')[0]);
  /** `#/design/a/<id>/otto` (the Generate hand-off) — the query may ride on it. */
  const ottoHandoff = $derived(router.parts[3]?.split('?')[0] === 'otto');
  const fromStory = $derived.by(() => {
    void router.parts;
    const m = /[?&]from=story\/([^&/]+)/.exec(typeof location === 'undefined' ? '' : location.hash);
    return m ? decodeURIComponent(m[1]) : null;
  });

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
  /** A write was refused as `mirrored` (the meta we hold may be stale). */
  let mirrorRefused = $state(false);
  let adopting = $state(false);
  /** An older version shown alone in the stage (read-only). */
  let viewing = $state<Viewing | null>(null);

  const vlist = new VersionList();
  const versions = $derived(vlist.items);
  let links = $state<DesignLinksResp | null>(null);
  let linksLoading = $state(false);
  let linksError = $state<string | null>(null);

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
  let rightTab = $state<DetailsTab>('otto');
  let refs = $state(false);
  let showSource = $state(false);
  let device = $state<DeviceKind>('none');

  const artifact = $derived<DesignArtifact | null>(detail?.artifact ?? null);
  const kind = $derived(artifact ? renderKind(artifact.format) : 'other');
  const textual = $derived(!!artifact && isTextFormat(artifact.format));
  const mirrored = $derived(isMirrored(artifact) || mirrorRefused);
  const canEdit = $derived(!!artifact && auth.can('design', 'edit') && artifact.status !== 'archived');
  const readonly = $derived(!canEdit || mirrored || !textual);
  const dirty = $derived(textual && source !== null && source !== baseSource);
  // Leaving the page (sidebar, link, back) with edits not yet saved asks first.
  $effect(() => guardUnsaved(() => dirty, { what: 'this design' }));
  const head = $derived<DesignVersion | null>(detail?.head ?? null);
  const project = $derived(library.projectOf(artifact?.project_id));
  /** Seqs of OTHER artifacts' versions that links pin (resolved lazily below). */
  let pinnedSeqs = $state<Record<string, number>>({});
  const ownSeq = $derived(seqLookup(versions));
  const seqOf = $derived((vid: string): number | null => ownSeq(vid) ?? pinnedSeqs[vid] ?? null);
  const split = $derived(artifact ? splitLinks(artifact.id, links?.links ?? [], links?.artifacts ?? [], storyLabel) : { uses: [], usedIn: [] });
  const meId = $derived(auth.me?.id ?? null);
  const approvedSeq = $derived(detail?.approved?.seq ?? (artifact?.approved_version_id ? ownSeq(artifact.approved_version_id) : null));
  const pill = $derived(artifact ? statusPillOf(artifact, head?.id ?? null, approvedSeq) : null);
  /** What the stage shows: an older version alone, else the working copy. */
  const shownSource = $derived(viewing ? viewing.text : source);
  const shownBlob = $derived(viewing ? viewing.blobUrl : blobUrl);
  const stageReadonly = $derived(readonly || !!viewing);

  function storyLabel(kind: string, dstId: string): string {
    if (kind === 'story') {
      const s = library.stories[dstId];
      return s ? `${s.source_key} · ${s.title}` : 'A product story';
    }
    return dstId;
  }

  // ── Loading (sequence-guarded) ────────────────────────────────────────────
  let loadSeq = 0;
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
      autosaver.cancel();
      detail = null;
      links = null;
      vlist.reset(target);
      compare = null;
      mirrorRefused = false;
      void view(null);
      // `#/design/a/<id>/otto` (the lobby's Generate hand-off) opens on Otto.
      rightTab = 'otto';
      refs = false;
      detailsOpen = ottoHandoff;
      phase = 'loading';
      void load(target);
      void vlist.load();
      void loadLinks();
      if (!library.loaded) void library.load();
    });
  });
  $effect(() => () => setBlob(null));
  // Site Studio opens its side column on the Design inspector (once per
  // design; the lobby's Otto hand-off still opens Otto).
  let tabSetFor = '';
  $effect(() => {
    const a = artifact;
    if (!a || tabSetFor === a.id) return;
    tabSetFor = a.id;
    if (a.format === 'otto-site' && !untrack(() => ottoHandoff)) untrack(() => (rightTab = 'design'));
  });

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
            void vlist.loadNew();
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
            if (ev.change === 'approved') void vlist.load();
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
      if (dirty) void vlist.load();
      else void load(id);
      void loadLinks();
    });
  });

  // ── Saving ────────────────────────────────────────────────────────────────
  // Autosave: ~1.5 s after the last edit; never for a mirror, an archived or
  // read-only design, or while an older version is shown.
  const autosaver = new Autosaver(() => save());
  const autosaveOk = $derived(canAutosave({ readonly, mirrored, archived: artifact?.status === 'archived', viewing: !!viewing }));
  $effect(() => {
    void source;
    const d = dirty;
    const ok = autosaveOk;
    untrack(() => (d && ok ? autosaver.schedule() : autosaver.cancel()));
  });
  $effect(() => () => autosaver.cancel());

  /** Throws when the save failed (the autosaver shows it beside Retry). */
  async function save(message?: string, named = false): Promise<void> {
    if (!artifact || readonly || saving || source === null) return;
    if (!dirty && !named) return;
    saving = true;
    const body = { content: source, base_version: baseVersionId ?? '', ...(message ? { message } : {}) };
    try {
      const res = named ? await api.commitVersion(id, { ...body, message: message ?? '' }) : await api.putContent(id, body);
      applySaved(res, body.content);
      if (named && !res.created) toasts.info('No changes to save', 'The content matches the current version.');
    } catch (e) {
      if (isMirroredError(e)) {
        mirrorRefused = true;
        void refreshMeta();
        throw e;
      }
      if (e instanceof ApiError && e.status === 409) await resolveConflict(named ? message : undefined);
      else throw e;
    } finally {
      saving = false;
    }
  }

  function applySaved(res: import('../../lib/api/types').DesignSaveResult, savedText = source): void {
    baseSource = savedText;
    baseVersionId = res.version.id;
    newerHead = null;
    if (detail) detail = { ...detail, artifact: res.artifact, head: res.version };
    if (res.created) vlist.prepend(res.version);
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
      void vlist.load();
    } else if (value === 'mine') {
      if (!latest) {
        toasts.error('Couldn’t read the current version', 'Your edits are kept. Choose Retry to try again.');
        return;
      }
      try {
        const body = { content: source ?? '', base_version: latest.id, ...(namedMessage ? { message: namedMessage } : {}) };
        const res = namedMessage ? await api.commitVersion(id, { ...body, message: namedMessage }) : await api.putContent(id, body);
        applySaved(res, body.content);
        if (byAgent) {
          api.captureSignal({ artifact_id: id, kind: 'variant_rejected', version_id: latest.id, payload: { source: 'conflict', rejected_version_id: latest.id, rejected_seq: latest.seq, kept_version_id: res.version.id, reason: 'kept my edits' } });
        }
        void vlist.load();
      } catch (e) {
        toastError('Couldn’t save the design', e);
      }
    }
  }

  async function saveNamed(): Promise<void> {
    if (readonly) return;
    const msg = await confirmer.promptText('Describe this version (shown in history).', {
      title: 'Save named version',
      confirmLabel: 'Save version',
      placeholder: 'Hero v2 — bolder headline',
    });
    if (!msg) return;
    autosaver.cancel();
    await save(msg, true).catch((e) => toastError('Couldn’t save the design', e));
  }

  // ── "Edit here": adopt a mirror ───────────────────────────────────────────
  async function adopt(): Promise<void> {
    if (!artifact || adopting) return;
    adopting = true;
    try {
      const a = await api.adoptArtifact(id);
      if (detail) detail = { ...detail, artifact: a };
      mirrorRefused = false;
      toasts.success('You can edit this design here now', `Later changes in ${a.source_kind === 'canvas_scene' ? 'Canvas' : 'the story'} arrive as a version you can accept or skip.`);
    } catch (e) {
      toastError('Couldn’t take over this design', e);
    } finally {
      adopting = false;
    }
  }

  // ── View one version / restore ────────────────────────────────────────────
  let viewSeq = 0;
  async function view(v: DesignVersion | null): Promise<void> {
    const my = ++viewSeq;
    if (viewing?.blobUrl) URL.revokeObjectURL(viewing.blobUrl);
    if (!v || v.id === head?.id || !artifact) {
      viewing = null;
      return;
    }
    viewing = { v, text: null, blobUrl: null, loading: true, error: null };
    try {
      const c = await api.fetchContent(id, { version: v.id, asText: textual });
      if (my !== viewSeq) {
        if (c.blobUrl) URL.revokeObjectURL(c.blobUrl);
        return;
      }
      viewing = { v, text: c.text, blobUrl: c.blobUrl, loading: false, error: null };
    } catch (e) {
      if (my === viewSeq) viewing = { v, text: null, blobUrl: null, loading: false, error: loadErrorText(e) };
    }
  }

  async function restore(versionId: string, seq: number): Promise<void> {
    if (!artifact || !head) return;
    if (dirty) {
      toasts.warn('Your latest edits are still saving', 'Wait for “Saved”, then restore.');
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
      void view(null);
      await load(id);
      void vlist.load();
      toasts.success(`Restored v${seq} as v${res.version.seq}`);
    } catch (e) {
      if (isMirroredError(e)) mirrorRefused = true;
      else if (e instanceof ApiError && e.status === 409) toasts.error('Couldn’t restore', 'The design changed meanwhile. Refresh and try again.');
      else toastError('Couldn’t restore', e);
    }
  }

  // ── Status (approve is human-only and explicit) ───────────────────────────
  function statusMenu(e: MouseEvent): void {
    if (!artifact) return;
    const cur = artifact.status;
    const set = (s: DesignStatus) => void setStatus(s);
    const headApproved = artifact.approved_version_id === head?.id;
    const items: MenuItem[] = cur === 'archived'
      ? [{ label: 'Unarchive', icon: 'archive', disabled: !auth.can('design', 'edit'), action: () => void unarchive() }]
      : [
          { label: 'Draft', icon: 'edit', disabled: cur === 'draft' || !canEdit, action: () => set('draft') },
          { label: 'In review', icon: 'eye', disabled: cur === 'review' || !canEdit, action: () => set('review') },
          { label: dirty ? 'Approve (saving…)' : `Approve v${head?.seq ?? ''}…`, icon: 'check', disabled: !canEdit || dirty || !head || headApproved, action: () => void approve() },
          { label: 'Shipped', icon: 'send', disabled: cur === 'shipped' || !canEdit, action: () => set('shipped') },
        ];
    ctxMenu.show(e, items);
  }

  async function setStatus(s: DesignStatus): Promise<void> {
    try {
      const a = await api.updateArtifact(id, { status: s });
      if (detail) detail = { ...detail, artifact: a };
    } catch (e) {
      // A design ships from an approved version: the daemon says so with a 409.
      if (s === 'shipped' && e instanceof ApiError && e.status === 409) toasts.warn('Approve a version before shipping', e.message || 'A design ships from its approved version.');
      else toastError(`Couldn’t move to ${statusLabel(s)}`, e);
    }
  }

  async function unarchive(): Promise<void> {
    try {
      const a = await api.updateArtifact(id, { status: 'draft' });
      if (detail) detail = { ...detail, artifact: a };
      void library.load();
      toasts.success('Moved back to Draft', a.title);
    } catch (e) {
      toastError('Couldn’t unarchive the design', e);
    }
  }

  /** Approve `v` (default: the current version). */
  async function approve(v?: DesignVersion): Promise<void> {
    if (!artifact || !head) return;
    // Approval belongs to the version the person sees in this sheet. Live
    // content updates may move head while the confirmation is open.
    const target = id;
    const version = v ?? head;
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

  async function discard(): Promise<void> {
    const ok = await confirmer.ask('Discard your unsaved edits? The design goes back to the last saved version.', {
      title: 'Discard edits',
      danger: true, confirmLabel: 'Discard',
    });
    if (ok) {
      autosaver.cancel();
      source = baseSource;
      if (newerHead) void load(id);
    }
  }

  // ── Compare ───────────────────────────────────────────────────────────────
  /** `v` against the current version (default: the one before it). */
  function openCompare(v?: DesignVersion): void {
    if (!artifact || !head || !versions.length) return;
    const left = v?.id ?? versions.find((x) => x.id !== head.id)?.id ?? head.id;
    compare = { left: { artifact, versionId: left }, right: { artifact, versionId: head.id } };
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
  function versionById(vid: string): DesignVersion | undefined {
    return versions.find((v) => v.id === vid);
  }

  // What the Otto tab focuses a turn on: the 3D Studio passes its selected
  // object (the `ottoPanel` snippet's argument); Site Studio's section
  // selection plugs in the same way.
  const assistBlocked = $derived(
    !canEdit
      ? 'You can view this design, but asking Otto to change it needs edit access.'
      : mirrored
        ? `This design mirrors ${mirrorSourceLabel(artifact?.source_kind)} — choose Edit here to work on it with Otto.`
        : !textual
          ? 'Otto edits text and JSON designs. Images, PDFs and 3D models can’t be changed by an agent.'
          : null,
  );

  function openTab(t: DetailsTab, findRefs = false): void {
    rightTab = t;
    refs = findRefs;
    void setDetails(true);
  }

  // ── ⌘K commands (only while a design is open) ─────────────────────────────
  $effect(() => {
    if (!artifact) return registry.register('design-artifact', []);
    return registry.register('design-artifact', [
      { id: 'design.named', title: 'Save named version…', group: 'Design Hall', shortcut: '⌘S', keywords: 'save commit message', run: () => void saveNamed() },
      { id: 'design.compare', title: 'Compare versions', group: 'Design Hall', keywords: 'diff history restore', run: () => openCompare() },
      { id: 'design.versions', title: 'Show versions', group: 'Design Hall', keywords: 'history restore undo', run: () => openTab('versions') },
      { id: 'design.approve', title: 'Approve this version…', group: 'Design Hall', keywords: 'status review sign off', run: () => void approve() },
      { id: 'design.references', title: 'Find references', group: 'Design Hall', keywords: 'inspiration library search', run: () => openTab('links', true) },
      { id: 'design.otto', title: 'Ask Otto about this design', group: 'Design Hall', keywords: 'agent assist variants accessibility refine', run: () => openTab('otto') },
    ]);
  });

  // ⌘S saves a named version while focus is anywhere inside this view (edits
  // autosave) — a scoped listener, not a global shortcut.
  let rootEl = $state<HTMLDivElement | null>(null);
  function onKeydown(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 's') {
      e.preventDefault();
      void saveNamed();
    }
  }
  $effect(() => {
    const el = rootEl;
    if (!el) return;
    el.addEventListener('keydown', onKeydown);
    return () => el.removeEventListener('keydown', onKeydown);
  });

  function moreMenu(e: MouseEvent): void {
    if (!artifact) return;
    const a = artifact;
    const archived = a.status === 'archived';
    const items: MenuItem[] = [
      { label: 'Save named version…', icon: 'commit', hint: '⌘S', disabled: readonly, action: () => void saveNamed() },
      { label: 'Discard edits…', icon: 'refresh', disabled: !dirty, action: () => void discard() },
      { label: 'Compare versions…', icon: 'columns', disabled: versions.length < 2, action: () => openCompare() },
      { label: `Approve v${head?.seq ?? ''}…`, icon: 'check', disabled: !canEdit || dirty || !head || a.approved_version_id === head?.id, action: () => void approve() },
      { separator: true },
      { label: 'Rename…', icon: 'edit', disabled: !canEdit, action: () => void rename() },
      { label: 'Move to project…', icon: 'folder', disabled: !canEdit, action: () => queueMicrotask(() => moveMenu(e)) },
      { label: 'Make a copy', icon: 'copy', action: () => void duplicateHere() },
      { label: 'Export…', icon: 'download', action: () => queueMicrotask(() => ctxMenu.show(e, exportItems(a, shownSource, shownBlob, false, d2Svg))) },
      ...popoutItems(`design/a/${encodeURIComponent(a.id)}`, a.title),
      { separator: true },
      archived
        ? { label: 'Unarchive', icon: 'archive', disabled: !auth.can('design', 'edit'), action: () => void unarchive() }
        : { label: 'Archive…', icon: 'archive', danger: true, disabled: !canEdit, action: () => void archive() },
    ];
    ctxMenu.show(e, items);
  }

  // An unfiled design has no project crumb: "Unfiled" only led back to the
  // lobby (the first crumb already does) and ate the title's room. Opened
  // from a Product story, the first crumb returns to that story's Design tab.
  const storyTitle = $derived(fromStory ? (library.stories[fromStory]?.title ?? 'Back to the story') : null);
  const crumbs = $derived([
    ...(fromStory ? [{ label: `‹ ${storyTitle}`, onclick: () => openStoryInProduct(fromStory) }] : []),
    { label: 'Design Hall', onclick: () => router.go('design') },
    ...(project ? [{ label: project.name, onclick: () => router.go(`design/p/${encodeURIComponent(project.id)}`) }] : []),
  ]);
  function back(): void {
    if (fromStory) openStoryInProduct(fromStory);
    else router.go(project ? `design/p/${encodeURIComponent(project.id)}` : 'design');
  }
</script>

<div class="view" bind:this={rootEl}>
  <PageHeader title={artifact?.title ?? 'Design'} crumbs={viewport.isPhone ? [] : crumbs}>
    {#snippet leading()}
      {#if viewport.isPhone}
        <button class="icon-btn" onclick={back} aria-label={storyTitle ? `Back to ${storyTitle}` : 'Back'} title={storyTitle ? `Back to ${storyTitle}` : 'Back'}>
          <Icon name="chevronLeft" size={16} />
        </button>
      {/if}
    {/snippet}
    {#snippet badge()}
      {#if artifact && pill}
        <span class="badges">
          {#if !viewport.isPhone}<StudioBadge studio={artifact.studio} />{/if}
          <StatusPill status={pill.status} suffix={pill.suffix} onclick={statusMenu} />
          {#if head}<!-- every width: where a phone sees its autosave land --><span class="ver" title={`Current version: v${head.seq}`}>v{head.seq}</span>{/if}
        </span>
      {/if}
    {/snippet}
    {#snippet actions()}
      {#if artifact}
        {#if !readonly}
          <SaveIndicator state={autosaver.state} {dirty} error={autosaver.error} onretry={() => void autosaver.flush()} />
        {/if}
        <button class="icon-btn" data-overflow="-10" data-icon="more" data-label="More actions" onclick={moreMenu} aria-label="More actions" title="More actions" aria-haspopup="menu" data-testid="design-more">
          <Icon name="more" size={14} />
        </button>
      {/if}
    {/snippet}
  </PageHeader>

  <PageBody fill padded={false}>
    {#if phase === 'loading'}
      <div class="pad"><Skeleton rows={4} height={64} /></div>
    {:else if phase === 'gone'}
      <EmptyState variant="page" icon="designHall" title="This design isn’t available"
        body="It was deleted, or you don’t have access to its workspace." actionLabel="Back to Design Hall" onaction={() => router.go('design')} />
    {:else if phase === 'error' || !artifact}
      <div class="pad err" role="alert">
        <Icon name="warning" size={16} />
        <div>
          <strong>Couldn’t open this design.</strong>
          <div class="dim">{loadError}</div>
        </div>
        <button class="btn small" onclick={() => void load(id)}>Retry</button>
      </div>
    {:else}
      <MirrorBanner {artifact} canEdit={auth.can('design', 'edit') && artifact.status !== 'archived'} refused={mirrorRefused} {adopting} onadopt={() => void adopt()} />
      {#snippet notices()}
        {#if viewing}
          <ViewingBar {viewing} {canEdit} approved={viewing.v.id === artifact.approved_version_id} onback={() => void view(null)}
            oncompare={() => viewing && openCompare(viewing.v)} onrestore={() => viewing && void restore(viewing.v.id, viewing.v.seq)}
            onapprove={() => viewing && void approve(viewing.v)} onretry={() => viewing && void view(viewing.v)} />
        {:else if newerHead}
          <span class="notice warnc" use:announceShown={'A newer version was saved.'}>
            <Icon name="info" size={12} /> A newer version was saved.
            <button class="linkbtn" onclick={() => void discard()}>Load it (discard mine)</button>
          </span>
        {/if}
        {#if !canEdit && !mirrored}<span class="notice"><Icon name="lock" size={12} /> Read-only</span>{/if}
      {/snippet}
      {#snippet ottoPanel(selection: AssistSelection | null)}
        <OttoPanel {artifact} {versions} {head} uses={split.uses} {dirty} {selection}
          readonlyReason={assistBlocked} oncompare={(left, right) => (compare = { left, right })} />
      {/snippet}
      {#snippet versionsPanel()}
        <VersionsPanel list={vlist} headId={head?.id ?? null} approvedId={artifact.approved_version_id} {meId} viewingId={viewing?.v.id ?? null}
          {canEdit} onview={(v) => void view(v)} oncompare={openCompare} onrestore={(v) => void restore(v.id, v.seq)} onapprove={(v) => void approve(v)} />
      {/snippet}
      {#snippet linksOnly()}
        <LinksPanel {artifact} uses={split.uses} usedIn={split.usedIn} loading={linksLoading} error={linksError}
          readonly={!canEdit} {seqOf} onreload={() => void loadLinks()} oncompare={comparePinned} onmirrored={() => (mirrorRefused = true)} />
      {/snippet}
      {#snippet referencesPanel()}
        <ReferencesPanel {artifact} uses={split.uses} readonly={!canEdit} {seqOf} onreload={() => void loadLinks()}
          oncompare={compareWithRef} onmirrored={() => (mirrorRefused = true)} />
      {/snippet}
      {#snippet linksPanel()}
        <LinksTab bind:refs links={linksOnly} references={referencesPanel} />
      {/snippet}
      {#snippet ottoTab()}{@render ottoPanel(null)}{/snippet}
      {#snippet details(design: Snippet | undefined)}
        <DetailsPanel bind:tab={rightTab} bind:backButton={designButton} linkCount={split.uses.length + split.usedIn.length}
          versionCount={vlist.total ?? (versions.length || null)} otto={ottoTab} versions={versionsPanel} links={linksPanel} {design}
          onback={design ? undefined : () => void setDetails(false)} />
      {/snippet}
      {#if kind === 'scene3d'}
        {#key artifact.id}
          <Studio3D {artifact} source={shownSource} readonly={stageReadonly} {dirty} {head} usedIn={split.usedIn}
            linkCount={split.uses.length + split.usedIn.length} {seqOf} openOtto={ottoHandoff}
            onchange={(s) => (source = s)} otto={ottoPanel} links={linksPanel} versions={versionsPanel} {notices} />
        {/key}
      {:else if artifact.format === 'otto-site'}
        {#key artifact.id}
          <SiteStudio {artifact} source={shownSource} {dirty} readonly={stageReadonly} onchange={(s) => (source = s)} {notices} {details} />
        {/key}
      {:else}
      <div class="studio" class:wide-right={rightTab === 'otto'} class:details-open={detailsOpen}>
        <section class="center" aria-label="Design">
          <div class="toolbar">
            {#if kind === 'html'}
              <div class="segmented" role="group" aria-label="Device frame">
                {#each DEVICES as d (d.id)}
                  <button aria-pressed={device === d.id} class:active={device === d.id} onclick={() => (device = d.id)}>{d.label}</button>
                {/each}
              </div>
            {/if}
            {#if textual}
              <button class="btn small ghost" class:on={showSource} aria-pressed={showSource} onclick={() => (showSource = !showSource)} data-testid="design-source-toggle">
                <Icon name="file" size={12} /> Source
              </button>
            {/if}
            <span class="fmt">{formatLabel(artifact.format)} · {studioInfo(artifact.studio).name}</span>
            <span class="grow"></span>
            <button class="btn small ghost compact-details" aria-label="Show design details" bind:this={detailsButton} onclick={() => void setDetails(true)}><Icon name="info" size={12} /> Details</button>
            {@render notices()}
          </div>
          <div class="stage-host">
            {#key `${artifact.id}:${viewing?.v.id ?? ''}`}
              <ArtifactStage {artifact} source={shownSource} {dirty} blobUrl={shownBlob} noVersion={!head} readonly={stageReadonly}
                {showSource} {device} onchange={(s) => (source = s)} />
            {/key}
          </div>
        </section>
        {@render details(undefined)}
      </div>
      {/if}
    {/if}
  </PageBody>
</div>

{#if compare && artifact}
  <CompareModal
    left={compare.left}
    right={compare.right}
    currentId={artifact.id}
    headId={head?.id ?? null}
    approvedId={artifact.approved_version_id}
    canApprove={canEdit}
    {meId}
    onclose={() => (compare = null)}
    onrestore={(v, s) => void restore(v, s)}
    onapprove={(v) => {
      const ver = versionById(v);
      if (ver) void approve(ver);
    }}
  />
{/if}

<style>
  .view {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    container-type: inline-size;
  }
  .badges {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .ver {
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .pad {
    padding: 20px;
  }
  .err {
    display: flex;
    align-items: flex-start;
    gap: 10px;
  }
  .err > :global(svg) {
    color: var(--danger);
    margin-block-start: 2px;
  }
  .dim {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .studio {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(0, 1fr) 320px;
  }
  /* The Otto tab holds a conversation and a variants tray: a little wider. */
  .studio.wide-right {
    grid-template-columns: minmax(0, 1fr) 360px;
  }
  .center {
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--term-bg);
    background-image: radial-gradient(color-mix(in srgb, var(--text) 9%, transparent) 1px, transparent 1px);
    background-size: 16px 16px;
  }
  .toolbar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    min-height: 40px;
    padding: 6px 12px;
    border-block-end: 1px solid var(--border);
    background: var(--bg);
  }
  .toolbar .on {
    background: var(--accent-soft);
  }
  .fmt {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
  .notice {
    display: inline-flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .warnc {
    color: var(--warning);
  }
  .linkbtn {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
  }
  .linkbtn:hover {
    text-decoration: underline;
  }
  .stage-host {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    container-type: inline-size;
  }
  .stage-host > :global(*) {
    flex: 1;
  }
  .compact-details { display: none; }
  @container (max-width: 900px) {
    .studio,
    .studio.wide-right {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: minmax(0, 1fr);
      overflow: hidden;
    }
    .compact-details { display: inline-flex; }
    .studio:not(.details-open) > :global(.right),
    .studio.details-open .center { display: none; }
  }
</style>
