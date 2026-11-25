package dao

import (
	"context"
	"database/sql"
	"errors"
	"fmt"

	"github.com/jmoiron/sqlx"
)

type SessionDaoImpl struct {
	koalaDbService KoalaDbService
}

func (dao *SessionDaoImpl) GetSessionWrapper(ctx context.Context, providerId int, providerSession string) (*SessionWrapper, error) {
	conn, err := dao.koalaDbService.GetConnection(ctx)
	if err != nil {
		logger.ErrorWithContext(ctx, fmt.Sprintf(DBConnectionFailureFormat, "eucalyptus", err))
		return nil, err
	}
	args := []interface{}{providerSession}
	var sessionWrapper SessionWrapper
	query := `
		SELECT session_info.id, game_id, account_id, currency, gp_session, session_route.created
		FROM session_info
		INNER JOIN session_route ON session_info.id = session_route.id
		WHERE session_route.gp_session = ?
		`
	if providerId > 0 {
		args = append(args, providerId)
		query += " AND session_route.game_provider_id = ?"
	}
	query = query + " ORDER BY session_info.id DESC LIMIT 1"
	err = conn.GetContext(ctx, &sessionWrapper, query, args...)
	return &sessionWrapper, err
}

func (dao *SessionDaoImpl) UpdateSessionRoute(ctx context.Context, providerID int, oldSessionID string, newSessionID string) error {
	conn, _ := dao.koalaDbService.GetConnection(ctx)
	tx, err := conn.BeginTx(ctx, nil)
	if err != nil {
		return err
	}
	defer func() {
		if err = tx.Rollback(); err != nil && !errors.Is(err, sql.ErrTxDone) {
			logger.ErrorWithContext(ctx, "failed to rollback transaction", err)
		}
	}()
	insertSessionRouteQuery := "UPDATE session_route SET gp_session = ? WHERE game_provider_id = ? AND gp_session = ?"
	if _, err = tx.ExecContext(ctx, insertSessionRouteQuery, newSessionID, providerID, oldSessionID); err != nil {
		return err
	}
	updateSessionQuery := "UPDATE player_session_info SET gp_session = ? WHERE game_provider_id = ? AND gp_session = ?"
	_, _ = tx.ExecContext(ctx, updateSessionQuery, newSessionID, providerID, oldSessionID)
	return tx.Commit()
}

func (dao *SessionDaoImpl) SaveSession(ctx context.Context, info *StartGameRequestInfo, providerSession string) (int64, error) {
	conn, _ := dao.koalaDbService.GetConnection(ctx)
	tx, _ := conn.BeginTx(ctx, nil)
	insertSessionInfoQuery := "INSERT INTO session_info (game_id, account_id, currency, created) values (?,?,?,NOW())"
	result, err := tx.ExecContext(ctx, insertSessionInfoQuery, info.GameId, info.AccountId, info.Currency)
	if err != nil {
		return -1, err
	}
	id, _ := result.LastInsertId()
	insertSessionRouteQuery := "INSERT INTO session_route (id, gp_session, created) values (?,?, NOW())"
	_, _ = tx.ExecContext(ctx, insertSessionRouteQuery, id, providerSession)

	// insert into player_session_info
	playerSessionInfoQuery := `
		INSERT INTO player_session_info (account_id, gp_session, update_time)
		VALUES (?,?, NOW())
		ON DUPLICATE KEY UPDATE gp_session = ?, update_time = NOW()
	`
	_, _ = tx.ExecContext(ctx, playerSessionInfoQuery, info.AccountId, providerSession, providerSession)
	return id, tx.Commit()
}

func (dao *SessionDaoImpl) findLatest(ctx context.Context, conn *sqlx.DB, accountId string) (string, error) {
	var gpSession string
	// fallback to session_info table
	query := `
				SELECT gp_session
				FROM player_session_info WHERE account_id = ? ORDER BY update_time DESC LIMIT 1
		`
	err := conn.GetContext(ctx, &gpSession, query, accountId)
	return gpSession, err
}
