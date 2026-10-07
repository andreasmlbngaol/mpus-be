use axum::Json;
use serde::Serialize;
use serde_json::{Value, json};

/// `{ "success": true, "data": ... }`
pub fn ok<T: Serialize>(data: T) -> Json<Value> {
    Json(json!({ "success": true, "data": data }))
}

/// `{ "success": true, "message": "..." }`
pub fn message(msg: &str) -> Json<Value> {
    Json(json!({ "success": true, "message": msg }))
}
