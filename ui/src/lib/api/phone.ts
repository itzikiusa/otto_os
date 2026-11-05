// Phone access API (docs/contracts/api.md "Phone access"). Owner routes need
// a person's desktop credential; `/phone/pair/*` are public (the one-time code
// / poll secret in the body is the credential).
import { api } from './client';
import type {
  PairClaimResult, PairPollResult, PairingCreated, PairingView, PhoneAccessStatus, PhoneDevice,
} from './types';

export const phoneApi = {
  status: () => api.get<PhoneAccessStatus>('/phone/status'),
  serve: (enabled: boolean) => api.long.post<PhoneAccessStatus>('/phone/serve', { enabled }),
  createPairing: (next: string | null) => api.post<PairingCreated>('/phone/pairings', { next }),
  pairing: (id: string) => api.bg.get<PairingView>(`/phone/pairings/${encodeURIComponent(id)}`),
  approve: (id: string) => api.post<PairingView>(`/phone/pairings/${encodeURIComponent(id)}/approve`),
  deny: (id: string) => api.post<PairingView>(`/phone/pairings/${encodeURIComponent(id)}/deny`),
  devices: () => api.get<PhoneDevice[]>('/phone/devices'),
  rename: (id: string, name: string) => api.patch<PhoneDevice>(`/phone/devices/${encodeURIComponent(id)}`, { name }),
  revoke: (id: string) => api.del<void>(`/phone/devices/${encodeURIComponent(id)}`),
  revokeAll: () => api.post<{ revoked: number }>('/phone/devices/revoke-all'),
  me: () => api.get<{ device: PhoneDevice | null }>('/phone/device/me'),
  unpairSelf: () => api.del<void>('/phone/device/me'),
  claim: (code: string) => api.post<PairClaimResult>('/phone/pair/claim', { code }),
  poll: (pairingId: string, pollSecret: string) =>
    api.bg.post<PairPollResult>('/phone/pair/poll', { pairing_id: pairingId, poll_secret: pollSecret }),
};
