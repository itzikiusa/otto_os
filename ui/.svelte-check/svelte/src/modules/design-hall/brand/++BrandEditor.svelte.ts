///<reference types="svelte" />
;
import { announceShown } from '../../../lib/announce.svelte';
import { viewport } from '../../../lib/stores/viewport.svelte';
import { plural } from '../../../lib/plural';
import { loadErrorText } from '../../../lib/loadError';
// Brand Kit editor (`#/design/brand/<id>`): one `otto-brand/1` kit.
//
//   left  — Colors (live contrast) · Typography · Spacing & radius · Logos ·
//           Imagery · Voice · Used in
//   right — "Applied to": a landing hero, a social tile and a 3D card swatch
//           re-tinting as you edit (CSS custom properties, no save needed)
//
// Saving is explicit and never blind (proposal §3.3): while there are unsaved
// token changes a banner says how many designs in how many studios they
// reach (`POST …/brand/impact`, debounced); "Save as vN" opens the impact
// preview first when designs use the kit, then commits a NAMED version with
// `base_version` (409 → the same "save mine / load theirs" choice as the
// artifact view). Consumers follow the APPROVED kit, so a saved version rolls
// out only when a person approves it — the header offers that explicitly.
import { untrack } from 'svelte';
import { newerHeadAfterTypedLoad } from '../model';
import PageHeader from '../../../lib/components/PageHeader.svelte';
import PageBody from '../../../lib/components/PageBody.svelte';
import EmptyState from '../../../lib/components/EmptyState.svelte';
import Skeleton from '../../../lib/components/Skeleton.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { confirmer } from '../../../lib/confirm.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { router } from '../../../lib/router.svelte';
import { auth } from '../../../lib/stores/auth.svelte';
import { designBus } from '../../../lib/events.svelte';
import { ApiError } from '../../../lib/api/client';
import * as api from '../../../lib/api/design';
import { downloadText } from '../../../lib/components/exporters';
import type {
    BrandDoc,
    BrandExportFormat,
    BrandImpactResp,
    DesignArtifact,
    DesignLearnedRule,
    DesignSaveResult,
    DesignSearchHit,
    DesignVersion,
  } from '../../../lib/api/types';
