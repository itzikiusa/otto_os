#!/usr/bin/env python3
"""Claude-code-like TUI simulator for terminal-robustness verification.

Mimics the behaviors that matter for the resize bug class:
- Emits a transcript of numbered messages, HARD-WRAPPED at the current
  terminal width (like claude's renderer), so old output cannot reflow.
- Manages a "tail region" (the last MANAGED visual lines). On SIGWINCH it
  re-renders that region at the new width by cursor-up + clear-line, exactly
  the claude repaint pattern that creates staircase frames in scrollback.
- Keeps running until killed, repainting on every SIGWINCH.
"""
import os
import signal
import shutil
import sys
import textwrap
import time

import os as _os
N_MSGS = int(_os.environ.get('SIM_MSGS', '40'))
MANAGED_MSGS = 6  # tail MESSAGES the TUI owns and re-renders on winch

msgs = [
    f"MSG_{i:03d} the quick brown fox jumps over the lazy dog while counting "
    f"tokens and painting terminal frames repeatedly {i:03d}"
    for i in range(N_MSGS)
]


def width() -> int:
    return shutil.get_terminal_size().columns


def wrapped_lines():
    w = max(10, width())
    out = []
    for m in msgs:
        out.extend(textwrap.wrap(m, w) or [""])
    return out


last_frame: list[str] = []  # hard lines of the managed tail as last printed


def frame_height_now(pieces) -> int:
    """Visual rows the previously-printed hard lines occupy AFTER the
    terminal reflowed them to the CURRENT width — the bookkeeping a correct
    TUI (claude) does before erasing its old frame."""
    w = max(10, width())
    return sum(max(1, -(-len(p) // w)) for p in pieces)


def frame_pieces():
    w = max(10, width())
    out = []
    for m in msgs[-MANAGED_MSGS:]:
        out.extend(textwrap.wrap(m, w) or [""])
    return out


def full_render():
    global last_frame
    w = max(10, width())
    head = []
    for m in msgs[:-MANAGED_MSGS]:
        head.extend(textwrap.wrap(m, w) or [""])
        head.append("")
    sys.stdout.write("\r\n".join(head) + "\r\n")
    frame = frame_pieces() + [f"[input box @ {width()}c]"]
    sys.stdout.write("\r\n".join(frame) + "\r\n")
    last_frame = frame
    sys.stdout.flush()


def repaint_tail(_sig=None, _frm=None):
    """SIGWINCH: erase the managed tail region (at its REFLOWED height) and
    re-render at the new width."""
    global last_frame
    up = frame_height_now(last_frame)
    sys.stdout.write(f"\x1b[{up}A\r")
    sys.stdout.write("\x1b[J")               # erase from here to end of screen
    frame = frame_pieces() + [f"[input box @ {width()}c]"]
    sys.stdout.write("\r\n".join(frame) + "\r\n")
    last_frame = frame
    sys.stdout.flush()


signal.signal(signal.SIGWINCH, repaint_tail)

full_render()
while True:
    time.sleep(0.2)
