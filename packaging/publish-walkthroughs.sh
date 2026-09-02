#!/bin/bash
# Re-encode the Remotion-rendered walkthrough MP4s to a web-friendly 720p and
# upload them as assets of the rolling GitHub release tagged `walkthroughs`.
#
# The in-app Walkthroughs page (ui/src/modules/help/Walkthroughs.svelte)
# streams <base>/<Name>.mp4 from that release instead of bundling ~135 MB of
# 1080p video into ottod (embed-ui) and Otto.app. Assets are replaced in place
# (`--clobber`) so the URLs never change.
#
# Usage:
#   packaging/publish-walkthroughs.sh [src-dir]        # default: marketing/videos/out
#   ENCODE_ONLY=1 packaging/publish-walkthroughs.sh    # re-encode, skip the upload
#   CRF=27 packaging/publish-walkthroughs.sh           # sharper (bigger) output
#   AUDIO_KBPS=96 packaging/publish-walkthroughs.sh    # audio bitrate (AAC), default 128
#   OUT_DIR=/path packaging/publish-walkthroughs.sh    # keep the encoded files
#
# Requires: ffmpeg + ffprobe on PATH (brew install ffmpeg), gh authenticated
# with push access to $REPO (gh auth login).
set -euo pipefail

REPO="${REPO:-itzikiusa/otto_os}"
TAG="${TAG:-walkthroughs}"
CRF="${CRF:-30}"
AUDIO_KBPS="${AUDIO_KBPS:-128}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="${1:-$ROOT/marketing/videos/out}"

if [[ -n "${OUT_DIR:-}" ]]; then
  OUT="$OUT_DIR"; mkdir -p "$OUT"
else
  OUT="$(mktemp -d)"; trap 'rm -rf "$OUT"' EXIT
fi

for bin in ffmpeg ffprobe gh; do
  command -v "$bin" >/dev/null || { echo "error: $bin not found on PATH" >&2; exit 1; }
done

shopt -s nullglob
FILES=("$SRC"/*.mp4)
(( ${#FILES[@]} )) || { echo "error: no *.mp4 in $SRC" >&2; exit 1; }

echo "== re-encoding ${#FILES[@]} file(s) from $SRC → $OUT (1280x720 H.264 crf=$CRF)"
before=0; after=0
for f in "${FILES[@]}"; do
  n="$(basename "$f")"
  # Re-encode audio to 128k AAC when a track is present. Remotion's masters carry
  # a ~317 kbps AAC track that would otherwise dominate the 720p files (video
  # lands around 165 kbps at crf 30).
  if ffprobe -v error -select_streams a -show_entries stream=codec_type -of csv=p=0 "$f" | grep -q audio; then
    audio=(-c:a aac -b:a "$AUDIO_KBPS"k)
  else
    audio=(-an)
  fi
  ffmpeg -y -v error -i "$f" \
    -vf scale=1280:720 -c:v libx264 -crf "$CRF" -preset slow -pix_fmt yuv420p \
    -movflags +faststart "${audio[@]}" "$OUT/$n"
  b=$(stat -f%z "$f"); a=$(stat -f%z "$OUT/$n")
  before=$((before + b)); after=$((after + a))
  printf '  %-22s %6.1f MB → %5.1f MB\n' "$n" "$(echo "$b/1048576" | bc -l)" "$(echo "$a/1048576" | bc -l)"
done
printf '== total %.1f MB → %.1f MB\n' "$(echo "$before/1048576" | bc -l)" "$(echo "$after/1048576" | bc -l)"

if [[ -n "${ENCODE_ONLY:-}" ]]; then
  echo "ENCODE_ONLY set — skipping upload. Files in $OUT"
  [[ -z "${OUT_DIR:-}" ]] && trap - EXIT && echo "(temp dir kept: $OUT)"
  exit 0
fi

if ! gh release view "$TAG" -R "$REPO" >/dev/null 2>&1; then
  echo "== creating rolling release $REPO@$TAG"
  gh release create "$TAG" -R "$REPO" \
    --title "Otto walkthrough videos" \
    --notes "Rolling release of the in-app walkthrough videos; replaced in place by packaging/publish-walkthroughs.sh" \
    --prerelease
fi

echo "== uploading to $REPO release $TAG (--clobber)"
gh release upload "$TAG" -R "$REPO" --clobber "$OUT"/*.mp4

echo "== done: https://github.com/$REPO/releases/download/$TAG/<Name>.mp4"
