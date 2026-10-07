use uuid::Uuid;

use crate::{error::AppError, state::AppState};

/// How many *different* people must report a piece of content before it hides itself.
/// Small enough that a bad name vanishes fast, big enough that one grudge can't hide
/// anything. No moderator is on call — the crowd is the moderator.
pub const REPORT_THRESHOLD: i64 = 3;

/// Which kind of thing is being reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Name,
    Review,
}

impl TargetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            TargetKind::Name => "name",
            TargetKind::Review => "review",
        }
    }
}

/// File a report. Idempotent per user — reporting twice is a no-op, so nobody can stack
/// their own weight. Once [REPORT_THRESHOLD] distinct reporters agree, the target hides.
pub async fn report(
    state: &AppState,
    reporter_id: Uuid,
    kind: TargetKind,
    target_id: Uuid,
    reason: Option<&str>,
) -> Result<bool, AppError> {
    // The target must exist; otherwise a report could hide nothing and just litter.
    let exists: Option<(Uuid,)> = match kind {
        TargetKind::Name => {
            sqlx::query_as("SELECT id FROM cat_names WHERE id = $1")
                .bind(target_id)
                .fetch_optional(&state.db)
                .await?
        }
        TargetKind::Review => {
            sqlx::query_as("SELECT id FROM cat_reviews WHERE id = $1")
                .bind(target_id)
                .fetch_optional(&state.db)
                .await?
        }
    };
    if exists.is_none() {
        return Err(AppError::NotFound);
    }

    sqlx::query(
        "INSERT INTO reports (reporter_id, target_kind, target_id, reason)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (reporter_id, target_kind, target_id) DO NOTHING",
    )
    .bind(reporter_id)
    .bind(kind.as_str())
    .bind(target_id)
    .bind(reason)
    .execute(&state.db)
    .await?;

    let (count,): (i64,) = sqlx::query_as(
        "SELECT count(*) FROM reports WHERE target_kind = $1 AND target_id = $2",
    )
    .bind(kind.as_str())
    .bind(target_id)
    .fetch_one(&state.db)
    .await?;

    if count >= REPORT_THRESHOLD {
        let sql = match kind {
            TargetKind::Name => "UPDATE cat_names SET hidden = true WHERE id = $1",
            TargetKind::Review => "UPDATE cat_reviews SET hidden = true WHERE id = $1",
        };
        sqlx::query(sql).bind(target_id).execute(&state.db).await?;
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_kind_labels_match_the_schema_check() {
        assert_eq!(TargetKind::Name.as_str(), "name");
        assert_eq!(TargetKind::Review.as_str(), "review");
    }
}
