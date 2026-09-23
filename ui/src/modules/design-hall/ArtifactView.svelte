<script lang="ts">
  // One design, open (canvas/studio archetype):
  //
  //   PageHeader: Design Hall › Project › Title · status ▾ · vN      Compare  ⋯  [Save]
  //   ┌ (3D: hierarchy) ┬ stage toolbar + ArtifactStage ──────────┬ Links | References ┐
  //   │                 │  existing viewers/editors per format      │ (3D: Inspector)    │
  //   └─────────────────┴───────────────────────────────────────────┴───────────────────┘
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
  import { untrack } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
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
    DesignArtifact,
    DesignArtifactDetail,
    DesignArtifactFormat,
    DesignLinksResp,
    DesignStatus,
    DesignVersion,
  } from '../../lib/api/types';
  import { DEVICES, type DeviceKind } from '../product/design/DeviceFrame.svelte';
  import { Hierarchy, Inspector, parseScene, serializeScene, type Scene3dDoc } from '../product/design/scene3d';
  import ArtifactStage from './ArtifactStage.svelte';
  import VersionStrip from './VersionStrip.svelte';
  import CompareModal, { type CompareSide } from './CompareModal.svelte';
  import LinksPanel from './LinksPanel.svelte';
  import ReferencesPanel from './ReferencesPanel.svelte';
  import OttoPanel from './assist/OttoPanel.svelte';
  import StatusPill from './StatusPill.svelte';
  import StudioBadge from './StudioBadge.svelte';
  import { formatLabel, isTextFormat, renderKind, seqLookup, splitLinks, statusLabel, studioInfo } from './model';
  import { library } from './library.svelte';
  import { originOf } from './nav';

  interface Props {
    id: string;
  }
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
  let rightTab = $state<'inspector' | 'otto' | 'links' | 'references'>('links');
  let showSource = $state(false);
  let device = $state<DeviceKind>('none');
  let sceneSel = $state<string | null>(null);

  const artifact = $derived<DesignArtifact | null>(detail?.artifact ?? null);
  const kind = $derived(artifact ? renderKind(artifact.format) : 'other');
  const textual = $derived(!!artifact && isTextFormat(artifact.format));
  const origin = $derived(artifact ? originOf(artifact) : null);
  /** Imported rows mirror Product/Canvas — they're edited at the origin (or forked). */
  const imported = $derived(!!artifact?.source_kind);
  const canEdit = $derived(!!artifact && auth.can('design', 'edit') && artifact.status !== 'archived');
  const readonly = $derived(!canEdit || imported || !textual);
  const dirty = $derived(textual && source !== null && source !== baseSource);
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
    phase = detail?.artifact.id === target ? phase : 'loading';
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
      const first = detail?.artifact.id !== d.artifact.id;
      detail = d;
      source = text;
      baseSource = text;
      baseVersionId = d.content_version_id ?? d.head?.id ?? null;
      newerHead = null;
      phase = 'ready';
      loadError = null;
      if (first && renderKind(d.artifact.format) === 'scene3d' && rightTab !== 'otto') rightTab = 'inspector';
    } catch (e) {
      if (my !== loadSeq) return;
      if (e instanceof ApiError && e.status === 404) phase = 'gone';
      else {
        phase = 'error';
        loadError = e instanceof Error ? e.message : String(e);
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
      if (my === versionsSeq) versionsError = e instanceof Error ? e.message : String(e);
    } finally {
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
      if (my === linksSeq) linksError = e instanceof Error ? e.message : String(e);
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
      sceneSel = null;
      // `#/design/a/<id>/otto` (the lobby's Generate hand-off) opens on Otto.
      rightTab = router.parts[3] === 'otto' ? 'otto' : 'links';
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
            void loadVersions();
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
      applySaved(res);
      if (!res.created) toasts.info('No changes to save', 'The content matches the current version.');
    } catch (e) {
      if (e instanceof ApiError && e.status === 409) await resolveConflict(named ? message : undefined);
      else toasts.error('Couldn’t save the design', e instanceof Error ? e.message : String(e));
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
      if (r.broken.length) parts.push(`${r.broken.length} broken otto://design reference${r.broken.length === 1 ? '' : 's'}`);
      if (r.cycles.length) parts.push(`${r.cycles.length} embed${r.cycles.length === 1 ? '' : 's'} skipped (would loop)`);
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
        await load(id);
        return;
      }
      try {
        const body = { content: source ?? '', base_version: latest.id, ...(namedMessage ? { message: namedMessage } : {}) };
        const res = namedMessage ? await api.commitVersion(id, { ...body, message: namedMessage }) : await api.putContent(id, body);
        applySaved(res);
        if (byAgent) {
          api.captureSignal({ artifact_id: id, kind: 'variant_rejected', version_id: latest.id, payload: { source: 'conflict', rejected_version_id: latest.id, rejected_seq: latest.seq, kept_version_id: res.version.id, reason: 'kept my edits' } });
        }
        void loadVersions();
      } catch (e) {
        toasts.error('Couldn’t save the design', e instanceof Error ? e.message : String(e));
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
      const asText = textual;
      const c = await api.fetchContent(id, { version: versionId, asText });
      let res;
      if (asText) {
        res = await api.putContent(id, { content: c.text ?? '', base_version: head.id, message: `Restored v${seq}` });
      } else {
        const blob = await fetch(c.blobUrl!).then((r) => r.blob());
        URL.revokeObjectURL(c.blobUrl!);
        const b64 = await new Promise<string>((resolve, reject) => {
          const fr = new FileReader();
          fr.onerror = () => reject(fr.error);
          fr.onload = () => resolve(String(fr.result).split(',')[1] ?? '');
          fr.readAsDataURL(blob);
        });
        res = await api.putContent(id, { content_b64: b64, base_version: head.id, message: `Restored v${seq}` });
      }
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
      if (e instanceof ApiError && e.status === 409) toasts.error('Couldn’t restore', 'The design changed meanwhile. Reload and try again.');
      else toasts.error('Couldn’t restore', e instanceof Error ? e.message : String(e));
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
      toasts.error(`Couldn’t move to ${statusLabel(s)}`, e instanceof Error ? e.message : String(e));
    }
  }

  async function approve(): Promise<void> {
    if (!artifact || !head) return;
    const followers = split.usedIn.filter((r) => r.link.policy === 'follow_approved').length;
    const ok = await confirmer.ask(
      `Approve v${head.seq} of “${artifact.title}”?` +
        (followers ? ` ${followers} design${followers === 1 ? '' : 's'} that follow its approved version will update to v${head.seq}.` : ''),
      { title: 'Approve design', confirmLabel: `Approve v${head.seq}`, danger: false },
    );
    if (!ok) return;
    try {
      const a = await api.approveArtifact(id, head.id);
      if (detail) detail = { ...detail, artifact: a, approved: head };
      toasts.success(`Approved v${head.seq}`);
    } catch (e) {
      toasts.error('Couldn’t approve', e instanceof Error ? e.message : String(e));
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
      toasts.error('Couldn’t rename', e instanceof Error ? e.message : String(e));
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
      toasts.error('Couldn’t move the design', e instanceof Error ? e.message : String(e));
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
      toasts.error('Couldn’t make a copy', e instanceof Error ? e.message : String(e));
    }
  }

  async function archive(): Promise<void> {
    if (!artifact) return;
    const ok = await confirmer.ask(
      `Archive “${artifact.title}”? It leaves the lobby and search; every version is kept and links to it keep working.`,
      { title: 'Archive design', confirmLabel: 'Archive' },
    );
    if (!ok) return;
    try {
      await api.archiveArtifact(id);
      toasts.success('Design archived');
      router.go('design');
    } catch (e) {
      toasts.error('Couldn’t archive', e instanceof Error ? e.message : String(e));
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
      confirmLabel: 'Discard',
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

  // ── 3D: hierarchy + inspector share the stage's parsed document ──────────
  const sceneDoc = $derived.by<Scene3dDoc | null>(() => {
    if (kind !== 'scene3d' || source === null) return null;
    const r = parseScene(source);
    return r.ok ? r.doc : null;
  });
  // What the Otto tab focuses a turn on: the selected 3D object today (Site
  // Studio's section selection plugs in here when it lands).
  const assistSelection = $derived.by(() => {
    if (kind !== 'scene3d' || !sceneSel || !sceneDoc) return null;
    const o = sceneDoc.objects.find((x) => x.id === sceneSel);
    return { node_id: sceneSel, label: o?.name || sceneSel };
  });
  const assistBlocked = $derived(
    !canEdit
      ? 'You can view this design, but asking Otto to change it needs edit access.'
      : imported
        ? 'Mirrored from Product/Canvas — make an editable copy to work on it with Otto.'
        : !textual
          ? 'Otto edits text and JSON designs. Images, PDFs and 3D models can’t be changed by an agent.'
          : null,
  );
  function onScene(d: Scene3dDoc): void {
    if (!readonly) source = serializeScene(d);
  }

  // ── ⌘K commands (only while a design is open) ─────────────────────────────
  $effect(() => {
    if (!artifact) return registry.register('design-artifact', []);
    return registry.register('design-artifact', [
      { id: 'design.save', title: 'Save design', group: 'Design Hall', shortcut: '⌘S', keywords: 'version commit', run: () => void save() },
      { id: 'design.named', title: 'Save named version…', group: 'Design Hall', keywords: 'commit message', run: () => void saveNamed() },
      { id: 'design.compare', title: 'Compare versions', group: 'Design Hall', keywords: 'diff history restore', run: openCompare },
      { id: 'design.references', title: 'Find references', group: 'Design Hall', keywords: 'inspiration library search', run: () => (rightTab = 'references') },
      { id: 'design.otto', title: 'Ask Otto about this design', group: 'Design Hall', keywords: 'agent assist variants accessibility refine', run: () => (rightTab = 'otto') },
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
      { separator: true },
      { label: 'Rename…', icon: 'edit', disabled: !canEdit, action: () => void rename() },
      { label: 'Move to project…', icon: 'folder', disabled: !canEdit, action: () => queueMicrotask(() => moveMenu(e)) },
      { label: 'Make an editable copy', icon: 'copy', action: () => void duplicateHere() },
      { label: 'Download', icon: 'download', action: download },
      { separator: true },
      { label: 'Archive…', icon: 'archive', danger: true, disabled: !canEdit, action: () => void archive() },
    ];
    ctxMenu.show(e, items);
  }

  const crumbs = $derived([
    { label: 'Design Hall', onclick: () => router.go('design') },
    project
      ? { label: project.name, onclick: () => router.go(`design/p/${encodeURIComponent(project.id)}`) }
      : { label: 'Unfiled', onclick: () => router.go('design') },
  ]);
</script>

<div class="view" bind:this={rootEl}>
  <PageHeader title={artifact?.title ?? 'Design'} crumbs={viewport.isPhone ? [] : crumbs}>
    {#snippet leading()}
      {#if viewport.isPhone}
        <button class="icon-btn" onclick={() => router.go(project ? `design/p/${encodeURIComponent(project.id)}` : 'design')} aria-label="Back" title="Back">
          <Icon name="chevronLeft" size={16} />
        </button>
      {/if}
    {/snippet}
    {#snippet badge()}
      {#if artifact}
        <span class="badges">
          <StudioBadge studio={artifact.studio} />
          <StatusPill status={artifact.status} onclick={statusMenu} />
          {#if head}<span class="ver" title={`Current version: v${head.seq}`}>v{head.seq}</span>{/if}
          {#if dirty}<span class="chip edited" data-testid="design-dirty">Edited</span>{/if}
        </span>
      {/if}
    {/snippet}
    {#snippet actions()}
      {#if artifact}
        <button class="btn small" data-icon="columns" onclick={openCompare} disabled={versions.length < 2}>
          <Icon name="columns" size={12} /> Compare
        </button>
        <button class="icon-btn" data-icon="more" data-label="More actions" onclick={moreMenu} aria-label="More actions" title="More actions" aria-haspopup="menu" data-testid="design-more">
          <Icon name="more" size={14} />
        </button>
        {#if !readonly}
          <button class="btn small primary" onclick={() => void save()} disabled={!dirty || saving} title="Save a new version (⌘S)" data-testid="design-save">
            {saving ? 'Saving…' : 'Save'}
          </button>
        {/if}
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
      <div class="studio" class:has-left={kind === 'scene3d' && !!sceneDoc} class:wide-right={rightTab === 'otto'}>
        {#if kind === 'scene3d' && sceneDoc}
          <aside class="left" aria-label="Scene hierarchy">
            <Hierarchy doc={sceneDoc} bind:selectedId={sceneSel} onchange={onScene} {readonly} />
          </aside>
        {/if}
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
            {#if newerHead}
              <span class="notice warnc" role="status">
                <Icon name="info" size={12} /> A newer version was saved.
                <button class="linkbtn" onclick={() => void discard()}>Load it (discard mine)</button>
              </span>
            {/if}
            {#if imported}
              <span class="notice">
                <Icon name="info" size={12} /> Mirrored from {artifact.source_kind === 'canvas_scene' ? 'Canvas' : 'Product'} — edit it there, or make a copy here.
                {#if origin}<button class="linkbtn" onclick={origin.open}>{origin.label}</button>{/if}
                <button class="linkbtn" onclick={() => void duplicateHere()}>Make an editable copy</button>
              </span>
            {:else if !canEdit}
              <span class="notice"><Icon name="lock" size={12} /> Read-only</span>
            {/if}
          </div>
          <div class="stage-host">
            {#key artifact.id}
              <ArtifactStage
                {artifact}
                {source}
                {blobUrl}
                {readonly}
                {showSource}
                {device}
                bind:selectedId={sceneSel}
                onchange={(s) => (source = s)}
              />
            {/key}
          </div>
        </section>
        <aside class="right" aria-label="Design details">
          <div class="tabs segmented" role="tablist" aria-label="Details panel">
            {#if kind === 'scene3d'}
              <button role="tab" aria-selected={rightTab === 'inspector'} class:active={rightTab === 'inspector'} onclick={() => (rightTab = 'inspector')}>Inspector</button>
            {/if}
            <button role="tab" aria-selected={rightTab === 'otto'} class:active={rightTab === 'otto'} onclick={() => (rightTab = 'otto')} data-testid="design-tab-otto">
              Otto
            </button>
            <button role="tab" aria-selected={rightTab === 'links'} class:active={rightTab === 'links'} onclick={() => (rightTab = 'links')} data-testid="design-tab-links">
              Links <span class="count">{split.uses.length + split.usedIn.length}</span>
            </button>
            <button role="tab" aria-selected={rightTab === 'references'} class:active={rightTab === 'references'} onclick={() => (rightTab = 'references')} data-testid="design-tab-references">
              References
            </button>
          </div>
          <div class="panel" role="tabpanel">
            {#if brief && rightTab !== 'inspector' && rightTab !== 'otto'}
              <div class="brief">
                <span class="k"><Icon name="sparkle" size={12} /> Brief</span>
                <p>{brief}</p>
              </div>
            {/if}
            {#if rightTab === 'inspector' && kind === 'scene3d'}
              {#if sceneDoc}
                <Inspector doc={sceneDoc} bind:selectedId={sceneSel} onchange={onScene} {readonly} />
              {:else}
                <p class="dim pad">Fix the scene document to inspect it.</p>
              {/if}
            {:else if rightTab === 'otto'}
              <OttoPanel {artifact} {versions} {head} uses={split.uses} {dirty} selection={assistSelection}
                readonlyReason={assistBlocked} oncompare={(left, right) => (compare = { left, right })} />
            {:else if rightTab === 'links'}
              <LinksPanel {artifact} uses={split.uses} usedIn={split.usedIn} loading={linksLoading} error={linksError}
                readonly={!canEdit} {seqOf} onreload={() => void loadLinks()} oncompare={comparePinned} />
            {:else}
              <ReferencesPanel {artifact} uses={split.uses} readonly={!canEdit} {seqOf} onreload={() => void loadLinks()}
                oncompare={compareWithRef} />
            {/if}
          </div>
        </aside>
      </div>
      <VersionStrip
        {versions}
        headId={head?.id ?? null}
        approvedId={artifact.approved_version_id}
        {meId}
        {selected}
        loading={versionsLoading}
        error={versionsError}
        onselect={toggleSelect}
        oncompare={openCompare}
        onretry={() => void loadVersions()}
      />
    {/if}
  </PageBody>
</div>

{#if compare && artifact}
  <CompareModal
    left={compare.left}
    right={compare.right}
    currentId={artifact.id}
    headId={head?.id ?? null}
    {meId}
    onclose={() => (compare = null)}
    onrestore={(v, s) => void restore(v, s)}
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
  .edited {
    color: var(--warning);
    border-color: color-mix(in srgb, var(--warning) 35%, transparent);
    background: var(--warning-soft);
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
  .studio.has-left {
    grid-template-columns: 240px minmax(0, 1fr) 320px;
  }
  /* The Otto tab holds a conversation and a variants tray: a little wider. */
  .studio.wide-right {
    grid-template-columns: minmax(0, 1fr) 360px;
  }
  .studio.has-left.wide-right {
    grid-template-columns: 240px minmax(0, 1fr) 360px;
  }
  .left {
    border-inline-end: 1px solid var(--border);
    background: var(--surface);
    overflow: auto;
    min-height: 0;
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
  .right {
    border-inline-start: 1px solid var(--border);
    background: var(--surface);
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .tabs {
    margin: 10px 14px 0;
    align-self: flex-start;
  }
  .count {
    color: var(--text-dim);
    margin-inline-start: 2px;
  }
  .panel {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    margin-block-start: 10px;
    border-block-start: 1px solid var(--border);
  }
  .brief {
    margin: 12px 14px 0;
    padding: 8px 10px;
    border-inline-start: 2px solid var(--border-strong);
    background: var(--surface-2);
    border-radius: var(--radius-s);
  }
  .brief .k {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  .brief p {
    margin: 4px 0 0;
    font-size: var(--fs-s);
    white-space: pre-wrap;
  }
  @container (max-width: 900px) {
    .studio,
    .studio.has-left,
    .studio.wide-right,
    .studio.has-left.wide-right {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: minmax(360px, 1fr) auto;
      overflow-y: auto;
    }
    .left {
      display: none;
    }
    .right {
      border-inline-start: 0;
      border-block-start: 1px solid var(--border);
      max-height: 60%;
    }
  }
</style>
