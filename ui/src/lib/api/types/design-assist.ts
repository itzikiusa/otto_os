// Contract mirror: design-assist. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';
import type {
  DesignArtifact, DesignLinkPolicy, DesignStatus, DesignStudio, DesignVersion,
} from './design';

// ---- Design assist (the unified agent turn, variants, learned rules) ------
// Mirrors crates/otto-design-assist/src/lib.rs + otto-design cite/learn.

/** `POST /design/artifacts/{id}/assist` modes (`variant` is `/variants` only). */
export type DesignAssistMode = 'generate' | 'refine' | 'critique' | 'a11y';
export type DesignAssistStatus =
  | 'starting'
  | 'running'
  /** A version was committed. */
  | 'done'
  /** The agent changed nothing (always for a critique). */
  | 'unchanged'
  /** The head moved while the agent worked: its draft was kept as the side
   *  version `variant/<turn_id>/1` (accept it via `…/variants/{v}/accept`). */
  | 'conflict'
  | 'failed';

/** A reference offered to a turn as `[R<n>]`. */
export interface DesignOfferedRef {
  /** `R1`, `R2`, … */
  label: string;
  artifact_id: Id;
  version_id: Id | null;
  seq: number | null;
  title: string;
  studio: DesignStudio | string;
  format: string;
  status: DesignStatus | string;
  /** Why it was offered. */
  source: 'explicit' | 'link' | 'search';
}

/** A citation the server verified against the offered set. */
export interface DesignCitedRef {
  label: string;
  artifact_id: Id;
  version_id: Id | null;
  seq: number | null;
}

export interface DesignAssistReq {
  prompt: string;
  /** Default `refine`. */
  mode?: DesignAssistMode;
  /** The focused node/section, e.g. `{node_id: 'hero'}` (≤ 4 KB JSON). */
  selection?: Record<string, unknown>;
  /** `<artifact_id>`, `<artifact_id>@v12` or `otto://design/<id>[@v12]` (≤ 8). */
  references?: string[];
  /** Provider for a NEW assist session (a resumed one keeps its own). */
  provider?: string;
  model?: string;
}

/** One design-assist agent turn (main or one variant). */
export interface DesignAssistTurn {
  turn_id: Id;
  artifact_id: Id;
  workspace_id: Id;
  mode: DesignAssistMode | 'variant';
  status: DesignAssistStatus;
  /** `main` or `variant/<run>/<k>`. */
  branch: string;
  /** Variant direction label (`defaults`, `explore`, `calm`, `story`, or custom). */
  direction: string | null;
  provider: string;
  session_id: Id | null;
  base_version_id: Id | null;
  version_id: Id | null;
  references: DesignOfferedRef[];
  cited: DesignCitedRef[];
  /** Citations the agent made that could not be verified (not in provenance). */
  unverified_citations: string[];
  /** Keys of the approved team rules the turn was given. */
  team_rules: string[];
  /** Critique / a11y findings (`{severity?, rule?, message?, node_id?, fix?, fixed?}`). */
  findings: Record<string, unknown>[];
  /** The agent's one-line summary. */
  message: string | null;
  error: string | null;
  started_at: string;
  finished_at: string | null;
}

export interface DesignVariantsReq {
  prompt: string;
  /** 1..=4 (default 3). */
  n?: number;
  references?: string[];
  selection?: Record<string, unknown>;
  provider?: string;
  /** Per-variant providers, cycled (overrides `provider`). */
  providers?: string[];
  model?: string;
  /** Custom direction per variant, cycled; default defaults/explore/calm/story. */
  directions?: string[];
}

export interface DesignVariantRun {
  run_id: Id;
  artifact_id: Id;
  base_version_id: Id | null;
  status: 'running' | 'ready' | 'accepted';
  /** Committed variant versions (branch `variant/<run_id>/<k>`), k ascending. */
  versions: DesignVersion[];
  /** The accepted VARIANT version, once one was. */
  accepted_version_id: Id | null;
  /** Live turn states (in memory; empty after a daemon restart). */
  turns: DesignAssistTurn[];
}

export interface DesignVariantAcceptReq {
  /** Accept even though main moved since the variants were drawn. */
  force?: boolean;
}

export interface DesignVariantAcceptResp {
  artifact: DesignArtifact;
  /** The new main version (head) carrying the variant's bytes. */
  version: DesignVersion;
  run_id: Id;
  accepted_version_id: Id;
  rejected_version_ids: Id[];
}

/** A candidate team rule from the deterministic signal extractor. */
export interface DesignRuleCandidate {
  /** e.g. `variant_direction:bold`, `edit_property:scene3d:material.color`. */
  key: string;
  kind: 'variant_preference' | 'reject_reason' | 'edit_after_draft' | 'a11y';
  rule: string;
  rationale: string;
  /** Evidence: design signal ids. */
  signal_ids: Id[];
  signal_count: number;
  artifact_count: number;
  /** Meets the thresholds (≥ 3 signals across ≥ 2 artifacts) → proposed. */
  ready: boolean;
}

