use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

use crate::user::User;

#[derive(Serialize, Deserialize, FromRow, Debug, Clone, ToSchema)]
pub struct ProjectRow {
  pub id: Uuid,
  pub name: String,
  pub code: String,
  pub options: Value,
  pub sync_with_exchange: u32,
  pub owner_id: Uuid,
  pub created_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, Debug, Clone, ToSchema)]
pub struct Project {
  pub id: Uuid,
  pub name: String,
  pub code: String,
  pub options: Value,
  pub sync_with_exchange: u32,
  pub owner: User,
  pub created_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}
