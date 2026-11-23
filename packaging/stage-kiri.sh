#!/bin/bash
# Stage the embedded headless Kiri runtime (`kiri` binary + `guest/` kernel &
# initramfs assets) so `packaging/deploy.sh` embeds them directly inside
# `Otto.app/Contents/MacOS/kiri` and `Otto.app/Contents/Resources/guest/`
# (ZERO separate `.app` bundle).
#
# Usage:
#   packaging/stage-kiri.sh [--from-release]         # fetch signed upstream release tarball + verify SHA-256
#   packaging/stage-kiri.sh --from-source <kiri-dir> # stage from a local Kiri source/build tree
#   packaging/stage-kiri.sh --clean                  # remove staged artifacts
#
# Environment:
#   OTTO_KIRI_STAGE_DIR  Override staging dir (default: <repo>/target/kiri-stage, gitignored)
#   OTTO_KIRI_FEED_URL   Override release feed JSON URL
#   OTTO_KIRI_SOURCE_DIR Default path for --from-source
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
STAGE_DIR="${OTTO_KIRI_STAGE_DIR:-$ROOT/target/kiri-stage}"

usage() {
    sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
}

MODE="release"
SOURCE_DIR="${OTTO_KIRI_SOURCE_DIR:-}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --from-release) MODE="release"; shift ;;
        --from-source)
            MODE="source"
            SOURCE_DIR="${2:?--from-source requires a directory path}"
            shift 2
            ;;
        --clean) MODE="clean"; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown argument: $1" >&2; usage >&2; exit 2 ;;
    esac
done

if [[ "$MODE" == "clean" ]]; then
    rm -rf "$STAGE_DIR"
    echo "cleaned: $STAGE_DIR"
    exit 0
fi

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/otto-stage-kiri.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

mkdir -p "$STAGE_DIR/bin" "$STAGE_DIR/guest"

copy_guest_assets() {
    local src_dir="$1"
    local copied=0
    for f in Image bzImage initramfs.cpio data.img.tgz data.img; do
        if [[ -f "$src_dir/$f" ]]; then
            cp -f "$src_dir/$f" "$STAGE_DIR/guest/$f"
            copied=$((copied + 1))
        fi
    done
    return $((copied > 0 ? 0 : 1))
}

if [[ "$MODE" == "source" ]]; then
    [[ -d "$SOURCE_DIR" ]] || { echo "source dir not found: $SOURCE_DIR" >&2; exit 1; }
    BIN_FOUND=""
    for cand in \
        "$SOURCE_DIR/kiri" \
        "$SOURCE_DIR/bin/kiri" \
        "$SOURCE_DIR/Contents/MacOS/kiri" \
        "$SOURCE_DIR/Kiri.app/Contents/MacOS/kiri" \
        "$SOURCE_DIR/target/release/kiri"; do
        if [[ -f "$cand" ]]; then
            BIN_FOUND="$cand"
            break
        fi
    done
    if [[ -z "$BIN_FOUND" && -f "$SOURCE_DIR/Cargo.toml" ]]; then
        echo "==> Building headless kiri binary from $SOURCE_DIR"
        ( cd "$SOURCE_DIR" && cargo build --release -p kiri )
        BIN_FOUND="$SOURCE_DIR/target/release/kiri"
    fi
    [[ -f "$BIN_FOUND" ]] || { echo "kiri binary not found in $SOURCE_DIR" >&2; exit 1; }
    cp -f "$BIN_FOUND" "$STAGE_DIR/bin/kiri"
    chmod 0755 "$STAGE_DIR/bin/kiri"

    GUEST_COPIED=0
    for gdir in \
        "$SOURCE_DIR/guest" \
        "$SOURCE_DIR/runtime/guest" \
        "$SOURCE_DIR/Contents/Resources/guest" \
        "$SOURCE_DIR/Kiri.app/Contents/Resources/guest" \
        "$SOURCE_DIR/guest/out"; do
        if [[ -d "$gdir" ]] && copy_guest_assets "$gdir"; then
            GUEST_COPIED=1
            break
        fi
    done
    if [[ "$GUEST_COPIED" == 0 ]]; then
        echo "WARN: no guest kernel/initramfs assets found under $SOURCE_DIR (run 'cargo xtask guest && cargo xtask kernel && cargo xtask disk' there first)" >&2
    fi
