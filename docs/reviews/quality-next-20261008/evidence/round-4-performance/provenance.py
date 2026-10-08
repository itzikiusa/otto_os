"""Record the exact local inputs to the isolated rendered-terminal workload."""
import hashlib, json, pathlib, platform, plistlib, subprocess
root = pathlib.Path(__file__).resolve().parents[5]
out = pathlib.Path(__file__).parent
paths = [
    'target/debug/ottod', 'Cargo.lock', 'ui/package-lock.json',
    'crates/otto-sessions/src/ws.rs', 'crates/otto-sessions/src/manager.rs',
    'crates/otto-pty/src/lib.rs', 'crates/otto-pty/src/held.rs', 'crates/otto-pty/src/holder.rs',
    'crates/otto-server/src/transcript_cache.rs', 'crates/otto-server/src/transcript_tail.rs',
    'crates/otto-server/src/skill_eval.rs', 'crates/otto-state/src/skill_evals.rs',
    'ui/src/lib/components/Terminal.svelte', 'ui/src/lib/components/termFlow.ts',
    'ui/src/lib/components/termCompactQueue.ts', 'ui/src/lib/components/termPark.ts',
    'ui/src/lib/stores/transcript.svelte.ts', 'ui/src/lib/stores/transcriptLifecycle.ts',
    'ui/e2e/desktop-terminal-rendered-load-perf.spec.ts', 'ui/e2e/process-resources.ts',
    'ui/e2e/global-setup.ts', 'ui/e2e/global-teardown.ts', 'ui/playwright.config.ts',
]
def command(*args):
    return subprocess.check_output(args, cwd=root, text=True).strip()
data = {
    'head': command('git', 'rev-parse', 'HEAD'),
    'worktree_status': command('git', 'status', '--short'),
    'node': command('/opt/homebrew/bin/node', '--version'),
    'rust': command('rustc', '--version'),
    'machine': command('sysctl', '-n', 'hw.model'), 'platform': platform.platform(),
    'files': {p: hashlib.sha256((root / p).read_bytes()).hexdigest() for p in paths},
    'command': 'PATH=/opt/homebrew/bin:$PATH OTTO_E2E_SLOT=qualityr4perf OTTO_E2E_PORT=7896 OTTO_E2E_PW_PORT=5296 OTTO_E2E_BIN=/Users/itziklavon/otto_os/target/debug/ottod OTTO_E2E_SWEEP_ORPHANS=0 OTTO_E2E_TELEMETRY=1 OTTO_TERMINAL_LOAD_SECONDS=90 OTTO_TERMINAL_RECOVERY_SECONDS=10 npx playwright test --project=desktop-webkit --workers=1 --output=/tmp/otto-quality-r4-perf-results e2e/desktop-terminal-rendered-load-perf.spec.ts',
    'note': 'Fresh debug daemon after round-4 correctness repairs; real WebKit + Terminal + PTY. Provider command is owned cat shim, Vite development UI. No user data or 7700 mutation. Timed run under exclusive campaign lease; ordinary user apps remain running.'
}
webkit = pathlib.Path('/Users/itziklavon/Library/Caches/ms-playwright/webkit-2359')
data['webkit'] = {'revision':2359, 'plist':plistlib.loads((webkit/'Playwright.app/Contents/Info.plist').read_bytes()), 'executable_sha256':hashlib.sha256((webkit/'Playwright.app/Contents/MacOS/Playwright').read_bytes()).hexdigest()}
(out / 'provenance.json').write_text(json.dumps(data, indent=2) + '\n')
print(json.dumps({k: data[k] for k in ['head','node','rust','machine','platform']}, indent=2))
