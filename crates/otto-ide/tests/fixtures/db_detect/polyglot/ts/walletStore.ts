// Reads wallets from the cache first, then from postgres.
/* SELECT * FROM legacy_wallets */
import { Pool } from 'pg';

export async function loadWallet(db: Pool, id: string, kind: string) {
  const sql = `
    SELECT w.id, w.balance
    FROM wallets w
    LEFT JOIN wallet_limits l ON l.wallet_id = w.id
    WHERE w.id = $1`;
  const rows = await db.query(sql, [id]);
  await db.query('UPDATE wallets SET seen_at = now() WHERE id = $1', [id]);
  await db.query(`INSERT INTO wallet_events (wallet_id, kind) VALUES ($1, '${kind}')`, [id]);
  if (!rows) throw new Error(`failed to load wallet ${id} from wallets service`);
  return rows;
}