else
    ARCH="$(uname -m)"
    if [[ "$ARCH" == "x86_64" ]]; then
        DEFAULT_FEED="https://download.runkiri.com/latest-mac-x86_64.json"
    else
        DEFAULT_FEED="https://download.runkiri.com/latest-mac-arm64.json"
    fi
    FEED_URL="${OTTO_KIRI_FEED_URL:-$DEFAULT_FEED}"
    echo "==> Fetching Kiri release feed: $FEED_URL"
    FEED_JSON="$TMP_DIR/feed.json"
    curl -fsSL --proto '=https' --tlsv1.2 --max-time 20 "$FEED_URL" -o "$FEED_JSON"

    read -r TAR_URL EXPECTED_SHA VERSION < <(
        python3 - "$FEED_JSON" <<'PY'
import json, sys
d = json.load(open(sys.argv[1]))
print(d["url"], d.get("sha256", ""), d.get("version", "unknown"))
PY
    )
    [[ -n "$TAR_URL" ]] || { echo "release feed missing url" >&2; exit 1; }

    TAR_FILE="$TMP_DIR/kiri.tar.gz"
    echo "==> Downloading Kiri v$VERSION ($TAR_URL)"
    curl -fsSL --proto '=https' --tlsv1.2 --max-time 300 "$TAR_URL" -o "$TAR_FILE"

    if [[ -n "$EXPECTED_SHA" ]]; then
        ACTUAL_SHA="$(shasum -a 256 "$TAR_FILE" | awk '{print $1}')"
        if [[ "$ACTUAL_SHA" != "$EXPECTED_SHA" ]]; then
            echo "SHA-256 mismatch: expected $EXPECTED_SHA, got $ACTUAL_SHA" >&2
            exit 1
        fi
        echo "    SHA-256 verified: $ACTUAL_SHA"
    fi

    # Extract ONLY the headless binary and guest assets (discarding any .app wrapper).
    UNPACK_DIR="$TMP_DIR/unpack"
    mkdir -p "$UNPACK_DIR"
    tar -xzf "$TAR_FILE" -C "$UNPACK_DIR"

    BIN_FOUND="$(find "$UNPACK_DIR" -type f \( -path '*/Contents/MacOS/kiri' -o -path '*/bin/kiri' -o -name 'kiri' \) | head -1)"
    [[ -n "$BIN_FOUND" && -f "$BIN_FOUND" ]] || { echo "archive did not contain kiri binary" >&2; exit 1; }
    cp -f "$BIN_FOUND" "$STAGE_DIR/bin/kiri"
    chmod 0755 "$STAGE_DIR/bin/kiri"

    GUEST_DIR="$(find "$UNPACK_DIR" -type d \( -path '*/Contents/Resources/guest' -o -name 'guest' \) | head -1)"
    if [[ -n "$GUEST_DIR" ]]; then
        copy_guest_assets "$GUEST_DIR" || true
    fi
fi

# Ensure virtualization entitlement on macOS (preserve existing valid signature if present).
if [[ "$(uname -s)" == "Darwin" ]]; then
    if ! codesign -d --entitlements - "$STAGE_DIR/bin/kiri" 2>&1 | grep -q 'com.apple.security.virtualization'; then
        codesign --force --sign - --entitlements "$HERE/kiri.entitlements" "$STAGE_DIR/bin/kiri"
    fi
fi

echo "staged embedded Kiri into: $STAGE_DIR"
ls -lh "$STAGE_DIR/bin/kiri"
ls -lh "$STAGE_DIR/guest" 2>/dev/null || true
