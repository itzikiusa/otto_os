#!/bin/sh
# Machine-wide 2-slot lock for heavy commands (model inference, bakes, big builds, test suites).
# Usage: tools/heavy.sh <command> [args...]
# At most two heavy commands run at once. A waiting command takes whichever slot frees first
# (it polls both; blocking on one slot would leave the other idle while a queue forms).
# A third slot is open only to commands started under a path prefix listed in
# /tmp/otto-heavy-slot-3.dirs (machine-local, one prefix per line; gone after a reboot).
slots="1 2"
if [ -r /tmp/otto-heavy-slot-3.dirs ]; then
  while read -r p; do
    case "$PWD/" in "$p"*) slots="3 1 2" ;; esac
  done < /tmp/otto-heavy-slot-3.dirs
fi
while :; do
  for slot in $slots; do
    /usr/bin/lockf -s -t 0 "/tmp/otto-heavy-slot-$slot.lock" "$@"
    rc=$?
    [ "$rc" -ne 75 ] && exit "$rc"   # 75 = slot busy; anything else is the command's own exit code
  done
  sleep 2
done
