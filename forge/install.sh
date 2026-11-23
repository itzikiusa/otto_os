#!/bin/sh
# Provision this checkout. Runtime files live beside the sources and are gitignored.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
cd "$root"
stage=all
models=no
plan=no
usage() {
    echo 'Usage: ./install.sh [--stage engine|voice|gameready|animlib|audio|motion|all] [--with-models] [--plan]'
}
fail() { echo "forge install: $*" >&2; exit 1; }
while [ "$#" -gt 0 ]; do
    case "$1" in
        --stage) [ "$#" -ge 2 ] || fail 'missing stage name'; stage=$2; shift ;;
        --with-models) models=yes ;;
        --plan) plan=yes ;;
        -h|--help) usage; exit 0 ;;
        *) usage >&2; fail "unknown argument $1" ;;
    esac
    shift
done
case "$stage" in all|engine|gameready|animlib|voice|audio|motion) ;; *) fail "unknown stage $stage" ;; esac
selected() { [ "$stage" = all ] || [ "$stage" = "$1" ]; }
tag=v0.6.0
commit=5f9040bfa00e8269104930a0f918a68d828ccd87
engine="$root/engines/assetfurnace"
voice="$root/forge/voice/.venv"
ace_commit=ca1e85fe9430179831e6bc6be790c332190a3866
moss_commit=4e8e3e36d4d9c991d16a3e65a4b7099d406b9ed8
show_plan() {
    echo "Forge install ($stage) under $root"
    selected engine && echo "Engine: AssetFurnace $tag @ $commit; frozen CPython 3.11 environment, Pixal3D Metal build, sd-cli and SkinTokens environment (~2 GB of rig packages)."
    selected gameready && echo 'Game-ready: CoACD and xatlas arm64 wheels for Blender Python 3.13 (~4 MB).'
    selected animlib && echo 'Animation library: packaged catalog and scripts; uses Blender and Python.'
    selected voice && echo 'Voice: CPython 3.11, MLX Audio and English language frontend packages.'
    selected motion && echo 'Motion: NVIDIA Kimodo @ e1c219c (no MotionCorrection build on arm64), CPython 3.11 with torch 2.9.1 and transformers 5.1.0.'
    selected audio && echo "Audio: ACE-Step 1.5 @ ${ace_commit%"${ace_commit#???????}"} with its frozen CPython 3.11 environment (torch, MLX), MOSS-SoundEffect MLX port @ ${moss_commit%"${moss_commit#???????}"}."
    if [ "$models" = yes ]; then
        echo 'Requested downloads (decimal sizes for an empty cache):'
        selected engine && echo '  Pixal3D single view + DINOv3 8.09 GB; BiRefNet GGUF 883 MB + ONNX 181 MB; Real-ESRGAN x4plus 67 MB; FLUX/Qwen/VAE 8.92 GB; SkinTokens 1.62 GB + Qwen tokenizer/config ~16 MB.'
        selected voice && echo '  Kokoro 355 MB; Qwen3-TTS VoiceDesign 2.32 GB; spaCy English and PocketSphinx acoustic data wheels ~45 MB combined.'
        selected animlib && echo '  KayKit assets/licences 8.53 MB; 100STYLE archive 1.469 GB (CC BY 4.0).'
        selected motion && echo '  Kimodo-SOMA-RP-v1.2 1.13 GB; LLM2Vec adapters 0.34 GB; Meta-Llama-3-8B-Instruct 16.07 GB (gated: accept Meta'"'"'s licence on its Hugging Face page and run hf auth login first).'
        selected audio && echo '  MOSS-SoundEffect v2.0 MLX 4-bit 6.40 GB (Apache-2.0); ACE-Step 1.5 turbo DiT, VAE and text encoder 6.34 GB (MIT).'
        echo 'See NOTICE.md for component and output distribution terms.'
    else
        echo 'Skipping optional model weights, trained speech-data wheels and animation packs. Use --with-models to provision them.'
    fi
}
if [ "$plan" = yes ]; then show_plan; exit 0; fi

[ "$(uname -sm)" = 'Darwin arm64' ] || fail 'the packaged local backend requires Apple Silicon macOS'
command -v uv >/dev/null || fail 'install uv and put it on PATH'
if selected engine; then
    for tool in git cmake xcrun; do command -v "$tool" >/dev/null || fail "missing $tool on PATH"; done
fi
mkdir -p engines models out
# Serialize installers for this checkout. Different stages still share the machine-wide
# heavy.sh slots with builds and synthesis from other checkouts.
if [ "${FORGE_INSTALL_LOCKED:-}" != "$root" ]; then
    set -- --stage "$stage"
    [ "$models" = no ] || set -- "$@" --with-models
    export FORGE_INSTALL_LOCKED="$root"
    exec /usr/bin/lockf -ks "$root/models/.forge-install.lock" "$root/install.sh" "$@"
fi
show_plan
export UV_PYTHON_INSTALL_DIR="${UV_PYTHON_INSTALL_DIR:-$root/models/uv-python}"
export UV_CACHE_DIR="${UV_CACHE_DIR:-$root/models/uv-cache}"
export HF_HOME="$root/models/hf" ASSETFURNACE_MODELS="$root/models/af" U2NET_HOME="$root/models/af/u2net"
export HF_HUB_DISABLE_IMPLICIT_TOKEN=1 HF_HUB_DISABLE_TELEMETRY=1 I3D_NO_USAGE=1 I3D_NO_NOTICES=1 DO_NOT_TRACK=1
unset UV_PROJECT_ENVIRONMENT VIRTUAL_ENV
heavy() {
    available=$(df -Pk "$root" | awk 'NR==2 {print $4}')
    [ "$available" -ge 57671680 ] || fail 'less than 55 GiB free on the install volume'
    "$root/tools/heavy.sh" "$@"
}

if selected engine; then
    if [ ! -e "$engine" ]; then
        checkout=$(mktemp -d "$root/engines/.assetfurnace.XXXXXX")
        trap 'rm -rf -- "$checkout"' EXIT
        heavy git clone --depth 1 --branch "$tag" --single-branch https://github.com/Bingeljell/image-to-3dlab.git "$checkout"
        [ "$(git -C "$checkout" rev-parse HEAD)" = "$commit" ] || fail "upstream $tag changed"
        mv "$checkout" "$engine"
        trap - EXIT
    fi
    [ -d "$engine/.git" ] || [ -f "$engine/.git" ] || fail "expected a Git checkout at $engine"
    [ "$(git -C "$engine" rev-parse HEAD)" = "$commit" ] &&
        [ "$(git -C "$engine" rev-parse "refs/tags/$tag^{commit}")" = "$commit" ] ||
        fail "existing AssetFurnace checkout is not the pinned $tag ($commit); left in place"
    afpy="$engine/.venv/bin/python"
    (cd "$engine" && heavy uv sync --frozen --no-dev --inexact --python 3.11)
    heavy uv pip install --python "$afpy" ninja==1.13.2
    export PATH="$engine/.venv/bin:$PATH"
    (cd "$engine" && heavy "$afpy" scripts/bootstrap_pixal3d.py --build-only --yes)
    (cd "$engine" && heavy "$afpy" scripts/bootstrap_qwen_image.py --build-only --yes)
    rig_env="$engine/vendor/SkinTokens/.venv"
    if [ -e "$rig_env" ] && ! grep -Eq '^version_info = 3\.11([.]|$)' "$rig_env/pyvenv.cfg"; then
        fail "SkinTokens bootstrap would replace the existing environment at $rig_env; choose a fresh install directory"
    fi
    # bpy's arm64 wheel has an inconsistent internal tag. uv reinstalls it on
    # every resolution, even on a healthy rig. Verify the pinned recipe first.
    rig="$engine/vendor/SkinTokens"
    if [ -x "$rig_env/bin/python" ] && (cd "$engine/scripts" &&
        "$rig_env/bin/python" -B - "$rig" <<'PYRIG'
import subprocess
import sys
from importlib.metadata import PackageNotFoundError, version
from pathlib import Path

from packaging.requirements import Requirement
import bootstrap_autorig as recipe

rig = Path(sys.argv[1])
if subprocess.check_output(["git", "-C", str(rig), "rev-parse", "HEAD"], text=True).strip() != recipe.COMMIT:
    sys.exit(1)
recipe.portable.patch(rig, check=True)
requirements = [*recipe.TORCH, *recipe.EXTRA_PACKAGES, *(rig / "requirements.txt").read_text().splitlines()]
try:
    for line in requirements:
        req = Requirement(line)
        if not req.specifier.contains(version(req.name)):
            sys.exit(1)
except PackageNotFoundError:
    sys.exit(1)
PYRIG
    ); then
        echo 'SkinTokens checkout, patches and package requirements verified.'
    else
        (cd "$engine" && heavy "$afpy" scripts/bootstrap_autorig.py --build-only --yes)
    fi
    if [ "$models" = yes ]; then
        if ! "$afpy" - <<'PYMODELS'
from forge.doctor import model_inventory
checks = {}
model_inventory(checks)
required = ("pixal3d", "dinov3", "birefnet-gguf", "birefnet-onnx")
raise SystemExit(any(checks[name]["status"] != "ok" for name in required))
PYMODELS
        then
            (cd "$engine" && heavy "$afpy" scripts/bootstrap_pixal3d.py --weights-only --yes)
        fi
        (cd "$engine" && heavy "$afpy" scripts/bootstrap_upscaler.py --yes)
        (cd "$engine" && heavy "$afpy" scripts/bootstrap_flux_image.py --yes)
        (cd "$engine" && heavy "$afpy" scripts/bootstrap_autorig.py --weights-only --yes)
    fi
fi
if selected gameready; then
    heavy uv pip install --no-deps --only-binary :all: --python-version 3.13 \
        --python-platform aarch64-apple-darwin --target "$root/forge/gameready/.venv" \
        coacd==1.0.14 xatlas==0.0.11
fi
if selected voice; then
    if [ ! -x "$voice/bin/python" ]; then heavy uv venv --python 3.11 "$voice"; fi
    "$voice/bin/python" -c 'import sys; sys.exit(sys.version_info[:2] != (3, 11))' || fail 'voice requires a Python 3.11 venv'
    if [ "$models" = yes ]; then
        heavy uv pip install --python "$voice/bin/python" -r forge/voice/requirements.txt
        heavy "$voice/bin/python" forge/voice/fetch_models.py kokoro qwen
    else
        # These two dependencies distribute trained models inside their wheels.
        sed -E '/^(en-core-web-sm|pocketsphinx)([ @=]|$)/d' forge/voice/requirements.txt |
            heavy uv pip install --python "$voice/bin/python" -r -
    fi
fi
if selected audio; then
    ace="$root/engines/ace-step"
    if [ ! -e "$ace" ]; then
        checkout=$(mktemp -d "$root/engines/.ace-step.XXXXXX")
        trap 'rm -rf -- "$checkout"' EXIT
        heavy git clone --quiet --filter=blob:none https://github.com/ace-step/ACE-Step-1.5.git "$checkout"
        git -C "$checkout" checkout --quiet "$ace_commit"
        mv "$checkout" "$ace"
        trap - EXIT
    fi
    [ "$(git -C "$ace" rev-parse HEAD)" = "$ace_commit" ] || fail "existing ACE-Step checkout is not $ace_commit; left in place"
    # The upstream lock pins every package; Forge adds the MOSS MLX port and a loudness meter.
    (cd "$ace" && UV_PROJECT_ENVIRONMENT="$root/forge/audio/.venv" heavy uv sync --frozen --no-dev --inexact --python 3.11)
    heavy uv pip install --python "$root/forge/audio/.venv/bin/python" --no-deps \
        "moss-sfx-mlx @ git+https://github.com/xocialize/moss-soundeffect-mlx@$moss_commit" \
        ftfy==6.3.1 wcwidth==0.2.14 pyloudnorm==0.2.0
    if [ "$models" = yes ]; then
        # Network-bound and resumable: re-run after an interruption. Not under heavy.sh.
        "$root/forge/audio/.venv/bin/python" -I forge/audio/fetch_models.py moss-sfx ace-step
    fi
fi
if selected animlib && [ "$models" = yes ]; then
    if ! python311=$(uv python find 3.11); then
        heavy uv python install 3.11
        python311=$(uv python find 3.11)
    fi
    heavy "$python311" forge/animlib/catalog_tools/fetch_kaykit.py
    # The publisher archive downloader takes heavy.sh once per resumable batch.
    "$python311" forge/animlib/catalog_tools/fetch_100style.py --all
fi
if selected motion; then
    kimodo="$root/engines/kimodo"
    kimodo_commit=e1c219c99b4b94ba48be1fb6a1cbc64ecee4c954
    if [ ! -e "$kimodo" ]; then
        checkout=$(mktemp -d "$root/engines/.kimodo.XXXXXX")
        trap 'rm -rf -- "$checkout"' EXIT
        heavy git clone --quiet --filter=blob:none https://github.com/nv-tlabs/kimodo.git "$checkout"
        git -C "$checkout" checkout --quiet "$kimodo_commit"
        mv "$checkout" "$kimodo"
        trap - EXIT
    fi
    [ "$(git -C "$kimodo" rev-parse HEAD)" = "$kimodo_commit" ] || fail "existing Kimodo checkout is not $kimodo_commit; left in place"
    motion="$root/forge/motion/.venv"
    if [ ! -x "$motion/bin/python" ]; then heavy uv venv --python 3.11 "$motion"; fi
    heavy uv pip install --python "$motion/bin/python" -r forge/motion/requirements.txt
    # The MotionCorrection extension compiles x86 SSE/AVX intrinsics and fails on arm64; skip it
    # (generation runs without Kimodo's foot-skate post-process; forge-animlib grounding still applies).
    SKIP_MOTION_CORRECTION_IN_SETUP=1 heavy uv pip install --python "$motion/bin/python" --no-deps "$kimodo"
    if [ "$models" = yes ]; then
        # Network-bound and resumable; reads the Hugging Face login token in place for the gated base.
        "$motion/bin/python" -I forge/motion/fetch_models.py
    fi
fi
echo 'Setup complete. Run bin/forge-doctor for stage readiness.'
