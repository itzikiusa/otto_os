// Realistic DB Watch fixture data for the layout / screenshot specs: a 40-column
// MySQL table, a Redis key prefix with TTLs and a Mongo collection with nested
// documents. Values are invented; nothing here comes from a real database.

export const SESSION_INFO_COLUMNS = [
  'id', 'game_id', 'account_id', 'brand_id', 'status', 'currency', 'balance', 'bonus_balance',
  'bet_total', 'win_total', 'round_count', 'provider', 'provider_session', 'gp_session', 'device',
  'platform', 'country', 'language', 'ip_address', 'user_agent', 'jurisdiction', 'license',
  'rtp_profile', 'max_bet', 'min_bet', 'jackpot_opt_in', 'free_spins_left', 'free_spins_total',
  'promo_code', 'campaign_id', 'affiliate_id', 'risk_score', 'kyc_level', 'self_excluded',
  'limits_json', 'last_round_id', 'closed_reason', 'created_at', 'updated_at', 'closed_at',
];

export function sessionInfoRow(id: number, over: Record<string, unknown> = {}): unknown[] {
  const base: Record<string, unknown> = {
    id, game_id: 1000 + (id % 40), account_id: 77000 + id, brand_id: 1001, status: 'open', currency: 'EUR',
    balance: 125.5, bonus_balance: 10, bet_total: 42, win_total: 37.5, round_count: 18, provider: 'pragmatic',
    provider_session: `pp-${id}-a91f`, gp_session: `gp-${id}-77c2`, device: 'mobile', platform: 'ios', country: 'MT',
    language: 'en', ip_address: '10.24.3.' + (id % 250), user_agent: 'Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X)',
    jurisdiction: 'MGA', license: 'MGA/B2C/394/2017', rtp_profile: 'default-96', max_bet: 100, min_bet: 0.2,
    jackpot_opt_in: 1, free_spins_left: 0, free_spins_total: 0, promo_code: null, campaign_id: null, affiliate_id: 'aff-221',
    risk_score: 12, kyc_level: 2, self_excluded: 0, limits_json: { daily_loss: 500, session_minutes: 120 },
    last_round_id: 900000 + id, closed_reason: null, created_at: '2026-11-25 11:58:02', updated_at: '2026-11-25 12:03:40', closed_at: null,
  };
  const v = { ...base, ...over };
  return SESSION_INFO_COLUMNS.map((c) => v[c] ?? null);
}

export const SESSION_ROUTE_COLUMNS = ['id', 'session_id', 'gp_session', 'route', 'node', 'updated_at'];
export function sessionRouteRow(id: number, over: Record<string, unknown> = {}): unknown[] {
  const v: Record<string, unknown> = { id, session_id: 400 + id, gp_session: `gp-${400 + id}-77c2`, route: 'eu-west-1a', node: 'gs-node-3', updated_at: '2026-11-25 12:03:40', ...over };
  return SESSION_ROUTE_COLUMNS.map((c) => v[c] ?? null);
}

export const REDIS_COLUMNS = ['key', 'type', 'value', 'ttl'];
export const MONGO_COLUMNS = ['_id', 'player_id', 'event', 'context', 'created_at'];
export function auditDoc(n: number, over: Record<string, unknown> = {}): unknown[] {
  const v: Record<string, unknown> = {
    _id: { $oid: `6744b1c2e4b0a9${String(n).padStart(10, '0')}` },
    player_id: 77000 + n,
    event: 'session.open',
    context: { device: { os: 'ios', model: 'iPhone16,2' }, geo: { country: 'MT', city: 'Valletta' }, limits: { daily_loss: 500 } },
    created_at: { $date: '2026-11-25T12:03:40Z' },
    ...over,
  };
  return MONGO_COLUMNS.map((c) => v[c] ?? null);
}
