package dao

import "context"

func (f *FailedTransactionDaoImpl) InsertFailedTransaction(ctx context.Context, t *FailedTransaction) (*FailedTransaction, error) {
	conn, err := f.koalaDbService.GetConnection(ctx)
	if err != nil {
		logger.ErrorWithContext(ctx, "failed to create connection to DB, err: "+err.Error())
		return nil, err
	}
	tx, _ := conn.BeginTx(ctx, nil)
	query := "INSERT INTO failed_transaction (raw_data, created, description) VALUES (?, NOW(), ?)"
	if _, err = tx.ExecContext(ctx, query, t.RawData, t.Description); err != nil {
		logger.ErrorWithContext(ctx, "failed to get session info")
		return nil, tx.Rollback()
	}
	return t, tx.Commit()
}