export interface DesignRuleLine {
  key: string;
  rule: string;
}

/** An active learned rule (a line of the `design-team-style` skill). */
export interface DesignLearnedRule {
  key: string;
  rule: string;
  /** The applied improvement edit that added it (roll back via
   *  `POST /improvement/edits/{edit_id}/rollback`). */
  edit_id: Id | null;
  evidence: Id[];
  applied_at: string | null;
}

/** One improvement edit of the design skill (approve / reject via
 *  `POST /improvement/edits/{edit_id}/approve|reject`). */
export interface DesignLearnedEdit {
  edit_id: Id;
  status: 'pending' | 'applied' | 'rejected' | 'rolled_back' | 'conflict';
  rules: DesignRuleLine[];
  rationale: string;
  evidence: Id[];
  created_at: string;
  applied_at: string | null;
  actor: string | null;
}

export interface DesignLearnedResp {
  workspace_id: Id;
  /** Workspace setting `design_learning` (`off`) — default `suggest`. */
  mode: 'suggest' | 'off';
  skill: string;
  skill_path: string;
  active: DesignLearnedRule[];
  pending: DesignLearnedEdit[];
  history: DesignLearnedEdit[];
  candidates: DesignRuleCandidate[];
}

export interface DesignLearnExtractReq {
  workspace_id: Id;
}

export interface DesignLearnExtractResp {
  mode: 'suggest' | 'off';
  run_id: Id | null;
  /** Newly proposed (pending) improvement edit ids. */
  proposed: Id[];
  skipped: number;
  candidates: DesignRuleCandidate[];
}

// ── Site Studio (otto-site v1): static export, local preview, publishes ─────
// Mirrors crates/otto-design/src/site/{export,http}.rs; contract:
// docs/contracts/api.md § "Site Studio". The document schema itself is typed in
// ui/src/modules/design-hall/site/engine/types.ts (it is content, not wire DTOs).

/** One row of `design_publishes`: a publish of ONE version, with the exact
 *  versions of everything it rendered (brand kit, 3D embeds, images). */
export interface DesignPublish {
  id: Id;
  artifact_id: Id;
  version_id: Id;
  /** `zip` | `local` today; `artifact` / `gh_pages` / `netlify` / `cloudflare` are reserved. */
  target: string;
  url: string | null;
  /** The pinned set: the site version first (`role: "site"`), then its brand
   *  kit, then every rendered 3D embed / image. */
  pinned_set: DesignPinnedRef[];
  created_by: Id;
  created_at: string;
}

/** One entry of a publish's pinned set (a reference resolved to one version). */
export interface DesignPinnedRef {
  /** `site` | `brand` | `embed` | `image`. */
  role: string;
  /** The reference as the document wrote it (`otto://design/<id>@approved`; the site: `…@v<seq>`). */
  uri: string;
  artifact_id: Id;
  version_id: Id | null;
  seq: number | null;
  title: string;
  /** What the document asked for: `follow_approved` | `follow_latest` | `pinned`. */
  policy: string;
  /** Unresolvable (missing artifact/version, no access) — rendered as a stand-in. */
  missing: boolean;
  /** Poster blob captured for an `embed` pin. Both poster fields are optional:
   *  an older `otto-publish.json` row that omits them still parses
   *  (`poster_pinned` absent means the same as false). `null` together with
   *  `poster_pinned: true` is a new publish that pinned the absence of a
   *  poster. A historical publish (field omitted or `poster_pinned: false`)
   *  did not capture a poster and renders a stand-in, not the artifact's
   *  later thumbnail. */
  poster_blob?: string | null;
  /** `true` when this publish stored a poster decision, including a decision
   *  that there was no poster. Omitted or `false` on manifests from before
   *  posters were captured. */
  poster_pinned?: boolean;
}

/** `POST /design/artifacts/{id}/export`. */
export interface DesignSiteExportReq {
  /** `zip` → the static site as a download; `local` → a loopback preview URL. */
  target: 'zip' | 'local';
  /** Version id / `v12` / `12`; default the head. */
  version?: string;
}

export interface DesignSitePage {
  id: string;
  title: string;
  slug: string;
  /** File name in the zip (`index.html`, `tiers.html`). */
  file: string;
  /** Loopback preview path of this page (bearer-authenticated). */
  url: string;
}

/** `POST …/export {target:"local"}` (the zip target answers with the archive). */
export interface DesignSiteLocalResp {
  publish: DesignPublish;
  /** `/api/v1/design/artifacts/{id}/preview?publish=<id>`. */
  url: string;
  pages: DesignSitePage[];
  pinned: DesignPinnedRef[];
  /** Non-fatal problems (a missing embed rendered as a stand-in, …). */
  warnings: string[];
}

