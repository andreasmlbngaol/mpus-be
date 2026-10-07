//! Keyset pagination cursors. A cursor is just `"<created_at_rfc3339>|<id>"`,
//! hex-encoded so clients treat it as an opaque token. Ordering is always
//! `(created_at DESC, id DESC)`, so the next page asks for rows strictly "older"
//! than the cursor — no OFFSET scan, no drift when new rows land on page 1.

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::error::AppError;

pub const DEFAULT_LIMIT: i64 = 20;
pub const MAX_LIMIT: i64 = 100;

/// Clamp a client-supplied limit into something sane.
pub fn limit(raw: Option<i64>) -> i64 {
    raw.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

pub fn encode(created_at: DateTime<Utc>, id: Uuid) -> String {
    hex::encode(format!("{}|{}", created_at.to_rfc3339(), id))
}

/// Decode a cursor, or `None` if it's missing/malformed (a bad cursor just means
/// "start from the top" rather than an error the client can't act on).
pub fn decode(cursor: Option<&str>) -> Result<Option<(DateTime<Utc>, Uuid)>, AppError> {
    let Some(raw) = cursor else { return Ok(None) };
    let bytes = hex::decode(raw)
        .map_err(|_| AppError::bad_request("That page cursor is not valid."))?;
    let text = String::from_utf8(bytes)
        .map_err(|_| AppError::bad_request("That page cursor is not valid."))?;
    let (ts, id) = text
        .split_once('|')
        .ok_or_else(|| AppError::bad_request("That page cursor is not valid."))?;
    let ts = DateTime::parse_from_rfc3339(ts)
        .map_err(|_| AppError::bad_request("That page cursor is not valid."))?
        .with_timezone(&Utc);
    let id = Uuid::parse_str(id)
        .map_err(|_| AppError::bad_request("That page cursor is not valid."))?;
    Ok(Some((ts, id)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trips() {
        let ts = DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z")
            .unwrap()
            .with_timezone(&Utc);
        let id = Uuid::new_v4();
        let decoded = decode(Some(&encode(ts, id))).unwrap().unwrap();
        assert_eq!(decoded, (ts, id));
    }

    #[test]
    fn absent_cursor_is_none() {
        assert!(decode(None).unwrap().is_none());
    }

    #[test]
    fn garbage_cursor_is_rejected() {
        assert!(decode(Some("not-a-cursor")).is_err());
    }

    #[test]
    fn limit_is_clamped() {
        assert_eq!(limit(None), DEFAULT_LIMIT);
        assert_eq!(limit(Some(0)), 1);
        assert_eq!(limit(Some(9999)), MAX_LIMIT);
    }
}
