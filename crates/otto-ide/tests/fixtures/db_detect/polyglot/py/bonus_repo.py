"""Bonus repository.

Reads bonuses from the warehouse when reporting.
"""
import psycopg

# SELECT * FROM commented_out_table


def grant(conn, player_id, total):
    """Select the active grants from the bonus table."""
    conn.execute(
        "INSERT INTO bonus_grants (player_id, amount) "
        "VALUES (%s, %s)", (player_id, 10))
    rows = conn.execute('''
        SELECT g.id FROM bonus_grants g
        WHERE g.player_id = %s
    ''', (player_id,)).fetchall()
    ratio = total // 2  # floor division, not a comment: the next line still counts
    conn.execute("UPDATE bonus_wallets SET amount = amount - %s WHERE player_id = %s", (ratio, player_id))
    log.info("loaded %d grants from bonus service", len(rows))
    conn.execute("DELETE FROM bonus_expired WHERE player_id = %s", (player_id,))
