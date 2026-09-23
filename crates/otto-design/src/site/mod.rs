//! Site Studio v1 — the `otto-site` document (pages → sections → blocks) and
//! everything the daemon does with it (docs/features/design-hall.md § Site
//! Studio; docs/contracts/api.md § Site Studio):
//!
//! - [`schema`] — the lenient typed model + the block catalog (names only);
//! - [`validate`] — the structural validator every save runs through
//!   (`format::validate`), so an agent's invalid edit is refused with a path;
//! - [`extract`] — links (`otto://design/…` by key: `src`/`image`/`poster`
//!   embed, `brand` uses_tokens, `derived_from` …) + searchable copy;
//! - [`theme`] — a brand kit (`otto-brand/1`, via [`crate::brand::BrandModel`])
//!   → `--brand-<group>-<name>` custom properties bound to theme slots;
//! - [`render`] — the HTML renderer (the SAME markup the UI canvas renders,
//!   `ui/src/modules/design-hall/site/engine/render.ts`, against the same
//!   `site.css`, embedded here byte-identical to the UI copy);
//! - [`export`] — pages + one CSS file (+ assets) for a static site;
//! - [`zip`] — a dependency-free stored ZIP writer;
//! - [`http`] — `POST …/export`, `GET …/preview[/<page>]`, `GET …/publishes`.
//!
//! Every publish records a `design_publishes` row with the exact version of
//! everything it rendered (the site, the brand kit, each 3D embed / image).

pub mod export;
pub mod extract;
pub mod http;
pub mod render;
pub mod schema;
pub mod theme;
pub mod validate;
pub mod zip;

pub use extract::extract;
pub use validate::validate;

/// The stylesheet every page renders with (kept byte-identical to the UI's
/// `engine/site.css`; `ui/unit/siteStudio.test.ts` checks).
pub const SITE_CSS: &str = include_str!("site.css");
