// Phone access (Tailscale serve + QR pairing + device tokens) — mirrors
// docs/contracts/api.md "Phone access".

/** One device on the tailnet, as `tailscale status --json` reports it. */
export interface TailnetDevice {
  name: string;
  dns_name: string | null;
  os: string | null;
  online: boolean;
  ips: string[];
}

/** Otto's own `tailscale serve` entry. `https: false` = plain HTTP over the
 *  WireGuard tunnel (encrypted by Tailscale, no padlock, no passkeys). */
export interface PhoneServing {
  url: string;
  https: boolean;
  port: number;
}

/** `GET /phone/status` — Tailscale detection + whether Otto is served. */
export interface PhoneAccessStatus {
  installed: boolean;
  cli_path: string | null;
  backend_state: string | null;
  logged_in: boolean;
  tailnet: string | null;
  magic_dns: boolean;
  /** This Mac's MagicDNS name, e.g. `macbook-pro-2.tail0b7b6b.ts.net`. */
  self_name: string | null;
  self_ips: string[];
  devices: TailnetDevice[];
  serving: PhoneServing | null;
  /** Something other than Otto is served on this Mac (left untouched). */
  other_serves: boolean;
  /** Why HTTPS isn't used, etc. */
  note: string | null;
  error: string | null;
  /** Root on a person's own desktop credential: may turn serving on/off. */
  can_manage: boolean;
}

/** `POST /phone/pairings` — a fresh one-time pairing code (shown once). */
export interface PairingCreated {
  id: string;
  code: string;
  /** `<served url>/#/pair/<code>[/<next>]` — what the QR encodes. */
  url: string;
  expires_at: string;
}

export type PairingStatus = 'waiting' | 'claimed' | 'approved' | 'denied' | 'consumed' | 'expired';

/** `GET /phone/pairings/{id}` — the Mac's view of one pairing. */
export interface PairingView {
  id: string;
  status: PairingStatus;
  expires_at: string;
  device_label: string | null;
  peer_ip: string | null;
  /** The 6-digit comparison code, present once a phone claimed the code. */
  confirm_code: string | null;
}

/** A paired phone (its device token's metadata; never the token). */
export interface PhoneDevice {
  id: string;
  name: string;
  user_agent: string | null;
  created_at: string;
  last_seen_at: string;
  last_ip: string | null;
  idle_expires_at: string;
  expires_at: string;
  /** This request came from this device. */
  current: boolean;
}

/** `POST /phone/pair/claim` (public, phone side). */
export interface PairClaimResult {
  pairing_id: string;
  poll_secret: string;
  confirm_code: string;
  expires_at: string;
}

/** `POST /phone/pair/poll` (public, phone side). `token` arrives exactly once. */
export interface PairPollResult {
  status: 'pending' | 'approved' | 'denied' | 'expired';
  token?: string;
  next?: string | null;
  device?: PhoneDevice;
}
