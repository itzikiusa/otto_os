// Pure helpers behind the status bar's daemon indicators (network listener,
// secret store). Kept free of Svelte/auth state so `node --test` can load it;
// the cached reads live in ./daemonStatus.svelte.ts.

/** The part of `GET /security-posture` the listener indicator reads. */
export interface ListenerPosture {
  /** The 0.0.0.0 listener is actually BOUND by the running daemon. */
  network_listener: boolean;
  /** The saved setting differs from what is bound: it applies on restart. */
  network_listener_restart_required: boolean;
}

export interface ListenerIndicator {
  text: string;
  title: string;
  /** The saved setting is not what the running daemon serves yet. */
  pending: boolean;
}

/**
 * What the status bar says about the network listener. `/meta` reports the
 * SAVED setting (`configured`), which a Save flips long before the listener
 * is bound — it is bound once, at daemon start. Root can read what is bound
 * (`/security-posture`); while the two differ the indicator says the saved
 * value is waiting on a restart instead of claiming a socket that isn't open.
 * Without a posture (non-root, or not read yet) the saved value is all we know.
 */
export function listenerIndicator(configured: boolean | undefined, posture: ListenerPosture | null): ListenerIndicator {
  const word = (network: boolean): string => (network ? 'network' : 'loopback');
  if (!posture) {
    return { text: word(!!configured), title: 'Network listener', pending: false };
  }
  const bound = posture.network_listener;
  if (!posture.network_listener_restart_required) {
    return {
      text: word(bound),
      title: bound ? 'Network listener: bound on 0.0.0.0' : 'Network listener: off — loopback only',
      pending: false,
    };
  }
  // The saved value is the opposite of what is bound (that is what
  // "restart required" means), whatever `/meta` said a moment ago.
  return {
    text: `${word(!bound)} (after restart)`,
    title: `Serving ${bound ? 'on your network' : 'loopback only'} now. The saved setting (${word(!bound)}) applies when the daemon restarts — Settings ▸ Daemon.`,
    pending: true,
  };
}

/** The part of `GET /admin/secrets/status` the warning reads. */
export interface SecretsModeLike {
  mode: 'plaintext' | 'encrypted' | 'keychain';
}

/** Integration secrets sit in the unencrypted `secrets.json` store. */
export function secretsUnencrypted(s: SecretsModeLike | null | undefined): boolean {
  return s?.mode === 'plaintext';
}
