use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::prelude::FromRow;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Serialize, Deserialize, FromRow, Debug, Clone, ToSchema)]
pub struct User {
  pub id: Uuid,
  pub username: String,
  pub role: String,
  pub email: Option<String>,
  #[serde(skip_serializing)]
  pub password: String,
  pub created_at: DateTime<Utc>,
  pub updated_at: DateTime<Utc>,
}
