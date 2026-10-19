//! Atomic acknowledgement of a fully reconciled remote-comment batch.
use super::*;

impl ProductRepo {
    /// Cursor ownership, question effects and events commit together. A failed
    /// provider/parse never calls this; a failed SQL write rolls back the cursor.
    pub async fn commit_watch_reconcile(
        &self,
        story: &Id,
        expected_cursor: Option<&str>,
        next_cursor: &str,
        answers: &[(Id, String)],
        questions: &[NewQuestion],
        events: &[NewEvent],
    ) -> Result<()> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(dberr("begin watch reconciliation"))?;
        let now = fmt(Utc::now());
        let changed = sqlx::query("UPDATE product_stories SET watch_cursor = ?, updated_at = ? WHERE id = ? AND watch_cursor IS ?")
            .bind(next_cursor).bind(&now).bind(story).bind(expected_cursor)
            .execute(&mut *tx).await.map_err(dberr("claim watch cursor"))?;
        if changed.rows_affected() != 1 {
            return Err(otto_core::Error::Conflict(
                "story or watch cursor changed during reconciliation".into(),
            ));
        }
        for (id, answer) in answers {
            // Do not overwrite a human answer/discard that arrived during the turn.
            sqlx::query("UPDATE product_questions SET status = 'answered', answer = ?, updated_at = ? WHERE id = ? AND story_id = ? AND status IN ('open', 'posted')")
                .bind(answer).bind(&now).bind(id).bind(story)
                .execute(&mut *tx).await.map_err(dberr("reconcile answer"))?;
        }
        for q in questions {
            if &q.story_id != story {
                return Err(otto_core::Error::Invalid(
                    "reconcile question belongs to another story".into(),
                ));
            }
            sqlx::query("INSERT INTO product_questions (id, story_id, analysis_id, text, rationale, category, status, answer, posted_ref, created_by, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, 'open', NULL, NULL, ?, ?, ?)")
                .bind(new_id()).bind(story).bind(&q.analysis_id).bind(&q.text).bind(&q.rationale)
                .bind(&q.category).bind(&q.created_by).bind(&now).bind(&now)
                .execute(&mut *tx).await.map_err(dberr("reconcile new question"))?;
        }
        for event in events {
            if &event.story_id != story {
                return Err(otto_core::Error::Invalid(
                    "reconcile event belongs to another story".into(),
                ));
            }
            sqlx::query("INSERT INTO product_events (id, story_id, section, kind, summary, actor_id, meta_json, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
                .bind(new_id()).bind(story).bind(&event.section).bind(&event.kind).bind(&event.summary)
                .bind(&event.actor_id).bind(&event.meta_json).bind(&now)
                .execute(&mut *tx).await.map_err(dberr("reconcile event"))?;
        }
        tx.commit()
            .await
            .map_err(dberr("commit watch reconciliation"))
    }
}
