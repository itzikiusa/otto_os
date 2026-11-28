//! Pinned managed LLVM / Clang downloads (rc.6 design B §1).
//!
//! Every value here is copied from what upstream actually publishes — the
//! asset name, byte size and SHA-256 (the GitHub release API `digest`) of the
//! official `llvm/llvm-project` and `clangd/clangd` releases — never built
//! from a URL template (rc.5 BUG-8 / BUG-9: guessed names that did not exist).
//! Verified with `gh api repos/llvm/llvm-project/releases/tags/llvmorg-<ver>`
//! on 2026-11-28. Bumping a pin is a dev task: re-run that query and paste.
//!
//! Version per platform: 23.1.5 is the newest release with macOS arm64, Linux
//! x64 and Linux arm64 archives (23.1.6 has no macOS build). Intel Macs stay on
//! 20.1.7, the last LLVM that publishes a macOS x64 build (`.tar.xz` only).

use crate::toolchains_catalog::{TargetArch, TargetOs, TargetPlatform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    Zst,
    Xz,
}

/// One official LLVM release archive.
#[derive(Debug, Clone, Copy)]
pub struct LlvmAsset {
    pub os: TargetOs,
    pub arch: TargetArch,
    pub version: &'static str,
    pub major: u32,
    pub asset: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub compression: Compression,
    /// The archive's single top-level directory.
    pub top_dir: &'static str,
    /// What the allowlisted members take on disk once unpacked (macOS arm64
    /// measured by the live install; the others estimated, see the report).
    pub installed_bytes: u64,
}

pub const LLVM_MANIFEST: &[LlvmAsset] = &[
    LlvmAsset {
        os: TargetOs::Macos,
        arch: TargetArch::Aarch64,
        version: "23.1.5",
        major: 23,
        asset: "LLVM-23.1.5-macOS-ARM64.tar.zst",
        bytes: 870_686_518,
        sha256: "503de30ef90311e0b143623801b19af2250adac6d69faed65ad0051170202e3b",
        compression: Compression::Zst,
        top_dir: "LLVM-23.1.5-macOS-ARM64",
        installed_bytes: 444_000_000,
    },
    LlvmAsset {
        os: TargetOs::Macos,
        arch: TargetArch::X86_64,
        version: "20.1.7",
        major: 20,
        asset: "LLVM-20.1.7-macOS-X64.tar.xz",
        bytes: 1_464_445_324,
        sha256: "ccf82ffe7e136ee49659cb57157856a7963d0950fac3d05aabba0db75bfba26f",
        compression: Compression::Xz,
        top_dir: "LLVM-20.1.7-macOS-X64",
        installed_bytes: 440_000_000,
    },
    LlvmAsset {
        os: TargetOs::Linux,
        arch: TargetArch::X86_64,
        version: "23.1.5",
        major: 23,
        asset: "LLVM-23.1.5-Linux-X64.tar.zst",
        bytes: 1_198_896_522,
        sha256: "c3107384835f788768f614db610e87fba7077f125ca950eb02a78bab0a308065",
        compression: Compression::Zst,
        top_dir: "LLVM-23.1.5-Linux-X64",
        installed_bytes: 640_000_000,
    },
    LlvmAsset {
        os: TargetOs::Linux,
        arch: TargetArch::Aarch64,
        version: "23.1.5",
        major: 23,
        asset: "LLVM-23.1.5-Linux-ARM64.tar.zst",
        bytes: 1_102_367_349,
        sha256: "830ceb70edf63849ce66e97bf85310a3bf2bdd04c860a9f2fa4ef1560e290cef",
        compression: Compression::Zst,
        top_dir: "LLVM-23.1.5-Linux-ARM64",
        installed_bytes: 640_000_000,
    },
];

impl LlvmAsset {
    pub fn url(&self) -> String {
        format!(
            "https://github.com/llvm/llvm-project/releases/download/llvmorg-{}/{}",
            self.version, self.asset
        )
    }

    /// The folder under `toolchains/cpp/sdks/` (read by SDK discovery).
    pub fn dir_name(&self) -> String {
        format!("llvm-{}", self.version)
    }

    /// Archive members to unpack, relative to [`Self::top_dir`]; a trailing
    /// `/` takes the whole subtree and a trailing `*` is a name prefix. Every
    /// entry must be present in the archive.
    /// macOS keeps Apple's `lldb-dap` (the tarball's lldb / debugserver code
    /// signing is unverified) and the SDK's libc++ headers.
    pub fn allowlist(&self) -> Vec<String> {
        let mut out: Vec<String> = [
            "bin/clang",
            "bin/clang++",
            "bin/clangd",
            "bin/clang-format",
            "bin/clang-tidy",
            "bin/clang-apply-replacements",
            "bin/llvm-symbolizer",
        ]
        .map(String::from)
        .to_vec();
        out.push(format!("bin/clang-{}", self.major));
        out.push(format!("lib/clang/{}/", self.major));
        if self.os == TargetOs::Linux {
            for m in [
                "bin/lldb",
                "bin/lldb-dap",
                "bin/lldb-server",
                "bin/lldb-argdumper",
                "lib/liblldb.so*",
            ] {
                out.push(m.to_string());
            }
        }
        out
    }
}

/// The pinned LLVM archive for `platform`, if upstream publishes one.
pub fn llvm_for(platform: TargetPlatform) -> Option<&'static LlvmAsset> {
    LLVM_MANIFEST
        .iter()
        .find(|a| a.os == platform.os && a.arch == platform.arch)
}

/// Pinned standalone clangd (`clangd/clangd` releases, the "Install LSP" path).
pub const CLANGD_VERSION: &str = "23.1.0";

/// `(url, sha256)` of the clangd zip for `platform`; Linux arm64 has none.
pub fn clangd_zip(platform: TargetPlatform) -> Option<(String, &'static str)> {
    let (slug, sha) = match (platform.os, platform.arch) {
        (TargetOs::Macos, _) => (
            "mac",
            "1082e6638223b785ca2daf0939f13afcd0bb95c84ee9a4bbaff4745365159253",
        ),
        (TargetOs::Linux, TargetArch::X86_64) => (
            "linux",
            "e53b1a96196095faedb7642cf64964f7fb9ad4a0c1f00dd2c172a3d9dcbafdfd",
        ),
        (TargetOs::Linux, TargetArch::Aarch64) => return None,
    };
    Some((
        format!(
            "https://github.com/clangd/clangd/releases/download/{CLANGD_VERSION}/clangd-{slug}-{CLANGD_VERSION}.zip"
        ),
        sha,
    ))
}

/// Megabytes as the confirm dialog shows them (`871 MB`).
pub fn mb(bytes: u64) -> String {
    format!("{} MB", (bytes + 500_000) / 1_000_000)
}
