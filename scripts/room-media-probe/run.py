#!/usr/bin/env python3
"""Run four synthetic WebRTC participants in WKWebView. No screen/mic capture.

Requires macOS, swiftc, and Python 3. Runs a temporary unsigned app bundle and
loopback-only STUN fixture; never starts Otto or a daemon. Output is not a
four-Mac acceptance benchmark: every participant shares one web content process.
"""
import argparse
import json
import plistlib
from pathlib import Path
import socket
import struct
import subprocess
import tempfile
import threading
import time

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--output', type=Path, help='New/empty directory for logs and temporary .app')
args = parser.parse_args()
output = args.output or Path(tempfile.mkdtemp(prefix='otto-room-media-'))
output.mkdir(parents=True, exist_ok=True)
if any(output.iterdir()):
    parser.error('Output must be empty; existing artifacts are never overwritten.')
fixture = Path(__file__).resolve().parent
stun = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
stun.bind(('127.0.0.1', 0))
port = stun.getsockname()[1]

def serve_stun():
    while True:
        try:
            data, address = stun.recvfrom(4096)
        except OSError:
            return
        if len(data) < 20 or data[:2] != b'\x00\x01':
            continue
        cookie = 0x2112A442
        mapped = struct.pack('!BBHI', 0, 1, address[1] ^ (cookie >> 16),
                             int.from_bytes(socket.inet_aton(address[0]), 'big') ^ cookie)
        attribute = struct.pack('!HH', 0x20, len(mapped)) + mapped
        stun.sendto(struct.pack('!HHI', 0x101, len(attribute), cookie) + data[8:20] + attribute, address)

threading.Thread(target=serve_stun, daemon=True).start()
script = output / 'benchmark.js'
script.write_text((fixture / 'benchmark.js').read_text().replace('stun:127.0.0.1:34789', f'stun:127.0.0.1:{port}'))
bundle = output / 'RoomProbe.app' / 'Contents'
(bundle / 'MacOS').mkdir(parents=True)
with (bundle / 'Info.plist').open('wb') as file:
    plistlib.dump({'CFBundleIdentifier': 'com.otto.synthetic-room-probe',
                  'CFBundleExecutable': 'RoomProbe', 'CFBundleName': 'RoomProbe',
                  'CFBundlePackageType': 'APPL', 'LSUIElement': True,
                  'NSMicrophoneUsageDescription': 'Synthetic capability probe; no microphone capture is requested.'}, file)
executable = bundle / 'MacOS' / 'RoomProbe'
subprocess.run(['swiftc', str(fixture / 'Probe.swift'), '-o', str(executable)], check=True)

def processes():
    rows = {}
    for line in subprocess.check_output(['ps', '-axo', 'pid=,ppid=,%cpu=,rss=,comm='], text=True).splitlines():
        parts = line.split(None, 4)
        if len(parts) == 5:
            pid, parent, cpu, rss, command = parts
            rows[int(pid)] = {'pid': int(pid), 'ppid': int(parent), 'cpu': float(cpu),
                              'rss_kib': int(rss), 'command': command}
    return rows

before = processes()
samples = []
with (output / 'benchmark.jsonl').open('w') as log, (output / 'benchmark.stderr').open('w') as error:
    process = subprocess.Popen([str(executable), str(script)], stdout=log, stderr=error)
    start = time.monotonic()
    try:
        while process.poll() is None:
            if time.monotonic() - start > 150:
                process.terminate()
                raise TimeoutError('Synthetic native probe exceeded 150 seconds')
            rows = processes()
            # WebKit XPC children are reparented to launchd. Identify new ones;
            # a concurrently launched WebKit app can contaminate attribution.
            selected = [v for pid, v in rows.items() if pid == process.pid or
                        (pid not in before and ('WebKit' in v['command'] or v['ppid'] == process.pid))]
            samples.append({'elapsed': time.monotonic() - start, 'cpu_sum': sum(v['cpu'] for v in selected),
                            'rss_kib_sum': sum(v['rss_kib'] for v in selected), 'processes': selected})
            time.sleep(2)
    finally:
        if process.poll() is None:
            process.terminate()
        process.wait(timeout=10)
        stun.close()
        (output / 'resources.json').write_text(json.dumps({'exit': process.returncode, 'samples': samples}, indent=2))
metadata = {'macOS': subprocess.check_output(['sw_vers'], text=True),
            'hardware': subprocess.check_output(['sysctl', '-n', 'machdep.cpu.brand_string', 'hw.memsize'], text=True),
            'fixture': str(fixture), 'note': 'All four participants run on one Mac; no actual screen/microphone capture.'}
(output / 'environment.json').write_text(json.dumps(metadata, indent=2))
events = [json.loads(line) for line in (output / 'benchmark.jsonl').read_text().splitlines() if line.startswith('{')]
errors = [event for event in events if event.get('event') == 'error']
print(f'Artifacts: {output}')
print(f'Exit: {process.returncode}; samples: {len(samples)}; fixture errors: {len(errors)}')
raise SystemExit(1 if process.returncode or errors or not any(e.get('event') == 'done' for e in events) else 0)
