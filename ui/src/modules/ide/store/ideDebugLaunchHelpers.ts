import type { IdeRunConfig } from '../../../lib/api/types';

const FRAMEWORK_DEV_CLIS = new Set(['vite', 'next', 'ng', 'nuxt', 'svelte-kit']);

/** Forward discovered framework dev CLI (`npx --no-install vite|next|ng|nuxt ...`)
 *  instead of launching `package.json` under Node debug (BUG-27). */
export function resolveFrameworkDevDebugLaunch(
  cfg: IdeRunConfig,
): { program: string; args: string[] } | null {
  if (cfg.runner !== 'npm') return null;
  const cmd = (cfg.command ?? '').trim();
  const rawArgs = cfg.args ?? [];
  const trimmed = rawArgs[0] === '--no-install' ? rawArgs.slice(1) : rawArgs;
  if (cmd === 'npx' && trimmed.length > 0 && FRAMEWORK_DEV_CLIS.has(trimmed[0])) {
    return { program: trimmed[0], args: trimmed.slice(1) };
  }
  const tp = (cfg.target_path ?? '').trim();
  if (FRAMEWORK_DEV_CLIS.has(tp)) {
    return { program: tp, args: trimmed[0] === tp ? trimmed.slice(1) : [] };
  }
  return null;
}

/** Runners the Node inspector cannot debug (Karma tests run in a browser, Playwright tests in
 *  worker processes): the daemon's own refusal (`unsupported_js_runner_error`, BUG-30). */
export function runOnlyDebugRefusal(runner: string | undefined): string | null {
  if (runner === 'playwright') return "Debug is not supported for Playwright under the Node inspector bridge (Playwright runs tests inside a separate worker process). Use Run or the Playwright Inspector (`PWDEBUG=1`).";
  if (runner === 'karma') return 'Debug is not supported for Karma/Angular browser tests under the Node inspector bridge. Use Run or attach a Chrome CDP session.';
  return null;
}