import StatusPill from '../StatusPill.svelte';
import AppliedPreview from './AppliedPreview.svelte';
import ColorsSection from './ColorsSection.svelte';
import TypeSection from './TypeSection.svelte';
import SpaceSection from './SpaceSection.svelte';
import LogosSection from './LogosSection.svelte';
import ProseSection from './ProseSection.svelte';
import UsedIn from './UsedIn.svelte';
import ImpactModal from './ImpactModal.svelte';
import { brandExport, brandImpact, getLearned } from './api';
import { copyWithToast } from '../../../lib/clipboard';
import { diffTokens, normalizeBrandDoc, parseBrandDoc, rulesForKit, serializeBrandDoc, validateBrandDoc } from './tokens';
import { guardUnsaved } from '../../../lib/leaveGuard';
import LoadState from '../../../lib/components/LoadState.svelte';
;

  interface Props {
    id: string;
    kits: DesignSearchHit[];
    onnew: () => void;
  };function $$render() {
  const brandBack/*Ωignore_positionΩ*/ = ()/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  if(viewport.isPhone){
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => router.go('design'),"aria-label":`Back to Design Hall`,"title":`Back to Design Hall`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});} }
  }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { id, kits, onnew }: Props = $props();

  const errText = loadErrorText;

  // ── Load ──────────────────────────────────────────────────────────────────
  let phase = $state<'loading' | 'ready' | 'error' | 'gone' | 'invalid'>('loading');
  let loadError = $state<string | null>(null);
  let artifact = $state<DesignArtifact | null>(null);
  let head = $state<DesignVersion | null>(null);
  let approved = $state<DesignVersion | null>(null);
  /** The saved kit, normalized + serialized (what "dirty" compares with). */
  let baseText = $state('');
  let baseVersionId = $state<string | null>(null);
  let doc = $state<BrandDoc>(normalizeBrandDoc({}));
  /** A newer head saved elsewhere while this copy had unsaved edits. */
  let newerHead = $state<string | null>(null);
  let saving = $state(false);

  /** Only the newest load (for the current kit) may assign (S18-09). */
  let loadSeq = 0;

  async function load(): Promise<void> {
    const my = ++loadSeq;
    const target = id;
    const current = () => my === loadSeq && target === id;
    // Edits typed WHILE this load is in flight (a live event or a resync
    // triggered it) win: the fetched head becomes "a newer version" instead.
    const sameKit = artifact?.id === target && phase === 'ready';
    const typedAt = sameKit ? serializeBrandDoc(doc) : null;
    if (phase !== 'ready') phase = 'loading';
    loadError = null;
    try {
      const d = await api.getArtifact(target, { content: true });
      if (!current()) return;
      artifact = d.artifact;
      head = d.head;
      approved = d.approved;
      if (d.artifact.format !== 'otto-brand') {
        phase = 'invalid';
        return;
      }
      let text = d.content;
      if (text == null || d.content_truncated) text = (await api.fetchContent(target, { asText: true })).text ?? '{}';
      if (!current()) return;
      if (typedAt !== null && serializeBrandDoc(doc) !== typedAt) {
        newerHead = newerHeadAfterTypedLoad(d.content_version_id ?? d.head?.id ?? null, baseVersionId);
        phase = 'ready';
        return;
      }
      const parsed = parseBrandDoc(text, d.artifact.title) ?? normalizeBrandDoc({}, d.artifact.title);
      doc = parsed;
      baseText = serializeBrandDoc(parsed);
      baseVersionId = d.content_version_id ?? d.head?.id ?? null;
      newerHead = null;
      phase = 'ready';
      void loadUsage();
      void loadRules(d.artifact.workspace_id);
    } catch (e) {
      if (!current()) return;
      if (e instanceof ApiError && e.status === 404) phase = 'gone';
      else {
        loadError = errText(e);
        phase = 'error';
      }
    }
  }

  async function refreshMeta(): Promise<void> {
    try {
      const d = await api.getArtifact(id);
      artifact = d.artifact;
      head = d.head;
      approved = d.approved;
    } catch {
      /* the next full load reports it */
    }
  }

  $effect(() => {
    void id;
    untrack(() => void load());
  });

  // ── Derived editing state ─────────────────────────────────────────────────
  const draftText = $derived(serializeBrandDoc(doc));
  const dirty = $derived(phase === 'ready' && draftText !== baseText);
  // Leaving the editor (sidebar, link, back) with unsaved token edits asks first.
  $effect(() => guardUnsaved(() => dirty, { what: 'the brand kit' }));
  const baseDoc = $derived(parseBrandDoc(baseText) ?? normalizeBrandDoc({}));
  const localChanges = $derived(dirty ? diffTokens(baseDoc, doc) : []);
  const issues = $derived(validateBrandDoc(doc));
  const canEdit = $derived(auth.can('design', 'edit'));
  const readonly = $derived(!canEdit);
  const nextSeq = $derived((head?.seq ?? 0) + 1);
  const needsApproval = $derived(!!head && !!artifact && head.id !== artifact.approved_version_id);

  // ── Used in (a plain impact listing) ──────────────────────────────────────
  let usage = $state<BrandImpactResp | null>(null);
  let usageLoading = $state(false);
  let usageError = $state<string | null>(null);
  let usageTimer: ReturnType<typeof setTimeout> | null = null;

  async function loadUsage(): Promise<void> {
    usageLoading = true;
    usageError = null;
    try {
      usage = await brandImpact(id);
    } catch (e) {
      usageError = errText(e);
    } finally {
      usageLoading = false;
    }
  }
  function usageSoon(): void {
    if (usageTimer) clearTimeout(usageTimer);
    usageTimer = setTimeout(() => void loadUsage(), 400);
  }
  $effect(() => () => {
    if (usageTimer) clearTimeout(usageTimer);
  });

  // ── Learned rules that talk about the brand ───────────────────────────────
  let rules = $state<DesignLearnedRule[]>([]);
  async function loadRules(workspaceId: string): Promise<void> {
    try {
      rules = (await getLearned(workspaceId)).active;
    } catch {
      rules = []; // optional garnish — never an error state
    }
  }
  const kitRules = $derived(rulesForKit(rules, doc));

  // ── Live impact of the unsaved changes (debounced) ────────────────────────
  let draftImpact = $state<BrandImpactResp | null>(null);
  let impactFor = $state('');
  let impactLoading = $state(false);
  let impactError = $state<string | null>(null);
  let ctrl: AbortController | null = null;

  async function runImpact(text: string): Promise<void> {
    ctrl?.abort();
    const c = new AbortController();
    ctrl = c;
    impactLoading = true;
    impactError = null;
    try {
      const r = await brandImpact(id, JSON.parse(text) as BrandDoc, c.signal);
      if (c.signal.aborted) return;
      draftImpact = r;
      impactFor = text;
    } catch (e) {
      if (!c.signal.aborted) impactError = errText(e);
    } finally {
      if (ctrl === c) impactLoading = false;
    }
  }
  $effect(() => {
    const text = draftText;
    if (!dirty || issues.length > 0) return;
    const t = setTimeout(() => untrack(() => void runImpact(text)), 450);
    return () => clearTimeout(t);
  });
  $effect(() => () => ctrl?.abort());
  const impactFresh = $derived(!!draftImpact && impactFor === draftText);
  /** What the unsaved-changes banner says once it has an answer (spoken by
   *  `announceShown`; the transient “Checking …” line stays quiet). */
  const bannerNote = $derived.by(() => {
    if (localChanges.length === 0) return 'Unsaved changes to logos, imagery or voice.';
    if (impactFresh && draftImpact) {
      const what = localChanges.length === 1 ? 'a token' : `${localChanges.length} tokens`;
      if (draftImpact.affected_count > 0) return `Changing ${what} affects ${plural(draftImpact.affected_count, 'artifact')} in ${plural(draftImpact.affected_studio_count, 'studio')}.`;
      if (draftImpact.artifact_count > 0) return `${localChanges.length === 1 ? 'This change doesn’t' : 'These changes don’t'} reach any of the ${draftImpact.artifact_count} designs that use this kit.`;
      return 'No designs use this kit yet — nothing else changes.';
    }
    return impactError ? 'Couldn’t check which designs use these tokens.' : '';
  });

  // ── Impact preview sheet ──────────────────────────────────────────────────
  let showImpact = $state(false);
  function openImpact(): void {
    showImpact = true;
    if (!impactFresh && issues.length === 0) void runImpact(draftText);
  }

  // ── Save ──────────────────────────────────────────────────────────────────
  function commitMessage(): string {
    const ch = localChanges;
    if (ch.length === 1) {
      const c = ch[0];
      return c.change === 'changed' ? `Brand: ${c.token} ${c.before} → ${c.after}` : `Brand: ${c.change} ${c.token}`;
    }
    if (ch.length > 1) {
      const names = ch.slice(0, 4).map((c) => c.token).join(', ');
      return `Brand: ${ch.length} token changes (${names}${ch.length > 4 ? ', …' : ''})`;
    }
    return 'Brand: updated logos, imagery or voice';
  }

  /** "Save as vN": preview the impact first whenever designs use this kit. */
  async function requestSave(): Promise<void> {
    if (!dirty || saving || readonly) return;
    if (issues.length) {
      toasts.warn('Fix the kit before saving', issues[0]);
      return;
    }
    if ((usage?.artifact_count ?? 0) > 0 && localChanges.length > 0 && !showImpact) {
      openImpact();
      return;
    }
    await save();
  }

  async function save(base: string | null = baseVersionId): Promise<void> {
    if (saving) return;
    saving = true;
    const text = draftText;
    const changes = localChanges;
    const message = commitMessage();
    try {
      let res: DesignSaveResult;
      try {
        res = await api.commitVersion(id, { content: text, base_version: base ?? '', message });
      } catch (e) {
        if (!(e instanceof ApiError) || e.status !== 409) throw e;
        const retryBase = await resolveConflict();
        if (!retryBase) return;
        res = await api.commitVersion(id, { content: text, base_version: retryBase, message });
      }
      applySaved(res, text);
      showImpact = false;
      if (!res.created) {
        toasts.info('No changes to save', 'The kit matches the current version.');
        return;
      }
      const followers = usage?.consumers.filter((c) => c.policy === 'follow_approved').length ?? 0;
      toasts.success(
        `Saved ${artifact?.title ?? 'the kit'} v${res.version.seq}`,
        followers ? `${followers} ${followers === 1 ? 'design follows' : 'designs follow'} the approved kit — accept v${res.version.seq} to roll it out.` : undefined,
      );
      if (changes.length) {
        api.captureSignal({
          artifact_id: id,
          kind: 'brand_correction',
          version_id: res.version.id,
          payload: { source: 'brand_kit', tokens: changes.slice(0, 20).map((c) => ({ token: c.token, change: c.change, before: c.before, after: c.after })) },
        });
      }
    } catch (e) {
      toasts.error('Couldn’t save the brand kit', errText(e));
    } finally {
      saving = false;
    }
  }

  function applySaved(res: DesignSaveResult, text: string): void {
    baseText = text;
    baseVersionId = res.version.id;
    artifact = res.artifact;
    head = res.version;
    newerHead = null;
    draftImpact = null;
    impactFor = '';
    if (res.links.broken.length) {
      toasts.warn('Saved with link problems', `${plural(res.links.broken.length, 'logo reference')} ${res.links.broken.length === 1 ? 'points' : 'point'} at a missing design.`);
    }
    void loadUsage();
  }

  async function resolveConflict(): Promise<string | null> {
    let latest: DesignVersion | null = null;
    try {
      latest = (await api.listVersions(id, { limit: 1 }))[0] ?? null;
    } catch {
      /* unknown author */
    }
    const who = latest ? `v${latest.seq}${latest.author_kind === 'agent' ? ' by Otto' : ''}` : 'a newer version';
    const { value } = await confirmer.choose(`Someone saved ${who} of this kit while you were editing. Your changes aren’t saved yet.`, {
      title: 'This brand kit changed',
      options: [
        { label: 'Save mine on top', value: 'mine', kind: 'primary' },
        { label: `Load ${latest ? `v${latest.seq}` : 'latest'} (discard my edits)`, value: 'theirs', kind: 'danger' },
      ],
    });
    if (value === 'theirs') await load();
    else if (value === 'mine' && latest) return latest.id;
    else if (value === 'mine') toasts.error('Couldn’t read the current version', 'Your edits are kept. Try Save again to retry.');
    return null;
  }

  async function revert(): Promise<void> {
    const ok = await confirmer.ask('Discard your unsaved changes to this kit?', { title: 'Revert changes', danger: true, confirmLabel: 'Discard changes' });
    if (!ok) return;
    const parsed = parseBrandDoc(baseText);
    if (parsed) doc = parsed;
  }

  // ── Approve ───────────────────────────────────────────────────────────────
  async function approve(): Promise<void> {
    if (!artifact || !head || dirty) return;
    // Approval belongs to the version the person sees in this sheet. Live
    // content updates may move head while the confirmation is open.
    const target = id;
    const version = head;
    const followers = usage?.consumers.filter((c) => c.policy === 'follow_approved').length ?? 0;
    const ok = await confirmer.ask(
      `Accept v${version.seq} of “${artifact.title}”?` +
        (followers ? ` ${followers} ${followers === 1 ? 'design that follows' : 'designs that follow'} the approved kit will switch to v${version.seq}.` : ''),
      { title: 'Accept brand kit', confirmLabel: `Accept v${version.seq}`, danger: false },
    );
    if (!ok) return;
    try {
      artifact = await api.approveArtifact(target, version.id);
      approved = version;
      toasts.success(`Accepted v${version.seq}`, followers ? 'Following designs re-tint now; pinned ones get an update prompt.' : undefined);
      void loadUsage();
    } catch (e) {
      toasts.error('Couldn’t accept the brand kit', errText(e));
    }
  }

  // ── Exports ───────────────────────────────────────────────────────────────
  const EXPORTS: { fmt: BrandExportFormat; label: string }[] = [
    { fmt: 'css', label: 'CSS variables' },
    { fmt: 'tailwind', label: 'Tailwind @theme' },
    { fmt: 'dtcg', label: 'DTCG tokens (JSON)' },
  ];
  async function doExport(fmt: BrandExportFormat, how: 'copy' | 'download'): Promise<void> {
    try {
      const r = await brandExport(id, fmt);
      const label = EXPORTS.find((x) => x.fmt === fmt)?.label ?? fmt;
      if (how === 'copy') await copyWithToast(r.text, label);
      else downloadText(r.text, r.fileName, r.mime);
      if (dirty && head) toasts.info(`Exported the saved v${head.seq}`, 'Save to include your unsaved changes.');
    } catch (e) {
      toasts.error('Couldn’t export the kit', errText(e));
    }
  }
  function exportsMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      ...EXPORTS.map((x) => ({ label: `Copy ${x.label}`, icon: 'copy', action: () => void doExport(x.fmt, 'copy') })),
      { separator: true },
      ...EXPORTS.map((x) => ({ label: `Download ${x.label}`, icon: 'download', action: () => void doExport(x.fmt, 'download') })),
    ];
    ctxMenu.show(e, items);
  }

  // ── Kit switcher ──────────────────────────────────────────────────────────
  function kitMenu(e: MouseEvent): void {
    const items: MenuItem[] = kits.map((h) => ({
      label: h.artifact.title,
      checked: h.artifact.id === id,
      icon: 'palette',
      action: () => router.go(`design/brand/${encodeURIComponent(h.artifact.id)}`),
    }));
    items.push({ separator: true, pinned: true }, { label: 'New brand kit…', icon: 'plus', pinned: true, disabled: !canEdit, action: onnew });
    ctxMenu.show(e, items, kits.length > 8 ? { filter: true, filterPlaceholder: 'Find a kit' } : undefined);
  }

  // ── Navigation within the page ────────────────────────────────────────────
  const SECTIONS: [string, string][] = [
    ['brand-colors', 'Colors'],
    ['brand-type', 'Typography'],
    ['brand-space', 'Spacing & radius'],
    ['brand-logos', 'Logos'],
    ['brand-imagery', 'Imagery'],
    ['brand-voice', 'Voice'],
  ];
  function jump(target: string): void {
    const reduced = typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
    document.getElementById(target)?.scrollIntoView({ behavior: reduced ? 'auto' : 'smooth', block: 'start' });
  }

  function onKey(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && !e.shiftKey && e.key.toLowerCase() === 's') {
      e.preventDefault();
      void requestSave();
    }
  }

  // ── Live updates ──────────────────────────────────────────────────────────
  let seen = designBus.seq;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      const evs = designBus.since(seen);
      seen = now;
      let usageDirty = false;
      for (const ev of evs) {
        if (ev.type === 'design_artifact_updated' && ev.artifact_id === id) {
          if (ev.change === 'deleted') phase = 'gone';
          else if (ev.change === 'content' || ev.change === 'created') {
            if (ev.version_id && ev.version_id === baseVersionId) continue; // our own save
            if (dirty) newerHead = ev.version_id;
            else void load();
          } else void refreshMeta();
        } else if (ev.type === 'design_link_updated' && ev.target_artifact_id === id) {
          usageDirty = true;
        } else if (ev.type === 'design_artifact_updated' && usage?.consumers.some((c) => c.artifact.id === ev.artifact_id)) {
          usageDirty = true;
        }
      }
      if (usageDirty) usageSoon();
    });
  });
  $effect(() => {
    const t = designBus.resyncTick;
    if (t === 0) return;
    untrack(() => {
      if (!dirty) void load();
      else void refreshMeta();
    });
  });
