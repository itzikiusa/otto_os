#!/usr/bin/env bash
#
# install-linux.sh — provision a Linux host and build the Otto daemon (ottod) +
# web UI from source. No DMG, no Tauri desktop shell (that part is macOS-only) —
# just the headless daemon + the browser UI it serves, exactly what you want on a
# server / CI box (e.g. Jenkins).
#
# It does, idempotently:
#   1. install OS build deps (cc/g++, cmake, pkg-config, perl, clang, zlib, …)
#   2. install Rust  (rustup, user-local) if `cargo` is missing
#   3. install Node  (official LTS tarball) if `node` is missing or older than 18
#   4. build the UI  (npm ci + npm run build → ui/dist)
#   5. build ottod   (cargo build --release -p ottod --features embed-ui)
#                    → the UI is baked into the binary and served on the same port
#   6. install ClickHouse (usage tracking) into ~/.local/bin if missing
#   7. (optional) install + start a systemd service that runs ottod on boot
#
# Memory: compiling otto-server at opt-level 3 peaks at ~11.5 GB even with one
# job, so the build needs RAM + swap >= 14 GB (some headroom). The preflight stops early with
# the swap commands instead of an out-of-memory kill ten minutes in.
#
# Why these system packages (verified against Cargo.lock, not guessed):
#   • rdkafka  → cmake-build + ssl-vendored  ⇒ cmake, a C compiler, perl, and
#                libcurl dev headers (librdkafka includes curl/curl.h)
#   • aws-lc   → cmake + C compiler                (the TLS crypto provider)
#   • lib{z,sqlite3,zstd}-sys, bindgen        ⇒ C compiler, zlib, clang/libclang
#   OpenSSL is *vendored* by rdkafka, so no system libssl is strictly required —
#   we install the -dev package anyway as cheap insurance.
#
# Secrets: there is no macOS Keychain on Linux, so the daemon must use its file
# store. This script runs/services it with OTTO_SECRETS=file (a 0600 JSON file
# under the data dir). That single env var is the only Linux-specific knob.
#
# Usage:
#   ./install-linux.sh                 # install deps + build (prints run command)
#   ./install-linux.sh --run           # …then launch ottod in the foreground
#   ./install-linux.sh --service       # …then install + enable a systemd service
#   ./install-linux.sh --port 7700     # override the listen port (default 7700)
#   ./install-linux.sh --no-deps       # skip OS/Rust/Node install, just build
#   ./install-linux.sh --force-ci      # force `npm ci` even if node_modules is fresh
#   ./install-linux.sh --no-clickhouse # skip ClickHouse (the Usage page stays empty)
#   ./install-linux.sh --skip-mem-check  # build even with RAM + swap < 14 GB
#   ./install-linux.sh -h | --help
#
set -uo pipefail

# ---- config ---------------------------------------------------------------
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# The UI pins its Node in ui/.nvmrc (CI uses the same file); install that
# version when Node is missing or older.
NODE_VERSION="v$(tr -d '[:space:]v' < "$ROOT/ui/.nvmrc" 2>/dev/null)"
[[ "$NODE_VERSION" == "v" ]] && { echo "ui/.nvmrc is missing" >&2; exit 1; }
PORT=7700
CLICKHOUSE_VERSION="26.8.34.13"   # the version CI pins (.github/workflows/ci.yml)
MIN_BUILD_MEM_GB=14
HEALTH_PATH="/api/v1/health"

DO_RUN=0
DO_SERVICE=0
NO_DEPS=0
FORCE_CI=0
NO_CLICKHOUSE=0
SKIP_MEM_CHECK=0
for arg in "$@"; do
    case "$arg" in
        --run)        DO_RUN=1 ;;
        --service)    DO_SERVICE=1 ;;
        --no-deps)    NO_DEPS=1 ;;
        --force-ci)   FORCE_CI=1 ;;
        --no-clickhouse)  NO_CLICKHOUSE=1 ;;
        --skip-mem-check) SKIP_MEM_CHECK=1 ;;
        --port)       PORT="__NEXT__" ;;
        --port=*)     PORT="${arg#--port=}" ;;
        -h|--help)    sed -n '2,50p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *)
            if [[ "$PORT" == "__NEXT__" ]]; then PORT="$arg"
            else echo "unknown flag: $arg (try --help)" >&2; exit 2; fi ;;
    esac
done
[[ "$PORT" == "__NEXT__" ]] && { echo "--port needs a value" >&2; exit 2; }

