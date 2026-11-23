// Pure CLI snippet generator for Kafka clusters & topics (`kcat`).
// Never emits real secrets or passwords — SASL credentials use shell-variable
// placeholders (`$KAFKA_PASSWORD`) and all user-controlled values are POSIX
// single-quoted so shell metacharacters cannot execute.

import type { BrokerCluster, SaslMechanism, SecurityProtocol } from '../../lib/api/types';

export type KcatClusterInput = Pick<BrokerCluster, 'bootstrap_servers'> & {
  security_protocol?: SecurityProtocol | null;
  sasl_mechanism?: SaslMechanism | null;
  sasl_username?: string | null;
  has_sasl_password?: boolean;
  tls_skip_verify?: boolean;
};

export interface KcatSnippetOptions {
  topic: string;
  partition?: number | null;
  offset?: number | null;
  mode?: 'consume' | 'produce';
}

const SASL_MECHANISM_NAMES: Record<SaslMechanism, string> = {
  plain: 'PLAIN',
  scram_sha_256: 'SCRAM-SHA-256',
  scram_sha_512: 'SCRAM-SHA-512',
};

/** POSIX-safe single-quoted shell argument. Embedded `'` becomes `'\''`. */
export function shellQuote(value: string): string {
  return `'${value.replace(/'/g, "'\\''")}'`;
}

function isNonNegInt(n: number | null | undefined): n is number {
  return typeof n === 'number' && Number.isInteger(n) && n >= 0;
}

/**
 * Build a copy-pasteable `kcat` command for consuming from or producing to a
 * topic on `cluster`.
 *
 * Security: never reads or embeds raw passwords/secrets. When TLS or SASL is
 * configured on the cluster, emits safe `-X` flags with `"$KAFKA_PASSWORD"`
 * (and `"$KAFKA_USERNAME"` when no username is set).
 */
export function buildKcatSnippet(cluster: KcatClusterInput, opts: KcatSnippetOptions): string {
  const mode = opts.mode ?? 'consume';
  const parts: string[] = [
    'kcat',
    '-b',
    shellQuote(cluster.bootstrap_servers.trim()),
    '-t',
    shellQuote(opts.topic),
    mode === 'produce' ? '-P' : '-C',
  ];

  if (isNonNegInt(opts.partition)) {
    parts.push('-p', String(opts.partition));
  }
  if (mode !== 'produce' && isNonNegInt(opts.offset)) {
    parts.push('-o', String(opts.offset));
  }

  const protocol = cluster.security_protocol ?? 'plaintext';
  const usesTls = protocol === 'ssl' || protocol === 'sasl_ssl';
  const usesSasl = protocol === 'sasl_plaintext' || protocol === 'sasl_ssl';

  if (protocol !== 'plaintext') {
    parts.push('-X', `security.protocol=${protocol}`);
  }
  if (usesTls && cluster.tls_skip_verify) {
    parts.push('-X', 'enable.ssl.certificate.verification=false');
  }
  if (usesSasl) {
    if (cluster.sasl_mechanism && cluster.sasl_mechanism in SASL_MECHANISM_NAMES) {
      parts.push('-X', `sasl.mechanisms=${SASL_MECHANISM_NAMES[cluster.sasl_mechanism]}`);
    }
    const user = cluster.sasl_username?.trim();
    if (user) {
      parts.push('-X', `sasl.username=${shellQuote(user)}`);
    } else {
      parts.push('-X', 'sasl.username="$KAFKA_USERNAME"');
    }
    parts.push('-X', 'sasl.password="$KAFKA_PASSWORD"');
  }

  return parts.join(' ');
}