;
async () => {



 { svelteHTML.createElement("svelte:window", {  "onkeydown":onKey,});}

if(phase === 'loading'){
   { const $$_redaeHegaP0C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP0C({ target: __sveltets_2_any(), props: {      "title":`Brand Kit`,"leading":brandBack,"crumbs":viewport.isPhone ? [] : [{ label: 'Design Hall', onclick: () => router.go('design') }],}});}
   { const $$_ydoBegaP0C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP0C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}}); { const $$_notelekS1C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS1C({ target: __sveltets_2_any(), props: {    "rows":3,"height":160,}});} PageBody}
} else if (phase === 'error'){
   { const $$_redaeHegaP0C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP0C({ target: __sveltets_2_any(), props: {      "title":`Brand Kit`,"leading":brandBack,"crumbs":viewport.isPhone ? [] : [{ label: 'Design Hall', onclick: () => router.go('design') }],}});}
   { const $$_ydoBegaP0C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP0C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`the brand kit`,"error":loadError || 'No details were reported.',"empty":true,"onretry":() => void load(),}});}
   PageBody}
} else if (phase === 'gone'){
   { const $$_redaeHegaP0C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP0C({ target: __sveltets_2_any(), props: {      "title":`Brand Kit`,"leading":brandBack,"crumbs":viewport.isPhone ? [] : [{ label: 'Design Hall', onclick: () => router.go('design') }],}});}
   { const $$_ydoBegaP0C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP0C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {            "variant":`page`,"icon":`palette`,"title":`This brand kit is gone`,"body":`It was deleted, or you no longer have access to its workspace.`,"actionLabel":`All brand kits`,"onaction":() => router.go('design/brand'),}});}
   PageBody}
} else if (phase === 'invalid' && artifact){
   { const $$_redaeHegaP0C = __sveltets_2_ensureComponent(PageHeader); new $$_redaeHegaP0C({ target: __sveltets_2_any(), props: {      "title":artifact.title,"leading":brandBack,"crumbs":viewport.isPhone ? [] : [{ label: 'Design Hall', onclick: () => router.go('design') }],}});}
   { const $$_ydoBegaP0C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP0C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {            "variant":`page`,"icon":`palette`,"title":`This design isn’t a brand kit`,"body":`Open this design in its studio instead.`,"actionLabel":`Open the design`,"onaction":() => router.go(`design/a/${encodeURIComponent(id)}`),}});}
   PageBody}
} else if (artifact){
   { const $$_redaeHegaP0C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP0 = new $$_redaeHegaP0C({ target: __sveltets_2_any(), props: {     "title":artifact.title,"leading":brandBack,"crumbs":viewport.isPhone ? [] : [{ label: 'Design Hall', onclick: () => router.go('design') }, { label: 'Brand Kit', onclick: () => router.go('design/brand') }],titleContent:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {         "class":`switcher`,"onclick":kitMenu,"aria-haspopup":`menu`,"title":`Switch brand kit`,...__sveltets_2_empty({"data-testid":`brand-switcher`}),});
         { svelteHTML.createElement("span", {   "class":`kit-tile`,"aria-hidden":`true`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`palette`,"size":12,}});} }
         { svelteHTML.createElement("span", { "class":`kit-name`,});artifact?.title; }
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
       }
    };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
      if(head){ { svelteHTML.createElement("span", {   "class":`vbadge mono`,"title":`Current version`,}); head.seq; }}
       { const $$_lliPsutatS0C = __sveltets_2_ensureComponent(StatusPill); new $$_lliPsutatS0C({ target: __sveltets_2_any(), props: {  "status":needsApproval && artifact?.status === 'approved' ? 'draft' : (artifact?.status ?? 'draft'),}});}
    };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {         "class":`btn small`,...__sveltets_2_empty({"data-overflow":`2`}),"onclick":() => jump('brand-used'),...__sveltets_2_empty({"data-testid":`brand-used-btn`}),...__sveltets_2_empty({"data-icon":`link`}),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`link`,"size":12,}});}   usage ? usage.artifact_count : '…';
       }
       { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-overflow":`1`}),"onclick":exportsMenu,"aria-haspopup":`menu`,...__sveltets_2_empty({"data-testid":`brand-exports`}),...__sveltets_2_empty({"data-icon":`download`}),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});}   { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});}
       }
       { svelteHTML.createElement("button", {         "class":`btn small`,...__sveltets_2_empty({"data-overflow":`0`}),"onclick":() => router.go(`design/a/${encodeURIComponent(id)}`),"title":`Versions, compare and the JSON source`,...__sveltets_2_empty({"data-icon":`file`}),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}   
       }
      if(needsApproval && !dirty && canEdit){
         { svelteHTML.createElement("button", {         "class":`btn small`,...__sveltets_2_empty({"data-overflow":`3`}),"onclick":approve,...__sveltets_2_empty({"data-testid":`brand-approve`}),...__sveltets_2_empty({"data-icon":`check`}),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  head?.seq; }
      }
      if(canEdit){
         { svelteHTML.createElement("button", {         "class":`btn small primary`,"onclick":() => void requestSave(),"disabled":!dirty || saving || issues.length > 0,...__sveltets_2_empty({"data-testid":`brand-save`}),"title":`Save as a new version (⌘S)`,});
          saving ? 'Saving…' : `Save as v${nextSeq}`;
         }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {titleContent, badge, actions} = $$_redaeHegaP0.$$prop_def;/*Ωignore_endΩ*/
    
    
    
   PageHeader}

   { const $$_ydoBegaP0C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP0C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
     { svelteHTML.createElement("div", {   "class":`brand-grid`,...__sveltets_2_empty({"data-testid":`brand-editor`}),});
       { svelteHTML.createElement("div", { "class":`main`,});
         { svelteHTML.createElement("nav", {   "class":`bnav`,"aria-label":`Brand sections`,});
             for(let [target, label] of __sveltets_2_ensureArray(SECTIONS)){target;
             { svelteHTML.createElement("button", {   "class":`chip navchip`,"onclick":() => jump(target),});label; }
          }
         }

        if(readonly){
           { svelteHTML.createElement("p", { "class":`banner neutral`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":14,}});}                   }
        }

        if(newerHead){
           {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),('A newer version of this kit was saved while you were editing.')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`banner warn`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
             { svelteHTML.createElement("span", {});            }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void load(),});    }
           }}
        }

        if(dirty && issues.length > 0){
           {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(`Fix ${issues.length === 1 ? 'this' : `${issues.length} problems`} before saving: ${issues[0]}`)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`banner bad`,...__sveltets_2_empty({"data-testid":`brand-issues`}),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
             { svelteHTML.createElement("span", {}); issues.length === 1 ? 'this' : `${issues.length} problems`;    { svelteHTML.createElement("span", { "class":`mono`,});issues[0]; } }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => void revert(),});  }
           }}
        } else if (dirty){
           {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),(bannerNote)));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {     "class":`banner info`,...__sveltets_2_empty({"data-testid":`brand-banner`}),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":14,}});}
             { svelteHTML.createElement("span", {});
              if(localChanges.length === 0){      } else if (impactFresh && draftImpact){
                if(draftImpact.affected_count > 0){ localChanges.length === 1 ? 'a token' : `${localChanges.length} tokens`; 
                   { svelteHTML.createElement("b", {});draftImpact.affected_count; draftImpact.affected_count === 1 ? 'artifact' : 'artifacts';  draftImpact.affected_studio_count; draftImpact.affected_studio_count === 1 ? 'studio' : 'studios'; } } else if (draftImpact.artifact_count > 0){
                  localChanges.length === 1 ? 'This change doesn’t' : 'These changes don’t';     draftImpact.artifact_count;     }else{         }
              } else if (impactError){      }else{    localChanges.length === 1 ? 'this token' : 'these tokens'; }
             }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
            if(localChanges.length > 0){
               { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":openImpact,...__sveltets_2_empty({"data-testid":`brand-preview-impact`}),});  }
            }
             { svelteHTML.createElement("button", {       "class":`btn small`,"onclick":() => void requestSave(),"disabled":saving,...__sveltets_2_empty({"data-testid":`brand-banner-save`}),});  nextSeq; }
             { svelteHTML.createElement("button", {   "class":`btn small ghost`,"onclick":() => void revert(),});  }
           }}
        } else if (needsApproval && head && canEdit){
           { svelteHTML.createElement("div", {   "class":`banner neutral`,...__sveltets_2_empty({"data-testid":`brand-approval`}),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":14,}});}
             { svelteHTML.createElement("span", {});
              head.seq;           
              approved ? `still use v${approved.seq}` : 'pick it up once you accept it';
             }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":approve,}); head.seq; }
           }
        }

         { const $$_noitceSsroloC3C = __sveltets_2_ensureComponent(ColorsSection); const $$_noitceSsroloC3 = new $$_noitceSsroloC3C({ target: __sveltets_2_any(), props: {     doc,readonly,"rules":kitRules,}});/*Ωignore_startΩ*/() => doc = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_noitceSsroloC3.$$bindings = 'doc';}
         { const $$_noitceSepyT3C = __sveltets_2_ensureComponent(TypeSection); const $$_noitceSepyT3 = new $$_noitceSepyT3C({ target: __sveltets_2_any(), props: {   doc,readonly,}});/*Ωignore_startΩ*/() => doc = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_noitceSepyT3.$$bindings = 'doc';}
         { const $$_noitceSecapS3C = __sveltets_2_ensureComponent(SpaceSection); const $$_noitceSecapS3 = new $$_noitceSecapS3C({ target: __sveltets_2_any(), props: {   doc,readonly,}});/*Ωignore_startΩ*/() => doc = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_noitceSecapS3.$$bindings = 'doc';}
         { const $$_noitceSsogoL3C = __sveltets_2_ensureComponent(LogosSection); const $$_noitceSsogoL3 = new $$_noitceSsogoL3C({ target: __sveltets_2_any(), props: {       doc,readonly,"workspaceId":artifact.workspace_id,"projectId":artifact.project_id,}});/*Ωignore_startΩ*/() => doc = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_noitceSsogoL3.$$bindings = 'doc';}
         { const $$_noitceSesorP3C = __sveltets_2_ensureComponent(ProseSection); const $$_noitceSesorP3 = new $$_noitceSesorP3C({ target: __sveltets_2_any(), props: {    doc,"group":`imagery`,readonly,}});/*Ωignore_startΩ*/() => doc = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_noitceSesorP3.$$bindings = 'doc';}
         { const $$_noitceSesorP3C = __sveltets_2_ensureComponent(ProseSection); const $$_noitceSesorP3 = new $$_noitceSesorP3C({ target: __sveltets_2_any(), props: {    doc,"group":`voice`,readonly,}});/*Ωignore_startΩ*/() => doc = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_noitceSesorP3.$$bindings = 'doc';}
         { const $$_nIdesU3C = __sveltets_2_ensureComponent(UsedIn); new $$_nIdesU3C({ target: __sveltets_2_any(), props: {          "usage":usage,"loading":usageLoading,"error":usageError,"kitId":id,"onretry":() => void loadUsage(),}});}
       }
       { svelteHTML.createElement("aside", {   "class":`side`,"aria-label":`Applied to — live preview`,});
         { const $$_weiverPdeilppA3C = __sveltets_2_ensureComponent(AppliedPreview); new $$_weiverPdeilppA3C({ target: __sveltets_2_any(), props: { doc,}});}
       }
     }
   PageBody}

  if(showImpact){
     { const $$_ladoMtcapmI0C = __sveltets_2_ensureComponent(ImpactModal); new $$_ladoMtcapmI0C({ target: __sveltets_2_any(), props: {                   "impact":impactFresh ? draftImpact : null,"loading":impactLoading,"error":impactError,nextSeq,"approvedSeq":approved?.seq ?? null,"canSave":dirty && issues.length === 0 && canEdit,saving,"onclose":() => (showImpact = false),"onsave":() => void save(),"onretry":() => void runImpact(draftText),}});}
  }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BrandEditor__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BrandEditor__SvelteComponent_ = ReturnType<typeof BrandEditor__SvelteComponent_>;
/*Ωignore_endΩ*/export default BrandEditor__SvelteComponent_;