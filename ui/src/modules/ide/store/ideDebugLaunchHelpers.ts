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
