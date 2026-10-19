import hashlib, json, os, pathlib, signal, socket, subprocess, tempfile, time, urllib.request

binary = pathlib.Path('/Users/itziklavon/otto_os/target/debug/ottod')
root = pathlib.Path(tempfile.mkdtemp(prefix='otto-active-restart-'))
workspace = root / 'workspace'
workspace.mkdir()
fake = root / 'fake-bin'
fake.mkdir()
for name in ['claude', 'codex', 'agy', 'gemini', 'grok']:
    p = fake / name
    p.write_text('#!/bin/sh\nexec cat\n')
    p.chmod(0o755)
with socket.socket() as s:
    s.bind(('127.0.0.1', 0))
    port = s.getsockname()[1]
base = f'http://127.0.0.1:{port}/api/v1'
env = dict(os.environ, OTTO_DATA_DIR=str(root / 'data'), OTTO_PORT=str(port),
           OTTO_E2E='1', OTTO_SELF_IMPROVE='0', OTTO_CLI_UPDATE='0',
           OTTO_SECRETS='file', OTTO_SECRETS_ALLOW_PLAINTEXT='1',
           OTTO_PLUGINS_HOME=str(root / 'plugins'), CLAUDE_BIN='/nonexistent/otto-test',
           PATH=str(fake) + ':' + os.environ['PATH'])
token = None
proc = None
sid = None
results = []

def request(path, body=None):
    headers = {'Content-Type': 'application/json'}
    if token:
        headers['Authorization'] = 'Bearer ' + token
    req = urllib.request.Request(base + path, data=json.dumps(body).encode() if body is not None else None, headers=headers)
    with urllib.request.urlopen(req, timeout=15) as response:
        data = response.read()
        return json.loads(data) if data else None

def await_condition(fn, seconds=30):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        try:
            value = fn()
            if value:
                return value
        except (OSError, ValueError):
            pass
        time.sleep(.1)
    raise AssertionError('condition timed out')

def start(n):
    global proc
    log = open(root / f'daemon-{n}.log', 'wb')
    proc = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=log)
    await_condition(lambda: request('/health'))

def stop():
    before = time.monotonic()
    proc.send_signal(signal.SIGTERM)
    code = proc.wait(timeout=25)
    assert code == 0, code
    results.append({'exit': code, 'seconds': round(time.monotonic() - before, 3)})

try:
    start(1)
    token = request('/onboarding/root', {'password': 'isolated-test-password-2026', 'display_name': 'Isolated restart test'})['token']
    wid = request('/workspaces', {'name': 'Isolated restart test', 'root_path': str(workspace)})['id']
    sid = request(f'/workspaces/{wid}/sessions', {'kind': 'agent', 'provider': 'shell', 'title': 'Isolated restart fixture', 'cwd': str(workspace)})['id']
    first = workspace / 'pid-before'
    request(f'/sessions/{sid}/input', {'text': f'echo $$ > {first}', 'submit': True})
    pid = int(await_condition(lambda: first.read_text().strip()))
    stop()
    os.kill(pid, 0)
    start(2)
    second = workspace / 'pid-after'
    request(f'/sessions/{sid}/input', {'text': f'echo $$ > {second}', 'submit': True})
    after = int(await_condition(lambda: second.read_text().strip()))
    assert pid == after, (pid, after)
    request(f'/sessions/{sid}/kill', {})
    stop()
    evidence = {'binary': str(binary), 'sha256': hashlib.sha256(binary.read_bytes()).hexdigest(),
                'isolated_data': str(root), 'same_shell_pid_after_restart': pid == after,
                'shell_pid': pid, 'results': results,
                'boundary': 'One isolated holder-backed shell survives graceful restart and accepts input after reattachment. Explicitly killed fixture before final shutdown. Does not prove every background job drains.'}
    out = pathlib.Path('docs/reviews/product-wide-2026-10-18/daemon-active-restart-smoke.json')
    out.write_text(json.dumps(evidence, indent=2) + '\n')
    print(json.dumps(evidence, indent=2))
finally:
    if proc and proc.poll() is None:
        if sid and token:
            try:
                request(f'/sessions/{sid}/kill', {})
            except Exception:
                pass
        proc.terminate()
        proc.wait(timeout=25)
