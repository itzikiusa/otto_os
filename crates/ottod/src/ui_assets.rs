//! Embed the UI at the binary boundary so asset edits leave otto-server cached.

use std::borrow::Cow;

#[derive(rust_embed::RustEmbed)]
#[folder = "../../ui/dist"]
struct UiAssets;

pub fn load(path: &str) -> Option<Cow<'static, [u8]>> {
    UiAssets::get(path).map(|asset| asset.data)
}
