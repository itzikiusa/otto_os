//! Map a Rust source file to the cargo target that compiles it, from its place under the
//! nearest `Cargo.toml`. Shared by the gutter Run/Test args and the Rust debug build.

use std::path::Path;

/// `selector` picks the target (`--bin x`, `--lib`, `--test x`, `--example x`, `--bench x`);
/// `modules` is the file's module path inside it (`src/parser/mod.rs` → `["parser"]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CargoTarget {
    pub selector: Vec<String>,
    pub modules: Vec<String>,
}

impl CargoTarget {
    /// The `--bin`/`--example` selector, the only ones `cargo run` takes.
    pub(crate) fn run_selector(&self) -> Option<&[String]> {
        matches!(self.selector[0].as_str(), "--bin" | "--example").then_some(&self.selector[..])
    }
}

/// `None` when no local `Cargo.toml` owns the file (a remote target) or it isn't in a
/// target folder; callers then keep plain `cargo run` / `cargo test`.
pub(crate) fn cargo_target_for(path: &str) -> Option<CargoTarget> {
    let file = Path::new(path);
    let crate_dir = file
        .ancestors()
        .skip(1)
        .find(|d| d.join("Cargo.toml").is_file())?;
    let rel = file
        .strip_prefix(crate_dir)
        .ok()?
        .to_str()?
        .replace('\\', "/");
    let (kind, rest) = rel.split_once('/')?;
    let package = || {
        let raw = std::fs::read_to_string(crate_dir.join("Cargo.toml")).ok()?;
        crate::environments::discovery::parse_cargo_package_name(&raw)
    };
    let target = |flag: &str, name: String, modules: &str| CargoTarget {
        selector: vec![flag.into(), name],
        modules: module_segments(modules),
    };
    // `<dir>/x.rs` or `<dir>/x/main.rs` (+ its modules `<dir>/x/sub.rs`) is target `x`.
    let named = |flag: &str, dir: &str, rest: &str| match rest.split_once('/') {
        None => Some(target(flag, rest.strip_suffix(".rs")?.into(), "")),
        Some((name, inner)) => crate_dir
            .join(dir)
            .join(name)
            .join("main.rs")
            .is_file()
            .then(|| target(flag, name.into(), inner)),
    };
    match kind {
        "src" => match rest.strip_prefix("bin/") {
            Some(bin) => named("--bin", "src/bin", bin),
            None if rest == "main.rs" => Some(target("--bin", package()?, "")),
            None if crate_dir.join("src/lib.rs").is_file() => Some(CargoTarget {
                selector: vec!["--lib".into()],
                modules: module_segments(rest),
            }),
            None => Some(target("--bin", package()?, rest)),
        },
        "tests" => named("--test", kind, rest),
        "examples" => named("--example", kind, rest),
        "benches" => named("--bench", kind, rest),
        _ => None,
    }
}

/// `parser/mod.rs` → `["parser"]`; a target root (`lib.rs`, `main.rs`) has none.
fn module_segments(rel: &str) -> Vec<String> {
    if matches!(rel, "" | "lib.rs" | "main.rs") {
        return Vec::new();
    }
    let without_ext = rel.strip_suffix(".rs").unwrap_or(rel);
    without_ext
        .split('/')
        .filter(|seg| !seg.is_empty() && *seg != "mod")
        .map(str::to_string)
        .collect()
}
