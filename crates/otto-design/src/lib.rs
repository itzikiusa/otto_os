//! otto-design — the Design Hall artifact graph (Phase 0 backend).
//!
//! One typed graph behind every design studio (Frames, Graphics, Site, 3D,
//! Whiteboard, Brand Kit, Spatial Hall): **projects** group **artifacts**; every
//! save commits an immutable **version** whose bytes live in a
//! content-addressed blob store; **links** (typed, version-aware) are extracted
//! from each document's `otto://design/<id>[@approved|@latest|@v<n>][#node]`
//! URIs on every save, plus explicit links made by users/agents; **signals**
//! capture design decisions from day one (nothing learns from them yet);
//! **publishes** pin the exact version set a release embedded.
//!
//! Storage is file-backed (docs/features/design-hall.md §2):
//!   - blobs:        `<data>/design/blobs/<sha256>` (dedup; never auto-deleted)
//!   - working copy: `<data>/design/<artifact>/work/<file>` (text formats only —
//!                   where an agent edits in place; `POST …/versions` snapshots it)
//!   - SQLite rows:  metadata only (`design_*` tables + the runtime FTS5 index).
//!
//! Legacy design data is mirrored, never moved: `import` creates graph rows for
//! `product_attachments` (design/mockup + images/models) and `canvas_scenes`
//! idempotently (keyed on `source_kind`/`source_id`), links them to their
//! stories (`implements`) and re-syncs a new version when the source changed.
//!
//! Routing (server nests this under `/api/v1`): every route is flat
//! `/design/…`; the workspace comes from the request body or the row. Reads
//! require workspace `Viewer`, writes `Editor`, hard deletes `Admin`; the
//! `Feature::Design` capability axis is enforced upstream by the server's
//! deny-by-default policy middleware (`/design/admin/*` = Design:Admin).
//!
//! Phase 1 agent co-design building blocks (the agent turn itself runs in
//! `otto-server`'s `design_assist`): `variants` (side branches
//! `variant/<run>/<k>` + accept = fast-forward main), `cite` (verifying an
//! agent's `[R1]` citations against the references it was offered) and
//! `learn` (the deterministic signal → candidate-rule extractor).

pub mod blobs;
pub mod brand;
pub mod cite;
pub mod diff;
pub mod extract;
pub mod format;
pub mod graph;
pub mod http;
pub mod import;
pub mod learn;
pub mod retention;
pub mod service;
pub mod site;
pub mod store;
pub mod types;
pub mod uri;
pub mod variants;

pub use http::{router, DesignCtx};
pub use service::DesignService;
pub use types::*;
