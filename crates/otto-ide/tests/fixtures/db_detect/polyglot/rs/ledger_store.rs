//! Ledger store: reads entries from the ledger table.
use sqlx::PgPool;

pub struct Store<'a> {
    pool: &'a PgPool,
}

impl<'a> Store<'a> {
    // DELETE FROM fake_rust_comment_table
    pub async fn append(&self, e: &'a Entry) -> sqlx::Result<()> {
        sqlx::query(r#"INSERT INTO ledger_entries (id, "amount") VALUES ($1, $2)"#)
            .execute(self.pool)
            .await?;
        let _q = "SELECT id
                 FROM ledger_accounts
                 WHERE id = $1";
        let _c = '"';
        sqlx::query("UPDATE ledger_accounts SET balance = balance + $1 WHERE id = $2")
            .execute(self.pool)
            .await?;
        Ok(())
    }
}