# ---- pretty output --------------------------------------------------------
BOLD=$'\033[1m'; DIM=$'\033[2m'; GRN=$'\033[32m'; RED=$'\033[31m'; YEL=$'\033[33m'; RST=$'\033[0m'
START_TS=$(date +%s)
step() { echo; echo "${BOLD}▸ $*${RST}"; }
ok()   { echo "  ${GRN}✓${RST} $*"; }
warn() { echo "  ${YEL}!${RST} $*"; }
die()  { echo; echo "${RED}✗ FAILED:${RST} $*" >&2; exit 1; }
run()  { echo "  ${DIM}\$ $*${RST}"; "$@"; }
have() { command -v "$1" >/dev/null 2>&1; }

# ---- preflight ------------------------------------------------------------
[[ "$(uname -s)" == "Linux" ]] || die "this installer targets Linux; on macOS use ./deploy.sh instead"

# sudo only when not already root (and only if available)
if [[ "$(id -u)" -eq 0 ]]; then SUDO=""
elif have sudo;               then SUDO="sudo"
else SUDO=""; fi

# Map uname -m → the arch token Node's release tarballs use.
case "$(uname -m)" in
    x86_64|amd64)  NODE_ARCH="x64"   ;;
    aarch64|arm64) NODE_ARCH="arm64" ;;
    *) NODE_ARCH="" ;;   # unknown → we'll fall back to the distro package manager
esac

# Building otto-server needs ~11.5 GB at its peak (measured on a clean build).
if [[ "$SKIP_MEM_CHECK" -eq 0 && -r /proc/meminfo ]]; then
    mem_kb=$(awk '/^(MemTotal|SwapTotal):/ {t += $2} END {print t}' /proc/meminfo)
    if (( mem_kb < MIN_BUILD_MEM_GB * 1024 * 1024 )); then
        die "RAM + swap is $(( mem_kb / 1024 / 1024 )) GB; building ottod needs about ${MIN_BUILD_MEM_GB} GB.
   Add swap, then re-run (or pass --skip-mem-check):
     sudo fallocate -l 8G /swapfile && sudo chmod 600 /swapfile
     sudo mkswap /swapfile && sudo swapon /swapfile"
    fi
fi

# ---- 1/5  OS build dependencies -------------------------------------------
# Detect the package manager once; reuse for both system deps and (Alpine) Node.
PM=""
for c in apt-get dnf yum zypper pacman apk; do have "$c" && { PM="$c"; break; }; done

sys_satisfied() {
    # The hard requirements for the native crates above. clang is insurance-only,
    # so it is installed but not part of this gate.
    have cmake && have pkg-config && have perl && have git && have curl && have xz \
        && { have cc || have gcc; } && pkg-config --exists libcurl
}

install_system_deps() {
    case "$PM" in
        apt-get)
            run $SUDO apt-get update -y
            run $SUDO apt-get install -y --no-install-recommends \
                build-essential pkg-config cmake perl clang libclang-dev \
                zlib1g-dev libsqlite3-dev libssl-dev libcurl4-openssl-dev \
                curl git ca-certificates xz-utils ;;
        dnf|yum)
            run $SUDO "$PM" install -y \
                gcc gcc-c++ make cmake perl clang clang-devel \
                zlib-devel sqlite-devel openssl-devel libcurl-devel pkgconf-pkg-config \
                curl git ca-certificates xz ;;
        zypper)
            run $SUDO zypper --non-interactive install \
                gcc gcc-c++ make cmake perl clang llvm-clang \
                zlib-devel sqlite3-devel libopenssl-devel libcurl-devel pkg-config \
                curl git xz ;;
        pacman)
            run $SUDO pacman -Sy --noconfirm --needed \
                base-devel cmake clang perl zlib sqlite openssl curl git xz ;;
        apk)
            run $SUDO apk add --no-cache \
                build-base cmake perl clang clang-dev zlib-dev sqlite-dev \
                openssl-dev curl-dev pkgconf curl git xz ;;
        *)
            die "no supported package manager found (apt/dnf/yum/zypper/pacman/apk).
   Install manually: a C/C++ compiler, make, cmake, pkg-config, perl, clang,
   zlib + sqlite + openssl dev headers, curl, git, xz — then re-run with --no-deps." ;;
    esac
}

if [[ "$NO_DEPS" -eq 1 ]]; then
    step "1/5  OS build deps  (--no-deps: skipped)"
elif sys_satisfied; then
    step "1/5  OS build deps"
    ok "toolchain already present (cmake, pkg-config, perl, compiler, …) — skipping"
else
    step "1/5  OS build deps  (via ${PM:-?})"
    install_system_deps || die "system dependency install failed"
    ok "build toolchain installed"
fi

# ---- 2/5  Rust (rustup, user-local) ---------------------------------------
step "2/5  Rust toolchain"
if have cargo; then
    ok "cargo present ($(cargo --version 2>/dev/null))"
