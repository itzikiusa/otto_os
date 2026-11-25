package test

func seedSession(t *testing.T, db *sqlx.DB) {
	_, err := db.Exec("INSERT INTO session_info (id, game_id, account_id) VALUES (1, 'g', 'from the fixture')")
	require.NoError(t, err)
}
