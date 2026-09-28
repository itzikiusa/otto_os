#!/usr/bin/env python3
"""Package/run the isolated room_native_probe Cargo example; never starts Otto.

Build separately with TAURI_CONFIG='{"bundle":{"externalBin":[]}}' cargo build
--manifest-path apps/desktop/src-tauri/Cargo.toml --example room_native_probe.
Pass --binary with the resulting executable. Default mode never requests capture.
--physical exposes explicit user-operated buttons; it never captures on launch.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile
import threading

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', required=True, type=Path)
parser.add_argument('--cycles', type=int, choices=range(1, 21), default=1, help='AudioWorklet acquire/flush/close cycles per webview (1-20)')
parser.add_argument('--physical', action='store_true', help='Open opt-in buttons for user microphone/screen checks; never captures automatically')
args = parser.parse_args()
if not args.binary.is_file():
    parser.error('Build the example first; --binary must name its executable.')
output = Path(tempfile.mkdtemp(prefix='otto-room-tauri-probe-'))
repo = Path(__file__).resolve().parents[2]
worklet = (repo / 'ui/public/room-recap-worklet.js').read_bytes()


class Fixture(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        is_worklet = self.path == '/room-recap-worklet.js'
        body = worklet if is_worklet else b'<!doctype html><title>Isolated guest room fixture</title>'
        self.send_response(200)
        self.send_header('Content-Type', 'text/javascript' if is_worklet else 'text/html')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args):
        pass


server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Fixture)
threading.Thread(target=server.serve_forever, daemon=True).start()
bundle = output / 'RoomNativeProbe.app' / 'Contents'
(bundle / 'MacOS').mkdir(parents=True)
binary = bundle / 'MacOS' / 'room_native_probe'
shutil.copy2(args.binary, binary)
info = plistlib.loads((repo / 'apps/desktop/src-tauri/Info.plist').read_bytes())
info.update(CFBundleIdentifier='com.otto.room-native-probe', CFBundleExecutable='room_native_probe',
            CFBundleName='RoomNativeProbe', CFBundlePackageType='APPL', LSUIElement=True)
(bundle / 'Info.plist').write_bytes(plistlib.dumps(info))
env = os.environ.copy()
env['OTTO_ROOM_PROBE_CYCLES'] = str(args.cycles)
env['OTTO_ROOM_PROBE_PHYSICAL'] = '1' if args.physical else '0'
env['OTTO_ROOM_PROBE_INVITATION'] = f'http://127.0.0.1:{server.server_port}/#/room/probe/synthetic_invitation_only'
try:
    with (output / 'probe.jsonl').open('w') as log, (output / 'probe.stderr').open('w') as error:
        result = subprocess.run([str(binary)], env=env, stdout=log, stderr=error, timeout=915 if args.physical else 45)
finally:
    server.shutdown()
    server.server_close()
events = [json.loads(line) for line in (output / 'probe.jsonl').read_text().splitlines() if line.startswith('{')]
print(f'Artifacts: {output}')
for event in events:
    print(json.dumps(event))
passed = result.returncode == 0 and any(event.get('event') == 'done' and event.get('passed') for event in events)
print(f'Exit: {result.returncode}; passed: {passed}')
raise SystemExit(0 if passed else 1)
