// Process-sandbox exemptions (Settings → Daemon). With the sandbox on, every
// agent provider is confined unless a person lists it in `exempt_providers`.
// Pure so the round-trip is unit-tested without the page.

export type SandboxNetwork = 'full' | 'loopback' | 'none';

export interface ProcessSandbox {
  enabled: boolean;
  network: SandboxNetwork;
  /** Providers a person exempted from confinement. */
  exempt_providers?: string[];
  /** Legacy allow-list of built-in providers (kept as-is on save). */
  providers?: string[];
}

/** Every provider the exemption list can name: the registered ones (`/meta`),
 *  the custom ones in the `providers` setting (a disabled one still resumes),
 *  and any name already exempt — never dropped just because it is gone. */
export function exemptCandidates(
  metaProviders: readonly string[],
  providersSetting: unknown,
  exempt: readonly string[],
): string[] {
  const custom =
    providersSetting && typeof providersSetting === 'object' && !Array.isArray(providersSetting)
      ? Object.keys(providersSetting)
      : [];
  return [...new Set([...metaProviders, ...custom, ...exempt])]
    .filter((p) => p.trim() !== '')
    .sort((a, b) => a.localeCompare(b));
}

/** The exempt list as stored: strings only, de-duplicated, sorted. */
export function readExempt(sb: Partial<ProcessSandbox> | undefined): string[] {
  const list = Array.isArray(sb?.exempt_providers) ? sb.exempt_providers : [];
  return [...new Set(list.filter((p): p is string => typeof p === 'string'))].sort((a, b) =>
    a.localeCompare(b),
  );
}

export function sameExempt(a: readonly string[], b: readonly string[]): boolean {
  return a.length === b.length && a.every((p) => b.includes(p));
}

/** The `process_sandbox` value to save: the keys this form does not edit
 *  (the legacy `providers` list) survive; the exempt list is what the person
 *  ticked. */
export function sandboxBody(
  saved: Partial<ProcessSandbox> | undefined,
  enabled: boolean,
  network: SandboxNetwork,
  exempt: readonly string[],
): ProcessSandbox {
  return { ...(saved ?? {}), enabled, network, exempt_providers: [...exempt].sort((a, b) => a.localeCompare(b)) };
}