elif [[ "$NO_DEPS" -eq 1 ]]; then
    die "cargo not found and --no-deps set"
else
    run sh -c "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --default-toolchain stable" \
        || die "rustup install failed"
    ok "rustup installed"
fi
# Make cargo visible to THIS shell regardless of how it was installed.
[[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
have cargo || export PATH="$HOME/.cargo/bin:$PATH"
have cargo || die "cargo still not on PATH after install"

# ---- 3/5  Node + npm ------------------------------------------------------
step "3/5  Node.js + npm"
node_ok() {
    have node || return 1
    # Version compare: the installed node must be >= the pinned one.
    local have_v; have_v="$(node -v 2>/dev/null)"
    [[ "$(printf '%s\n%s\n' "${NODE_VERSION#v}" "${have_v#v}" | sort -V | head -1)" == "${NODE_VERSION#v}" ]]
}
install_node() {
    # Alpine (musl) OR an arch with no official tarball: use the package manager.
    if [[ "$PM" == "apk" || -z "$NODE_ARCH" ]]; then
        case "$PM" in
            apt-get) run $SUDO apt-get install -y --no-install-recommends nodejs npm ;;
            dnf|yum) run $SUDO "$PM" install -y nodejs npm ;;
            zypper)  run $SUDO zypper --non-interactive install nodejs npm ;;
            pacman)  run $SUDO pacman -Sy --noconfirm --needed nodejs npm ;;
            apk)     run $SUDO apk add --no-cache nodejs npm ;;
            *) die "no tarball for $(uname -m) and no known package manager for Node" ;;
        esac
        return $?
    fi

    # glibc distros: install the pinned LTS tarball deterministically.
    local prefix tmp url
    if [[ -w /usr/local || "$(id -u)" -eq 0 || -n "$SUDO" ]]; then prefix="/usr/local"
    else prefix="$HOME/.local"; fi
    url="https://nodejs.org/dist/${NODE_VERSION}/node-${NODE_VERSION}-linux-${NODE_ARCH}.tar.xz"
    tmp="$(mktemp -d)"
    run curl -fsSL "$url" -o "$tmp/node.tar.xz" || return 1
    run mkdir -p "$prefix"
    # --strip-components=1 merges bin/ lib/ include/ share/ into the prefix.
    if [[ "$prefix" == "/usr/local" && -n "$SUDO" ]]; then
        run $SUDO tar -xJf "$tmp/node.tar.xz" -C "$prefix" --strip-components=1
    else
        run tar -xJf "$tmp/node.tar.xz" -C "$prefix" --strip-components=1
    fi
    rm -rf "$tmp"
    export PATH="$prefix/bin:$PATH"
}
if node_ok; then
    ok "node present ($(node -v), npm $(npm -v 2>/dev/null))"
elif [[ "$NO_DEPS" -eq 1 ]]; then
    die "node >= ${NODE_VERSION#v} not found and --no-deps set"
else
    install_node || die "node install failed"
    node_ok || die "node still missing/old after install ($(node -v 2>/dev/null || echo none))"
    ok "node installed ($(node -v))"
fi

# ---- 4/5  Build the UI (ui/dist) ------------------------------------------
step "4/5  Build UI  (ui/dist)"
cd "$ROOT/ui" || die "no ui/ directory at $ROOT"
need_ci=0
if [[ ! -d node_modules ]]; then need_ci=1
elif [[ "$FORCE_CI" -eq 1 ]]; then need_ci=1
elif [[ package-lock.json -nt node_modules/.package-lock.json ]]; then need_ci=1; fi
if [[ "$need_ci" -eq 1 ]]; then run npm ci || die "npm ci failed"
else ok "node_modules fresh — skipping npm ci  (--force-ci to override)"; fi
run npm run build || die "npm run build failed"
ok "UI built → ui/dist"

# ---- 5/5  Build ottod (release, embed-ui) ---------------------------------
step "5/5  Build ottod  (release, embed-ui)"
cd "$ROOT" || die "lost repo root"
run cargo build --release -p ottod --features embed-ui || die "cargo build ottod failed"
BIN="$ROOT/target/release/ottod"
[[ -x "$BIN" ]] || die "expected binary not found at $BIN"
ok "daemon built → $BIN"