// ---------------------------------------------------------------------------
// Design Hall — Brand Kit (`otto-brand/1`; mirrors crates/otto-design/src/brand;
// contract: docs/contracts/api.md § Brand Kit). Other studios reference tokens
// as `token:<group>.<name>` (or `var(--brand-<group>-<name>)`) and link to the
// kit with rel `uses_tokens`; helpers: ui/src/modules/design-hall/brand/tokens.ts.
// ---------------------------------------------------------------------------

export type BrandTokenGroup = 'color' | 'font' | 'type' | 'radius' | 'space';
export type BrandFontRole = 'display' | 'body' | 'mono';
export type BrandLogoKind = 'full' | 'mark' | 'mono';

export interface BrandColorToken {
  /** `#RGB` / `#RRGGBB` / `#RRGGBBAA`. */
  $value: string;
  $description?: string;
}

export interface BrandFontToken {
  /** A CSS family stack (`"Inter", system-ui, sans-serif`); no `; { } < > \`. */
  $value: string;
  /** 1–1000, at most 9. */
  weights?: number[];
}

/** A text style: size and line height in px, weight 1–1000. */
export interface BrandTypeStyle {
  size: number;
  line: number;
  weight: number;
}

/** A radius / space token in px (0–10 000). */
export interface BrandPxToken {
  $value: number;
  $description?: string;
}

export interface BrandLogo {
  name: string;
  kind: BrandLogoKind;
  /** `otto://design/<id>` (an image artifact) or `blob:<sha256>`; empty = a
   *  placeholder slot, not uploaded yet. */
  asset?: string;
}

/** `voice` / `imagery`: a summary and do / don't examples (≤ 20 each). */
export interface BrandProse {
  summary?: string;
  do?: string[];
  dont?: string[];
}

/** The `otto-brand/1` document (DTCG-flavoured). Every group is optional on
 *  the wire; token names are `[A-Za-z0-9][A-Za-z0-9_-]{0,63}`, ≤ 64 per group. */
export interface BrandDoc {
  $schema: 'otto-brand/1';
  name: string;
  color: Record<string, BrandColorToken>;
  font: Partial<Record<BrandFontRole, BrandFontToken>>;
  type?: Record<string, BrandTypeStyle>;
  radius: Record<string, BrandPxToken>;
  space: Record<string, BrandPxToken>;
  logos: BrandLogo[];
  imagery?: BrandProse;
  voice?: BrandProse;
}

export type BrandExportFormat = 'css' | 'tailwind' | 'dtcg';

export interface BrandImpactReq {
  /** The proposed kit (object or JSON text); omitted = the head (a plain
   *  "used in" listing with no changes). */
  content?: BrandDoc | string;
}

export interface BrandTokenChange {
  /** `group.name`, e.g. `color.primary`. */
  token: string;
  change: 'changed' | 'added' | 'removed';
  /** Canonical values: `#5B3DF5`, `14px`, `64/72/800` (size/line/weight) … */
  before: string | null;
  after: string | null;
}

export interface BrandStudioCount {
  studio: DesignStudio;
  count: number;
}

export interface BrandConsumer {
  artifact: DesignArtifact;
  /** `follow_approved` moves when a kit version is approved; `follow_latest`
   *  on every save; `pinned` stays until updated. */
  policy: DesignLinkPolicy;
  pinned_version_id: Id | null;
  /** Tokens the consumer's head names (sorted). */
  tokens: string[];
  /** Names no single token → the whole kit applies (a site theme). */
  whole_kit: boolean;
  /** The changed tokens it would see. */
  affected: string[];
  /** false = the head isn't readable text (counted as whole-kit). */
  scanned: boolean;
}

export type BrandContrastLevel = 'AAA' | 'AA' | 'AA-large' | 'fail';

export interface BrandContrastColor {
  name: string;
  hex: string;
  on_white: number;
  on_ink: number;
  white_level: BrandContrastLevel;
  ink_level: BrandContrastLevel;
}

export interface BrandContrastPair {
  a: string;
  b: string;
  ratio: number;
  level: BrandContrastLevel;
}

export interface BrandContrastReport {
  /** The `ink` token, else `text`, else the darkest colour (null name = black). */
  ink: { name: string | null; hex: string };
  colors: BrandContrastColor[];
  /** Every unordered pair of colour tokens. */
  pairs: BrandContrastPair[];
}

export interface BrandImpactResp {
  kit_id: Id;
  /** What the proposal is compared with. */
  base: 'approved' | 'head' | 'none';
  base_version_id: Id | null;
  base_seq: number | null;
  changes: BrandTokenChange[];
  artifact_count: number;
  studio_count: number;
  by_studio: BrandStudioCount[];
  affected_count: number;
  affected_studio_count: number;
  affected_by_studio: BrandStudioCount[];
  /** Affected first, then newest. */
  consumers: BrandConsumer[];
  /** Consumers in workspaces the caller can't view. */
  hidden_count: number;
  contrast: BrandContrastReport;
  warnings: string[];
}
