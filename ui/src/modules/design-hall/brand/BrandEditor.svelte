<script lang="ts">
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
  import { copyWithToast } from './edit';
  import { diffTokens, normalizeBrandDoc, parseBrandDoc, rulesForKit, serializeBrandDoc, validateBrandDoc } from './tokens';

  interface Props {
    id: string;
    kits: DesignSearchHit[];
    onnew: () => void;
  }
  let { id, kits, onnew }: Props = $props();

  function errText(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

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

  async function load(): Promise<void> {
    if (phase !== 'ready') phase = 'loading';
    loadError = null;
    try {
      const d = await api.getArtifact(id, { content: true });
      artifact = d.artifact;
      head = d.head;
      approved = d.approved;
      if (d.artifact.format !== 'otto-brand') {
        phase = 'invalid';
        return;
      }
      let text = d.content;
      if (text == null || d.content_truncated) text = (await api.fetchContent(id, { asText: true })).text ?? '{}';
      const parsed = parseBrandDoc(text, d.artifact.title) ?? normalizeBrandDoc({}, d.artifact.title);
      doc = parsed;
      baseText = serializeBrandDoc(parsed);
      baseVersionId = d.content_version_id ?? d.head?.id ?? null;
      newerHead = null;
      phase = 'ready';
      void loadUsage();
      void loadRules(d.artifact.workspace_id);
    } catch (e) {
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
      const res = await api.commitVersion(id, { content: text, base_version: base ?? '', message });
      applySaved(res, text);
      showImpact = false;
      if (!res.created) {
        toasts.info('No changes to save', 'The kit matches the current version.');
        return;
      }
      const followers = usage?.consumers.filter((c) => c.policy === 'follow_approved').length ?? 0;
      toasts.success(
        `Saved ${artifact?.title ?? 'the kit'} v${res.version.seq}`,
        followers ? `${followers} ${followers === 1 ? 'design follows' : 'designs follow'} the approved kit — approve v${res.version.seq} to roll it out.` : undefined,
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
      if (e instanceof ApiError && e.status === 409) await resolveConflict();
      else toasts.error('Couldn’t save the brand kit', errText(e));
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
      toasts.warn('Saved with link problems', `${res.links.broken.length} logo reference${res.links.broken.length === 1 ? '' : 's'} point at a missing design.`);
    }
    void loadUsage();
  }

  async function resolveConflict(): Promise<void> {
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
    else if (value === 'mine' && latest) await save(latest.id);
    else if (value === 'mine') await load();
  }

  async function revert(): Promise<void> {
    const ok = await confirmer.ask('Discard your unsaved changes to this kit?', { title: 'Revert changes', confirmLabel: 'Discard changes' });
    if (!ok) return;
    const parsed = parseBrandDoc(baseText);
    if (parsed) doc = parsed;
  }

  // ── Approve ───────────────────────────────────────────────────────────────
  async function approve(): Promise<void> {
    if (!artifact || !head || dirty) return;
    const followers = usage?.consumers.filter((c) => c.policy === 'follow_approved').length ?? 0;
    const ok = await confirmer.ask(
      `Approve v${head.seq} of “${artifact.title}”?` +
        (followers ? ` ${followers} ${followers === 1 ? 'design that follows' : 'designs that follow'} the approved kit will switch to v${head.seq}.` : ''),
      { title: 'Approve brand kit', confirmLabel: `Approve v${head.seq}`, danger: false },
    );
    if (!ok) return;
    try {
      artifact = await api.approveArtifact(id, head.id);
      approved = head;
      toasts.success(`Approved v${head.seq}`, followers ? 'Following designs re-tint now; pinned ones get an update prompt.' : undefined);
      void loadUsage();
    } catch (e) {
      toasts.error('Couldn’t approve', errText(e));
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
      label: h.artifact.id === id ? `✓ ${h.artifact.title}` : h.artifact.title,
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
</script>

<svelte:window onkeydown={onKey} />

{#if phase === 'loading'}
  <PageHeader title="Brand Kit" crumbs={[{ label: 'Design Hall', onclick: () => router.go('design') }]} />
  <PageBody><Skeleton rows={3} height={160} /></PageBody>
{:else if phase === 'error'}
  <PageHeader title="Brand Kit" crumbs={[{ label: 'Design Hall', onclick: () => router.go('design') }]} />
  <PageBody>
    <p class="err" role="alert"><Icon name="warning" size={14} /> Couldn’t load the brand kit. <span class="dim">{loadError}</span> <button class="btn small" onclick={() => void load()}>Retry</button></p>
  </PageBody>
{:else if phase === 'gone'}
  <PageHeader title="Brand Kit" crumbs={[{ label: 'Design Hall', onclick: () => router.go('design') }]} />
  <PageBody>
    <EmptyState variant="page" icon="palette" title="This brand kit is gone" body="It was deleted, or you no longer have access to its workspace." actionLabel="All brand kits" onaction={() => router.go('design/brand')} />
  </PageBody>
{:else if phase === 'invalid' && artifact}
  <PageHeader title={artifact.title} crumbs={[{ label: 'Design Hall', onclick: () => router.go('design') }]} />
  <PageBody>
    <EmptyState variant="page" icon="palette" title="This design isn’t a brand kit" body="Only otto-brand documents open in the Brand Kit editor." actionLabel="Open the design" onaction={() => router.go(`design/a/${encodeURIComponent(id)}`)} />
  </PageBody>
{:else if artifact}
  <PageHeader title={artifact.title} crumbs={[{ label: 'Design Hall', onclick: () => router.go('design') }, { label: 'Brand Kit', onclick: () => router.go('design/brand') }]}>
    {#snippet titleContent()}
      <button class="switcher" onclick={kitMenu} aria-haspopup="menu" title="Switch brand kit" data-testid="brand-switcher">
        <span class="kit-tile" aria-hidden="true"><Icon name="palette" size={12} /></span>
        <span class="kit-name">{artifact?.title}</span>
        <Icon name="chevronDown" size={12} />
      </button>
    {/snippet}
    {#snippet badge()}
      {#if head}<span class="vbadge mono" title="Current version">v{head.seq}</span>{/if}
      <StatusPill status={artifact?.status ?? 'draft'} />
    {/snippet}
    {#snippet actions()}
      <button class="btn small" data-overflow="2" onclick={() => jump('brand-used')} data-testid="brand-used-btn" data-icon="link">
        <Icon name="link" size={12} /> Used in {usage ? usage.artifact_count : '…'}
      </button>
      <button class="btn small" data-overflow="1" onclick={exportsMenu} aria-haspopup="menu" data-testid="brand-exports" data-icon="download">
        <Icon name="download" size={12} /> Export <Icon name="chevronDown" size={12} />
      </button>
      <button class="btn small" data-overflow="0" onclick={() => router.go(`design/a/${encodeURIComponent(id)}`)} title="Versions, compare and the JSON source" data-icon="file">
        <Icon name="file" size={12} /> Versions &amp; source
      </button>
      {#if needsApproval && !dirty && canEdit}
        <button class="btn small" data-overflow="3" onclick={approve} data-testid="brand-approve" data-icon="check"><Icon name="check" size={12} /> Approve v{head?.seq}</button>
      {/if}
      {#if canEdit}
        <button class="btn small primary" onclick={() => void requestSave()} disabled={!dirty || saving || issues.length > 0} data-testid="brand-save" title="Save as a new version (⌘S)">
          {saving ? 'Saving…' : `Save as v${nextSeq}`}
        </button>
      {/if}
    {/snippet}
  </PageHeader>

  <PageBody>
    <div class="brand-grid" data-testid="brand-editor">
      <div class="main">
        <nav class="bnav" aria-label="Brand sections">
          {#each SECTIONS as [target, label] (target)}
            <button class="chip navchip" onclick={() => jump(target)}>{label}</button>
          {/each}
        </nav>

        {#if readonly}
          <p class="banner neutral"><Icon name="eye" size={14} /> You can view this kit but not edit it. Ask a workspace admin for Design Hall edit access.</p>
        {/if}

        {#if newerHead}
          <div class="banner warn" role="status">
            <Icon name="warning" size={14} />
            <span>A newer version of this kit was saved while you were editing.</span>
            <span class="grow"></span>
            <button class="btn small" onclick={() => void load()}>Load it (discard mine)</button>
          </div>
        {/if}

        {#if dirty && issues.length > 0}
          <div class="banner bad" role="alert" data-testid="brand-issues">
            <Icon name="warning" size={14} />
            <span>Fix {issues.length === 1 ? 'this' : `${issues.length} problems`} before saving: <span class="mono">{issues[0]}</span></span>
            <span class="grow"></span>
            <button class="btn small ghost" onclick={() => void revert()}>Revert</button>
          </div>
        {:else if dirty}
          <div class="banner info" role="status" data-testid="brand-banner">
            <Icon name="info" size={14} />
            <span>
              {#if localChanges.length === 0}
                Unsaved changes to logos, imagery or voice.
              {:else if impactFresh && draftImpact}
                {#if draftImpact.affected_count > 0}
                  Changing {localChanges.length === 1 ? 'a token' : `${localChanges.length} tokens`} affects
                  <b>{draftImpact.affected_count} {draftImpact.affected_count === 1 ? 'artifact' : 'artifacts'} in {draftImpact.affected_studio_count} {draftImpact.affected_studio_count === 1 ? 'studio' : 'studios'}</b>.
                {:else if draftImpact.artifact_count > 0}
                  {localChanges.length === 1 ? 'This change doesn’t' : 'These changes don’t'} reach any of the {draftImpact.artifact_count} designs that use this kit.
                {:else}
                  No designs use this kit yet — nothing else changes.
                {/if}
              {:else if impactError}
                Couldn’t check which designs use these tokens.
              {:else}
                Checking which designs use {localChanges.length === 1 ? 'this token' : 'these tokens'}…
              {/if}
            </span>
            <span class="grow"></span>
            {#if localChanges.length > 0}
              <button class="btn small" onclick={openImpact} data-testid="brand-preview-impact">Preview impact</button>
            {/if}
            <button class="btn small" onclick={() => void requestSave()} disabled={saving} data-testid="brand-banner-save">Save as v{nextSeq}</button>
            <button class="btn small ghost" onclick={() => void revert()}>Revert</button>
          </div>
        {:else if needsApproval && head && canEdit}
          <div class="banner neutral" data-testid="brand-approval">
            <Icon name="info" size={14} />
            <span>
              v{head.seq} is saved but not approved. Designs that follow the approved kit
              {approved ? `still use v${approved.seq}` : 'pick it up once you approve it'}.
            </span>
            <span class="grow"></span>
            <button class="btn small" onclick={approve}>Approve v{head.seq}</button>
          </div>
        {/if}

        <ColorsSection bind:doc {readonly} rules={kitRules} />
        <TypeSection bind:doc {readonly} />
        <SpaceSection bind:doc {readonly} />
        <LogosSection bind:doc {readonly} workspaceId={artifact.workspace_id} projectId={artifact.project_id} />
        <ProseSection bind:doc group="imagery" {readonly} />
        <ProseSection bind:doc group="voice" {readonly} />
        <UsedIn usage={usage} loading={usageLoading} error={usageError} kitId={id} onretry={() => void loadUsage()} />
      </div>
      <aside class="side" aria-label="Applied to — live preview">
        <AppliedPreview {doc} />
      </aside>
    </div>
  </PageBody>

  {#if showImpact}
    <ImpactModal
      impact={impactFresh ? draftImpact : null}
      loading={impactLoading}
      error={impactError}
      {nextSeq}
      approvedSeq={approved?.seq ?? null}
      canSave={dirty && issues.length === 0 && canEdit}
      {saving}
      onclose={() => (showImpact = false)}
      onsave={() => void save()}
      onretry={() => void runImpact(draftText)}
    />
  {/if}
{/if}

<style>
  .brand-grid {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 336px;
    gap: 24px;
    max-width: 1360px;
    margin: 0 auto;
    align-items: start;
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: 32px;
    min-width: 0;
  }
  .side {
    position: sticky;
    top: 0;
  }
  .bnav {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
    margin-block-end: -16px;
  }
  .navchip {
    cursor: pointer;
    color: var(--text);
  }
  .navchip:hover {
    background: var(--hover);
  }
  .banner {
    margin: 0;
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 12px;
    border-radius: var(--radius-m);
    flex-wrap: wrap;
    font-size: var(--fs-m);
    margin-block-end: -16px;
  }
  .banner.info {
    background: var(--info-soft);
    border: 1px solid color-mix(in srgb, var(--info) 30%, transparent);
  }
  .banner.info > :global(svg) {
    color: var(--info);
  }
  .banner.bad {
    background: var(--danger-soft);
    border: 1px solid color-mix(in srgb, var(--danger) 30%, transparent);
  }
  .banner.bad > :global(svg) {
    color: var(--danger);
  }
  .banner.warn {
    background: var(--warning-soft);
    border: 1px solid color-mix(in srgb, var(--warning) 30%, transparent);
  }
  .banner.warn > :global(svg) {
    color: var(--warning);
  }
  .banner.neutral {
    background: var(--surface);
    border: 1px solid var(--border);
    color: var(--text);
  }
  .banner.neutral > :global(svg) {
    color: var(--text-dim);
  }
  .banner b {
    font-weight: 600;
  }
  .banner .mono {
    font-size: var(--fs-s);
  }
  .grow {
    flex: 1;
  }
  .switcher {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-width: 100%;
    padding: 2px 6px;
    margin-inline-start: -6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-weight: 600;
    cursor: pointer;
    min-width: 0;
  }
  .switcher:hover {
    background: var(--hover);
  }
  .kit-tile {
    width: 18px;
    height: 18px;
    border-radius: var(--radius-s);
    display: grid;
    place-items: center;
    background: var(--studio-brand);
    color: var(--studio-glyph);
    flex: none;
  }
  .kit-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .vbadge {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .err {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
    font-size: var(--fs-s);
  }
  .err > :global(svg) {
    color: var(--danger);
  }
  .dim {
    color: var(--text-dim);
  }
  @media (max-width: 1100px) {
    .brand-grid {
      grid-template-columns: minmax(0, 1fr) 280px;
    }
  }
  @media (max-width: 900px) {
    .brand-grid {
      grid-template-columns: minmax(0, 1fr);
    }
    .side {
      position: static;
    }
  }
</style>