# ---- ClickHouse (usage tracking) -----------------------------------------
# ottod finds `clickhouse` on PATH or in ~/.local/bin; without it the Usage page
# stays empty. Same pinned build CI uses, checked against its published SHA-512.
install_clickhouse() {
    local arch tmp base url bin
    case "$(uname -m)" in
        x86_64|amd64)  arch="amd64" ;;
        aarch64|arm64) arch="arm64" ;;
        *) warn "no ClickHouse build for $(uname -m) — usage tracking stays off"; return 0 ;;
    esac
    base="clickhouse-common-static-${CLICKHOUSE_VERSION}-${arch}.tgz"
    url="https://github.com/ClickHouse/ClickHouse/releases/download/v${CLICKHOUSE_VERSION}-lts/${base}"
    tmp="$(mktemp -d)"
    run curl -fsSL --retry 3 "$url" -o "$tmp/$base" || return 1
    run curl -fsSL --retry 3 "$url.sha512" -o "$tmp/$base.sha512" || return 1
    (cd "$tmp" && sha512sum --check --status "$base.sha512") || { rm -rf "$tmp"; die "ClickHouse download failed its SHA-512 check"; }
    tar -xzf "$tmp/$base" -C "$tmp" || return 1
    bin="$(find "$tmp" -type f -name clickhouse -path '*/bin/*' | head -n 1)"
    [[ -n "$bin" ]] || return 1
    mkdir -p "$HOME/.local/bin" && install -m 0755 "$bin" "$HOME/.local/bin/clickhouse"
    rm -rf "$tmp"
}
step "ClickHouse  (usage tracking)"
if [[ "$NO_CLICKHOUSE" -eq 1 ]]; then
    warn "--no-clickhouse: skipped — the Usage page stays empty"
elif have clickhouse || [[ -x "$HOME/.local/bin/clickhouse" ]]; then
    ok "clickhouse present"
else
    install_clickhouse || die "ClickHouse install failed (re-run, or pass --no-clickhouse)"
    ok "clickhouse ${CLICKHOUSE_VERSION} → ~/.local/bin/clickhouse"
fi

# ---- optional: systemd service --------------------------------------------
install_service() {
    local unit name scope wanted
    if [[ "$(id -u)" -eq 0 ]]; then
        unit="/etc/systemd/system/ottod.service"; scope="system"; wanted="multi-user.target"
        SYSTEMCTL=("systemctl")
    else
        mkdir -p "$HOME/.config/systemd/user"
        unit="$HOME/.config/systemd/user/ottod.service"; scope="user"; wanted="default.target"
        SYSTEMCTL=("systemctl" "--user")
    fi
    have systemctl || { warn "systemd not available — skipping service install"; return 0; }
    cat > "$unit" <<UNIT
[Unit]
Description=Otto daemon (ottod) — headless coding-agent server
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=OTTO_SECRETS=file
Environment=OTTO_SECRETS_ALLOW_PLAINTEXT=1
Environment=OTTO_PORT=$PORT
WorkingDirectory=$ROOT
ExecStart=$BIN
Restart=on-failure
RestartSec=3

[Install]
WantedBy=$wanted
UNIT
    ok "wrote $scope unit → $unit"
    run "${SYSTEMCTL[@]}" daemon-reload
    run "${SYSTEMCTL[@]}" enable --now ottod.service \
        || die "could not enable/start ottod.service (check: ${SYSTEMCTL[*]} status ottod)"
    ok "ottod.service enabled + started ($scope scope)"
    [[ "$scope" == "user" ]] && warn "user services stop at logout unless: loginctl enable-linger $(id -un)"
}

# ---- summary --------------------------------------------------------------
ELAPSED=$(( $(date +%s) - START_TS ))
step "Done in ${ELAPSED}s"
ok "binary: $BIN"
echo
echo "  ${BOLD}Run it (foreground):${RST}"
echo "    ${DIM}OTTO_SECRETS=file OTTO_PORT=$PORT $BIN${RST}"
echo "  Then open ${BOLD}http://127.0.0.1:${PORT}/${RST}  (UI is baked into the binary)."
echo
echo "  ${DIM}Loopback-only by default. For remote access (Jenkins → browser) either:${RST}"
echo "  ${DIM}  • SSH-tunnel:  ssh -L ${PORT}:127.0.0.1:${PORT} <host>${RST}"
echo "  ${DIM}  • or enable the 0.0.0.0 TLS listener via the network_listener setting in the UI.${RST}"
echo "  ${DIM}Agent sessions (claude/codex) require those CLIs on PATH (or set CLAUDE_BIN).${RST}"

if [[ "$DO_SERVICE" -eq 1 ]]; then
    step "Install systemd service"
    install_service
    echo; echo "  Health:  curl -fsS http://127.0.0.1:${PORT}${HEALTH_PATH}"
elif [[ "$DO_RUN" -eq 1 ]]; then
    step "Launching ottod (Ctrl-C to stop)"
    echo "  ${DIM}\$ OTTO_SECRETS=file OTTO_PORT=$PORT $BIN${RST}"
    exec env OTTO_SECRETS=file OTTO_PORT="$PORT" "$BIN"
fi
