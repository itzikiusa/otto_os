#!/bin/sh
# The scene suite's stand-in Otto Forge: wired in as the built-in provider's
# executable of an isolated daemon (the e2e fake engine's seam), it hands back
# the model the suite prepared for each prompt and makes nothing.
#   generate   <dir>/standins/<sha16 of --prompt>.glb (else fails)
#   library    <dir>/standins/lib-<sha16 of --prompt>.glb + .json manifest (else "no library match")
#   rig        fails with <dir>/standins/rigfail-<sha16 of the mesh>.txt when there is one, else passes the mesh on
#   other      passes the mesh on
stage="$1"; shift
out=""; mesh=""; prompt=""
while [ $# -gt 0 ]; do
  case "$1" in
    --output) out="$2"; shift 2;;
    --mesh) mesh="$2"; shift 2;;
    --prompt) prompt="$2"; shift 2;;
    --help) echo "usage: forge $stage [--prompt P] [--output O]"; exit 0;;
    *) shift;;
  esac
done
dir="$(cd "$(dirname "$0")" && pwd)/standins"
key() { shasum -a 256 | cut -c1-16; }
case "$stage" in
  generate)
    src="$dir/$(printf '%s' "$prompt" | key).glb"
    [ -f "$src" ] || { printf '{"error":"no stand-in for this prompt"}\n' >&2; exit 1; }
    cp "$src" "$out";;
  library)
    k="$(printf '%s' "$prompt" | key)"
    [ -f "$dir/lib-$k.glb" ] || { printf '{"error":"no library match"}\n' >&2; exit 1; }
    cp "$dir/lib-$k.glb" "$out"; cp "$dir/lib-$k.json" "$out.manifest.json"; exit 0;;
  rig)
    f="$dir/rigfail-$(key < "$mesh").txt"
    [ -f "$f" ] && { printf '{"error":"%s"}\n' "$(cat "$f")" >&2; exit 1; }
    cp "$mesh" "$out";;
  *) cp "$mesh" "$out";;
esac
printf '{"stand_in":true}' > "$out.manifest.json"
