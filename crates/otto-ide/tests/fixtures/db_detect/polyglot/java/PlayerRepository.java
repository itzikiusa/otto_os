package com.example.players;

import javax.persistence.Entity;
import javax.persistence.Table;

/**
 * Loads players from the legacy store. Select rows from players_archive only in reports.
 */
@Entity
@Table(name = "players")
public class PlayerRepository {
    // UPDATE ghost_table SET x = 1
    private static final String FIND = "SELECT p.id, p.login " +
            "FROM players p " +
            "JOIN player_balance b ON b.player_id = p.id " +
            "WHERE p.id = ?";
    private static final String AUDIT = """
            INSERT INTO player_audit (player_id, action)
            VALUES (?, ?)
            """;
    private final char quote = '"';
    private static final String PURGE = "DELETE FROM player_sessions WHERE expires_at < NOW()";

    void load() {
        logger.info("Loaded player from cache");
        throw new IllegalStateException("cannot read from database");
    }
}
