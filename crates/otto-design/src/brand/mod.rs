//! Brand Kit v1 — the `otto-brand/1` token document every studio reads
//! (proposal §2 "Brand Kit drives every studio", §3.3 "follow the latest
//! approved kit; impact preview before save"; docs/features/design-hall.md
//! § Brand Kit).
//!
//! - [`doc`] — the schema validator (run on every save through
//!   `format::validate`) and the lenient [`BrandModel`];
//! - [`extract`] — links (logo assets `embeds`) + search text for a kit;
//! - [`export`] — CSS custom properties (`--brand-<group>-<name>`), a
//!   Tailwind v4 `@theme`, W3C DTCG JSON;
//! - [`contrast`] — WCAG ratios of every colour vs white / the kit's ink /
//!   each other;
//! - [`impact`] — the token diff and "which consumers see it" (artifacts with
//!   a `uses_tokens` link into the kit, the tokens they name as
//!   `token:<group>.<name>` or `var(--brand-…)`);
//! - [`http`] — `POST …/brand/impact`, `GET …/brand/export`.

pub mod contrast;
pub mod doc;
pub mod export;
pub mod extract;
pub mod http;
pub mod impact;

pub use doc::{default_doc, validate, BrandModel, SCHEMA};
pub use extract::extract;
