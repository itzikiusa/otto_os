#!/bin/sh
# Git provides the effective destination after URL rewriting and pushRemote.
# Never answer generic prompts (including SSH key/passphrase prompts).
case "$1" in
  "Username for '"*"': "|"Password for '"*"': ") ;;
  *) exit 1 ;;
esac
url=${1#*\'}
url=${url%%\'*}
case "$url" in
  https://*|http://*) ;;
  *) exit 1 ;;
esac
scheme=${url%%://*}
authority=${url#*://}
authority=${authority%%/*}
authority=${authority##*@}
authority=$(printf '%s' "$authority" | /usr/bin/tr '[:upper:]' '[:lower:]')
case "$scheme:$authority" in
  https:*:443) authority=${authority%:443} ;;
  http:*:80) authority=${authority%:80} ;;
esac
origin="$scheme://$authority"
approved=false
set -f
IFS='
'
for allowed in $OTTO_GIT_ORIGINS; do
  if [ "$origin" = "$allowed" ]; then approved=true; break; fi
done
if [ "$approved" != true ]; then
  printf '%s\n' 'Otto: Git credential destination changed; review the remote and account binding.' >&2
  exit 1
fi
case "$1" in
  Username*) printf '%s\n' "${OTTO_GIT_USERNAME:-x-token-auth}" ;;
  Password*) printf '%s\n' "$OTTO_GIT_TOKEN" ;;
esac
